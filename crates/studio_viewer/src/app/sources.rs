//! The project's sources besides its code (packages, git, tools, ...): started when a project
//! opens, read off the UI thread, and put on the map when they arrive. Results are kept here
//! and put on the graph again whenever the graph is replaced.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use studio_graph::Graph;
use studio_sources::{
    agents_job, commit_files_job, disk_job, history_job, packages_job, part_commits_job, settings_job, tickets_job,
    tools_job, AgentIndex, FileChange, GitDisk, GitHistory, Packages, PartCommits, Settings, SourceEvent, SourceHub,
    SourceKind, SourceState, TicketIndex, Tools, Worktree,
};

use super::title_bar::TitlePills;
use super::StudioApp;

/// One project's sources.
pub struct ProjectSources {
    hub: SourceHub,
    pub status: BTreeMap<SourceKind, SourceState>,
    pub packages: Option<Arc<Packages>>,
    pub tools: Option<Arc<Tools>>,
    pub disk: Option<Arc<GitDisk>>,
    /// Bytes on disk of top-level and ignored entries, relative to the root.
    pub sizes: BTreeMap<PathBuf, u64>,
    pub settings: Option<Arc<Settings>>,
    /// Whether the jobs that need the parsed project have been started.
    started: bool,
    /// Branches, worktrees and HEAD's history.
    pub history: Option<Arc<GitHistory>>,
    /// How long the history took to read (the git status is reused by later git jobs).
    pub history_elapsed: Option<Duration>,
    history_started: Option<Instant>,
    /// Coding agents' sessions in the project's worktrees.
    pub agents: Option<Arc<AgentIndex>>,
    /// Touches per kind (label), from main logs and from subagents' logs.
    pub agent_touches: BTreeMap<&'static str, (usize, usize)>,
    pub tickets: Option<Arc<TicketIndex>>,
    /// The last commit that touched each folder.
    pub part_commits: Option<Arc<PartCommits>>,
    /// How long the whole-history pass for `part_commits` took, from its start.
    pub part_commits_elapsed: Option<Duration>,
    part_commits_started: Option<Instant>,
    /// The files of each commit read so far, by full id.
    pub commit_files: BTreeMap<String, Arc<Vec<FileChange>>>,
    /// Commits whose files are being read.
    commit_files_pending: BTreeSet<String>,
    /// The jobs started once git has answered.
    follow_ups: FollowUps,
    /// Frames drawn since the first jobs started.
    frames_shown: u32,
    /// An input of the Changes district changed: build its view again.
    changes_dirty: bool,
    /// The last commit of each folder on the map, by cluster id: short id and unix time.
    part_folders: HashMap<String, (String, i64)>,
    /// The commit mark last put on each folder's subtitle (cluster id), to replace it.
    commit_marks: HashMap<String, String>,
    /// What the title bar's pills say.
    pub pills: TitlePills,
    /// For the start-up timings: whether the history job and the part-commits job have finished.
    timed_history: bool,
    timed_part_commits: bool,
}

impl ProjectSources {
    pub fn new(root: &Path, ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        Self {
            hub: SourceHub::new(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()), move || {
                ctx.request_repaint()
            }),
            status: BTreeMap::new(),
            packages: None,
            tools: None,
            disk: None,
            sizes: BTreeMap::new(),
            settings: None,
            started: false,
            history: None,
            history_elapsed: None,
            history_started: None,
            agents: None,
            agent_touches: BTreeMap::new(),
            tickets: None,
            part_commits: None,
            part_commits_elapsed: None,
            part_commits_started: None,
            commit_files: BTreeMap::new(),
            commit_files_pending: BTreeSet::new(),
            follow_ups: FollowUps::default(),
            frames_shown: 0,
            changes_dirty: true,
            part_folders: HashMap::new(),
            commit_marks: HashMap::new(),
            pills: TitlePills::default(),
            timed_history: false,
            timed_part_commits: false,
        }
    }

    /// The job a status change belongs to, for the start-up timings: git's jobs share one kind
    /// (history first, then part commits, then commit pages).
    fn timing_name(&mut self, source: SourceKind, state: &SourceState) -> &'static str {
        if source != SourceKind::Git {
            return source.label();
        }
        let done = !matches!(state, SourceState::Running);
        if !self.timed_history {
            self.timed_history = done;
            return "git history";
        }
        if self.part_commits_started.is_some() && !self.timed_part_commits {
            self.timed_part_commits = done;
            return "git part commits";
        }
        "git commit files"
    }

    /// The canonical project root.
    pub fn root(&self) -> &Path {
        self.hub.root()
    }

    /// What git has answered so far, as the follow-up jobs need it.
    fn git_known(&self) -> GitKnown {
        if self.history.is_some() {
            return GitKnown::History;
        }
        match self.status.get(&SourceKind::Git) {
            Some(SourceState::Unavailable(_) | SourceState::Failed(_)) => GitKnown::NoHistory,
            _ => GitKnown::Reading,
        }
    }

    /// The sources' states as the Changes district reads them: once the history is in, git is
    /// ready whatever the later git jobs (part commits, commit pages) report.
    fn changes_status(&self) -> BTreeMap<SourceKind, SourceState> {
        let mut status = self.status.clone();
        if self.history.is_some() {
            let elapsed = self.history_elapsed.unwrap_or_default();
            status.insert(SourceKind::Git, SourceState::Ready { elapsed });
        }
        status
    }

    /// Roots for the agents index: the project root, then the same folder in every other
    /// worktree of its repository.
    fn agent_roots(&self) -> Vec<PathBuf> {
        let root = self.root().to_path_buf();
        match &self.history {
            Some(history) => agent_roots(&root, &history.worktrees),
            None => vec![root],
        }
    }
}

/// The follow-up jobs: started once git has answered, the heavy ones only after the first
/// layout is on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FollowUps {
    pub tickets: bool,
    pub agents: bool,
    pub part_commits: bool,
}

/// What git has answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitKnown {
    /// The history job is still running.
    Reading,
    History,
    /// No repository, or git failed.
    NoHistory,
}

/// Which follow-up jobs to start now, given those already `started`. Tickets need only git's
/// answer (their commits are checked against it). The agents index (every core for up to a
/// second) and the whole-history pass for part commits wait until the first layout is on
/// screen, so they never delay it; part commits need a history.
pub(crate) fn follow_ups(git: GitKnown, layout_shown: bool, started: FollowUps) -> FollowUps {
    let answered = git != GitKnown::Reading;
    FollowUps {
        tickets: answered && !started.tickets,
        agents: answered && layout_shown && !started.agents,
        part_commits: git == GitKnown::History && layout_shown && !started.part_commits,
    }
}

/// The worktree the project lies in (the longest path that holds `root`) and the project's
/// folder inside it, `/`-separated (`""` when the project is the worktree itself).
pub(crate) fn project_worktree<'a>(root: &Path, worktrees: &'a [Worktree]) -> Option<(&'a Worktree, String)> {
    worktrees
        .iter()
        .filter_map(|w| root.strip_prefix(&w.path).ok().map(|rel| (w, rel)))
        .max_by_key(|(w, _)| w.path.components().count())
        .map(|(w, rel)| (w, slash_path(rel)))
}

/// `rel` with `/` between its parts.
fn slash_path(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// The project root first, then the project's folder in each other worktree, each once.
pub(crate) fn agent_roots(root: &Path, worktrees: &[Worktree]) -> Vec<PathBuf> {
    let mut roots = vec![root.to_path_buf()];
    let prefix = project_worktree(root, worktrees).map(|(_, prefix)| prefix).unwrap_or_default();
    for worktree in worktrees {
        let path = if prefix.is_empty() { worktree.path.clone() } else { worktree.path.join(&prefix) };
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots
}

/// Unix time now, in ms.
pub(crate) fn now_unix_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl StudioApp {
    /// Starts the sources once the project's files are known, and takes in what arrived.
    /// Called every frame; costs nothing when nothing arrived.
    pub(crate) fn poll_sources(&mut self) {
        let Some(sources) = &mut self.sources else { return };
        if sources.started {
            sources.frames_shown = sources.frames_shown.saturating_add(1);
        }
        if !sources.started && self.project_stats.is_some() && !self.is_loading {
            sources.started = true;
            let manifests = cargo_manifests(&self.graph);
            let json: Vec<PathBuf> = self
                .graph
                .nodes
                .values()
                .filter(|n| n.title.ends_with(".json"))
                .filter_map(|n| n.file_path.as_ref().map(PathBuf::from))
                .collect();
            sources.hub.spawn(SourceKind::Packages, packages_job(manifests.clone()));
            sources.hub.spawn(SourceKind::Disk, disk_job());
            sources.hub.spawn(SourceKind::Settings, settings_job(manifests, json));
            // A few quick git commands on their own thread: never in the way of the first frame.
            sources.history_started = Some(Instant::now());
            sources.hub.spawn(SourceKind::Git, history_job());
        }
        let mut changed = false;
        for event in sources.hub.drain() {
            match event {
                SourceEvent::Status { source, state } => {
                    if crate::timing::enabled() {
                        let name = sources.timing_name(source, &state);
                        crate::timing::event(|| crate::timing::job(name, &state));
                    }
                    let failed = matches!(state, SourceState::Failed(_) | SourceState::Unavailable(_));
                    if source == SourceKind::Git && failed {
                        // A commit page that could not be read may be asked for again.
                        sources.commit_files_pending.clear();
                    }
                    // Later git jobs report as git too; the district only follows the history.
                    let git_after_history = source == SourceKind::Git && sources.history.is_some();
                    if matches!(source, SourceKind::Git | SourceKind::Agents | SourceKind::Tickets)
                        && !git_after_history
                    {
                        sources.changes_dirty = true;
                        changed = true;
                    }
                    sources.status.insert(source, state);
                }
                SourceEvent::Packages(packages) => {
                    // Tools need the packages (their binaries), and the files on disk.
                    let files: Vec<PathBuf> =
                        self.graph.nodes.values().filter_map(|n| n.file_path.as_ref().map(PathBuf::from)).collect();
                    sources.hub.spawn(SourceKind::Tools, tools_job(packages.clone(), files));
                    sources.packages = Some(packages);
                    changed = true;
                }
                SourceEvent::Tools(tools) => {
                    sources.tools = Some(tools);
                    changed = true;
                }
                SourceEvent::GitDisk(disk) => {
                    sources.disk = Some(disk);
                    changed = true;
                }
                SourceEvent::FolderSize(path, bytes) => {
                    sources.sizes.insert(path, bytes);
                    changed = true;
                }
                SourceEvent::GitHistory(history) => {
                    sources.history_elapsed = sources.history_started.map(|t| t.elapsed());
                    sources.history = Some(history);
                    sources.changes_dirty = true;
                    changed = true;
                }
                SourceEvent::PartCommits(parts) => {
                    sources.part_commits_elapsed = sources.part_commits_started.map(|t| t.elapsed());
                    let worktrees = sources.history.as_ref().map_or(&[][..], |h| &h.worktrees[..]);
                    // A project outside every worktree (a path git spells differently) gets no marks.
                    sources.part_folders = project_worktree(sources.hub.root(), worktrees)
                        .map(|(_, prefix)| part_folders(&prefix, &parts))
                        .unwrap_or_default();
                    sources.part_commits = Some(parts);
                    changed = true;
                }
                SourceEvent::CommitFiles(id, files) => {
                    sources.commit_files_pending.remove(&id);
                    sources.commit_files.insert(id, files);
                    sources.changes_dirty = true;
                    changed = true;
                }
                SourceEvent::Tickets(tickets) => {
                    sources.tickets = Some(tickets);
                    sources.changes_dirty = true;
                    changed = true;
                }
                SourceEvent::Agents(agents) => {
                    sources.agent_touches = touches_by_kind(&agents);
                    sources.agents = Some(agents);
                    sources.changes_dirty = true;
                    changed = true;
                }
                SourceEvent::Settings(settings) => {
                    sources.settings = Some(settings);
                    changed = true;
                }
            }
        }
        // The first frame with the project's layout has been drawn once a frame has passed
        // since the first jobs started and the scene is built.
        let layout_shown = sources.frames_shown >= 1 && !self.canvas_state.scene_dirty;
        if layout_shown {
            crate::timing::first_layout(self.project_stats.as_ref().map_or(0, |s| s.file_count));
        }
        let start = follow_ups(sources.git_known(), layout_shown, sources.follow_ups);
        if start.tickets {
            sources.follow_ups.tickets = true;
            sources.hub.spawn(SourceKind::Tickets, tickets_job());
        }
        if start.agents {
            sources.follow_ups.agents = true;
            sources.hub.spawn(SourceKind::Agents, agents_job(sources.agent_roots()));
        }
        if start.part_commits {
            sources.follow_ups.part_commits = true;
            sources.part_commits_started = Some(Instant::now());
            sources.hub.spawn(SourceKind::Git, part_commits_job());
        }
        if sources.pills.valid_until_ms <= now_unix_ms() && sources.history.is_some() {
            // A new day: "today" moved.
            sources.changes_dirty = true;
            changed = true;
        }
        if changed {
            self.apply_sources();
        }
    }

    /// Puts what the sources read on the graph: run after they arrive and after the graph is
    /// replaced.
    pub(crate) fn apply_sources(&mut self) {
        let (Some(sources), Some(root)) = (&mut self.sources, &self.current_project_path) else { return };
        let files = super::districts::files_view(sources.disk.as_deref(), &sources.sizes, sources.settings.as_deref());
        self.canvas_state.districts.files = Some(Arc::new(files));
        if let Some(packages) = &sources.packages {
            mark_packages(&mut self.graph, root, packages);
            if let Some(tools) = &sources.tools {
                let view = super::districts::run_view(tools, packages, &self.graph);
                self.canvas_state.districts.run = Some(Arc::new(view));
            }
        }
        let now = now_unix_ms();
        if !sources.part_folders.is_empty() {
            mark_part_commits(&mut self.graph, &sources.part_folders, now, &mut sources.commit_marks);
        }
        if sources.changes_dirty {
            sources.changes_dirty = false;
            let view = super::districts::changes_view(
                sources.history.as_deref(),
                sources.agents.as_deref(),
                sources.tickets.as_deref(),
                &sources.commit_files,
                &sources.changes_status(),
                now,
            );
            self.canvas_state.districts.changes = Some(Arc::new(view));
            sources.pills = TitlePills::build(
                sources.hub.root(),
                sources.history.as_deref(),
                sources.agents.as_deref(),
                now,
                super::title_bar::local_offset_secs(now / 1000),
            );
        }
    }

    /// Reads one commit's files in the background, unless they are read or being read.
    pub(crate) fn load_commit_files(&mut self, id: String) {
        let Some(sources) = &mut self.sources else { return };
        if sources.commit_files.contains_key(&id) || sources.commit_files_pending.contains(&id) {
            return;
        }
        sources.commit_files_pending.insert(id.clone());
        sources.hub.spawn(SourceKind::Git, commit_files_job(id));
    }
}

/// Touches per kind, from main logs and from subagents' logs.
fn touches_by_kind(agents: &AgentIndex) -> BTreeMap<&'static str, (usize, usize)> {
    let mut counts = BTreeMap::new();
    for touch in &agents.touches {
        let entry: &mut (usize, usize) = counts.entry(touch.kind.label()).or_default();
        if touch.subagent {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    counts
}

/// Every `Cargo.toml` in the project, as found on disk.
fn cargo_manifests(graph: &Graph) -> Vec<PathBuf> {
    graph
        .nodes
        .values()
        .filter(|n| n.title == "Cargo.toml")
        .filter_map(|n| n.file_path.as_ref().map(PathBuf::from))
        .collect()
}

/// Marks the folder of each package with what it is and builds, e.g. "crate · 2 binaries".
fn mark_packages(graph: &mut Graph, root: &Path, packages: &Packages) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for package in &packages.packages {
        let folder = package.root.canonicalize().unwrap_or_else(|_| package.root.clone());
        let Ok(rel) = folder.strip_prefix(&root) else { continue };
        let id = studio_parser::folder_cluster_id(rel);
        let Some(cluster) = graph.clusters.iter_mut().find(|c| c.id == id) else { continue };
        let bins = package.targets.iter().filter(|t| t.kind == studio_sources::TargetKind::Bin).count();
        cluster.category = match bins {
            0 => format!("crate {}", package.name),
            1 => format!("crate {} · 1 binary", package.name),
            n => format!("crate {} · {n} binaries", package.name),
        };
    }
}

/// How long ago `then_ms` was at `now_ms`, short: "just now", "5 min ago", "3 h ago", "3 d ago",
/// "4 mo ago", "2 y ago". A time in the future reads "just now".
pub(crate) fn age_text(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    let (min, hour, day) = (60, 3_600, 86_400);
    match secs {
        s if s < min => "just now".to_string(),
        s if s < hour => format!("{} min ago", s / min),
        s if s < day => format!("{} h ago", s / hour),
        s if s < 30 * day => format!("{} d ago", s / day),
        s if s < 365 * day => format!("{} mo ago", s / (30 * day)),
        s => format!("{} y ago", s / (365 * day)),
    }
}

/// `subtitle` with the commit mark `old` (put there before) taken out and `new` put after the
/// folder's facts, before what it is for (" — …").
fn remark(subtitle: &str, old: Option<&str>, new: Option<&str>) -> String {
    let base = match old {
        Some(old) if !old.is_empty() => subtitle.replacen(old, "", 1),
        _ => subtitle.to_string(),
    };
    let Some(new) = new else { return base };
    match base.find(" — ") {
        Some(at) => format!("{}{new}{}", &base[..at], &base[at..]),
        None => format!("{base}{new}"),
    }
}

/// The last commit of each folder of the map, by cluster id: short id and unix time. Git's
/// folders are relative to the repository's top; `prefix` is the project's folder inside it
/// (`""` at the top), and folders outside it are not on the map.
fn part_folders(prefix: &str, parts: &PartCommits) -> HashMap<String, (String, i64)> {
    parts
        .by_folder
        .iter()
        .filter_map(|(folder, commit)| {
            let rel = if prefix.is_empty() {
                folder.as_str()
            } else if folder == prefix {
                ""
            } else {
                folder.strip_prefix(prefix)?.strip_prefix('/')?
            };
            Some((studio_parser::folder_cluster_id(Path::new(rel)), (commit.short.clone(), commit.time)))
        })
        .collect()
}

/// Puts the last commit that touched each folder on its subtitle, after its facts:
/// "12 files · 3 tests · 1a2b3c4 3 d ago". `marks` keeps the mark put on each folder, so
/// marking again replaces it.
fn mark_part_commits(
    graph: &mut Graph,
    folders: &HashMap<String, (String, i64)>,
    now_ms: i64,
    marks: &mut HashMap<String, String>,
) {
    for cluster in &mut graph.clusters {
        let Some(subtitle) = cluster.subtitle.as_deref().filter(|s| !s.is_empty()) else { continue };
        let new = folders.get(&cluster.id).map(|(short, time)| format!(" · {short} {}", age_text(now_ms, time * 1000)));
        let old = marks.get(&cluster.id).map(String::as_str);
        let marked = remark(subtitle, old.filter(|old| subtitle.contains(*old)), new.as_deref());
        cluster.subtitle = Some(marked);
        match new {
            Some(new) => marks.insert(cluster.id.clone(), new),
            None => marks.remove(&cluster.id),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_graph::GroupCluster;
    use studio_sources::{PartCommit, StatusCounts};

    #[test]
    fn age_reads_in_the_largest_whole_unit() {
        let now = 1_000_000_000_000;
        assert_eq!(age_text(now, now), "just now");
        assert_eq!(age_text(now, now + 5_000), "just now");
        assert_eq!(age_text(now, now - 59_000), "just now");
        assert_eq!(age_text(now, now - 5 * 60_000), "5 min ago");
        assert_eq!(age_text(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(age_text(now, now - 3 * 86_400_000 - 5_000), "3 d ago");
        assert_eq!(age_text(now, now - 120 * 86_400_000), "4 mo ago");
        assert_eq!(age_text(now, now - 800 * 86_400_000), "2 y ago");
    }

    #[test]
    fn follow_up_jobs_wait_for_git_and_heavy_ones_for_the_first_layout() {
        let none = FollowUps::default();
        assert_eq!(follow_ups(GitKnown::Reading, true, none), none);
        assert_eq!(
            follow_ups(GitKnown::History, false, none),
            FollowUps { tickets: true, agents: false, part_commits: false }
        );
        assert_eq!(
            follow_ups(GitKnown::History, true, none),
            FollowUps { tickets: true, agents: true, part_commits: true }
        );
        // No repository: sessions and tickets still, no part commits.
        assert_eq!(
            follow_ups(GitKnown::NoHistory, true, none),
            FollowUps { tickets: true, agents: true, part_commits: false }
        );
        // Each starts once.
        let all = FollowUps { tickets: true, agents: true, part_commits: true };
        assert_eq!(follow_ups(GitKnown::History, true, all), none);
        let tickets = FollowUps { tickets: true, ..none };
        assert_eq!(
            follow_ups(GitKnown::History, true, tickets),
            FollowUps { tickets: false, agents: true, part_commits: true }
        );
    }

    fn worktree(path: &str, is_main: bool) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: "0".repeat(40),
            branch: Some("main".into()),
            is_main,
            changes: StatusCounts::default(),
            files: Vec::new(),
        }
    }

    #[test]
    fn the_project_folder_is_found_in_the_longest_worktree() {
        let worktrees = [worktree("/r", true), worktree("/r/target/wt", false), worktree("/other", false)];
        let (w, prefix) = project_worktree(Path::new("/r"), &worktrees).unwrap();
        assert_eq!((w.path.as_path(), prefix.as_str()), (Path::new("/r"), ""));
        let (w, prefix) = project_worktree(Path::new("/r/target/wt/crates/a"), &worktrees).unwrap();
        assert_eq!((w.path.as_path(), prefix.as_str()), (Path::new("/r/target/wt"), "crates/a"));
        let (_, prefix) = project_worktree(Path::new("/r/app"), &worktrees).unwrap();
        assert_eq!(prefix, "app");
        assert!(project_worktree(Path::new("/elsewhere"), &worktrees).is_none());
    }

    #[test]
    fn agent_roots_are_the_project_then_its_folder_in_each_other_worktree() {
        let worktrees = [worktree("/r", true), worktree("/wt/s6", false), worktree("/wt/s7", false)];
        assert_eq!(
            agent_roots(Path::new("/r"), &worktrees),
            [PathBuf::from("/r"), PathBuf::from("/wt/s6"), PathBuf::from("/wt/s7")]
        );
        // Opened in a linked worktree: it comes first, the main checkout after.
        assert_eq!(
            agent_roots(Path::new("/wt/s6"), &worktrees),
            [PathBuf::from("/wt/s6"), PathBuf::from("/r"), PathBuf::from("/wt/s7")]
        );
        // A subfolder project: the same subfolder in each worktree.
        assert_eq!(
            agent_roots(Path::new("/r/app"), &worktrees),
            [PathBuf::from("/r/app"), PathBuf::from("/wt/s6/app"), PathBuf::from("/wt/s7/app")]
        );
        assert_eq!(agent_roots(Path::new("/x"), &[]), [PathBuf::from("/x")]);
    }

    fn cluster(rel: &str, subtitle: Option<&str>) -> GroupCluster {
        GroupCluster {
            id: studio_parser::folder_cluster_id(Path::new(rel)),
            label: rel.to_string(),
            category: "Directory".to_string(),
            subtitle: subtitle.map(str::to_string),
            parent_id: None,
            child_cluster_ids: Vec::new(),
            color_index: 0,
            position: [0.0, 0.0],
            size: [0.0, 0.0],
            node_ids: Vec::new(),
            detail: Default::default(),
            depth: 0,
            lazy: None,
            about: None,
        }
    }

    fn part(short: &str, time: i64) -> PartCommit {
        PartCommit { id: short.repeat(5), short: short.to_string(), time, subject: String::new() }
    }

    fn subtitles(graph: &Graph) -> Vec<Option<&str>> {
        graph.clusters.iter().map(|c| c.subtitle.as_deref()).collect()
    }

    #[test]
    fn part_commits_go_on_folder_subtitles_once_and_after_the_facts() {
        let day = 86_400;
        let now_ms = 100 * day * 1000;
        let mut graph = Graph::new();
        graph.clusters = vec![
            cluster("", Some("9 files")),
            cluster("src", Some("4 files · 2 tests — The core.")),
            cluster("docs", None),
            cluster("old", Some("1 file")),
        ];
        let parts = PartCommits {
            by_folder: [("", part("aaaaaaa", 97 * day)), ("src", part("bbbbbbb", 99 * day)), ("docs", part("c", 0))]
                .into_iter()
                .map(|(f, c)| (f.to_string(), c))
                .collect(),
            commits_scanned: 3,
        };
        let mut marks = HashMap::new();
        mark_part_commits(&mut graph, &part_folders("", &parts), now_ms, &mut marks);
        let expected = vec![
            Some("9 files · aaaaaaa 3 d ago"),
            Some("4 files · 2 tests · bbbbbbb 1 d ago — The core."),
            None,
            Some("1 file"),
        ];
        assert_eq!(subtitles(&graph), expected);
        // Marking again (another source arrived, a day later) replaces the marks.
        mark_part_commits(&mut graph, &part_folders("", &parts), now_ms + day * 1000, &mut marks);
        assert_eq!(subtitles(&graph)[0], Some("9 files · aaaaaaa 4 d ago"));
        assert_eq!(subtitles(&graph)[1], Some("4 files · 2 tests · bbbbbbb 2 d ago — The core."));
        // A replaced graph has fresh subtitles: marked once.
        graph.clusters[0].subtitle = Some("10 files".to_string());
        mark_part_commits(&mut graph, &part_folders("", &parts), now_ms, &mut marks);
        assert_eq!(subtitles(&graph)[0], Some("10 files · aaaaaaa 3 d ago"));
    }

    #[test]
    fn a_subfolder_project_reads_part_commits_below_its_own_folder() {
        let mut graph = Graph::new();
        graph.clusters = vec![cluster("", Some("2 files")), cluster("ui", Some("1 file"))];
        let parts = PartCommits {
            by_folder: [
                ("", part("top", 0)),
                ("app", part("app", 0)),
                ("app/ui", part("appui", 0)),
                ("ui", part("otherui", 0)),
                ("application", part("nope", 0)),
            ]
            .into_iter()
            .map(|(f, c)| (f.to_string(), c))
            .collect(),
            commits_scanned: 5,
        };
        let mut marks = HashMap::new();
        mark_part_commits(&mut graph, &part_folders("app", &parts), 0, &mut marks);
        assert_eq!(subtitles(&graph), [Some("2 files · app just now"), Some("1 file · appui just now")]);
    }

    #[test]
    fn remark_replaces_only_its_own_mark() {
        assert_eq!(remark("3 files", None, Some(" · a 1 d ago")), "3 files · a 1 d ago");
        assert_eq!(remark("3 files · a 1 d ago", Some(" · a 1 d ago"), Some(" · b just now")), "3 files · b just now");
        assert_eq!(remark("3 files · a 1 d ago — x", Some(" · a 1 d ago"), None), "3 files — x");
    }
}
