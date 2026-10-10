//! Git over time: branches with how far they are from their upstream, worktrees with their
//! changes, the history of HEAD with each commit's co-authors (agents sign theirs), the files
//! each commit changed, and the last commit that touched each folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::disk::StatusCounts;
use crate::exec::{Command, Runner};
use crate::hub::{JobContext, JobError, SourceEvent};
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub head: String,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    /// Unix time of its last commit.
    pub time: i64,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    /// The repository's own checkout, not a linked worktree.
    pub is_main: bool,
    pub changes: StatusCounts,
    /// Tracked files that differ from HEAD in this worktree (staged or not), by path. Untracked
    /// files are only counted, in `changes.untracked`.
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub short: String,
    pub author: String,
    pub time: i64,
    pub subject: String,
    pub parents: usize,
    /// `Co-authored-by:` trailers, as written.
    pub co_authors: Vec<String>,
}

/// What git says about the project over time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHistory {
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    /// HEAD's history, newest first, at most [`HISTORY_LIMIT`] commits.
    pub commits: Vec<Commit>,
    /// Every commit HEAD can reach.
    pub commit_count: usize,
    /// How many commits each co-author is on.
    pub co_authors: BTreeMap<String, usize>,
    /// Commits read with at least one `Co-authored-by:` trailer (the key in any case).
    pub co_authored_commits: usize,
}

/// A file a commit or a worktree changed. Counts are `None` for binary files.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct FileChange {
    /// Relative to the repository's top folder, `/`-separated.
    pub path: String,
    /// Where a renamed or copied file came from.
    pub old_path: Option<String>,
    pub added: Option<usize>,
    pub deleted: Option<usize>,
}

/// The last commit that touched a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartCommit {
    pub id: String,
    pub short: String,
    /// Unix commit time.
    pub time: i64,
    pub subject: String,
}

/// The last commit on HEAD that touched each folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PartCommits {
    /// By folder relative to the repository's top folder, `/`-separated; `""` is the top folder.
    pub by_folder: BTreeMap<String, PartCommit>,
    pub commits_scanned: usize,
}

/// Most commits read for the history.
pub const HISTORY_LIMIT: usize = 5000;
/// Format version of the cached file lists of commits.
pub const COMMIT_FILES_VERSION: u32 = 1;
/// How long the whole-history pass for [`read_part_commits`] may take.
pub const PART_COMMITS_TIMEOUT: Duration = Duration::from_secs(120);

const UNIT: char = '\u{1f}';
const RECORD: char = '\u{1e}';
const GROUP: char = '\u{1d}';

fn git(runner: &Runner, cwd: &Path, args: &[&str]) -> Result<String, JobError> {
    Ok(String::from_utf8_lossy(&runner.run(&Command::git(cwd, args.iter().copied())?)?.stdout).into_owned())
}

/// Reads branches, worktrees and history for the repository at `root`.
pub fn read_history(runner: &Runner, root: &Path) -> Result<GitHistory, JobError> {
    let git = |cwd: &Path, args: &[&str]| git(runner, cwd, args);
    if git(root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Err(JobError::Unavailable("No git repository here".to_string()));
    }
    let format = "--format=%(refname:short)%1f%(objectname)%1f%(upstream:short)%1f%(upstream:track)%1f%(committerdate:unix)%1f%(subject)";
    let branches = git(root, &["for-each-ref", format, "refs/heads"])?
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(UNIT).collect();
            let [name, head, upstream, track, time, subject] = f[..] else { return None };
            let number = |word: &str| {
                track
                    .split(word)
                    .nth(1)
                    .and_then(|r| r.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
                    .unwrap_or(0)
            };
            Some(Branch {
                name: name.to_string(),
                head: head.to_string(),
                upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                ahead: number("ahead"),
                behind: number("behind"),
                time: time.parse().unwrap_or(0),
                subject: subject.to_string(),
            })
        })
        .collect();

    let mut worktrees = Vec::new();
    let listing = git(root, &["worktree", "list", "--porcelain"])?;
    for (i, block) in listing.split("\n\n").filter(|b| !b.trim().is_empty()).enumerate() {
        let mut path = None;
        let mut head = String::new();
        let mut branch = None;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.trim_start_matches("refs/heads/").to_string());
            }
        }
        let Some(path) = path else { continue };
        let (changes, files) = if path.is_dir() {
            let changes = runner
                .run(&Command::git(&path, ["status", "--porcelain=v2", "-z", "--untracked-files=normal"])?)
                .map(|o| crate::disk::parse_status_counts(&o.stdout))
                .unwrap_or_default();
            // Fails only without a first commit, when nothing tracked can differ from HEAD.
            let files = runner
                .run(&Command::git(&path, ["diff", "--numstat", "-z", "-M", "--no-relative", "HEAD"])?)
                .map(|o| parse_numstat_z(&o.stdout))
                .unwrap_or_default();
            (changes, files)
        } else {
            (StatusCounts::default(), Vec::new())
        };
        worktrees.push(Worktree { path, head, branch, is_main: i == 0, changes, files });
    }

    let log_format = format!("--format=%H{UNIT}%h{UNIT}%an{UNIT}%ct{UNIT}%P{UNIT}%s{UNIT}%(trailers:key=Co-authored-by,valueonly,separator=%x1d){RECORD}");
    let limit = format!("-n{HISTORY_LIMIT}");
    let commits: Vec<Commit> = git(root, &["log", "--no-show-signature", &log_format, &limit])?
        .split(RECORD)
        .filter_map(|record| {
            let f: Vec<&str> = record.trim_start_matches('\n').split(UNIT).collect();
            let [id, short, author, time, parents, subject, trailers] = f[..] else { return None };
            Some(Commit {
                id: id.to_string(),
                short: short.to_string(),
                author: author.to_string(),
                time: time.parse().unwrap_or(0),
                subject: subject.to_string(),
                parents: parents.split_whitespace().count(),
                co_authors: trailers
                    .split(GROUP)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect();
    let commit_count =
        git(root, &["rev-list", "--count", "HEAD"]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(commits.len());
    let mut co_authors = BTreeMap::new();
    for c in &commits {
        for a in &c.co_authors {
            // The name, without the address.
            let name = a.split('<').next().unwrap_or(a).trim().to_string();
            *co_authors.entry(name).or_insert(0) += 1;
        }
    }
    let co_authored_commits = commits.iter().filter(|c| !c.co_authors.is_empty()).count();
    Ok(GitHistory { branches, worktrees, commits, commit_count, co_authors, co_authored_commits })
}

/// The job for a [`crate::SourceHub`].
pub fn history_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send + 'static {
    move |ctx| {
        let history = read_history(&ctx.runner, &ctx.root)?;
        ctx.send(SourceEvent::GitHistory(Arc::new(history)));
        Ok(())
    }
}

/// Reads `git … --numstat -z` output: `added\tdeleted\tpath\0` for a changed file,
/// `added\tdeleted\t\0old\0new\0` for a renamed or copied one, `-\t-` for binary counts. Sorted
/// by path.
pub fn parse_numstat_z(bytes: &[u8]) -> Vec<FileChange> {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0');
    let mut files = Vec::new();
    while let Some(field) = fields.next() {
        // `show` puts a newline between the (empty) commit header and the stats.
        let field = field.trim_start_matches('\n');
        let mut parts = field.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (path, old_path) = if path.is_empty() {
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else { break };
            (new.to_string(), Some(old.to_string()))
        } else {
            (path.to_string(), None)
        };
        files.push(FileChange { path, old_path, added: added.parse().ok(), deleted: deleted.parse().ok() });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.old_path.cmp(&b.old_path)));
    files
}

/// Files a commit changed, with lines added and removed, by path. Renames (`-M`) and copies
/// (`-C`, from files the same commit changed) keep where they came from. A merge lists what it
/// brought in: its diff against its first parent.
pub fn commit_files(runner: &Runner, root: &Path, commit: &str) -> Result<Vec<FileChange>, JobError> {
    if commit.is_empty() || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(JobError::Failed("not a commit id".to_string()));
    }
    let args = [
        "show",
        "--numstat",
        "-z",
        "-M",
        "-C",
        "--diff-merges=first-parent",
        "--no-show-signature",
        "--format=",
        commit,
    ];
    let out = runner.run(&Command::git(root, args)?)?.stdout;
    Ok(parse_numstat_z(&out))
}

/// A full commit id: 40 lowercase hex digits.
fn is_full_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// [`commit_files`] for a full commit id, kept in `store` as `commit-<id>`: a commit never
/// changes, so the second call runs no git at all.
pub fn commit_files_cached(
    runner: &Runner,
    root: &Path,
    store: &Store,
    id: &str,
) -> Result<Arc<Vec<FileChange>>, JobError> {
    if !is_full_id(id) {
        return Err(JobError::Failed("not a full commit id".to_string()));
    }
    let name = format!("commit-{id}");
    // The id's first 64 bits: a file renamed by hand never answers for another commit.
    let key = u64::from_str_radix(&id[..16], 16).unwrap_or(0);
    if let Some(files) = store.load::<Vec<FileChange>>(&name, COMMIT_FILES_VERSION, key) {
        return Ok(Arc::new(files));
    }
    let files = commit_files(runner, root, id)?;
    // A cache that cannot be written only costs another read next time.
    let _ = store.save(&name, COMMIT_FILES_VERSION, key, &files);
    Ok(Arc::new(files))
}

/// The job that reads one commit's files when they are asked for and sends
/// [`SourceEvent::CommitFiles`].
pub fn commit_files_job(id: String) -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send + 'static {
    move |ctx| {
        let files = commit_files_cached(&ctx.runner, &ctx.root, &Store::for_project(&ctx.root), &id)?;
        ctx.send(SourceEvent::CommitFiles(id, files));
        Ok(())
    }
}

/// The last commit on HEAD that touched each folder, in one pass over the whole history
/// (`log --name-only`, newest first): each path a commit touched gives that commit to every
/// folder above the path that has none yet.
///
/// Renames are unpaired on purpose (`--no-renames`): both the old and the new path count as
/// touched, which is what "the last commit touching this folder" means, and the pass stays
/// fast. Merges list no files here; the commits they bring in are on HEAD themselves.
pub fn read_part_commits(runner: &Runner, root: &Path) -> Result<PartCommits, JobError> {
    if git(runner, root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Err(JobError::Unavailable("No git repository here".to_string()));
    }
    if git(runner, root, &["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        return Ok(PartCommits::default()); // no commits yet
    }
    let format = format!("--format={RECORD}%H{UNIT}%h{UNIT}%ct{UNIT}%s");
    let args = ["log", "--name-only", "--no-renames", "-z", "--no-show-signature", format.as_str(), "HEAD"];
    let out = runner.run(&Command::git(root, args)?.with_timeout(PART_COMMITS_TIMEOUT))?.stdout;
    Ok(parse_part_commits(&String::from_utf8_lossy(&out)))
}

/// Reads `log --name-only -z` output: each commit is `RECORD id UNIT short UNIT time UNIT
/// subject \0`, then `\n` and its paths, each ending in `\0`.
fn parse_part_commits(text: &str) -> PartCommits {
    let mut parts = PartCommits::default();
    let mut current: Option<PartCommit> = None;
    for field in text.split('\0') {
        let field = field.strip_prefix('\n').unwrap_or(field);
        if let Some(header) = field.strip_prefix(RECORD) {
            let mut f = header.splitn(4, UNIT);
            current = match (f.next(), f.next(), f.next(), f.next()) {
                (Some(id), Some(short), Some(time), Some(subject)) => {
                    parts.commits_scanned += 1;
                    Some(PartCommit {
                        id: id.to_string(),
                        short: short.to_string(),
                        time: time.parse().unwrap_or(0),
                        subject: subject.to_string(),
                    })
                }
                _ => None,
            };
            continue;
        }
        let Some(commit) = &current else { continue };
        if field.is_empty() {
            continue;
        }
        // From the file's own folder up; a folder already given a commit has all of its
        // parents given one too.
        let mut folder = field;
        loop {
            folder = folder.rfind('/').map_or("", |i| &folder[..i]);
            if parts.by_folder.contains_key(folder) {
                break;
            }
            parts.by_folder.insert(folder.to_string(), commit.clone());
            if folder.is_empty() {
                break;
            }
        }
    }
    parts
}

/// The job for [`read_part_commits`]; sends [`SourceEvent::PartCommits`].
pub fn part_commits_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send + 'static {
    move |ctx| {
        let parts = read_part_commits(&ctx.runner, &ctx.root)?;
        ctx.send(SourceEvent::PartCommits(Arc::new(parts)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CancelToken;

    #[test]
    fn history_reads_branches_worktrees_commits_and_co_authors() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str], cwd: &Path| {
            std::process::Command::new("git")
                .args(["-c", "user.name=Sam", "-c", "user.email=s@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(cwd)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q", "-b", "main"], &repo) {
            return; // no git here
        }
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        assert!(git(&["add", "."], &repo) && git(&["commit", "-qm", "first"], &repo));
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(git(&["commit", "-qam", "second\n\nCo-Authored-By: Claude <noreply@anthropic.com>"], &repo));
        let wt = dir.path().join("wt");
        assert!(git(&["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()], &repo));
        std::fs::write(wt.join("b.txt"), "new\n").unwrap();

        let runner = Runner::new(CancelToken::default());
        let h = read_history(&runner, &repo).unwrap();
        assert_eq!(h.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["agent", "main"]);
        assert_eq!(h.commit_count, 2);
        assert_eq!(h.commits[0].subject, "second");
        assert_eq!(h.commits[0].co_authors, ["Claude <noreply@anthropic.com>"]);
        assert_eq!(h.co_authors.get("Claude"), Some(&1));
        assert_eq!(h.co_authored_commits, 1);
        assert_eq!(h.worktrees.len(), 2);
        assert!(h.worktrees[0].is_main && !h.worktrees[1].is_main);
        assert_eq!(h.worktrees[1].branch.as_deref(), Some("agent"));
        assert_eq!(h.worktrees[1].changes.untracked, 1);
        assert!(h.worktrees[1].files.is_empty(), "untracked files are only counted");

        let files = commit_files(&runner, &repo, &h.commits[0].id).unwrap();
        let a = FileChange { path: "a.txt".into(), old_path: None, added: Some(1), deleted: Some(0) };
        assert_eq!(files, [a]);
        assert!(commit_files(&runner, &repo, "HEAD; rm -rf /").is_err(), "only commit ids");
    }

    #[test]
    fn numstat_reads_renames_copies_and_binary_files() {
        let out = b"\n0\t0\t\0a.txt\0b.txt\0-\t-\tbin.dat\x003\t1\tsrc/x y.rs\0";
        let change = |path: &str, old: Option<&str>, a, d| FileChange {
            path: path.into(),
            old_path: old.map(Into::into),
            added: a,
            deleted: d,
        };
        assert_eq!(
            parse_numstat_z(out),
            [
                change("b.txt", Some("a.txt"), Some(0), Some(0)),
                change("bin.dat", None, None, None),
                change("src/x y.rs", None, Some(3), Some(1)),
            ]
        );
        assert!(parse_numstat_z(b"").is_empty());
    }

    #[test]
    fn each_folder_gets_the_newest_commit_that_touched_it() {
        let text = "\u{1e}c2\u{1f}c\u{1f}20\u{1f}two\0\na/b/x.rs\0\u{1e}m\u{1f}m\u{1f}15\u{1f}merge\0\
                    \u{1e}c1\u{1f}c\u{1f}10\u{1f}one\0\na/y.rs\0top.rs\0a/b/z.rs\0";
        let parts = parse_part_commits(text);
        assert_eq!(parts.commits_scanned, 3);
        let ids: Vec<(&str, &str)> = parts.by_folder.iter().map(|(f, c)| (f.as_str(), c.id.as_str())).collect();
        assert_eq!(ids, [("", "c2"), ("a", "c2"), ("a/b", "c2")]);
    }
}
