//! One indexing run: find the logs, reuse what the cache knows, read only what is new, and put
//! sessions, touches, plans and tasks together from the numbers (cheap, every run).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use rayon::prelude::*;

use super::cache::{self, use_kind, AgentCache, CachedFile, FileFacts};
use super::discover::{self, Found, Resolver};
use super::parse::{first_cwd, later_cwd, FileParser};
use super::{
    AgentIndex, FolderStat, HunkRange, IndexOptions, IndexStats, LogFile, PlanRef, Session, TaskRef, TaskState, Touch,
    TouchKind, NO_SESSIONS,
};
use crate::exec::CancelToken;
use crate::hub::JobError;
use crate::store::Store;

/// What a file needs this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    /// Same length and time as cached.
    Same,
    /// Same start, maybe more lines: read on from where the cache stopped.
    Grown,
    /// Not cached.
    New,
    /// Cached, but shrunk or replaced: read again from the start.
    Replaced,
}

struct Work {
    found: Found,
    record: CachedFile,
    change: Change,
    /// The file belongs to the project.
    mapped: bool,
    /// The file's facts were read (partly or wholly) this run.
    read: bool,
}

pub(crate) fn run(
    claude_dir: &Path,
    roots: &[PathBuf],
    store: Option<&Store>,
    cancel: Option<&CancelToken>,
    options: &IndexOptions,
) -> Result<AgentIndex, JobError> {
    let started = Instant::now();
    let cancelled = || cancel.is_some_and(CancelToken::is_cancelled);
    let mut canonical_roots: Vec<PathBuf> = Vec::new();
    for root in roots {
        let root = discover::canonical(root);
        if !canonical_roots.contains(&root) {
            canonical_roots.push(root);
        }
    }
    let projects = claude_dir.join("projects");
    if canonical_roots.is_empty() || !projects.is_dir() {
        return Err(JobError::Unavailable(NO_SESSIONS.to_string()));
    }
    let (folders, found) = discover::discover(&projects).map_err(|e| JobError::Failed(e.to_string()))?;
    let key = cache::key(claude_dir, &canonical_roots, options.map_parent_cwds);
    let mut cached: HashMap<String, CachedFile> = store
        .and_then(|s| s.load::<AgentCache>(cache::NAME, cache::VERSION, key))
        .map(|c| c.files.into_iter().map(|f| (f.path.clone(), f)).collect())
        .unwrap_or_default();
    let cached_count = cached.len();
    let resolver = Resolver::new(canonical_roots.clone(), options.map_parent_cwds);
    let main_ids: HashSet<String> = found.iter().filter(|f| !f.subagent).map(|f| f.session_id.clone()).collect();

    // What changed, and the first cwd of every file that decides by its own.
    let previous: Vec<Option<CachedFile>> =
        found.iter().map(|f| cached.remove(f.path.to_string_lossy().as_ref())).collect();
    let removed = !cached.is_empty();
    let mut work: Vec<Work> = found
        .into_par_iter()
        .zip(previous)
        .map(|(found, previous)| {
            let (mut record, change) = classify(&found, previous);
            if decides(&found, &main_ids) && record.first_cwd.is_none() && record.scanned_to < found.len && !cancelled()
            {
                if let Ok((cwd, end)) = first_cwd(&found.path, record.scanned_to) {
                    record.first_cwd = cwd;
                    record.scanned_to = end;
                }
            }
            Work { found, record, change, mapped: false, read: false }
        })
        .collect();
    if cancelled() {
        return Err(JobError::Cancelled);
    }

    // A log whose first cwd is outside the roots may move in later: its bytes are scanned for
    // `"cwd":"<a spelling of a root>`, and the first such line inside a root decides.
    let needles = spellings(&work, &resolver, roots, std::env::home_dir().as_deref());
    let needle_key = cache::hash(&needles.join(&0u8));
    let later_bytes = AtomicU64::new(0);
    let rescanned = AtomicBool::new(false);
    work.par_iter_mut().filter(|w| decides(&w.found, &main_ids)).for_each(|w| {
        let r = &mut w.record;
        let outside = r.first_cwd.as_deref().is_some_and(|c| resolver.root_of_cwd(c).is_none());
        if !outside || r.later_cwd.is_some() || cancelled() {
            return;
        }
        if r.later_key != needle_key {
            (r.later_key, r.later_scanned_to) = (needle_key, 0);
            rescanned.store(true, Ordering::Relaxed);
        }
        let start = r.later_scanned_to.max(r.scanned_to);
        if start >= w.found.len {
            return;
        }
        if let Ok((cwd, end)) = later_cwd(&w.found.path, start, &needles, &resolver, &cancelled) {
            (r.later_cwd, r.later_scanned_to) = (cwd, end);
            later_bytes.fetch_add(end.saturating_sub(start), Ordering::Relaxed);
            rescanned.store(true, Ordering::Relaxed);
        }
    });
    if cancelled() {
        return Err(JobError::Cancelled);
    }

    // Main logs map by the first cwd inside a root; a subagent's log follows its session's
    // main log.
    let mut mapped_ids: HashSet<String> = HashSet::new();
    for w in work.iter_mut().filter(|w| !w.found.subagent) {
        w.mapped = entry_cwd(&w.record, &resolver).is_some();
        if w.mapped {
            mapped_ids.insert(w.found.session_id.clone());
        }
    }
    for w in work.iter_mut().filter(|w| w.found.subagent) {
        w.mapped = if main_ids.contains(&w.found.session_id) {
            mapped_ids.contains(&w.found.session_id)
        } else {
            entry_cwd(&w.record, &resolver).is_some()
        };
    }

    // Read what is new in the project's files.
    let failed = std::sync::atomic::AtomicBool::new(false);
    work.par_iter_mut().filter(|w| w.mapped).for_each(|w| {
        if cancelled() {
            return;
        }
        let resume = matches!(w.change, Change::Same | Change::Grown) && w.record.facts.is_some();
        if resume && (w.change == Change::Same || w.record.facts.as_ref().is_some_and(|f| f.parsed_to >= w.found.len)) {
            return;
        }
        let facts = if resume { w.record.facts.take().unwrap_or_default() } else { FileFacts::default() };
        let mut parser = FileParser::new(facts, &resolver, w.found.subagent);
        match parser.read(&w.found.path, &cancelled) {
            Ok(()) => {
                w.record.facts = Some(parser.facts);
                w.read = true;
                if !resume && w.change != Change::Replaced {
                    w.change = Change::New;
                }
            }
            Err(_) if cancelled() => {}
            Err(_) => {
                // Unreadable now (removed mid-run): left out of this run and the cache.
                w.mapped = false;
                w.record.facts = None;
                failed.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    if cancelled() {
        return Err(JobError::Cancelled);
    }
    for w in work.iter_mut().filter(|w| !w.mapped) {
        w.record.facts = None;
    }

    let mut stats = IndexStats {
        scanned_logs: work.len(),
        later_cwd_bytes: later_bytes.load(Ordering::Relaxed),
        ..Default::default()
    };
    let mut later_sessions: HashSet<&str> = HashSet::new();
    for w in work.iter().filter(|w| w.mapped && decides(&w.found, &main_ids)) {
        if w.record.first_cwd.as_deref().and_then(|c| resolver.root_of_cwd(c)).is_none() {
            stats.later_cwd_logs += 1;
            later_sessions.insert(w.found.session_id.as_str());
        }
    }
    stats.later_cwd_sessions = later_sessions.len();
    for w in work.iter().filter(|w| w.read) {
        match w.change {
            Change::Grown | Change::Same => stats.resumed_files += 1,
            Change::Replaced => stats.reparsed_files += 1,
            Change::New => stats.new_files += 1,
        }
    }

    if let Some(store) = store {
        let changed = removed
            || work.len() != cached_count
            || work.iter().any(|w| w.read || w.change != Change::Same)
            || failed.load(std::sync::atomic::Ordering::Relaxed)
            || rescanned.load(Ordering::Relaxed);
        if changed {
            let files = work.iter().map(|w| w.record.clone()).collect();
            // A cache that cannot be written only costs time on the next run.
            let _ = store.save(cache::NAME, cache::VERSION, key, &AgentCache { files });
        }
    }

    let mut index = assemble(claude_dir, canonical_roots, &folders, &work, &resolver, stats);
    if index.sessions.is_empty() {
        return Err(JobError::Unavailable(NO_SESSIONS.to_string()));
    }
    index.stats.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(index)
}

/// What the cache knows of `found`, and what changed since.
fn classify(found: &Found, previous: Option<CachedFile>) -> (CachedFile, Change) {
    let fresh = |change| {
        let head_len = found.len.min(cache::HEAD_BYTES);
        let head = cache::head_hash(&found.path, head_len).unwrap_or(0);
        let record = CachedFile {
            path: found.path.to_string_lossy().into_owned(),
            len: found.len,
            mtime: found.mtime,
            head_len,
            head,
            ..Default::default()
        };
        (record, change)
    };
    let Some(mut previous) = previous else { return fresh(Change::New) };
    if previous.len == found.len && previous.mtime == found.mtime {
        return (previous, Change::Same);
    }
    let read_to = previous.facts.as_ref().map_or(0, |f| f.parsed_to).max(previous.scanned_to).max(previous.head_len);
    let read_to = read_to.max(previous.later_scanned_to);
    if found.len >= read_to && cache::head_hash(&found.path, previous.head_len).ok() == Some(previous.head) {
        previous.len = found.len;
        previous.mtime = found.mtime;
        return (previous, Change::Grown);
    }
    fresh(Change::Replaced)
}

/// Sessions, touches, plans and tasks from every mapped file's numbers.
fn assemble(
    claude_dir: &Path,
    roots: Vec<PathBuf>,
    folders: &[PathBuf],
    work: &[Work],
    resolver: &Resolver,
    mut stats: IndexStats,
) -> AgentIndex {
    // Folders, mapped or not.
    let mut folder_stats: Vec<FolderStat> =
        folders.iter().map(|f| FolderStat { folder: f.clone(), mapped: false, logs: 0, bytes: 0 }).collect();
    for w in work {
        let f = &mut folder_stats[w.found.folder];
        f.logs += 1;
        f.bytes += w.found.len;
        f.mapped |= w.mapped;
    }

    // The project's logs, oldest first; a copied line belongs to the first log that has it.
    let mut mapped: Vec<(&Work, &FileFacts)> =
        work.iter().filter(|w| w.mapped).filter_map(|w| Some((w, w.record.facts.as_ref()?))).collect();
    mapped.sort_by(|a, b| a.1.started.cmp(&b.1.started).then_with(|| a.0.found.path.cmp(&b.0.found.path)));

    // Sessions, in the order of their first log.
    let mut session_of: HashMap<&str, usize> = HashMap::new();
    let mut sessions: Vec<Session> = Vec::new();
    let mut logs: Vec<LogFile> = Vec::with_capacity(mapped.len());
    for (log, (w, facts)) in mapped.iter().enumerate() {
        let s = *session_of.entry(w.found.session_id.as_str()).or_insert_with(|| {
            sessions.push(Session {
                id: w.found.session_id.clone(),
                slug: None,
                root: 0,
                branches: Vec::new(),
                started: i64::MAX,
                ended: i64::MIN,
                logs: Vec::new(),
                plan: None,
                tasks: Vec::new(),
                touch_count: 0,
            });
            sessions.len() - 1
        });
        logs.push(LogFile { path: w.found.path.clone(), subagent: w.found.subagent, session: Some(s) });
        let session = &mut sessions[s];
        session.logs.push(log);
        session.started = session.started.min(facts.started);
        session.ended = session.ended.max(facts.ended);
        for b in &facts.branches {
            if !session.branches.contains(b) {
                session.branches.push(b.clone());
            }
        }
        if facts.slug.is_some() && (!w.found.subagent || session.slug.is_none()) {
            session.slug = facts.slug.clone();
        }
        stats.lines += facts.lines;
        stats.parsed_lines += facts.parsed_lines;
        stats.bad_lines += facts.bad_lines;
        for t in &facts.unknown_types {
            *stats.unknown_types.entry(t.name.clone()).or_insert(0) += t.count;
        }
        if w.found.subagent {
            stats.subagent_logs += 1;
            stats.subagent_bytes += w.found.len;
        } else {
            stats.main_logs += 1;
            stats.main_bytes += w.found.len;
        }
    }
    stats.bytes = stats.main_bytes + stats.subagent_bytes;
    for session in &mut sessions {
        if session.started == i64::MAX {
            (session.started, session.ended) = (0, 0);
        }
        // The root of the first main log that maps, else of any log.
        let by_main = session.logs.iter().map(|&l| mapped[l]).filter(|(w, _)| !w.found.subagent);
        let by_any = session.logs.iter().map(|&l| mapped[l]);
        session.root = by_main
            .chain(by_any)
            .find_map(|(w, _)| entry_cwd(&w.record, resolver).and_then(|c| resolver.root_of_cwd(c)))
            .unwrap_or(0);
    }

    // Lines and calls copied by a resume or a fork count once.
    let total: usize = mapped.iter().map(|(_, f)| f.uuids.len()).sum();
    let mut seen_lines: HashSet<u64> = HashSet::with_capacity(total);
    let mut seen_uses: HashSet<u64> = HashSet::new();
    let mut touches: Vec<Touch> = Vec::new();
    let mut updates: Vec<Vec<(i64, String, TaskState)>> = vec![Vec::new(); sessions.len()];
    let mut outside: HashMap<&str, u64> = HashMap::new();
    for (log, (w, facts)) in mapped.iter().enumerate() {
        for u in &facts.uuids {
            if !seen_lines.insert(*u) {
                stats.duplicate_lines += 1;
            }
        }
        let s = session_of[w.found.session_id.as_str()];
        for u in &facts.uses {
            if !seen_uses.insert(u.id) {
                continue;
            }
            let kind = match u.kind {
                use_kind::READ => TouchKind::Read,
                use_kind::EDIT => TouchKind::Edit,
                use_kind::WRITE => TouchKind::Write,
                use_kind::BASH_EDIT => TouchKind::BashEdit,
                use_kind::PLAN => {
                    let session = &mut sessions[s];
                    if session.plan.as_ref().is_none_or(|p| p.time <= u.time) {
                        session.plan = Some(PlanRef {
                            path: u.path.as_ref().map(PathBuf::from),
                            log,
                            offset: u.offset,
                            len: u.len,
                            time: u.time,
                        });
                    }
                    continue;
                }
                use_kind::TASK_CREATE => {
                    if let (Some(id), false) = (&u.task_id, u.error) {
                        sessions[s].tasks.push(TaskRef {
                            id: id.clone(),
                            log,
                            offset: u.offset,
                            len: u.len,
                            created: u.time,
                            states: Vec::new(),
                        });
                    }
                    continue;
                }
                use_kind::TASK_UPDATE => {
                    if let (Some(id), Some(state), false) = (&u.task_id, TaskState::from_code(u.status), u.error) {
                        updates[s].push((u.time, id.clone(), state));
                    }
                    continue;
                }
                _ => continue,
            };
            if u.error {
                continue;
            }
            let Some(path) = &u.path else {
                if u.out_of_repo {
                    stats.out_of_repo_touches += 1;
                    if let Some(folder) = &u.outside {
                        *outside.entry(folder.as_str()).or_insert(0) += 1;
                    }
                }
                continue;
            };
            touches.push(Touch {
                session: s,
                path: PathBuf::from(path),
                kind,
                time: u.time,
                log,
                offset: u.offset,
                len: u.len,
                result: u.result.map(|(offset, len)| (log, offset, len)),
                read: u.read,
                hunks: u
                    .hunks
                    .iter()
                    .map(|h| HunkRange {
                        old_start: h.old_start,
                        old_lines: h.old_lines,
                        new_start: h.new_start,
                        new_lines: h.new_lines,
                    })
                    .collect(),
                subagent: w.found.subagent,
                task: None,
            });
        }
    }

    // Out-of-repo files: whether their folder is still there, and where they were.
    let base = discover::common_ancestor(roots.iter().map(|r| r.parent().unwrap_or(r)));
    let mut top: BTreeMap<String, u64> = BTreeMap::new();
    for (folder, n) in outside {
        let folder = Path::new(folder);
        if folder.is_dir() {
            stats.out_of_repo_existing += n;
        } else {
            stats.out_of_repo_gone += n;
        }
        *top.entry(top_folder(folder, &base).to_string_lossy().into_owned()).or_insert(0) += n;
    }
    let mut top: Vec<(String, u64)> = top.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top.truncate(5);
    stats.out_of_repo_top = top;

    // Tasks and their states; plans named by the session's slug.
    let plans = claude_dir.join("plans");
    for (s, session) in sessions.iter_mut().enumerate() {
        session.tasks.sort_by_key(|t| (t.created, t.log, t.offset));
        updates[s].sort_by_key(|u| u.0);
        for (time, id, state) in std::mem::take(&mut updates[s]) {
            // The latest task with that id created by then (ids restart only in a new list).
            let task = session.tasks.iter_mut().rev().find(|t| t.id == id && t.created <= time);
            if let Some(task) = task {
                task.states.push((time, state));
            }
        }
        let slug_plan = session.slug.as_ref().map(|slug| plans.join(format!("{slug}.md"))).filter(|p| p.is_file());
        match &mut session.plan {
            Some(plan) if plan.path.is_none() => plan.path = slug_plan,
            Some(_) => {}
            None => {
                if let Some(path) = slug_plan {
                    let time =
                        std::fs::metadata(&path).ok().map(|m| cache::mtime_ns(&m) / 1_000_000).unwrap_or(session.ended);
                    session.plan = Some(PlanRef { path: Some(path), log: 0, offset: 0, len: 0, time });
                }
            }
        }
    }

    // Each touch joins the task in progress when it was made.
    for t in &mut touches {
        t.task = task_at(&sessions[t.session].tasks, t.time);
        sessions[t.session].touch_count += 1;
    }

    // Sessions oldest first; logs keep their order and are renumbered in sessions.
    let mut order: Vec<usize> = (0..sessions.len()).collect();
    order.sort_by(|&a, &b| (sessions[a].started, &sessions[a].id).cmp(&(sessions[b].started, &sessions[b].id)));
    let mut new_index = vec![0usize; sessions.len()];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    let mut slots: Vec<Option<Session>> = sessions.into_iter().map(Some).collect();
    let sessions: Vec<Session> = order.iter().filter_map(|&old| slots[old].take()).collect();
    for l in &mut logs {
        l.session = l.session.map(|s| new_index[s]);
    }
    for t in &mut touches {
        t.session = new_index[t.session];
    }
    touches.sort_by(|a, b| {
        (a.session, a.time, a.log, a.offset, &a.path).cmp(&(b.session, b.time, b.log, b.offset, &b.path))
    });

    AgentIndex { roots, logs, sessions, touches, folders: folder_stats, stats }
}

/// Main logs, and subagent logs without one, decide by their own lines whether they belong.
fn decides(found: &Found, main_ids: &HashSet<String>) -> bool {
    !found.subagent || !main_ids.contains(&found.session_id)
}

/// The cwd that puts a log in the project: its first, else the first later one inside a root.
fn entry_cwd<'a>(record: &'a CachedFile, resolver: &Resolver) -> Option<&'a str> {
    let first = record.first_cwd.as_deref().filter(|c| resolver.root_of_cwd(c).is_some());
    first.or(record.later_cwd.as_deref())
}

/// The `"cwd":"<prefix>` needles for every spelling of the roots: the canonical roots, the
/// roots as given, `$HOME` symlinks onto them, and every first cwd of any log that is a
/// different spelling of a path inside a root. Sorted; one that another one starts is dropped.
fn spellings(work: &[Work], resolver: &Resolver, given: &[PathBuf], home: Option<&Path>) -> Vec<Vec<u8>> {
    let mut prefixes: BTreeSet<PathBuf> = resolver.roots.iter().cloned().collect();
    prefixes.extend(given.iter().filter(|r| r.is_absolute()).map(|r| discover::normalise(r)));
    if let Some(home) = home {
        prefixes.extend(discover::home_aliases(home, &resolver.roots));
    }
    let firsts: BTreeSet<&str> = work.iter().filter_map(|w| w.record.first_cwd.as_deref()).collect();
    for cwd in firsts {
        let canonical = resolver.cwd(cwd);
        if let Some(root) = resolver.root_containing(&canonical) {
            prefixes.extend(discover::alias_of(cwd, &canonical, &resolver.roots[root]));
        }
    }
    let needles: Vec<Vec<u8>> = prefixes.iter().map(|p| discover::cwd_needle(p)).collect();
    let mut kept: Vec<Vec<u8>> = Vec::new();
    for n in needles {
        // Sorted, so a needle that starts this one came before it.
        if !kept.iter().any(|k| n.starts_with(k)) {
            kept.push(n);
        }
    }
    kept
}

/// The folder an out-of-repo folder is counted under: its first component below `base` (the
/// deepest folder holding the roots' parents), else below the deepest folder it shares with
/// `base` (two components when that is the filesystem root).
fn top_folder(folder: &Path, base: &Path) -> PathBuf {
    let common = discover::common_ancestor([folder, base]);
    let depth = common.components().count();
    let keep = if folder.starts_with(base) || depth > 1 { depth + 1 } else { depth + 2 };
    folder.components().take(keep).collect()
}

/// The task whose latest state at `time` is in progress, set by the latest such update.
fn task_at(tasks: &[TaskRef], time: i64) -> Option<usize> {
    tasks
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let (at, state) = t.states.iter().rev().find(|(at, _)| *at <= time)?;
            (*state == TaskState::InProgress).then_some((*at, i))
        })
        .max()
        .map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_repo_folders_are_cut_below_the_roots_parent() {
        let base = Path::new("/disk/Projects");
        let top = |f: &str| top_folder(Path::new(f), base);
        assert_eq!(top("/disk/Projects/repo-s4/src/deep"), Path::new("/disk/Projects/repo-s4"));
        assert_eq!(top("/disk/Projects"), Path::new("/disk/Projects"));
        assert_eq!(top("/disk/Other/x"), Path::new("/disk/Other"));
        assert_eq!(top("/home/u/.claude/plans"), Path::new("/home/u"));
    }
}
