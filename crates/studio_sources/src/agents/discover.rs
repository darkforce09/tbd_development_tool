//! Finding the session logs under `~/.claude/projects` and deciding which belong to the project.
//! Folder names are lossy (`/`, `_` and `.` all become `-`), so every folder is scanned and the
//! decision is made by the canonical `cwd` the lines carry, never by the folder name.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;

use super::cache::mtime_ns;

/// One log file found on disk.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub path: PathBuf,
    /// Index into the folder list.
    pub folder: usize,
    pub subagent: bool,
    /// The file name for a main log; the `<session-id>` folder for a subagent's.
    pub session_id: String,
    pub len: u64,
    pub mtime: i64,
}

/// Every folder under `projects` (sorted) and every log file in them.
pub(crate) fn discover(projects: &Path) -> std::io::Result<(Vec<PathBuf>, Vec<Found>)> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(projects)?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    folders.sort();
    let found: Vec<Found> = folders
        .par_iter()
        .enumerate()
        .flat_map_iter(|(folder, dir)| {
            let mut out = Vec::new();
            let Ok(entries) = std::fs::read_dir(dir) else { return out };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
                    let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
                    if let Ok(meta) = entry.metadata() {
                        out.push(Found {
                            path,
                            folder,
                            subagent: false,
                            session_id: stem,
                            len: meta.len(),
                            mtime: mtime_ns(&meta),
                        });
                    }
                } else if kind.is_dir() {
                    let session_id = entry.file_name().to_string_lossy().into_owned();
                    let Ok(subs) = std::fs::read_dir(path.join("subagents")) else { continue };
                    for sub in subs.filter_map(Result::ok) {
                        let path = sub.path();
                        if path.extension().is_some_and(|e| e == "jsonl") && sub.file_type().is_ok_and(|t| t.is_file())
                        {
                            if let Ok(meta) = sub.metadata() {
                                out.push(Found {
                                    path,
                                    folder,
                                    subagent: true,
                                    session_id: session_id.clone(),
                                    len: meta.len(),
                                    mtime: mtime_ns(&meta),
                                });
                            }
                        }
                    }
                }
            }
            out
        })
        .collect();
    let mut found = found;
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((folders, found))
}

/// `path` with `.` and `..` folded away, without touching the disk.
pub(crate) fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The canonical form of `path`; for a path that no longer exists, its longest existing
/// ancestor made canonical with the rest appended (lexically normalised).
pub(crate) fn canonical(path: &Path) -> PathBuf {
    let path = normalise(path);
    if let Ok(c) = std::fs::canonicalize(&path) {
        return c;
    }
    for ancestor in path.ancestors().skip(1) {
        if let Ok(c) = std::fs::canonicalize(ancestor) {
            return match path.strip_prefix(ancestor) {
                Ok(rest) => c.join(rest),
                Err(_) => path.clone(),
            };
        }
    }
    path
}

/// Resolves the paths lines name against the project's roots, remembering each answer.
pub(crate) struct Resolver {
    pub roots: Vec<PathBuf>,
    /// Map cwds that are a parent of a root (off by default).
    pub map_parent_cwds: bool,
    known: Mutex<HashMap<String, Place>>,
    cwds: Mutex<HashMap<String, PathBuf>>,
}

impl Resolver {
    pub fn new(roots: Vec<PathBuf>, map_parent_cwds: bool) -> Self {
        Self { roots, map_parent_cwds, known: Mutex::default(), cwds: Mutex::default() }
    }

    /// The canonical form of a `cwd` string (remembered per distinct string).
    pub fn cwd(&self, cwd: &str) -> PathBuf {
        if let Some(c) = self.cwds.lock().unwrap_or_else(|e| e.into_inner()).get(cwd) {
            return c.clone();
        }
        let c = canonical(Path::new(cwd));
        self.cwds.lock().unwrap_or_else(|e| e.into_inner()).insert(cwd.to_string(), c.clone());
        c
    }

    /// The root a session with this `cwd` belongs to: the longest root the cwd is in (or a
    /// root the cwd contains, when parent folders are mapped).
    pub fn root_of_cwd(&self, cwd: &str) -> Option<usize> {
        let cwd = self.cwd(cwd);
        let inside = self.root_containing(&cwd);
        inside.or_else(|| self.map_parent_cwds.then(|| self.roots.iter().position(|r| r.starts_with(&cwd))).flatten())
    }

    /// The longest root a canonical path is in.
    pub fn root_containing(&self, path: &Path) -> Option<usize> {
        self.roots
            .iter()
            .enumerate()
            .filter(|(_, r)| path.starts_with(r))
            .max_by_key(|(_, r)| r.components().count())
            .map(|(i, _)| i)
    }

    /// The canonical form of a file path a tool named (relative ones against the line's
    /// `cwd`), or `None` when it is outside every root.
    #[cfg(test)]
    pub fn resolve(&self, path: &str, cwd: Option<&str>) -> Option<String> {
        match self.locate(path, cwd) {
            Place::Inside(p) => Some(p),
            Place::Outside(_) | Place::Unknown => None,
        }
    }

    /// Where a file path a tool named is (relative ones against the line's `cwd`).
    pub fn locate(&self, path: &str, cwd: Option<&str>) -> Place {
        let relative = !Path::new(path).is_absolute();
        let key: Cow<str> = match (relative, cwd) {
            (true, Some(cwd)) => Cow::Owned(format!("{cwd}\0{path}")),
            _ => Cow::Borrowed(path),
        };
        if let Some(known) = self.known.lock().unwrap_or_else(|e| e.into_inner()).get(key.as_ref()) {
            return known.clone();
        }
        let absolute = match (relative, cwd) {
            (true, Some(cwd)) => self.cwd(cwd).join(path),
            (true, None) => {
                self.known.lock().unwrap_or_else(|e| e.into_inner()).insert(key.into_owned(), Place::Unknown);
                return Place::Unknown;
            }
            (false, _) => PathBuf::from(path),
        };
        let c = canonical(&absolute);
        let text = c.to_string_lossy().into_owned();
        let answer =
            if self.roots.iter().any(|r| c.starts_with(r)) { Place::Inside(text) } else { Place::Outside(text) };
        self.known.lock().unwrap_or_else(|e| e.into_inner()).insert(key.into_owned(), answer.clone());
        answer
    }
}

/// Where a path a tool named is, canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    Inside(String),
    Outside(String),
    /// A relative path on a line without a `cwd`.
    Unknown,
}

/// The spelling of the root that `cwd` (a raw `cwd` whose canonical form `canonical` is inside
/// `root`) starts with: `cwd` less the part below the root, or `cwd` itself when the two
/// spellings differ below the root too.
pub(crate) fn alias_of(cwd: &str, canonical: &Path, root: &Path) -> Option<PathBuf> {
    let raw = Path::new(cwd);
    if !raw.is_absolute() || normalise(raw).as_os_str() != raw.as_os_str() {
        return None;
    }
    let rest = canonical.strip_prefix(root).ok()?;
    let mut alias = raw.to_path_buf();
    if raw.ends_with(rest) {
        for _ in rest.components() {
            alias.pop();
        }
    }
    (alias != root).then_some(alias)
}

/// Spellings of `roots` through symlinks directly in `home` (e.g. `~/Projects` linking to
/// another disk), sorted.
pub(crate) fn home_aliases(home: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if !entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        let link = entry.path();
        let Ok(target) = std::fs::canonicalize(&link) else { continue };
        for root in roots {
            if let Ok(rest) = root.strip_prefix(&target) {
                out.push(if rest.as_os_str().is_empty() { link.clone() } else { link.join(rest) });
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The bytes a line holds when its `cwd` starts with `prefix`: `"cwd":"<prefix>` with the
/// prefix escaped as JSON writes it.
pub(crate) fn cwd_needle(prefix: &Path) -> Vec<u8> {
    let quoted = serde_json::to_string(&prefix.to_string_lossy()).unwrap_or_default();
    let mut needle = b"\"cwd\":".to_vec();
    needle.extend_from_slice(&quoted.as_bytes()[..quoted.len().saturating_sub(1)]);
    needle
}

/// The deepest folder every path is in.
pub(crate) fn common_ancestor<'a>(paths: impl IntoIterator<Item = &'a Path>) -> PathBuf {
    let mut common: Option<PathBuf> = None;
    for path in paths {
        common = Some(match common {
            None => path.to_path_buf(),
            Some(c) => c.components().zip(path.components()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect(),
        });
    }
    common.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_through_symlinks_even_when_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "").unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, dir.path().join("link")).unwrap();
        let r = Resolver::new(vec![root.clone()], false);
        let a = root.join("src/a.rs").to_string_lossy().into_owned();
        assert_eq!(r.resolve(&a, None).as_deref(), Some(a.as_str()));
        #[cfg(unix)]
        {
            let via = dir.path().join("link/src/./a.rs");
            assert_eq!(r.resolve(via.to_str().unwrap(), None).as_deref(), Some(a.as_str()));
            let gone = dir.path().join("link/src/gone.rs");
            let expected = root.join("src/gone.rs").to_string_lossy().into_owned();
            assert_eq!(r.resolve(gone.to_str().unwrap(), None), Some(expected));
        }
        assert_eq!(r.resolve("src/a.rs", Some(root.to_str().unwrap())).as_deref(), Some(a.as_str()));
        assert_eq!(r.resolve("/etc/hosts", None), None);
        assert_eq!(r.resolve(&format!("{}/../outside", root.display()), None), None);
        // A removed sibling worktree whose name starts like the root stays outside: the
        // existing prefix is made canonical, the rest is compared by whole components.
        let sibling = dir.path().join("repo-s4/src/a.rs");
        assert_eq!(
            r.locate(sibling.to_str().unwrap(), None),
            Place::Outside(base_of(&root).join("repo-s4/src/a.rs").to_string_lossy().into_owned())
        );
        assert_eq!(r.root_of_cwd(root.join("src").to_str().unwrap()), Some(0));
        assert_eq!(r.root_of_cwd(dir.path().to_str().unwrap()), None, "a parent folder is not mapped");
        let parents = Resolver::new(vec![root.clone()], true);
        assert_eq!(parents.root_of_cwd(dir.path().to_str().unwrap()), Some(0));
    }

    fn base_of(root: &Path) -> PathBuf {
        root.parent().unwrap().to_path_buf()
    }

    #[test]
    fn spellings_of_a_root() {
        let root = Path::new("/disk/Projects/repo");
        let alias = |cwd: &str, canonical: &str| alias_of(cwd, Path::new(canonical), root);
        assert_eq!(alias("/home/u/Projects/repo/src", "/disk/Projects/repo/src"), Some("/home/u/Projects/repo".into()));
        assert_eq!(alias("/disk/Projects/repo/src", "/disk/Projects/repo/src"), None, "the root itself");
        assert_eq!(alias("/l/inner", "/disk/Projects/repo/src/inner"), Some("/l/inner".into()), "differs below");
        assert_eq!(alias("/home/u/./x", "/disk/Projects/repo"), None);
        assert_eq!(cwd_needle(Path::new("/a\"b\\c")), br#""cwd":"/a\"b\\c"#.to_vec());
        let common = common_ancestor([Path::new("/a/b/c"), Path::new("/a/b/d/e"), Path::new("/a/b")]);
        assert_eq!(common, Path::new("/a/b"));
        assert_eq!(common_ancestor([Path::new("/x"), Path::new("/y")]), Path::new("/"));
    }

    #[cfg(unix)]
    #[test]
    fn home_symlinks_give_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let disk = base.join("disk/Projects");
        std::fs::create_dir_all(disk.join("repo")).unwrap();
        let home = base.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(&disk, home.join("Projects")).unwrap();
        std::os::unix::fs::symlink(disk.join("repo"), home.join("r")).unwrap();
        std::os::unix::fs::symlink(&base, home.join("up")).unwrap();
        let got = home_aliases(&home, &[disk.join("repo")]);
        assert_eq!(got, [home.join("Projects/repo"), home.join("r"), home.join("up/disk/Projects/repo")]);
    }
}
