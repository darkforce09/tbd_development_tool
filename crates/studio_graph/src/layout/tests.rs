use super::*;
use crate::{DataType, EdgeKind, GroupCluster, NodeArchetype};

/// Small deterministic generator so the property tests need no extra dependency.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn card(g: &mut Graph, name: &str, folder: &str) -> NodeId {
    let id = g.add_node(
        name,
        NodeArchetype::File,
        "",
        None,
        vec![("in".into(), DataType::RustFlow), ("docs".into(), DataType::Documentation)],
        vec![("out".into(), DataType::RustFlow)],
        [0.0, 0.0],
    );
    let n = g.nodes.get_mut(&id).unwrap();
    n.size = [220.0, 42.0];
    n.group_id = Some(folder.to_string());
    id
}

fn wire(g: &mut Graph, provider: NodeId, consumer: NodeId, kind: EdgeKind) {
    let out = g.nodes[&provider].outputs[0].id;
    let input = if kind == EdgeKind::Documentation {
        g.nodes[&consumer].doc_port().unwrap()
    } else {
        g.nodes[&consumer].inputs[0].id
    };
    g.connect_kind(provider, out, consumer, input, kind);
}

/// A random folder tree with random wires. Roughly one wire in eight runs backwards, so loops
/// appear; some wires are documentation.
fn random_graph(seed: u64, folders: usize, files: usize, wires: usize) -> Graph {
    let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut ids = vec!["root".to_string()];
    let mut clusters = vec![GroupCluster::new("root", "root", "Folder", 0)];
    for i in 1..folders {
        let parent = rng.below(i);
        let id = format!("{}/f{i}", ids[parent]);
        let mut c = GroupCluster::new(id.clone(), format!("f{i}"), "Folder", 0);
        c.parent_id = Some(ids[parent].clone());
        clusters[parent].child_cluster_ids.push(id.clone());
        ids.push(id);
        clusters.push(c);
    }
    let mut cards = Vec::new();
    for i in 0..files {
        let f = rng.below(folders);
        let id = card(&mut g, &format!("file{i}.rs"), &ids[f]);
        clusters[f].node_ids.push(id);
        cards.push(id);
    }
    g.clusters = clusters;
    for _ in 0..wires {
        let (a, b) = (rng.below(files), rng.below(files));
        if a == b {
            continue;
        }
        // Mostly "lower index provides to higher index", so the graph is mostly acyclic.
        let (p, c) = if a < b || rng.below(8) == 0 { (a, b) } else { (b, a) };
        let kind = if rng.below(6) == 0 { EdgeKind::Documentation } else { EdgeKind::Import };
        wire(&mut g, cards[p], cards[c], kind);
    }
    g.rebuild_fast_indices();
    g
}

type Rect = [f32; 4];

fn node_rect(g: &Graph, id: NodeId) -> Rect {
    let n = &g.nodes[&id];
    [n.position[0], n.position[1], n.position[0] + n.size[0], n.position[1] + n.size[1]]
}

fn cluster_rect(c: &GroupCluster) -> Rect {
    [c.position[0], c.position[1], c.position[0] + c.size[0], c.position[1] + c.size[1]]
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner[0] >= outer[0] - 0.01
        && inner[1] >= outer[1] - 0.01
        && inner[2] <= outer[2] + 0.01
        && inner[3] <= outer[3] + 0.01
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[2] - 0.01 && b[0] < a[2] - 0.01 && a[1] < b[3] - 0.01 && b[1] < a[3] - 0.01
}

fn cycle_rects(g: &Graph) -> Vec<Rect> {
    g.flow.as_ref().map_or(Vec::new(), |f| {
        f.cycle_boxes
            .iter()
            .map(|b| [b.position[0], b.position[1], b.position[0] + b.size[0], b.position[1] + b.size[1]])
            .collect()
    })
}

/// L1: for every code wire whose ends are not in the same cycle box, the provider is entirely
/// left of the consumer.
fn assert_providers_left(g: &Graph) {
    let boxes = cycle_rects(g);
    for e in g.edges.iter().filter(|e| e.kind.is_code_flow()) {
        let (p, c) = (node_rect(g, e.from_node), node_rect(g, e.to_node));
        if boxes.iter().any(|b| inside(p, *b) && inside(c, *b)) {
            continue;
        }
        assert!(p[2] <= c[0] + 0.01, "provider {:?} must be left of consumer {:?}", p, c);
    }
}

/// Children inside parents, and nothing overlaps anything it is not nested in.
fn assert_nested_without_overlap(g: &Graph) {
    let rect: HashMap<&str, Rect> = g.clusters.iter().map(|c| (c.id.as_str(), cluster_rect(c))).collect();
    for c in &g.clusters {
        if let Some(p) = &c.parent_id {
            assert!(inside(rect[c.id.as_str()], rect[p.as_str()]), "{} escapes {}", c.id, p);
        }
        for &id in &c.node_ids {
            assert!(inside(node_rect(g, id), rect[c.id.as_str()]), "card escapes {}", c.id);
        }
    }
    // Every pair of cards, and every card against every folder that is not its ancestor.
    let cards: Vec<(NodeId, Rect)> = g.nodes.keys().map(|&id| (id, node_rect(g, id))).collect();
    for (i, a) in cards.iter().enumerate() {
        for b in &cards[i + 1..] {
            assert!(!overlaps(a.1, b.1), "cards overlap: {:?} {:?}", a.1, b.1);
        }
    }
    for c in &g.clusters {
        let r = cluster_rect(c);
        for (id, cr) in &cards {
            let home = g.nodes[id].group_id.as_deref().unwrap();
            let is_ancestor = home == c.id || home.starts_with(&format!("{}/", c.id));
            if !is_ancestor {
                assert!(!overlaps(*cr, r), "card {home} overlaps folder {}", c.id);
            }
        }
    }
    // Cycle boxes contain their cards.
    for b in g.flow.as_ref().unwrap().cycle_boxes.iter() {
        let br = [b.position[0], b.position[1], b.position[0] + b.size[0], b.position[1] + b.size[1]];
        for &id in &b.node_ids {
            assert!(inside(node_rect(g, id), br), "card escapes {}", b.id);
        }
    }
}

/// Gates sit on the correct edge of their folder.
fn assert_gates_on_edges(g: &Graph) {
    for gate in &g.flow.as_ref().unwrap().gates {
        let Some(c) = g.clusters.iter().find(|c| c.id == gate.container) else { continue };
        let r = cluster_rect(c);
        let x = if gate.side == GateSide::Input { r[0] } else { r[2] };
        assert!((gate.position[0] - x).abs() < 0.01, "gate off the {:?} edge of {}", gate.side, c.id);
        assert!(gate.position[1] >= r[1] && gate.position[1] <= r[3], "gate outside {}", c.id);
    }
}

#[test]
fn random_trees_obey_the_layout_laws() {
    let (mut boxes, mut gates) = (0, 0);
    for seed in 0..40 {
        let mut g = random_graph(seed, 1 + (seed as usize % 7), 6 + (seed as usize * 3) % 40, 10 + seed as usize * 2);
        g.layout_folder_tree();
        assert_providers_left(&g);
        assert_nested_without_overlap(&g);
        assert_gates_on_edges(&g);
        let flow = g.flow.as_ref().unwrap();
        boxes += flow.cycle_boxes.len();
        gates += flow.gates.len();
    }
    // The generator must exercise loops and folder crossings, or the checks above prove little.
    assert!(boxes >= 10, "only {boxes} cycle boxes generated");
    assert!(gates >= 50, "only {gates} gates generated");
}

#[test]
fn layout_is_deterministic_whatever_the_wire_order() {
    let mut a = random_graph(7, 6, 30, 60);
    let mut b = a.clone();
    b.edges.reverse();
    b.rebuild_fast_indices();
    a.layout_folder_tree();
    b.layout_folder_tree();
    for (id, n) in &a.nodes {
        assert_eq!(n.position, b.nodes[id].position, "card positions differ");
    }
    assert_eq!(a.flow, b.flow);
}

#[test]
fn a_chain_reads_left_to_right_in_one_row() {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut root = GroupCluster::new("root", "root", "Folder", 0);
    let [one, two, three] = ["1.rs", "2.rs", "3.rs"].map(|n| card(&mut g, n, "root"));
    root.node_ids = vec![one, two, three];
    g.clusters = vec![root];
    // 1 feeds 3 and 2; 3 feeds 2. So 1 < 3 < 2.
    wire(&mut g, one, three, EdgeKind::Import);
    wire(&mut g, three, two, EdgeKind::Import);
    wire(&mut g, one, two, EdgeKind::Import);
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    let x = |id: NodeId| g.nodes[&id].position[0];
    assert!(x(one) < x(three) && x(three) < x(two));
    assert_providers_left(&g);
    assert_nested_without_overlap(&g);
}

#[test]
fn wires_between_folders_pass_through_gates() {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut root = GroupCluster::new("root", "root", "Folder", 0);
    let mut lib = GroupCluster::new("root/lib", "lib", "Folder", 0);
    let mut app = GroupCluster::new("root/app", "app", "Folder", 0);
    lib.parent_id = Some("root".into());
    app.parent_id = Some("root".into());
    root.child_cluster_ids = vec!["root/app".into(), "root/lib".into()];
    let util = card(&mut g, "util.rs", "root/lib");
    let main = card(&mut g, "main.rs", "root/app");
    let cli = card(&mut g, "cli.rs", "root/app");
    lib.node_ids = vec![util];
    app.node_ids = vec![main, cli];
    g.clusters = vec![root, lib, app];
    wire(&mut g, util, main, EdgeKind::Import);
    wire(&mut g, util, cli, EdgeKind::Call);
    g.rebuild_fast_indices();
    g.layout_folder_tree();

    let rect = |id: &str| cluster_rect(g.clusters.iter().find(|c| c.id == id).unwrap());
    assert!(rect("root/lib")[2] <= rect("root/app")[0], "lib provides, so it is left of app");
    let gates = &g.flow.as_ref().unwrap().gates;
    let has =
        |container: &str, side: GateSide| gates.iter().filter(|x| x.container == container && x.side == side).count();
    assert_eq!(has("root/lib", GateSide::Output), 1, "one output gate for util.rs");
    assert_eq!(has("root/app", GateSide::Input), 1, "one input gate for util.rs, fanning out inside");
    assert_gates_on_edges(&g);
}

#[test]
fn a_loop_is_boxed_and_everything_else_flows() {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut root = GroupCluster::new("root", "root", "Folder", 0);
    let [base, a, b, top] = ["base.rs", "a.rs", "b.rs", "top.rs"].map(|n| card(&mut g, n, "root"));
    root.node_ids = vec![base, a, b, top];
    g.clusters = vec![root];
    wire(&mut g, base, a, EdgeKind::Import);
    wire(&mut g, a, b, EdgeKind::Import);
    wire(&mut g, b, a, EdgeKind::Import);
    wire(&mut g, b, top, EdgeKind::Import);
    g.rebuild_fast_indices();
    g.layout_folder_tree();

    let boxes = &g.flow.as_ref().unwrap().cycle_boxes;
    assert_eq!(boxes.len(), 1);
    let mut members = boxes[0].node_ids.clone();
    members.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(members, expected);
    assert_providers_left(&g);
    assert_nested_without_overlap(&g);
}

#[test]
fn documentation_wires_do_not_move_code() {
    let mut code_only = random_graph(3, 4, 20, 40);
    code_only.edges.retain(|e| e.kind.is_code_flow());
    code_only.rebuild_fast_indices();
    let mut with_docs = code_only.clone();
    let ids: Vec<NodeId> = with_docs.nodes.keys().copied().collect();
    for w in ids.windows(2) {
        wire(&mut with_docs, w[1], w[0], EdgeKind::Documentation);
    }
    with_docs.rebuild_fast_indices();
    code_only.layout_folder_tree();
    with_docs.layout_folder_tree();
    for (id, n) in &code_only.nodes {
        assert_eq!(n.position, with_docs.nodes[id].position);
    }
}

#[test]
fn collapsed_folders_are_leaves_with_middle_gates() {
    let mut g = random_graph(11, 5, 25, 50);
    let target = g.clusters[1].id.clone();
    g.clusters[1].is_collapsed = true;
    g.layout_folder_tree();
    let c = g.clusters.iter().find(|c| c.id == target).unwrap();
    assert_eq!(c.size, COLLAPSED_FOLDER_SIZE);
    for gate in g.flow.as_ref().unwrap().gates.iter().filter(|x| x.container == target) {
        assert!((gate.position[1] - (c.position[1] + COLLAPSED_FOLDER_SIZE[1] * 0.5)).abs() < 0.01);
    }
    assert_providers_left_visible(&g);
}

/// L1 for wires whose ends are both visible (not hidden in a collapsed folder).
fn assert_providers_left_visible(g: &Graph) {
    let boxes = cycle_rects(g);
    for e in g.edges.iter().filter(|e| e.kind.is_code_flow()) {
        if g.is_node_in_collapsed_cluster(e.from_node) || g.is_node_in_collapsed_cluster(e.to_node) {
            continue;
        }
        let (p, c) = (node_rect(g, e.from_node), node_rect(g, e.to_node));
        if boxes.iter().any(|b| inside(p, *b) && inside(c, *b)) {
            continue;
        }
        assert!(p[2] <= c[0] + 0.01);
    }
}
