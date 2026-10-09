pub mod builder;
pub mod cache;
pub mod edit;
pub mod extractor;
pub mod project;
pub mod search_index;
pub mod sync;
pub mod tree;

pub use builder::{
    build_files_graph, build_project_graph, build_skeleton_files_graph, folder_cluster_id, materialize_folder,
    ProjectStats,
};
pub use cache::{
    clear_project_cache, load_project_cache, save_project_cache, user_cache_dir_for_project, CachedProjectData,
};
pub use edit::{apply_edit, atomic_write, content_hash, EditError, EditOrigin};
pub use extractor::{
    extract_file, extract_project, EnumItem, ExtractedCrate, ExtractedFile, ExtractedProject, FieldInfo, FunctionItem,
    ImplItem, ItemVisibility, ParamInfo, StructItem, TraitItem, UseItem,
};
pub use project::{scan_project, CrateInfo, ProjectError, RustProject};
pub use search_index::{SearchItem, SymbolSearchIndex};
pub use sync::{save_and_reparse, SaveReport};

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use studio_graph::Graph;

#[derive(Debug, Clone)]
pub enum LoaderMessage {
    Progress { stage: String, files_done: usize, total_files: usize, percentage: f32 },
    InitialLayoutReady { graph: Graph, stats: ProjectStats, search_index: SymbolSearchIndex },
    Complete { graph: Graph, stats: ProjectStats, search_index: SymbolSearchIndex, from_cache: bool },
    Error(String),
}

/// Scans a collapsed folder and parses its text files, for [`materialize_folder`]. Runs off the UI
/// thread; heavy folders nested inside stay collapsed, but git-ignore status is not re-applied
/// (everything inside an ignored folder is ignored).
pub fn load_folder_contents(folder: &Path) -> (tree::ProjectTree, Vec<extractor::ExtractedFile>) {
    use rayon::prelude::*;
    let tree = tree::scan_tree(folder, tree::ScanOptions { gitignored_heavy: false });
    let text: Vec<PathBuf> = tree.text_files().collect();
    let parsed = text.par_iter().map(|p| extract_file(p, p.strip_prefix(folder).unwrap_or(p))).collect();
    (tree, parsed)
}

/// Convenience function to scan, extract, and build a Graph for a Rust project at given path.
pub fn load_rust_project(path: impl AsRef<Path>) -> Result<(Graph, ProjectStats), ProjectError> {
    let scanned = scan_project(path)?;
    let extracted = extract_project(&scanned);
    let (graph, stats) = build_project_graph(&extracted);
    Ok((graph, stats))
}

/// Spawns a background worker thread that extracts and builds the graph asynchronously,
/// streaming non-blocking progress messages to the UI thread.
/// Checks and populates user rkyv cache by default.
pub fn spawn_load_project(path: PathBuf, tx: Sender<LoaderMessage>) -> std::thread::JoinHandle<()> {
    spawn_load_project_opt(path, false, tx)
}

/// Spawns a background worker thread with an option to force re-parsing (bypassing cache).
pub fn spawn_load_project_opt(
    path: PathBuf,
    force_reparse: bool,
    tx: Sender<LoaderMessage>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // 1. Check user cache if force_reparse is false
        if !force_reparse {
            let _ = tx.send(LoaderMessage::Progress {
                stage: "Checking user rkyv cache...".to_string(),
                files_done: 0,
                total_files: 1,
                percentage: 0.10,
            });

            match cache::load_project_cache(&path) {
                Ok(Some((graph, stats))) => {
                    let _ = tx.send(LoaderMessage::Progress {
                        stage: "Instant zero-copy load from user rkyv cache...".to_string(),
                        files_done: stats.file_count,
                        total_files: stats.file_count,
                        percentage: 0.90,
                    });

                    let search_index = SymbolSearchIndex::build(&graph);

                    let _ = tx.send(LoaderMessage::Complete { graph, stats, search_index, from_cache: true });
                    return;
                }
                Ok(None) => {} // Cache miss or stale -> proceed to fresh parse
                Err(e) => {
                    eprintln!("Cache read error (falling back to parse): {}", e);
                }
            }
        }

        let _ = tx.send(LoaderMessage::Progress {
            stage: "Scanning workspace manifests and source files...".to_string(),
            files_done: 0,
            total_files: 0,
            percentage: 0.05,
        });

        let scanned = match scan_project(&path) {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(LoaderMessage::Error(format!("Failed to scan project: {}", e)));
                return;
            }
        };

        // Tier 1 instant layout: immediately emit skeleton files & clusters (< 100ms)
        let (skeleton_graph, skeleton_stats) = builder::build_skeleton_files_graph(&scanned);
        let search_index = SymbolSearchIndex::build(&skeleton_graph);
        let _ =
            tx.send(LoaderMessage::InitialLayoutReady { graph: skeleton_graph, stats: skeleton_stats, search_index });

        let total_files = scanned.total_files();
        let _ = tx.send(LoaderMessage::Progress {
            stage: format!(
                "Parsing {} source files across {} crates in parallel...",
                total_files,
                scanned.crates.len()
            ),
            files_done: 0,
            total_files,
            percentage: 0.20,
        });

        let extracted = extract_project(&scanned);

        let _ = tx.send(LoaderMessage::Progress {
            stage: "Constructing visual architecture graph & indices...".to_string(),
            files_done: total_files,
            total_files,
            percentage: 0.85,
        });

        let (graph, stats) = build_project_graph(&extracted);

        let _ = tx.send(LoaderMessage::Progress {
            stage: "Building in-memory Trigram search index...".to_string(),
            files_done: total_files,
            total_files,
            percentage: 0.95,
        });

        let search_index = SymbolSearchIndex::build(&graph);

        // Send Complete immediately so UI displays rich nodes & wires with zero lag
        let _ = tx.send(LoaderMessage::Complete {
            graph: graph.clone(),
            stats: stats.clone(),
            search_index,
            from_cache: false,
        });

        // Save to user rkyv cache asynchronously in background thread
        let save_root = path.clone();
        std::thread::spawn(move || {
            if let Err(e) = cache::save_project_cache(&save_root, &graph, &stats) {
                eprintln!("Failed to save project cache: {}", e);
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_crate() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (graph, stats) = load_rust_project(manifest_dir).expect("Failed to load studio_parser crate");

        assert!(!graph.nodes.is_empty(), "Graph should contain nodes");
        assert_eq!(stats.crate_count, 1);
        assert!(stats.file_count >= 5, "Should find at least 5 files");
        assert!(stats.node_count > 20, "Should extract nodes");
        assert!(stats.wire_count > 10, "Should connect wires");

        let index = SymbolSearchIndex::build(&graph);
        let results = index.search("parse", 10);
        assert!(!results.is_empty(), "Search should find symbols");
    }

    #[test]
    fn test_parse_workspace_root() {
        let workspace_root =
            Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("Workspace root should exist");
        let (graph, stats) = load_rust_project(workspace_root).expect("Failed to load workspace");

        assert!(!graph.nodes.is_empty(), "Graph should contain nodes");
        assert!(stats.crate_count >= 4, "Should detect workspace crates");
        assert!(stats.file_count >= 10, "Should detect source files");
        assert!(stats.node_count > 50, "Node count should be substantial");
    }

    #[test]
    fn test_save_and_reparse() {
        use std::io::Write;
        let temp_dir_guard = tempfile::tempdir().unwrap();
        let temp_dir = temp_dir_guard.path().to_path_buf();
        let test_file = temp_dir.join("sample.rs");

        let initial_code = "pub fn add_numbers(a: i32, b: i32) -> i32 { a + b }";
        let mut file = std::fs::File::create(&test_file).unwrap();
        file.write_all(initial_code.as_bytes()).unwrap();

        let mut graph = Graph::new();
        let node_id = graph.add_node(
            "sample.rs",
            studio_graph::NodeArchetype::File,
            "sample",
            None,
            vec![],
            vec![],
            [100.0, 100.0],
        );
        graph.set_node_metadata(
            node_id,
            Some(test_file.to_string_lossy().to_string()),
            Some(1),
            None,
            None,
            None,
            Some(initial_code.to_string()),
            None,
        );

        let updated_code = "pub fn add_numbers(a: i32, b: i32) -> i32 { a + b + 10 }";
        let result = save_and_reparse(&test_file, updated_code, &mut graph);
        assert!(result.is_ok(), "save_and_reparse should succeed: {:?}", result.as_ref().err());
        assert_eq!(result.unwrap().updated_nodes, 1, "Should update 1 node");

        let updated_node = graph.nodes.get(&node_id).unwrap();
        assert!(updated_node.source_code.as_ref().unwrap().contains("10"), "Node source code should be hot-reloaded");

        let disk_content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(disk_content, updated_code, "Disk file must match saved content");
    }

    /// (member id, visibility, signature, line) per member, and (caller, callee) member ids per call edge.
    type CardShape = (Vec<(String, String, String, usize)>, Vec<(String, String)>);

    /// Member list + call edges of a file card, by name, for comparing a saved graph with a fresh load.
    fn file_card_shape(graph: &Graph, title: &str) -> CardShape {
        let node = graph.nodes.values().find(|n| n.title == title).expect("file card");
        let members = node
            .member_nodes
            .iter()
            .map(|m| (m.id.clone(), m.visibility.clone(), m.signature.clone(), m.line_number))
            .collect();
        let port_owner = |port: studio_graph::PortId| {
            graph
                .nodes
                .values()
                .flat_map(|n| n.member_nodes.iter())
                .find(|m| m.in_port_id == Some(port) || m.out_port_id == Some(port))
                .map(|m| m.id.clone())
        };
        let mut calls: Vec<(String, String)> = graph
            .edges
            .iter()
            .filter(|e| e.from_node == node.id)
            .filter_map(|e| Some((port_owner(e.from_port)?, port_owner(e.to_port)?)))
            .collect();
        calls.sort();
        (members, calls)
    }

    #[test]
    fn test_save_matches_fresh_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "pub fn beta() {}\npub fn delta() {}\n").unwrap();
        let a = dir.path().join("src/a.rs");
        std::fs::write(&a, "pub fn alpha() {\n    beta();\n}\n").unwrap();

        let (mut graph, _) = load_rust_project(dir.path()).unwrap();
        let edited = "/// doc\npub fn gamma() {\n    delta();\n}\n\nfn alpha() {\n    beta();\n    delta();\n}\n\nimpl S {\n    fn m(&self) { beta(); }\n}\n";
        save_and_reparse(&a, edited, &mut graph).unwrap();

        let (fresh, _) = load_rust_project(dir.path()).unwrap();
        let saved_shape = file_card_shape(&graph, "a.rs");
        assert_eq!(saved_shape, file_card_shape(&fresh, "a.rs"));
        assert!(saved_shape.1.iter().any(|(_, to)| to == "fn:delta"), "new call wired: {saved_shape:?}");
        assert!(saved_shape.0.iter().any(|m| m.0 == "method:S::m"), "impl methods are kept: {saved_shape:?}");

        let card = graph.nodes.values().find(|n| n.title == "a.rs").unwrap();
        let port_ids: std::collections::HashSet<_> =
            card.inputs.iter().chain(card.outputs.iter()).map(|p| p.id).collect();
        for m in &card.member_nodes {
            assert!(port_ids.contains(&m.in_port_id.unwrap()) && port_ids.contains(&m.out_port_id.unwrap()));
        }
        assert_eq!(card.inputs.len(), card.member_nodes.len() + 1, "stale member ports removed");
    }

    #[test]
    fn test_files_view_shows_every_file_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        let write = |rel: &str, content: &[u8]| {
            let p = r.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        };
        write("src/main.py", b"from lib.util import helper\n\ndef main():\n    helper()\n");
        write("src/lib/util.py", b"def helper():\n    pass\n");
        write("assets/logo.png", b"\x89PNG\r\n\x1a\n\0\0");
        write("assets/blob.bin", b"abc\0def");
        write("node_modules/pkg/index.js", b"export function dep() {}\n");
        write("node_modules/pkg/package.json", b"{}");
        std::fs::create_dir_all(r.join("empty")).unwrap();
        std::fs::create_dir_all(r.join("nest/only/dirs")).unwrap();

        let (mut graph, stats) = load_rust_project(r).unwrap();
        let scanned = tree::scan_tree(&r.canonicalize().unwrap(), tree::ScanOptions::default());
        assert_eq!(stats.file_count, scanned.files.len());

        // Every folder is a cluster under its real parent; every file is a card in its folder.
        for d in &scanned.dirs {
            let id = folder_cluster_id(&d.rel);
            let cluster = graph.clusters.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no cluster {id}"));
            let parent = d.parent.map(|p| folder_cluster_id(&scanned.dirs[p].rel));
            assert_eq!(cluster.parent_id, parent, "{id}");
            assert!(cluster.size[0] > 1.0 && cluster.size[1] > 1.0, "{id} must be visible");
        }
        for f in &scanned.files {
            let node = graph
                .nodes
                .values()
                .find(|n| n.file_path.as_deref().map(Path::new) == Some(&scanned.root.join(&f.rel)));
            let node = node.unwrap_or_else(|| panic!("no card for {:?}", f.rel));
            assert_eq!(node.group_id.as_deref(), Some(folder_cluster_id(f.rel.parent().unwrap()).as_str()));
        }
        let card_in = |g: &Graph, name: &str| g.nodes.values().find(|n| n.title == name).unwrap().clone();
        let card = |name: &str| card_in(&graph, name);
        assert_eq!(card("logo.png").content, studio_graph::FileContent::Image);
        assert_eq!(card("blob.bin").content, studio_graph::FileContent::Binary);
        assert!(!card("main.py").member_nodes.is_empty(), "text files are parsed");
        assert!(!graph.edges.is_empty(), "imports are wired");

        // node_modules is a collapsed placeholder with totals and no cards yet.
        let nm = graph.clusters.iter().find(|c| c.id == "dir:node_modules").unwrap().clone();
        assert!(nm.is_collapsed);
        assert_eq!(nm.lazy.as_ref().map(|l| l.file_count), Some(2));
        assert!(nm.node_ids.is_empty() && nm.child_cluster_ids.is_empty());
        assert!(graph.clusters.iter().find(|c| c.id == "dir:empty").unwrap().subtitle.as_deref() == Some("empty"));

        // Expanding it loads its files, parsed, under the right folders.
        let (subtree, parsed) = load_folder_contents(Path::new(&nm.lazy.unwrap().abs_path));
        let added = materialize_folder(&mut graph, "dir:node_modules", &subtree, &parsed).unwrap();
        assert_eq!(added, 2);
        let nm = graph.clusters.iter().find(|c| c.id == "dir:node_modules").unwrap();
        assert!(!nm.is_collapsed && nm.lazy.is_none());
        let pkg = graph.clusters.iter().find(|c| c.id == "dir:node_modules/pkg").expect("nested folder");
        assert_eq!(pkg.parent_id.as_deref(), Some("dir:node_modules"));
        assert_eq!(pkg.node_ids.len(), 2);
        assert!(!card_in(&graph, "index.js").member_nodes.is_empty());
    }

    #[test]
    fn test_spawn_load_project() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = spawn_load_project(manifest_dir, tx);
        handle.join().expect("Thread should join");

        let mut got_progress = false;
        let mut got_complete = false;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LoaderMessage::Progress { .. } => got_progress = true,
                LoaderMessage::InitialLayoutReady { .. } => {}
                LoaderMessage::Complete { graph, stats, .. } => {
                    got_complete = true;
                    assert!(!graph.nodes.is_empty());
                    assert!(stats.file_count >= 5);
                }
                LoaderMessage::Error(e) => panic!("Loader error: {}", e),
            }
        }

        assert!(got_progress, "Should have received progress message");
        assert!(got_complete, "Should have received completion message");
    }

    #[test]
    fn test_spawn_load_project_initial_layout() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = spawn_load_project_opt(manifest_dir, true, tx);
        handle.join().expect("Thread should join");

        let mut got_initial_layout = false;
        let mut got_complete = false;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LoaderMessage::InitialLayoutReady { graph, stats, .. } => {
                    got_initial_layout = true;
                    assert!(!graph.nodes.is_empty());
                    assert!(stats.file_count >= 5);
                }
                LoaderMessage::Complete { .. } => got_complete = true,
                LoaderMessage::Progress { .. } => {}
                LoaderMessage::Error(e) => panic!("Loader error: {}", e),
            }
        }

        assert!(got_initial_layout, "Should have received initial layout message");
        assert!(got_complete, "Should have received completion message");
    }

    #[test]
    fn test_parse_files_and_folders_view() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (graph, stats) = load_rust_project(manifest_dir).expect("Should load files and folders");

        assert!(!graph.nodes.is_empty(), "Should contain file nodes");
        assert!(stats.file_count >= 5, "Should have scanned files");

        // Every node must be NodeArchetype::File
        for node in graph.nodes.values() {
            assert_eq!(node.archetype, studio_graph::NodeArchetype::File);
            assert_eq!(node.size, [220.0, 42.0]);
            assert!(node.file_path.is_some());
        }

        // Must have created hierarchical directory clusters
        assert!(!graph.clusters.is_empty(), "Should have directory and crate clusters");
        assert!(graph.clusters.iter().all(|c| c.category == "Folder"));
        let src = graph.clusters.iter().find(|c| c.id == "dir:src").expect("src folder");
        assert_eq!(src.parent_id.as_deref(), Some("dir:."), "folders nest under their real parent");
        let builder = graph.clusters.iter().find(|c| c.id == "dir:src/builder").expect("nested folder");
        assert_eq!(builder.parent_id.as_deref(), Some("dir:src"));

        // Verify member nodes are extracted
        let total_members: usize = graph.nodes.values().map(|n| n.member_nodes.len()).sum();
        assert!(total_members > 0, "File nodes must contain extracted member nodes");
        let sample_node_with_members = graph.nodes.values().find(|n| !n.member_nodes.is_empty()).unwrap();
        assert!(sample_node_with_members.member_nodes[0].line_number > 0);
        assert!(!sample_node_with_members.member_nodes[0].name.is_empty());
    }

    #[test]
    fn test_parse_mixed_project_with_markdown_and_enforce_script() {
        let temp_dir_guard = tempfile::tempdir().unwrap();
        let temp_dir = temp_dir_guard.path().to_path_buf();
        let _ = std::fs::create_dir_all(temp_dir.join("scripts"));

        let readme_path = temp_dir.join("README.md");
        let readme_content = r#"# Map Program Hub
Overview of the project.

## Slice specs (read these - not optional)
| Slice | Spec |
| **T-090.0** | ['satellite.md'](scripts/satellite.md) |
| **T-090.1** | [Engine](scripts/engine.c) |
"#;
        std::fs::write(&readme_path, readme_content).unwrap();

        let engine_path = temp_dir.join("scripts").join("engine.c");
        let engine_content = r#"
modded class DayZGameEngine
{
    override void OnInit()
    {
        super.OnInit();
        Print("Engine initialized.");
    }
}
"#;
        std::fs::write(&engine_path, engine_content).unwrap();

        let sat_path = temp_dir.join("scripts").join("satellite.md");
        let sat_content = r#"# Satellite Specs
Details for satellite.
Link back: [Hub](../README.md)
"#;
        std::fs::write(&sat_path, sat_content).unwrap();

        let (graph, stats) = load_rust_project(&temp_dir).expect("Should load mixed project");

        assert_eq!(stats.file_count, 3, "Should detect 3 files");
        assert_eq!(graph.nodes.len(), 3, "Should create 3 file cards");

        // Verify file badges
        let readme_node = graph.nodes.values().find(|n| n.title == "README.md").unwrap();
        assert_eq!(readme_node.badge.as_deref(), Some("MD"));

        let engine_node = graph.nodes.values().find(|n| n.title == "engine.c").unwrap();
        assert_eq!(engine_node.badge.as_deref(), Some("ENS"));

        let sat_node = graph.nodes.values().find(|n| n.title == "satellite.md").unwrap();
        assert_eq!(sat_node.badge.as_deref(), Some("MD"));

        // Verify member nodes: headings, links, modded classes
        let readme_members = &readme_node.member_nodes;
        assert!(readme_members.iter().any(|m| m.visibility.starts_with('H')));
        assert!(readme_members.iter().any(|m| m.visibility == "LNK"));

        let engine_members = &engine_node.member_nodes;
        assert!(engine_members.iter().any(|m| m.visibility == "MOD"));

        // Verify wires exist connecting markdown links to destination files
        assert!(!graph.edges.is_empty(), "Should connect wires between linked files");
    }

    #[test]
    fn test_save_and_reparse_markdown_and_enforce() {
        let temp_dir_guard = tempfile::tempdir().unwrap();
        let temp_dir = temp_dir_guard.path().to_path_buf();

        let md_file = temp_dir.join("spec.md");
        let initial_md = "# Initial Spec\n[Target](target.md)\n";
        std::fs::write(&md_file, initial_md).unwrap();

        let mut graph = Graph::new();
        let nid = graph.add_node(
            "spec.md",
            studio_graph::NodeArchetype::File,
            "spec",
            Some("MD".into()),
            vec![],
            vec![],
            [0.0, 0.0],
        );
        graph.set_node_metadata(
            nid,
            Some(md_file.to_string_lossy().to_string()),
            Some(1),
            None,
            None,
            None,
            Some(initial_md.into()),
            None,
        );

        let updated_md = "# Updated Spec\n## New Heading\n";
        let res = save_and_reparse(&md_file, updated_md, &mut graph);
        assert!(res.is_ok(), "Should reparse markdown without error");
        assert_eq!(res.unwrap().updated_nodes, 1);

        let updated_node = graph.nodes.get(&nid).unwrap();
        assert_eq!(updated_node.source_code.as_deref(), Some(updated_md));
        assert!(updated_node.member_nodes.iter().any(|m| m.name == "Updated Spec"));
        assert!(updated_node.member_nodes.iter().any(|m| m.name == "New Heading"));
    }
}
