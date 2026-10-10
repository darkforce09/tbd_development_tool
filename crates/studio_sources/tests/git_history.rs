//! Git history read from small repositories made in temp folders, with exact expected numbers.
//! Each test returns early when git is not installed.

use std::path::{Path, PathBuf};

use studio_sources::{
    commit_files, commit_files_cached, read_history, read_part_commits, CancelToken, FileChange, Runner, Store,
};

/// A repository in a temp folder, driven by the real `git`.
struct Repo {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Repo {
    fn new() -> Option<Self> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repo");
        std::fs::create_dir_all(&path).unwrap();
        let repo = Self { _dir: dir, path };
        repo.try_git(&["init", "-q", "-b", "main"]).then_some(repo)
    }

    fn try_git(&self, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(["-c", "user.name=Sam", "-c", "user.email=s@example.com", "-c", "commit.gpgsign=false"])
            .args(["-c", "core.autocrlf=false"])
            .args(args)
            .current_dir(&self.path)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn git(&self, args: &[&str]) {
        assert!(self.try_git(args), "git {args:?}");
    }

    fn rev(&self, rev: &str) -> String {
        let out = std::process::Command::new("git").args(["rev-parse", rev]).current_dir(&self.path).output().unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn write(&self, rel: &str, bytes: impl AsRef<[u8]>) {
        let path = self.path.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
        self.rev("HEAD")
    }
}

fn runner() -> Runner {
    Runner::new(CancelToken::default())
}

fn change(path: &str, old: Option<&str>, added: Option<usize>, deleted: Option<usize>) -> FileChange {
    FileChange { path: path.into(), old_path: old.map(Into::into), added, deleted }
}

fn lines(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn commit_files_keep_renames_copies_binary_files_and_merges() {
    let Some(repo) = Repo::new() else { return };
    repo.write("src.txt", lines(50));
    repo.write("a.txt", lines(20));
    let first = repo.commit_all("first");

    // A rename, a copy of a file the commit also changes, and a binary file.
    repo.git(&["mv", "a.txt", "b.txt"]);
    repo.write("copy.txt", lines(50));
    repo.write("src.txt", lines(51));
    repo.write("bin.dat", b"x\0y\0z");
    let second = repo.commit_all("second");

    // A merge brings in what its branch added.
    repo.git(&["checkout", "-qb", "feature"]);
    repo.write("feature.txt", lines(3));
    repo.commit_all("feature");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("main.txt", lines(1));
    repo.commit_all("main");
    repo.git(&["merge", "-q", "--no-ff", "feature", "-m", "merge feature"]);
    let merge = repo.rev("HEAD");

    let runner = runner();
    assert_eq!(
        commit_files(&runner, &repo.path, &first).unwrap(),
        [change("a.txt", None, Some(20), Some(0)), change("src.txt", None, Some(50), Some(0))]
    );
    assert_eq!(
        commit_files(&runner, &repo.path, &second).unwrap(),
        [
            change("b.txt", Some("a.txt"), Some(0), Some(0)),
            change("bin.dat", None, None, None),
            change("copy.txt", Some("src.txt"), Some(0), Some(0)),
            change("src.txt", None, Some(1), Some(0)),
        ]
    );
    assert_eq!(commit_files(&runner, &repo.path, &merge).unwrap(), [change("feature.txt", None, Some(3), Some(0))]);
}

#[test]
fn commit_files_are_cached_by_full_id() {
    let Some(repo) = Repo::new() else { return };
    repo.write("a.txt", lines(4));
    let id = repo.commit_all("first");
    let cache = tempfile::tempdir().unwrap();
    let store = Store::in_dir(cache.path().join("sources"));

    let cold = commit_files_cached(&runner(), &repo.path, &store, &id).unwrap();
    assert_eq!(*cold, [change("a.txt", None, Some(4), Some(0))]);
    assert!(cache.path().join(format!("sources/commit-{id}.rkyv")).is_file());

    // A cancelled runner fails any command it runs, so a result proves no git ran.
    let cancel = CancelToken::default();
    cancel.cancel();
    let cancelled = Runner::new(cancel);
    assert_eq!(commit_files_cached(&cancelled, &repo.path, &store, &id).unwrap(), cold);
    let other = repo.rev("HEAD").replace(&id[..1], if id.starts_with('0') { "1" } else { "0" });
    assert!(commit_files_cached(&cancelled, &repo.path, &store, &other).is_err(), "not cached, not run");

    for refused in [&id[..7], &id.to_uppercase(), "HEAD", "--all", ""] {
        assert!(commit_files_cached(&runner(), &repo.path, &store, refused).is_err(), "{refused}");
    }
}

#[test]
fn worktrees_list_their_changed_files_with_line_counts() {
    let Some(repo) = Repo::new() else { return };
    repo.write("keep.txt", lines(10));
    repo.write("old.txt", lines(30));
    repo.write("gone.txt", lines(2));
    repo.commit_all("first");

    // 10 lines → 8 lines with one changed: +1 −3.
    let mut text: Vec<String> = (1..=8).map(|i| format!("line {i}\n")).collect();
    text[0] = "changed\n".into();
    repo.write("keep.txt", text.concat());
    repo.git(&["mv", "old.txt", "new.txt"]);
    std::fs::remove_file(repo.path.join("gone.txt")).unwrap();
    repo.write("staged.txt", lines(5));
    repo.git(&["add", "staged.txt"]);
    repo.write("untracked.txt", lines(1));

    let history = read_history(&runner(), &repo.path).unwrap();
    assert_eq!(history.worktrees.len(), 1);
    let main = &history.worktrees[0];
    assert_eq!(
        main.files,
        [
            change("gone.txt", None, Some(0), Some(2)),
            change("keep.txt", None, Some(1), Some(3)),
            change("new.txt", Some("old.txt"), Some(0), Some(0)),
            change("staged.txt", None, Some(5), Some(0)),
        ]
    );
    assert_eq!(main.changes.untracked, 1);
}

#[test]
fn co_authored_commits_are_counted_in_any_case() {
    let Some(repo) = Repo::new() else { return };
    repo.write("a.txt", "1\n");
    repo.commit_all("plain");
    repo.write("a.txt", "2\n");
    repo.commit_all("one\n\nCo-authored-by: Ada <ada@example.com>");
    repo.write("a.txt", "3\n");
    repo.commit_all("two\n\nCO-AUTHORED-BY: Ada <ada@example.com>\nco-authored-by: Bob <bob@example.com>");
    repo.write("a.txt", "4\n");
    let subject = "tab\there, ünïcödé ✓ \u{1c}";
    repo.commit_all(&format!("{subject}\n\nCo-Authored-By: Claude <noreply@anthropic.com>"));

    let history = read_history(&runner(), &repo.path).unwrap();
    assert_eq!(history.commit_count, 4);
    assert_eq!(history.commits.len(), 4);
    assert_eq!(history.co_authored_commits, 3);
    let names: Vec<(&str, usize)> = history.co_authors.iter().map(|(n, c)| (n.as_str(), *c)).collect();
    assert_eq!(names, [("Ada", 2), ("Bob", 1), ("Claude", 1)]);

    let newest = &history.commits[0];
    assert_eq!(newest.subject, subject);
    assert_eq!(newest.author, "Sam");
    assert_eq!(newest.parents, 1);
    assert_eq!(newest.co_authors, ["Claude <noreply@anthropic.com>"]);
    assert_eq!(newest.id, repo.rev("HEAD"));
    assert!(newest.id.starts_with(&newest.short));
    assert_eq!(history.commits[1].co_authors, ["Ada <ada@example.com>", "Bob <bob@example.com>"]);
    assert_eq!(history.commits[3].parents, 0);
}

#[test]
fn each_folder_gets_the_last_commit_that_touched_it() {
    let Some(repo) = Repo::new() else { return };
    repo.write("a/b/c.txt", lines(3));
    repo.write("a/d.txt", lines(3));
    repo.write("top.txt", lines(3));
    let first = repo.commit_all("first");
    repo.write("a/b/c.txt", lines(4));
    let second = repo.commit_all("second");
    // A rename: both sides count as touched.
    std::fs::create_dir_all(repo.path.join("e/f")).unwrap();
    repo.git(&["mv", "a/d.txt", "e/f/d.txt"]);
    let third = repo.commit_all("third");

    let parts = read_part_commits(&runner(), &repo.path).unwrap();
    assert_eq!(parts.commits_scanned, 3);
    let by: Vec<(&str, &str)> = parts.by_folder.iter().map(|(f, c)| (f.as_str(), c.id.as_str())).collect();
    let (second_id, third_id) = (second.as_str(), third.as_str());
    assert_eq!(by, [("", third_id), ("a", third_id), ("a/b", second_id), ("e", third_id), ("e/f", third_id)]);
    let ab = &parts.by_folder["a/b"];
    assert_eq!(ab.subject, "second");
    assert!(second.starts_with(&ab.short) && ab.time > 0);
    assert_ne!(first, second);

    let empty = Repo::new().unwrap();
    assert_eq!(read_part_commits(&runner(), &empty.path).unwrap().commits_scanned, 0, "no commits yet");
    let plain = tempfile::tempdir().unwrap();
    assert!(read_part_commits(&runner(), Path::new(plain.path())).is_err(), "no repository");
}
