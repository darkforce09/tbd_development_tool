//! The disk as git and the file system see it: which files git tracks, which entries it ignores
//! and by which rule, which files live in Git LFS, what changed, and how much space each
//! top-level entry takes, measured the way `du` measures it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::exec::{Command, Runner};
use crate::hub::{JobContext, JobError, SourceEvent};

/// The `.gitignore` line that ignores an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    pub source: PathBuf,
    pub line: usize,
    pub pattern: String,
}

/// Something git ignores: a folder (ending in `/`) or a file, relative to the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredEntry {
    pub path: String,
    pub rule: Option<IgnoreRule>,
}

/// Changes in the working tree, as `git status` counts them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub renamed: usize,
    pub untracked: usize,
    pub conflicted: usize,
}

impl StatusCounts {
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.renamed + self.untracked + self.conflicted
    }
}

/// What git says about the files on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitDisk {
    pub tracked: usize,
    /// Tracked files whose `filter` attribute is `lfs`.
    pub lfs: usize,
    pub ignored: Vec<IgnoredEntry>,
    pub changes: StatusCounts,
}

fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| String::from_utf8_lossy(f).into_owned()).collect()
}

/// Reads tracked files, LFS files, ignored entries with their rules, and changes.
pub fn read_git_disk(runner: &Runner, root: &Path) -> Result<GitDisk, JobError> {
    let git = |args: &[&str]| -> Result<Vec<u8>, JobError> {
        Ok(runner.run(&Command::git(root, args.iter().copied())?)?.stdout)
    };
    if git(&["rev-parse", "--is-inside-work-tree"]).is_err() {
        return Err(JobError::Unavailable("No git repository here".to_string()));
    }
    let listed = git(&["ls-files", "-z"])?;
    let tracked = listed.split(|&b| b == 0).filter(|f| !f.is_empty()).count();

    let attrs = runner.run(&Command::git(root, ["check-attr", "--stdin", "-z", "filter"])?.with_stdin(listed))?.stdout;
    let lfs = nul_fields(&attrs).chunks(3).filter(|c| c.len() == 3 && c[2] == "lfs").count();

    let ignored_paths =
        nul_fields(&git(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])?);
    let mut ignored: Vec<IgnoredEntry> =
        ignored_paths.iter().map(|p| IgnoredEntry { path: p.clone(), rule: None }).collect();
    if !ignored_paths.is_empty() {
        // `-v` names the file, line and pattern; `--no-index` keeps it from consulting the index.
        let input: Vec<u8> = ignored_paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
        let command = Command::git(root, ["check-ignore", "-v", "-z", "--stdin", "--no-index"])?.with_stdin(input);
        let out = runner.run(&command);
        // check-ignore exits 1 when a path is not ignored; it still prints the ones that are.
        let stdout = match out {
            Ok(o) => o.stdout,
            Err(crate::exec::ExecError::Failed { .. }) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        for chunk in nul_fields(&stdout).chunks(4) {
            let [source, line, pattern, path] = chunk else { continue };
            if let Some(entry) = ignored.iter_mut().find(|e| e.path == *path) {
                entry.rule = Some(IgnoreRule {
                    source: root.join(source),
                    line: line.parse().unwrap_or(0),
                    pattern: pattern.clone(),
                });
            }
        }
    }

    let status = git(&["status", "--porcelain=v2", "-z", "--untracked-files=normal"])?;
    let changes = parse_status_counts(&status);
    Ok(GitDisk { tracked, lfs, ignored, changes })
}

/// Counts `git status --porcelain=v2 -z` entries.
pub fn parse_status_counts(bytes: &[u8]) -> StatusCounts {
    let mut counts = StatusCounts::default();
    let mut fields = bytes.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        let mut parts = text.splitn(3, ' ');
        let (kind, xy) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let (x, y) = (xy.chars().next().unwrap_or('.'), xy.chars().nth(1).unwrap_or('.'));
        match kind {
            "1" => {
                if x == 'A' {
                    counts.added += 1;
                } else if x == 'D' || y == 'D' {
                    counts.deleted += 1;
                } else {
                    counts.modified += 1;
                }
            }
            "2" => {
                counts.renamed += 1;
                fields.next(); // the path it was renamed from
            }
            "u" => counts.conflicted += 1,
            "?" => counts.untracked += 1,
            _ => {}
        }
    }
    counts
}

/// Bytes `path` takes on disk, as `du -s --block-size=1` counts them: allocated blocks, each hard
/// link's file once, never crossing into another file system. Elsewhere than Unix, file lengths.
pub fn du_bytes(path: &Path) -> u64 {
    let seen = Mutex::new(HashSet::new());
    match std::fs::symlink_metadata(path) {
        Ok(meta) => du(path, &meta, device(&meta), &seen),
        Err(_) => 0,
    }
}

#[cfg(unix)]
fn device(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(meta)
}

#[cfg(not(unix))]
fn device(_: &std::fs::Metadata) -> u64 {
    0
}

/// What one entry takes, counting a file with several hard links only the first time.
fn own_bytes(meta: &std::fs::Metadata, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !meta.is_dir()
            && meta.nlink() > 1
            && !seen.lock().unwrap_or_else(|e| e.into_inner()).insert((meta.dev(), meta.ino()))
        {
            return 0;
        }
        meta.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        let _ = seen;
        meta.len()
    }
}

fn du(path: &Path, meta: &std::fs::Metadata, dev: u64, seen: &Mutex<HashSet<(u64, u64)>>) -> u64 {
    if device(meta) != dev {
        return 0;
    }
    let own = own_bytes(meta, seen);
    if !meta.is_dir() {
        return own;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return own };
    let entries: Vec<(PathBuf, std::fs::Metadata)> =
        entries.flatten().filter_map(|e| Some((e.path(), std::fs::symlink_metadata(e.path()).ok()?))).collect();
    own + entries.par_iter().map(|(p, m)| du(p, m, dev, seen)).sum::<u64>()
}

/// The job for a [`crate::SourceHub`]: git's facts first, then the size of each top-level entry
/// of the project and of each entry git ignores, as each is measured, on 4 threads (sizes are
/// disk-bound; more would only thrash).
pub fn disk_job() -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send {
    move |ctx| {
        let mut entries: Vec<PathBuf> =
            std::fs::read_dir(&ctx.root).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        entries.sort();
        match read_git_disk(&ctx.runner, &ctx.root) {
            Ok(git) => {
                for e in &git.ignored {
                    let path = ctx.root.join(e.path.trim_end_matches('/'));
                    if !entries.contains(&path) {
                        entries.push(path);
                    }
                }
                ctx.send(SourceEvent::GitDisk(std::sync::Arc::new(git)));
            }
            Err(JobError::Unavailable(_)) => {}
            Err(e) => return Err(e),
        }
        let pool =
            rayon::ThreadPoolBuilder::new().num_threads(4).build().map_err(|e| JobError::Failed(e.to_string()))?;
        for entry in entries {
            if ctx.is_cancelled() {
                return Err(JobError::Cancelled);
            }
            let bytes = pool.install(|| du_bytes(&entry));
            let rel = entry.strip_prefix(&ctx.root).unwrap_or(&entry).to_path_buf();
            ctx.send(SourceEvent::FolderSize(rel, bytes));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CancelToken;

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    #[test]
    fn status_counts_each_kind_of_change() {
        let out = b"1 .M N... 100644 100644 100644 aa bb src/a.rs\0\
1 A. N... 000000 100644 100644 00 cc new.rs\0\
1 .D N... 100644 100644 000000 aa aa gone.rs\0\
2 R. N... 100644 100644 100644 aa aa R100 moved.rs\0was.rs\0\
u UU N... 100644 100644 100644 100644 aa bb cc both.rs\0\
? notes.txt\0";
        let c = parse_status_counts(out);
        assert_eq!((c.modified, c.added, c.deleted, c.renamed, c.conflicted, c.untracked), (1, 1, 1, 1, 1, 1));
        assert_eq!(c.total(), 6);
    }

    #[test]
    fn git_says_what_is_tracked_ignored_and_by_which_rule() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(r)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !git(&["init", "-q"]) {
            return; // no git here
        }
        write(r, ".gitignore", b"# build output\n/target/\n*.log\n");
        write(r, ".gitattributes", b"*.png filter=lfs diff=lfs merge=lfs -text\n");
        write(r, "src/main.rs", b"fn main() {}\n");
        write(r, "logo.png", b"not really a png");
        write(r, "target/debug/app", b"binary");
        write(r, "run.log", b"log");
        assert!(git(&["add", "."]) && git(&["commit", "-qm", "one"]));
        write(r, "src/main.rs", b"fn main() { println!(); }\n");
        write(r, "draft.txt", b"new");

        let facts = read_git_disk(&Runner::new(CancelToken::default()), r).unwrap();
        assert_eq!(facts.tracked, 4, ".gitignore, .gitattributes, src/main.rs, logo.png");
        assert_eq!(facts.lfs, 1);
        let rule = |path: &str| facts.ignored.iter().find(|e| e.path == path).and_then(|e| e.rule.clone()).unwrap();
        assert_eq!((rule("target/").line, rule("target/").pattern.as_str()), (2, "/target/"));
        assert_eq!(rule("run.log").pattern, "*.log");
        assert_eq!((facts.changes.modified, facts.changes.untracked), (1, 1));

        let outside = tempfile::tempdir().unwrap();
        // A folder with a broken .git file is outside any repository, whatever surrounds it.
        write(outside.path(), ".git", b"not a gitdir\n");
        assert!(matches!(
            read_git_disk(&Runner::new(CancelToken::default()), outside.path()),
            Err(JobError::Unavailable(_)) | Err(JobError::Failed(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn sizes_match_du_and_count_hard_links_once() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "a/one.bin", &vec![1u8; 10_000]);
        write(r, "a/b/two.bin", &vec![2u8; 70_000]);
        std::fs::hard_link(r.join("a/one.bin"), r.join("a/b/again.bin")).unwrap();
        let ours = du_bytes(&r.join("a"));
        let theirs = std::process::Command::new("du").args(["-s", "--block-size=1"]).arg(r.join("a")).output();
        if let Some(out) = theirs.ok().filter(|o| o.status.success()) {
            let du: u64 = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().parse().unwrap();
            assert_eq!(ours, du);
        }
        let twice = du_bytes(&r.join("a/one.bin")) + du_bytes(&r.join("a/b/again.bin"));
        assert!(ours < twice + du_bytes(&r.join("a/b/two.bin")) + 3 * 4096, "the linked file counts once");
    }
}
