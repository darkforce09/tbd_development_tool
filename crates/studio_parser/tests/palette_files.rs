//! E4: the file segment lists what the disk scan sees (D1): every file except single files git
//! ignores, and one entry per heavy or ignored folder. It is the same whether the project loads
//! fresh or from the cache.

use std::path::Path;
use std::sync::mpsc::channel;

use studio_parser::search_index::FILES_SLOT;
use studio_parser::{
    build_skeleton_files_graph, clear_project_cache, scan_project, spawn_load_project_opt, tree, Entry, EntryKind,
    FileListing, LoaderMessage, SearchIndex,
};

fn write(root: &Path, rel: &str, content: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// A git project with an ignored folder, an ignored file in the root and one in a folder, a
/// dependency folder, a build cache, and untracked files. `None` when git is unavailable.
fn fixture() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".gitignore", b"secret/\n*.log\n");
    write(r, "secret/key.txt", b"k");
    write(r, "debug.log", b"log");
    write(r, "src/gen.log", b"log");
    write(r, "node_modules/.package-lock.json", b"{}");
    write(r, "node_modules/pkg/index.js", b"export const a = 1;\n");
    write(r, "target/CACHEDIR.TAG", b"Signature: 8a477f597d28d172789f06886806bc55");
    write(r, "target/debug/app", b"\x7fELF\0\0");
    write(r, "src/main.rs", b"fn main() {}\n");
    write(r, "src/ui/hud.rs", b"pub fn render_hud() {}\n");
    write(r, "README.md", b"# Demo\n");
    let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(r).args(args).output();
    if !git(&["init", "-q"]).is_ok_and(|o| o.status.success()) {
        return None;
    }
    // One tracked file; the rest stay untracked.
    git(&["add", "src/main.rs"]).ok()?;
    Some(dir)
}

fn shown(entries: &[Entry]) -> Vec<(EntryKind, &str, &str)> {
    entries.iter().map(|e| (e.kind, e.name.as_str(), e.detail.as_str())).collect()
}

/// The `Complete` index of one loader run.
fn loaded_index(root: &Path, force_reparse: bool) -> (SearchIndex, bool) {
    let (tx, rx) = channel();
    spawn_load_project_opt(root.to_path_buf(), force_reparse, tx).join().expect("loader thread");
    rx.try_iter()
        .find_map(|m| match m {
            LoaderMessage::Complete { search_index, from_cache, .. } => Some((search_index, from_cache)),
            LoaderMessage::Error(e) => panic!("{e}"),
            _ => None,
        })
        .expect("complete message")
}

#[test]
fn the_file_segment_lists_files_and_stands_in_for_heavy_and_ignored_folders() {
    let Some(dir) = fixture() else { return };
    let root = dir.path().canonicalize().unwrap();
    let scanned = tree::scan_tree(&root, Default::default());
    let segment = studio_parser::file_segment(&FileListing::from_tree(&scanned, ""));
    assert_eq!(
        shown(segment.entries()),
        [
            (EntryKind::File, ".gitignore", ".gitignore"),
            (EntryKind::File, "README.md", "README.md"),
            (EntryKind::File, "main.rs", "src/main.rs"),
            (EntryKind::File, "hud.rs", "src/ui/hud.rs"),
            (EntryKind::Folder, ".git", ".git · version control"),
            (EntryKind::Folder, "node_modules", "node_modules · dependencies"),
            (EntryKind::Folder, "secret", "secret · gitignored"),
            (EntryKind::Folder, "target", "target · build cache"),
        ]
    );
    // The map still shows the ignored files; only the palette leaves them out.
    let on_map: Vec<_> = scanned.files.iter().map(|f| f.rel.to_string_lossy().replace('\\', "/")).collect();
    assert!(on_map.contains(&"debug.log".to_string()) && on_map.contains(&"src/gen.log".to_string()));
}

#[test]
fn the_cache_path_rebuilds_the_same_file_segment() {
    let Some(dir) = fixture() else { return };
    let root = dir.path().canonicalize().unwrap();

    // From the graph of the same scan, without the scan.
    let scanned = scan_project(&root).unwrap();
    let (graph, _) = build_skeleton_files_graph(&scanned);
    let (_, ignored) = tree::git_ignored(&root);
    assert_eq!(FileListing::from_graph(&graph, &ignored), FileListing::from_tree(&scanned.tree, ""));

    // Through the loader: a fresh parse, then a load from the cache it saved.
    let (fresh, fresh_cached) = loaded_index(&root, true);
    let (cached, from_cache) = loaded_index(&root, false);
    clear_project_cache(&root).unwrap();
    assert!(!fresh_cached && from_cache, "the second load reads the cache");
    let files = |index: &SearchIndex| index.segments()[FILES_SLOT].entries().to_vec();
    assert_eq!(files(&cached), files(&fresh));
    assert_eq!(files(&fresh).len(), 8);
    let symbols = |index: &SearchIndex| index.segments()[1].entries().to_vec();
    assert_eq!(symbols(&cached), symbols(&fresh), "symbols too");
    assert!(symbols(&fresh).iter().any(|e| e.name == "render_hud" && e.detail == "src/ui/hud.rs:1"));
}
