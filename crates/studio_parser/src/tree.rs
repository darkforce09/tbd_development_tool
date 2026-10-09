//! Discovery of every file and folder under a project root.
//!
//! Nothing is dropped: binary, large, symlinked and unreadable entries are recorded with a kind
//! instead of being skipped. "Heavy" folders (version control, gitignored, build caches,
//! dependency trees) are recorded with counts but not descended, so they can be shown collapsed
//! and materialized on demand with [`scan_tree`] on their own path.

use rayon::prelude::*;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub type DirId = usize;

/// Files above this size are listed but not parsed.
pub const MAX_PARSE_BYTES: u64 = 1_500_000;

#[derive(Debug, Clone, Default)]
pub struct ProjectTree {
    pub root: PathBuf,
    /// `dirs[0]` is the root itself (empty relative path).
    pub dirs: Vec<DirNode>,
    pub files: Vec<FileNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DirNode {
    /// Path relative to the tree root.
    pub rel: PathBuf,
    pub parent: Option<DirId>,
    pub kind: DirKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DirKind {
    Normal,
    /// Listed with totals but not descended.
    Heavy(HeavyInfo),
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeavyInfo {
    pub reason: HeavyReason,
    pub file_count: u64,
    pub dir_count: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeavyReason {
    VersionControl,
    GitIgnored,
    BuildCache,
    Dependencies,
}

impl HeavyReason {
    pub fn label(self) -> &'static str {
        match self {
            HeavyReason::VersionControl => "version control",
            HeavyReason::GitIgnored => "gitignored",
            HeavyReason::BuildCache => "build cache",
            HeavyReason::Dependencies => "dependencies",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileNode {
    pub rel: PathBuf,
    pub dir: DirId,
    pub size: u64,
    pub kind: FileKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileKind {
    /// UTF-8-ish text small enough to parse.
    Text,
    Binary,
    Image,
    /// Larger than [`MAX_PARSE_BYTES`].
    TooLarge,
    Symlink(PathBuf),
    Unreadable(String),
}

#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    /// Treat folders git ignores as heavy. Off when materializing inside an ignored folder,
    /// where everything is ignored.
    pub gitignored_heavy: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { gitignored_heavy: true }
    }
}

impl ProjectTree {
    pub fn abs(&self, rel: &Path) -> PathBuf {
        self.root.join(rel)
    }

    /// Absolute paths of files that can be parsed.
    pub fn text_files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.files.iter().filter(|f| f.kind == FileKind::Text).map(|f| self.root.join(&f.rel))
    }

    pub fn heavy_dirs(&self) -> impl Iterator<Item = (DirId, &DirNode, &HeavyInfo)> {
        self.dirs.iter().enumerate().filter_map(|(i, d)| match &d.kind {
            DirKind::Heavy(info) => Some((i, d, info)),
            _ => None,
        })
    }
}

/// Lists every file and folder under `root`.
pub fn scan_tree(root: &Path, opts: ScanOptions) -> ProjectTree {
    let ignored_dirs = if opts.gitignored_heavy { git_ignored_dirs(root) } else { HashSet::new() };
    let mut tree = ProjectTree {
        root: root.to_path_buf(),
        dirs: vec![DirNode { rel: PathBuf::new(), parent: None, kind: DirKind::Normal }],
        files: Vec::new(),
    };
    let mut dir_ids: std::collections::HashMap<PathBuf, DirId> = std::collections::HashMap::new();
    dir_ids.insert(PathBuf::new(), 0);
    let mut heavy_paths: Vec<(DirId, PathBuf, HeavyReason)> = Vec::new();
    let mut pending_files: Vec<(PathBuf, DirId, Option<PathBuf>)> = Vec::new();

    let mut walker = WalkDir::new(root).follow_links(false).sort_by_file_name().into_iter();
    loop {
        let entry = match walker.next() {
            None => break,
            Some(Ok(e)) => e,
            Some(Err(err)) => {
                // A folder whose contents cannot be listed stays visible, marked unreadable.
                if let Some(path) = err.path() {
                    let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
                    let msg = err.io_error().map_or_else(|| err.to_string(), |e| e.to_string());
                    match dir_ids.get(&rel) {
                        Some(&id) => tree.dirs[id].kind = DirKind::Unreadable(msg),
                        None => {
                            let parent = rel.parent().and_then(|p| dir_ids.get(p)).copied().unwrap_or(0);
                            let is_dir = path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
                            if is_dir {
                                let id = tree.dirs.len();
                                tree.dirs.push(DirNode {
                                    rel: rel.clone(),
                                    parent: Some(parent),
                                    kind: DirKind::Unreadable(msg),
                                });
                                dir_ids.insert(rel, id);
                            } else {
                                tree.files.push(FileNode {
                                    rel,
                                    dir: parent,
                                    size: 0,
                                    kind: FileKind::Unreadable(msg),
                                });
                            }
                        }
                    }
                }
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        let parent = rel.parent().and_then(|p| dir_ids.get(p)).copied().unwrap_or(0);
        let file_type = entry.file_type();

        if file_type.is_dir() {
            let id = tree.dirs.len();
            let heavy = heavy_reason(path, &rel, &ignored_dirs);
            tree.dirs.push(DirNode { rel: rel.clone(), parent: Some(parent), kind: DirKind::Normal });
            dir_ids.insert(rel.clone(), id);
            if let Some(reason) = heavy {
                heavy_paths.push((id, path.to_path_buf(), reason));
                walker.skip_current_dir();
            }
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(path).unwrap_or_default();
            pending_files.push((rel, parent, Some(target)));
        } else {
            pending_files.push((rel, parent, None));
        }
    }

    // Classify files in parallel (stat + 8 KB sniff each).
    let classified: Vec<FileNode> = pending_files
        .into_par_iter()
        .map(|(rel, dir, link)| {
            let abs = root.join(&rel);
            match link {
                Some(target) => FileNode { rel, dir, size: 0, kind: FileKind::Symlink(target) },
                None => {
                    let (size, kind) = classify_file(&abs);
                    FileNode { rel, dir, size, kind }
                }
            }
        })
        .collect();
    tree.files.extend(classified);
    tree.files.sort_by(|a, b| a.rel.cmp(&b.rel));

    let totals: Vec<(DirId, HeavyInfo)> = heavy_paths
        .into_par_iter()
        .map(|(id, path, reason)| {
            let (file_count, dir_count, total_bytes) = count_subtree(&path);
            (id, HeavyInfo { reason, file_count, dir_count, total_bytes })
        })
        .collect();
    for (id, info) in totals {
        tree.dirs[id].kind = DirKind::Heavy(info);
    }
    tree
}

fn heavy_reason(path: &Path, rel: &Path, ignored_dirs: &HashSet<PathBuf>) -> Option<HeavyReason> {
    let name = rel.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if matches!(name.as_ref(), ".git" | ".hg" | ".svn") {
        return Some(HeavyReason::VersionControl);
    }
    if ignored_dirs.contains(rel) {
        return Some(HeavyReason::GitIgnored);
    }
    // Standard cache-directory marker (Cargo's target/, many build tools).
    if path.join("CACHEDIR.TAG").is_file() {
        return Some(HeavyReason::BuildCache);
    }
    if matches!(name.as_ref(), "node_modules" | "__pycache__" | ".gradle" | ".tox" | ".mypy_cache" | ".pytest_cache")
        || path.join("pyvenv.cfg").is_file()
    {
        return Some(HeavyReason::Dependencies);
    }
    None
}

/// Folders git ignores entirely (relative to `root`). Empty when git is unavailable.
fn git_ignored_dirs(root: &Path) -> HashSet<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--others", "--ignored", "--exclude-standard", "--directory", "-z"])
        .output();
    let Ok(output) = output else { return HashSet::new() };
    if !output.status.success() {
        return HashSet::new();
    }
    output
        .stdout
        .split(|&b| b == 0)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter_map(|p| p.strip_suffix('/'))
        .map(|p| p.split('/').collect::<PathBuf>())
        .collect()
}

fn classify_file(path: &Path) -> (u64, FileKind) {
    let size = match path.metadata() {
        Ok(m) => m.len(),
        Err(e) => return (0, FileKind::Unreadable(e.to_string())),
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" | "tga") {
        return (size, FileKind::Image);
    }
    if size > MAX_PARSE_BYTES {
        return (size, FileKind::TooLarge);
    }
    let mut head = [0u8; 8192];
    let read = match std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) {
        Ok(n) => n,
        Err(e) => return (size, FileKind::Unreadable(e.to_string())),
    };
    if head[..read].contains(&0) {
        (size, FileKind::Binary)
    } else {
        (size, FileKind::Text)
    }
}

/// (files, folders, bytes) under `path`, without following links. Subfolders are counted in
/// parallel, so one huge folder (node_modules, a vendored SDK) does not serialize the scan.
fn count_subtree(path: &Path) -> (u64, u64, u64) {
    let Ok(entries) = std::fs::read_dir(path) else { return (0, 0, 0) };
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(t) if t.is_dir() => subdirs.push(entry.path()),
            _ => {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    let dirs = subdirs.len() as u64;
    subdirs
        .par_iter()
        .map(|d| count_subtree(d))
        .reduce(|| (files, dirs, bytes), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "src/a/b/c/deep.rs", b"fn deep() {}");
        std::fs::create_dir_all(r.join("empty")).unwrap();
        std::fs::create_dir_all(r.join("only_dirs/x")).unwrap();
        std::fs::create_dir_all(r.join("only_dirs/y")).unwrap();
        write(r, "node_modules/pkg/index.js", b"module.exports = 1;");
        write(r, "node_modules/pkg/lib/util.js", b"x");
        write(r, "target/CACHEDIR.TAG", b"Signature: 8a477f597d28d172789f06886806bc55");
        write(r, "target/debug/app", b"\x7fELF\0\0");
        write(r, "assets/logo.png", b"\x89PNG\r\n\x1a\n\0\0");
        write(r, "assets/blob.bin", b"abc\0def");
        write(r, "data/huge.txt", &vec![b'a'; (MAX_PARSE_BYTES + 1) as usize]);
        write(r, "Cargo.lock", b"# lock");
        write(r, "app.min.js", b"var a=1;");
        dir
    }

    #[test]
    fn every_entry_is_listed_with_its_kind() {
        let dir = fixture();
        #[cfg(unix)]
        std::os::unix::fs::symlink("src", dir.path().join("link_to_src")).unwrap();
        let tree = scan_tree(dir.path(), ScanOptions::default());

        let dir_rels: Vec<String> = tree.dirs.iter().map(|d| d.rel.to_string_lossy().replace('\\', "/")).collect();
        for expected in [
            "",
            "src",
            "src/a",
            "src/a/b",
            "src/a/b/c",
            "empty",
            "only_dirs",
            "only_dirs/x",
            "only_dirs/y",
            "node_modules",
            "target",
            "assets",
            "data",
        ] {
            assert!(dir_rels.contains(&expected.to_string()), "missing dir {expected:?} in {dir_rels:?}");
        }
        assert!(!dir_rels.iter().any(|d| d.starts_with("node_modules/")), "heavy folders are not descended");

        let kind = |rel: &str| tree.files.iter().find(|f| f.rel == Path::new(rel)).map(|f| f.kind.clone());
        assert_eq!(kind("src/a/b/c/deep.rs"), Some(FileKind::Text));
        assert_eq!(kind("assets/logo.png"), Some(FileKind::Image));
        assert_eq!(kind("assets/blob.bin"), Some(FileKind::Binary));
        assert_eq!(kind("data/huge.txt"), Some(FileKind::TooLarge));
        assert_eq!(kind("Cargo.lock"), Some(FileKind::Text));
        assert_eq!(kind("app.min.js"), Some(FileKind::Text));
        #[cfg(unix)]
        assert_eq!(kind("link_to_src"), Some(FileKind::Symlink(PathBuf::from("src"))));

        let parent_of = |rel: &str| {
            let d = tree.dirs.iter().find(|d| d.rel == Path::new(rel)).unwrap();
            tree.dirs[d.parent.unwrap()].rel.clone()
        };
        assert_eq!(parent_of("src/a/b"), PathBuf::from("src/a"));
        assert_eq!(parent_of("only_dirs/x"), PathBuf::from("only_dirs"));
    }

    #[test]
    fn heavy_folders_carry_counts() {
        let dir = fixture();
        let tree = scan_tree(dir.path(), ScanOptions::default());
        let heavy: Vec<(String, HeavyReason, u64)> = tree
            .heavy_dirs()
            .map(|(_, d, info)| (d.rel.to_string_lossy().to_string(), info.reason, info.file_count))
            .collect();
        assert!(heavy.contains(&("node_modules".to_string(), HeavyReason::Dependencies, 2)), "{heavy:?}");
        assert!(heavy.contains(&("target".to_string(), HeavyReason::BuildCache, 2)), "{heavy:?}");
    }

    #[test]
    fn gitignored_folders_are_heavy_when_git_is_available() {
        let dir = fixture();
        write(dir.path(), ".gitignore", b"secret/\n");
        write(dir.path(), "secret/key.txt", b"k");
        let git_ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q"])
            .status()
            .is_ok_and(|s| s.success());
        if !git_ok {
            return;
        }
        let tree = scan_tree(dir.path(), ScanOptions::default());
        let secret = tree.dirs.iter().find(|d| d.rel == Path::new("secret")).unwrap();
        assert!(matches!(&secret.kind, DirKind::Heavy(i) if i.reason == HeavyReason::GitIgnored && i.file_count == 1));
        let git = tree.dirs.iter().find(|d| d.rel == Path::new(".git")).unwrap();
        assert!(matches!(&git.kind, DirKind::Heavy(i) if i.reason == HeavyReason::VersionControl));

        let inside = scan_tree(&dir.path().join("secret"), ScanOptions { gitignored_heavy: false });
        assert_eq!(inside.files.len(), 1, "materializing an ignored folder lists its contents");
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_folders_stay_visible() {
        use std::os::unix::fs::PermissionsExt;
        let dir = fixture();
        let locked = dir.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("x.txt"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let tree = scan_tree(dir.path(), ScanOptions::default());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let node = tree.dirs.iter().find(|d| d.rel == Path::new("locked")).expect("locked dir listed");
        // Root can read anything; only assert when the permission actually applied.
        if std::fs::read_dir("/root").is_err() {
            assert!(matches!(node.kind, DirKind::Unreadable(_)), "{:?}", node.kind);
        }
    }
}
