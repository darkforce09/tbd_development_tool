//! `read_history`, `commit_files` and `read_part_commits` on a repository the test builds with plain `git`
//! (fixed names, emails and dates, so every id is stable), checked against `fixtures/git/expected.toml`.
//! Returns early when git is not installed.

use std::path::{Path, PathBuf};

use studio_sources::{
    commit_files, read_history, read_part_commits, CancelToken, FileChange, GitHistory, PartCommits, Runner,
};

fn expected() -> toml::Table {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git/expected.toml");
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn strings(value: &toml::Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// A temp folder holding the repository (`repo`) and its linked worktree (`wt`).
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }

    /// Runs `git` in `cwd` (relative to the temp folder) with no user or system config, as Sam, at
    /// 2026-01-0`day` 12:00 UTC. False when git is not installed.
    fn try_git(&self, cwd: &str, day: u32, args: &[&str]) -> bool {
        let date = format!("2026-01-0{day}T12:00:00Z");
        std::process::Command::new("git")
            .args(args)
            .current_dir(self.dir.path().join(cwd))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Sam")
            .env("GIT_AUTHOR_EMAIL", "sam@example.invalid")
            .env("GIT_COMMITTER_NAME", "Sam")
            .env("GIT_COMMITTER_EMAIL", "sam@example.invalid")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn git(&self, cwd: &str, day: u32, args: &[&str]) {
        assert!(self.try_git(cwd, day, args), "git {args:?} in {cwd}");
    }

    /// Writes `n` lines `"<tag> line i"` to `rel` (relative to the temp folder).
    fn lines(&self, rel: &str, tag: &str, n: usize) {
        self.write(rel, &(1..=n).map(|i| format!("{tag} line {i}\n")).collect::<String>());
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

/// Five commits on `main` (a rename, a branch merged with `--no-ff`, co-author trailers), branches `feature`,
/// `release` (tracking `main`) and `wip` (checked out in the linked worktree `wt`), and uncommitted changes in
/// both worktrees. `None` when git is not installed.
fn build() -> Option<Fixture> {
    let fx = Fixture { dir: tempfile::tempdir().unwrap() };
    std::fs::create_dir_all(fx.repo()).unwrap();
    if !fx.try_git("repo", 1, &["init", "-q", "-b", "main", "."]) {
        return None;
    }
    fx.lines("repo/app/main.rs", "main", 5);
    fx.lines("repo/lib/util.rs", "util", 3);
    fx.lines("repo/README.md", "readme", 2);
    fx.git("repo", 1, &["add", "-A"]);
    fx.git("repo", 1, &["commit", "-qm", "Start the app"]);

    fx.git("repo", 2, &["mv", "lib/util.rs", "lib/helpers.rs"]);
    fx.git("repo", 2, &["commit", "-qm", "Rename util to helpers\n\nCo-Authored-By: Ada <ada@example.com>"]);
    fx.git("repo", 2, &["branch", "release"]);

    // Touches two parts, `app` and `lib`.
    fx.git("repo", 3, &["checkout", "-qb", "feature"]);
    fx.lines("repo/app/feature.rs", "feature", 4);
    fx.lines("repo/lib/helpers.rs", "util", 4);
    fx.git("repo", 3, &["add", "-A"]);
    let trailers = "Co-Authored-By: Ada <ada@example.com>\nCo-Authored-By: Bob <bob@example.com>";
    fx.git("repo", 3, &["commit", "-qm", &format!("Add the feature\n\n{trailers}")]);

    fx.git("repo", 4, &["checkout", "-q", "main"]);
    fx.lines("repo/docs/guide.md", "guide", 3);
    fx.lines("repo/README.md", "readme", 3);
    fx.git("repo", 4, &["add", "-A"]);
    fx.git("repo", 4, &["commit", "-qm", "Write the guide"]);
    fx.git("repo", 5, &["merge", "-q", "--no-ff", "feature", "-m", "Merge feature"]);

    fx.git("repo", 5, &["branch", "-q", "--set-upstream-to=main", "release"]);
    fx.git("repo", 5, &["worktree", "add", "-q", "-b", "wip", "../wt"]);

    // The main worktree: one unstaged change.
    fx.lines("repo/README.md", "readme", 4);
    // The linked worktree: modified, deleted, staged new, renamed and untracked.
    fx.write("wt/app/main.rs", "main line 1\nchanged\nmain line 3\nmain line 4\nmain line 5\nmain line 6\n");
    std::fs::remove_file(fx.dir.path().join("wt/docs/guide.md")).unwrap();
    fx.lines("wt/notes.txt", "notes", 2);
    fx.git("wt", 5, &["add", "notes.txt"]);
    fx.git("wt", 5, &["mv", "README.md", "README.txt"]);
    fx.write("wt/scratch.txt", "scratch\n");
    Some(fx)
}

fn runner() -> Runner {
    Runner::new(CancelToken::default())
}

fn change(f: &FileChange) -> String {
    let count = |n: Option<usize>| n.map_or("-".to_string(), |n| n.to_string());
    let from = f.old_path.as_ref().map(|o| format!(" <- {o}")).unwrap_or_default();
    format!("{}{from} +{} -{}", f.path, count(f.added), count(f.deleted))
}

fn branches(history: &GitHistory) -> Vec<String> {
    history
        .branches
        .iter()
        .map(|b| {
            let up = b.upstream.as_deref().unwrap_or("-");
            format!(
                "{} {} up={up} ahead={} behind={} time={} | {}",
                b.name, b.head, b.ahead, b.behind, b.time, b.subject
            )
        })
        .collect()
}

/// Each worktree by its folder's name, after checking the path git reports is that folder.
fn worktrees(fx: &Fixture, history: &GitHistory) -> (Vec<String>, toml::Table) {
    let mut lines = Vec::new();
    let mut files = toml::Table::new();
    for w in &history.worktrees {
        let name = w.path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(w.path.canonicalize().unwrap(), fx.dir.path().join(&name).canonicalize().unwrap());
        let c = w.changes;
        lines.push(format!(
            "{name} main={} branch={} head={} | modified={} added={} deleted={} renamed={} untracked={} conflicted={}",
            w.is_main,
            w.branch.as_deref().unwrap_or("-"),
            w.head,
            c.modified,
            c.added,
            c.deleted,
            c.renamed,
            c.untracked,
            c.conflicted
        ));
        let list = w.files.iter().map(|f| toml::Value::String(change(f))).collect();
        files.insert(name, toml::Value::Array(list));
    }
    (lines, files)
}

fn commits(history: &GitHistory) -> Vec<String> {
    history
        .commits
        .iter()
        .map(|c| {
            format!(
                "{} {} {} {} parents={} | {} | co={}",
                c.id,
                c.short,
                c.author,
                c.time,
                c.parents,
                c.subject,
                c.co_authors.join("; ")
            )
        })
        .collect()
}

fn parts(parts: &PartCommits) -> toml::Table {
    parts
        .by_folder
        .iter()
        .map(|(folder, c)| {
            (folder.clone(), toml::Value::String(format!("{} {} {} | {}", c.id, c.short, c.time, c.subject)))
        })
        .collect()
}

#[test]
fn the_fixture_repository_reads_as_expected() {
    let Some(fx) = build() else { return };
    let expected = expected();
    let runner = runner();
    let history = read_history(&runner, &fx.repo()).unwrap();

    assert_eq!(branches(&history), strings(&expected["branches"]));
    let (trees, files) = worktrees(&fx, &history);
    assert_eq!(trees, strings(&expected["worktrees"]));
    assert_eq!(&files, expected["worktree_files"].as_table().unwrap());

    assert_eq!(history.commits.len() as i64, expected["history_length"].as_integer().unwrap());
    assert_eq!(history.commit_count as i64, expected["commit_count"].as_integer().unwrap());
    assert_eq!(commits(&history), strings(&expected["commits"]));
    assert_eq!(history.co_authored_commits as i64, expected["co_authored_commits"].as_integer().unwrap());
    let co_authors: toml::Table =
        history.co_authors.iter().map(|(name, n)| (name.clone(), toml::Value::Integer(*n as i64))).collect();
    assert_eq!(&co_authors, expected["co_authors"].as_table().unwrap());

    let per_commit: toml::Table = history
        .commits
        .iter()
        .map(|c| {
            let files = commit_files(&runner, &fx.repo(), &c.id).unwrap();
            (c.short.clone(), toml::Value::Array(files.iter().map(|f| toml::Value::String(change(f))).collect()))
        })
        .collect();
    assert_eq!(&per_commit, expected["commit_files"].as_table().unwrap());

    let part = read_part_commits(&runner, &fx.repo()).unwrap();
    assert_eq!(part.commits_scanned as i64, expected["part_commits_scanned"].as_integer().unwrap());
    assert_eq!(&parts(&part), expected["part_commits"].as_table().unwrap());
}

#[test]
fn the_linked_worktree_reads_the_same_repository() {
    let Some(fx) = build() else { return };
    let runner = runner();
    let from_main = read_history(&runner, &fx.repo()).unwrap();
    let from_linked = read_history(&runner, &fx.dir.path().join("wt")).unwrap();
    assert_eq!(from_linked.branches, from_main.branches);
    assert_eq!(from_linked.worktrees, from_main.worktrees);
    assert_eq!(from_linked.commits, from_main.commits);
}
