//! `read_git_disk` on `tests/fixtures/disk`, checked against its `expected.toml`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use studio_sources::{read_git_disk, CancelToken, GitDisk, Runner};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/disk")
}

fn expected() -> toml::Table {
    std::fs::read_to_string(fixture().join("expected.toml")).unwrap().parse().unwrap()
}

/// The fixture's `tree/` copied into a new temporary folder, `dot.x` as `.x`.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let from = fixture().join("tree");
    for entry in walkdir::WalkDir::new(&from).sort_by_file_name() {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(&from).unwrap();
        let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let to = match name.strip_prefix("dot.") {
            Some(rest) => dir.path().join(rel).with_file_name(format!(".{rest}")),
            None => dir.path().join(rel),
        };
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&to).unwrap();
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
    dir
}

/// Runs git in `dir` with no user or system config. False when git is not installed.
fn git(dir: &Path, args: &[&str]) -> bool {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output();
    match out {
        Ok(o) => {
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            true
        }
        Err(_) => false,
    }
}

fn strings(value: &toml::Value) -> BTreeSet<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn count(table: &toml::Value, key: &str) -> usize {
    table[key].as_integer().unwrap() as usize
}

/// Every ignored entry as `expected.toml` writes it: path, `file:line`, pattern.
fn ignored(disk: &GitDisk, root: &Path) -> Vec<(String, String, String)> {
    disk.ignored
        .iter()
        .map(|e| {
            let rule = e.rule.as_ref().unwrap_or_else(|| panic!("{} has no rule", e.path));
            let file = rule.source.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            (e.path.clone(), format!("{file}:{}", rule.line), rule.pattern.clone())
        })
        .collect()
}

#[test]
fn the_disk_fixture_reads_as_expected() {
    // Neither the user's nor the system's git config (an LFS filter, a global ignore file) may
    // change what this repository says. This binary runs this one test, so the setting is ours.
    std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    let dir = project();
    let root = dir.path();
    if !git(root, &["init", "-q", "."]) {
        return; // no git here
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "fixture"]);
    std::fs::write(root.join("src/main.rs"), "fn main() { println!(); }\n").unwrap();
    std::fs::remove_file(root.join("README.md")).unwrap();
    std::fs::write(root.join("added.txt"), "staged\n").unwrap();
    git(root, &["add", "added.txt"]);
    std::fs::write(root.join("notes.txt"), "untracked\n").unwrap();

    let disk = read_git_disk(&Runner::new(CancelToken::default()), root).unwrap();
    let want = expected();
    assert_eq!(disk.tracked, want["tracked"].as_integer().unwrap() as usize);
    assert_eq!(disk.lfs_paths, strings(&want["lfs"]));
    assert_eq!(disk.lfs, disk.lfs_paths.len());
    assert_eq!(disk.generated_paths, strings(&want["generated"]));
    assert_eq!(disk.vendored_paths, strings(&want["vendored"]));

    let expected_ignored: Vec<(String, String, String)> = want["ignored"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let field = |k: &str| e[k].as_str().unwrap().to_string();
            (field("path"), field("rule"), field("pattern"))
        })
        .collect();
    assert_eq!(ignored(&disk, root), expected_ignored);
    for path in strings(&want["not_ignored"]) {
        assert!(!disk.ignored.iter().any(|e| e.path == path), "{path} is taken back by a negation");
    }

    let status = &want["status"];
    let c = disk.changes;
    assert_eq!(
        (c.modified, c.added, c.deleted, c.renamed, c.untracked, c.conflicted),
        (
            count(status, "modified"),
            count(status, "added"),
            count(status, "deleted"),
            count(status, "renamed"),
            count(status, "untracked"),
            count(status, "conflicted"),
        )
    );
}
