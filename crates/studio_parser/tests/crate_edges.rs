//! Manifest wires on the Code map: exact package-to-package wires on the `crate_edges` fixture (`expected.toml`
//! beside it), and the same wires after a cache round trip.

use std::path::{Path, PathBuf};

use studio_graph::{Basis, EdgeKind, Graph};
use studio_parser::analysis::crate_graph::{path_dependencies, DepKind};
use studio_parser::{clear_project_cache, load_project_cache, load_rust_project, save_project_cache};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/crate_edges").canonicalize().unwrap()
}

fn expected() -> toml::Table {
    let text = std::fs::read_to_string(fixture_root().join("expected.toml")).unwrap();
    text.parse().unwrap()
}

fn count(expected: &toml::Table, key: &str) -> usize {
    expected["counts"][key].as_integer().unwrap() as usize
}

/// Every manifest wire as "<tier> <basis> <provider> -> <consumer>:<line>", sorted.
fn manifest_wires(graph: &Graph, root: &Path) -> Vec<String> {
    let rel = |node| {
        let path = graph.nodes[&node].file_path.as_deref().unwrap();
        Path::new(path).strip_prefix(root).unwrap().display().to_string()
    };
    let mut out: Vec<String> = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Depends)
        .map(|e| {
            let basis = format!("{:?}", e.provenance.basis).to_lowercase();
            let line = e.source_line.map_or("?".to_string(), |l| l.to_string());
            format!("{} {basis} {} -> {}:{line}", e.provenance.tier.label(), rel(e.from_node), rel(e.to_node))
        })
        .collect();
    out.sort();
    out
}

fn expected_wires(expected: &toml::Table) -> Vec<String> {
    let want: Vec<String> =
        expected["wires"]["all"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let mut sorted = want.clone();
    sorted.sort();
    assert_eq!(want, sorted, "expected.toml lists the wires sorted");
    want
}

#[test]
fn manifest_wires_are_exact() {
    let root = fixture_root();
    let expected = expected();
    let (graph, stats) = load_rust_project(&root).unwrap();
    // Every wire listed is on the map and nothing else is (100% precision and recall).
    assert_eq!(manifest_wires(&graph, &root), expected_wires(&expected));
    assert_eq!(stats.r1.manifest_edges, count(&expected, "manifest_edges"));
    for e in graph.edges.iter().filter(|e| e.kind == EdgeKind::Depends) {
        assert_eq!(e.provenance.basis, Basis::Manifest);
        assert!(e.label.is_none(), "manifest wires carry no label");
    }
    // Manifest cards carry no other wire.
    for e in graph.edges.iter().filter(|e| e.kind != EdgeKind::Depends) {
        for node in [e.from_node, e.to_node] {
            let path = graph.nodes[&node].file_path.as_deref().unwrap_or_default();
            assert!(!path.ends_with("Cargo.toml"), "{:?} wire on a manifest card", e.kind);
        }
    }
}

#[test]
fn path_dependencies_count_every_table() {
    let root = fixture_root();
    let expected = expected();
    let manifests: Vec<PathBuf> =
        ["", "core", "model", "render", "app"].iter().map(|d| root.join(d).join("Cargo.toml")).collect();
    let deps = path_dependencies(&root, &manifests);
    assert_eq!(deps.len(), count(&expected, "path_deps"));
    assert_eq!(deps.iter().filter(|d| d.kind == DepKind::Dev).count(), count(&expected, "dev_deps"));
    assert!(deps.iter().all(|d| d.line.is_some()), "every entry has its line");
}

#[test]
fn manifest_wires_survive_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("crate_edges");
    copy_dir(&fixture_root(), &root);
    let root = root.canonicalize().unwrap();
    let (graph, stats) = load_rust_project(&root).unwrap();
    save_project_cache(&root, &graph, &stats).unwrap();
    let loaded = load_project_cache(&root).unwrap();
    clear_project_cache(&root).unwrap();
    let (cached, cached_stats) = loaded.expect("the cache was just written");
    assert_eq!(manifest_wires(&cached, &root), manifest_wires(&graph, &root));
    assert_eq!(cached_stats.r1.manifest_edges, stats.r1.manifest_edges);
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Manifest wires on real repositories: run with `--ignored --nocapture` and `STUDIO_REAL_REPOS` (colon-separated).
#[test]
#[ignore]
fn manifest_wires_on_real_repos() {
    let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else {
        eprintln!("STUDIO_REAL_REPOS unset: skipped");
        return;
    };
    for root in repos.split(':').map(PathBuf::from) {
        let (graph, stats) = load_rust_project(&root).unwrap();
        let wires = graph.edges.iter().filter(|e| e.kind == EdgeKind::Depends).count();
        assert_eq!(wires, stats.r1.manifest_edges);
        eprintln!("{}: {wires} manifest wires", root.display());
    }
}
