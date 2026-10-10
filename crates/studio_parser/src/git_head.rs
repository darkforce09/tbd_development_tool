//! Finds a checkout's HEAD commit by reading git's files, without running git. Handles plain
//! checkouts, linked worktrees (where `.git` is a file naming the real git directory), symbolic
//! refs and packed refs.

use std::path::{Path, PathBuf};

/// Where a checkout's HEAD points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHead {
    /// The checkout's own git directory: `.git`, or `<repo>/.git/worktrees/<name>` for a linked
    /// worktree.
    pub git_dir: PathBuf,
    /// The repository's shared directory, which holds the branches of every worktree.
    pub common_dir: PathBuf,
    /// The branch HEAD is on (`refs/heads/main`), or `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The commit HEAD resolves to (hex), or `None` for a branch with no commits yet.
    pub commit: Option<String>,
}

/// The git directory of the checkout containing `start`, looking in `start` and then each folder
/// above it.
pub fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            // A linked worktree or submodule: `gitdir: <path>`, relative to the checkout. A `.git`
            // file that is not one ends the search, as it does for git.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
            let path = dir.join(target);
            return path.is_dir().then_some(path);
        }
    }
    None
}

/// Reads where HEAD points for the checkout containing `start`. `None` outside a repository.
pub fn read_head(start: &Path) -> Option<GitHead> {
    let git_dir = find_git_dir(start)?;
    let common_dir = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|rel| git_dir.join(rel.trim()))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| git_dir.clone());
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    let (branch, commit) = match head.strip_prefix("ref:") {
        Some(name) => {
            let name = name.trim().to_string();
            let commit = resolve_ref(&git_dir, &common_dir, &name);
            (Some(name), commit)
        }
        None => (None, is_object_id(head).then(|| head.to_string())),
    };
    Some(GitHead { git_dir, common_dir, branch, commit })
}

/// The commit a ref names, following symbolic refs: a loose ref file (the worktree's own, then the
/// shared one), then `packed-refs`.
fn resolve_ref(git_dir: &Path, common_dir: &Path, name: &str) -> Option<String> {
    let mut name = name.to_string();
    // Symbolic refs can point at other refs; git itself stops after a few hops.
    for _ in 0..5 {
        let loose = [git_dir, common_dir].iter().find_map(|dir| std::fs::read_to_string(dir.join(&name)).ok());
        match loose.as_deref().map(str::trim) {
            Some(text) => match text.strip_prefix("ref:") {
                Some(next) => name = next.trim().to_string(),
                None => return is_object_id(text).then(|| text.to_string()),
            },
            None => return packed_ref(common_dir, &name),
        }
    }
    None
}

/// A ref's commit from `packed-refs` (`<id> <name>` lines; `#` headers and `^` peeled lines skipped).
fn packed_ref(common_dir: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(common_dir.join("packed-refs")).ok()?;
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .filter_map(|l| l.split_once(' '))
        .find(|(_, n)| n.trim() == name)
        .map(|(id, _)| id.to_string())
        .filter(|id| is_object_id(id))
}

/// 40 (SHA-1) or 64 (SHA-256) hex digits.
fn is_object_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A repository at `repo` on branch main (packed), with a feature branch (loose) checked out
    /// in a linked worktree at `wt`.
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let git = repo.join(".git");
        write(&git.join("HEAD"), "ref: refs/heads/main\n");
        write(
            &git.join("packed-refs"),
            &format!("# pack-refs with: peeled fully-peeled sorted\n{A} refs/heads/main\n^{B}\n"),
        );
        write(&git.join("refs/heads/feature"), &format!("{B}\n"));
        let wt_git = git.join("worktrees/wt");
        write(&wt_git.join("HEAD"), "ref: refs/heads/feature\n");
        write(&wt_git.join("commondir"), "../..\n");
        let wt = dir.path().join("wt");
        write(&wt.join(".git"), &format!("gitdir: {}\n", wt_git.display()));
        (dir, repo, wt)
    }

    #[test]
    fn a_branch_in_packed_refs_resolves() {
        let (_dir, repo, _) = repo();
        let head = read_head(&repo).unwrap();
        assert_eq!(head.branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(head.commit.as_deref(), Some(A));
    }

    #[test]
    fn a_linked_worktree_resolves_through_the_shared_refs() {
        let (_dir, repo, wt) = repo();
        let head = read_head(&wt).unwrap();
        assert_eq!(head.branch.as_deref(), Some("refs/heads/feature"));
        assert_eq!(head.commit.as_deref(), Some(B));
        assert_eq!(head.common_dir.canonicalize().unwrap(), repo.join(".git").canonicalize().unwrap());
    }

    #[test]
    fn a_folder_inside_the_checkout_finds_it() {
        let (_dir, repo, _) = repo();
        std::fs::create_dir_all(repo.join("crates/a")).unwrap();
        assert_eq!(read_head(&repo.join("crates/a")).unwrap().commit.as_deref(), Some(A));
    }

    #[test]
    fn detached_heads_and_empty_branches() {
        let (_dir, repo, _) = repo();
        write(&repo.join(".git/HEAD"), &format!("{B}\n"));
        let head = read_head(&repo).unwrap();
        assert_eq!((head.branch, head.commit.as_deref()), (None, Some(B)));

        write(&repo.join(".git/HEAD"), "ref: refs/heads/new\n");
        assert_eq!(read_head(&repo).unwrap().commit, None, "a branch with no commits yet");
    }

    #[test]
    fn a_broken_git_file_ends_the_search() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".git"), "not a gitdir line\n");
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        assert_eq!(find_git_dir(&dir.path().join("sub")), None);
        assert_eq!(read_head(dir.path()), None);
    }
}
