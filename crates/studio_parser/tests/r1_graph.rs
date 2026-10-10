//! R1 on the Code map: exact wires and counters on the `r1_graph` fixture (`expected.toml` beside it).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use studio_graph::{EdgeKind, Graph, PortId};
use studio_parser::load_rust_project;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r1_graph").canonicalize().unwrap()
}

fn expected() -> toml::Table {
    let text = std::fs::read_to_string(fixture_root().join("expected.toml")).unwrap();
    text.parse().unwrap()
}

/// Every code wire as "<tier> <kind> <provider> -> <consumer>", sorted.
fn wires(graph: &Graph, root: &Path) -> Vec<String> {
    let mut member_of: HashMap<PortId, String> = HashMap::new();
    for node in graph.nodes.values() {
        for m in &node.member_nodes {
            for port in [m.in_port_id, m.out_port_id].into_iter().flatten() {
                member_of.insert(port, m.name.clone());
            }
        }
    }
    let side = |node, port| {
        let n = &graph.nodes[&node];
        let file = Path::new(n.file_path.as_deref().unwrap()).strip_prefix(root).unwrap().display().to_string();
        match member_of.get(&port) {
            Some(member) => format!("{file}#{member}"),
            None => file,
        }
    };
    let mut out: Vec<String> = graph
        .edges
        .iter()
        .filter(|e| e.kind != EdgeKind::Documentation)
        .map(|e| {
            format!(
                "{} {} {} -> {}",
                e.provenance.tier.label(),
                format!("{:?}", e.kind).to_lowercase(),
                side(e.from_node, e.from_port),
                side(e.to_node, e.to_port)
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn r1_wires_and_counters_are_exact() {
    let root = fixture_root();
    let (graph, stats) = load_rust_project(&root).unwrap();
    let expected = expected();

    let want: Vec<String> =
        expected["wires"]["all"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let mut sorted_want = want.clone();
    sorted_want.sort();
    assert_eq!(want, sorted_want, "expected.toml lists the wires sorted");
    // Every wire on the map is listed (precision) and every listed wire is on the map (recall).
    assert_eq!(wires(&graph, &root), want);
    for e in &graph.edges {
        if e.provenance.tier == studio_graph::EvidenceTier::Proven && e.kind != EdgeKind::Documentation {
            assert_eq!(e.provenance.basis, studio_graph::Basis::PathResolution);
        }
    }

    let counts = expected["counts"].as_table().unwrap();
    let c = |key: &str| counts[key].as_integer().unwrap() as usize;
    let r1 = &stats.r1;
    let got = [
        ("files", r1.files),
        ("upgraded", r1.upgraded),
        ("retargeted", r1.retargeted),
        ("added", r1.added),
        ("dropped", r1.dropped),
        ("external", r1.external),
        ("unresolved", r1.unresolved),
        ("cfg_gated", r1.cfg_gated),
        ("proven_edges", r1.proven_edges),
    ];
    for (key, value) in got {
        assert_eq!(value, c(key), "counter {key}");
    }
    assert_eq!(counts.len(), got.len(), "every counter in expected.toml is checked");
}
