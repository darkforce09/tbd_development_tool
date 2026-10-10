//! Git over time: branches with how far they are from their upstream, worktrees with their
//! changes, and the history of HEAD with each commit's co-authors (agents sign theirs).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::disk::StatusCounts;
use crate::exec::{Command, Runner};
use crate::hub::{JobContext, JobError, SourceEvent};

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
}

/// Most commits read for the history.
pub const HISTORY_LIMIT: usize = 5000;

const UNIT: char = '\u{1f}';
const RECORD: char = '\u{1e}';
const GROUP: char = '\u{1d}';

/// Reads branches, worktrees and history for the repository at `root`.
pub fn read_history(runner: &Runner, root: &Path) -> Result<GitHistory, JobError> {
    let git = |cwd: &Path, args: &[&str]| -> Result<String, JobError> {
        Ok(String::from_utf8_lossy(&runner.run(&Command::git(cwd, args.iter().copied())?)?.stdout).into_owned())
    };
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
        let changes = if path.is_dir() {
            runner
                .run(&Command::git(&path, ["status", "--porcelain=v2", "-z", "--untracked-files=normal"])?)
                .map(|o| crate::disk::parse_status_counts(&o.stdout))
                .unwrap_or_default()
        } else {
            StatusCounts::default()
        };
        worktrees.push(Worktree { path, head, branch, is_main: i == 0, changes });
    }

    let log_format = format!("--format=%H{UNIT}%h{UNIT}%an{UNIT}%ct{UNIT}%P{UNIT}%s{UNIT}%(trailers:key=Co-authored-by,valueonly,separator=%x1d){RECORD}");
    let limit = format!("-n{HISTORY_LIMIT}");
    let commits: Vec<Commit> = git(root, &["log", &log_format, &limit])?
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
    Ok(GitHistory { branches, worktrees, commits, commit_count, co_authors })
}

/// The job for a [`crate::SourceHub`].
pub fn history_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send {
    move |ctx| {
        let history = read_history(&ctx.runner, &ctx.root)?;
        ctx.send(SourceEvent::GitHistory(std::sync::Arc::new(history)));
        Ok(())
    }
}

/// Files a commit changed, with lines added and removed (`-` for binary files).
/// A changed file: its path, lines added and lines removed (`None` for binary files).
pub type FileChange = (String, Option<usize>, Option<usize>);

pub fn commit_files(runner: &Runner, root: &Path, commit: &str) -> Result<Vec<FileChange>, JobError> {
    if !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(JobError::Failed("not a commit id".to_string()));
    }
    let out = runner.run(&Command::git(root, ["show", "--numstat", "--format=", "-z", commit])?)?.stdout;
    let text = String::from_utf8_lossy(&out);
    Ok(text
        .split('\0')
        .filter(|f| !f.trim().is_empty())
        .filter_map(|f| {
            let mut parts = f.trim_start_matches('\n').splitn(3, '\t');
            let (added, removed, path) = (parts.next()?, parts.next()?, parts.next()?);
            Some((path.to_string(), added.parse().ok(), removed.parse().ok()))
        })
        .collect())
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
        assert_eq!(h.worktrees.len(), 2);
        assert!(h.worktrees[0].is_main && !h.worktrees[1].is_main);
        assert_eq!(h.worktrees[1].branch.as_deref(), Some("agent"));
        assert_eq!(h.worktrees[1].changes.untracked, 1);

        let files = commit_files(&runner, &repo, &h.commits[0].id).unwrap();
        assert_eq!(files, [("a.txt".to_string(), Some(1), Some(0))]);
        assert!(commit_files(&runner, &repo, "HEAD; rm -rf /").is_err(), "only commit ids");
    }
}
