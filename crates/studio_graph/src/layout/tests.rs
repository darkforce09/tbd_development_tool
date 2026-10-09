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

/// Documentation never changes the order: every container's items keep their columns and their
/// order, and code routes keep their shape. Only the documentation strip at the top of a folder
/// grows to fit its tracks, which can shift the contents down.
#[test]
fn documentation_wires_do_not_change_the_order() {
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
    assert_eq!(with_docs.flow.as_ref().unwrap().doc_routes.len(), ids.len() - 1);
    // Cards sharing a folder keep their left-to-right and top-to-bottom order.
    for c in &code_only.clusters {
        for (i, a) in c.node_ids.iter().enumerate() {
            for b in &c.node_ids[i + 1..] {
                let (pa, pb) = (code_only.nodes[a].position, code_only.nodes[b].position);
                let (qa, qb) = (with_docs.nodes[a].position, with_docs.nodes[b].position);
                assert_eq!(pa[0].total_cmp(&pb[0]), qa[0].total_cmp(&qb[0]), "column order changed in {}", c.id);
                if pa[0] == pb[0] {
                    assert_eq!(pa[1].total_cmp(&pb[1]), qa[1].total_cmp(&qb[1]), "row order changed in {}", c.id);
                }
            }
        }
    }
    assert_eq!(code_only.flow.as_ref().unwrap().cycle_boxes.len(), with_docs.flow.as_ref().unwrap().cycle_boxes.len());
    assert_routes_obey_the_laws(&with_docs);
}

#[test]
fn collapsed_folders_are_leaves_with_middle_gates() {
    let mut g = random_graph(11, 5, 25, 50);
    let target = g.clusters[1].id.clone();
    g.clusters[1].is_collapsed = true;
    g.layout_folder_tree();
    let c = g.clusters.iter().find(|c| c.id == target).unwrap();
    assert_eq!(c.size, COLLAPSED_FOLDER_SIZE);
    for gate in g.flow.as_ref().unwrap().gates.iter().filter(|x| x.container == target && x.kind == GateKind::Code) {
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

fn shrink(r: Rect, by: f32) -> Rect {
    [r[0] + by, r[1] + by, r[2] - by, r[3] - by]
}

/// Whether an axis-aligned segment passes through the inside of a rectangle.
fn segment_hits(a: [f32; 2], b: [f32; 2], r: Rect) -> bool {
    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
    x0 < r[2] && x1 > r[0] && y0 < r[3] && y1 > r[1]
}

/// Boxes a route may be inside of: folders and cycle boxes around exactly one or both ends.
struct Region {
    rect: Rect,
    id: String,
    around_provider: bool,
    around_consumer: bool,
}

fn regions(g: &Graph, provider: NodeId, consumer: NodeId) -> Vec<Region> {
    let pr = node_rect(g, provider);
    let cr = node_rect(g, consumer);
    let mut out: Vec<Region> = g
        .clusters
        .iter()
        .filter(|c| !g.hidden_cluster_ids.contains(&c.id))
        .map(|c| {
            let rect = cluster_rect(c);
            Region { rect, id: c.id.clone(), around_provider: inside(pr, rect), around_consumer: inside(cr, rect) }
        })
        .collect();
    for b in &g.flow.as_ref().unwrap().cycle_boxes {
        let rect = [b.position[0], b.position[1], b.position[0] + b.size[0], b.position[1] + b.size[1]];
        out.push(Region {
            rect,
            id: b.id.clone(),
            around_provider: inside(pr, rect),
            around_consumer: inside(cr, rect),
        });
    }
    out
}

/// L2 and L3 for every route, code and documentation: axis-aligned, from port to port, never
/// through a card or a folder it does not belong to, and across folder edges only at a gate of
/// its own kind on the correct side.
fn assert_routes_obey_the_laws(g: &Graph) {
    let flow = g.flow.as_ref().unwrap();
    let routes = flow.routes.iter().map(|r| (r, GateKind::Code));
    for (route, kind) in routes.chain(flow.doc_routes.iter().map(|r| (r, GateKind::Documentation))) {
        assert_route_obeys_the_laws(g, route, kind);
    }
}

fn assert_route_obeys_the_laws(g: &Graph, route: &WireRoute, kind: GateKind) {
    let flow = g.flow.as_ref().unwrap();
    let gate_at = |id: &str, side: GateSide, p: [f32; 2]| {
        flow.gates.iter().any(|x| {
            x.container == id
                && x.side == side
                && x.kind == kind
                && (x.position[0] - p[0]).abs() < 0.05
                && (x.position[1] - p[1]).abs() < 0.05
        })
    };
    let cards: Vec<(NodeId, Rect)> = g.nodes.keys().map(|&id| (id, node_rect(g, id))).collect();
    {
        let (u, v) = (route.provider, route.consumer);
        if g.is_node_in_collapsed_cluster(u) || g.is_node_in_collapsed_cluster(v) {
            return;
        }
        let pts = &route.points;
        assert!(pts.len() >= 2, "route too short");
        for w in pts.windows(2) {
            assert!(
                (w[0][0] - w[1][0]).abs() < 0.01 || (w[0][1] - w[1][1]).abs() < 0.01,
                "diagonal segment {:?} → {:?}",
                w[0],
                w[1]
            );
        }
        let (pr, cr) = (node_rect(g, u), node_rect(g, v));
        assert!((pts[0][0] - pr[2]).abs() < 0.01, "route must start on the provider's right edge");
        assert!((pts[pts.len() - 1][0] - cr[0]).abs() < 0.01, "route must end on the consumer's left edge");
        assert!((pts[0][1] - pts[1][1]).abs() < 0.01 && (pts[pts.len() - 1][1] - pts[pts.len() - 2][1]).abs() < 0.01);

        for w in pts.windows(2) {
            for (id, r) in &cards {
                if *id != u && *id != v {
                    assert!(!segment_hits(w[0], w[1], shrink(*r, 0.05)), "wire crosses a card: {:?}", r);
                }
            }
            for region in regions(g, u, v) {
                let r = region.rect;
                if !region.around_provider && !region.around_consumer {
                    assert!(
                        !segment_hits(w[0], w[1], shrink(r, 0.05)),
                        "wire enters {} it has no business in",
                        region.id
                    );
                    continue;
                }
                // Crossing a vertical edge of a folder around one end only happens at a gate.
                let (a, b) = (w[0], w[1]);
                let horizontal = (a[1] - b[1]).abs() < 0.01;
                for (edge_x, side) in [(r[0], GateSide::Input), (r[2], GateSide::Output)] {
                    let crosses = horizontal
                        && a[0].min(b[0]) < edge_x - 0.05
                        && a[0].max(b[0]) > edge_x + 0.05
                        && a[1] > r[1]
                        && a[1] < r[3];
                    if crosses {
                        let both = region.around_provider && region.around_consumer;
                        assert!(!both, "wire leaves {} that holds both ends", region.id);
                        assert!(gate_at(&region.id, side, [edge_x, a[1]]), "{} crossed away from a gate", region.id);
                    }
                }
                // Pieces join exactly on folder edges, so every route point on an edge is a gate.
                for (edge_x, side) in [(r[0], GateSide::Input), (r[2], GateSide::Output)] {
                    if (a[0] - edge_x).abs() < 0.05 && a[1] > r[1] && a[1] < r[3] {
                        assert!(
                            gate_at(&region.id, side, [edge_x, a[1]]),
                            "{} edge touched away from a gate",
                            region.id
                        );
                    }
                }
                if !horizontal {
                    for edge_y in [r[1], r[3]] {
                        let crosses = a[1].min(b[1]) < edge_y - 0.05
                            && a[1].max(b[1]) > edge_y + 0.05
                            && a[0] > r[0]
                            && a[0] < r[2];
                        assert!(!crosses, "wire crosses the top or bottom of {}", region.id);
                    }
                }
            }
        }
    }
}

#[test]
fn routes_never_cross_cards_and_use_gates() {
    let (mut total, mut docs) = (0, 0);
    for seed in 0..40 {
        let mut g = random_graph(seed, 1 + (seed as usize % 7), 6 + (seed as usize * 3) % 40, 10 + seed as usize * 2);
        g.layout_folder_tree();
        assert_routes_obey_the_laws(&g);
        total += g.flow.as_ref().unwrap().routes.len();
        docs += g.flow.as_ref().unwrap().doc_routes.len();
    }
    assert!(total > 300, "only {total} routes generated");
    assert!(docs > 60, "only {docs} documentation routes generated");
}

#[test]
fn documentation_routes_survive_collapsed_folders() {
    for seed in 0..20 {
        let mut g = random_graph(seed, 2 + seed as usize % 6, 10 + seed as usize % 30, 30 + seed as usize);
        let n = g.clusters.len();
        g.clusters[1 + seed as usize % (n - 1)].is_collapsed = true;
        g.rebuild_collapsed_cache();
        g.layout_folder_tree();
        assert_routes_obey_the_laws(&g);
    }
}

#[test]
fn every_documentation_pair_gets_a_route() {
    let mut g = random_graph(5, 6, 30, 120);
    g.layout_folder_tree();
    let pairs: BTreeSet<(NodeId, NodeId)> =
        g.edges.iter().filter(|e| e.kind == EdgeKind::Documentation).map(|e| (e.from_node, e.to_node)).collect();
    assert!(pairs.len() > 5);
    let routed: BTreeSet<(NodeId, NodeId)> =
        g.flow.as_ref().unwrap().doc_routes.iter().map(|r| (r.provider, r.consumer)).collect();
    assert_eq!(pairs, routed);
    for e in g.edges.iter().filter(|e| e.kind == EdgeKind::Documentation) {
        let route = g.route_of(e).unwrap();
        let end = route.points.last().unwrap();
        let card = &g.nodes[&e.to_node];
        assert_eq!(*end, [card.position[0], card.position[1] + DOC_PORT_OFFSET_Y], "ends at the documentation port");
    }
}

#[test]
fn every_code_pair_gets_a_route() {
    let mut g = random_graph(5, 6, 30, 70);
    g.layout_folder_tree();
    let pairs: BTreeSet<(NodeId, NodeId)> =
        g.edges.iter().filter(|e| e.kind.is_code_flow()).map(|e| (e.from_node, e.to_node)).collect();
    let routed: BTreeSet<(NodeId, NodeId)> =
        g.flow.as_ref().unwrap().routes.iter().map(|r| (r.provider, r.consumer)).collect();
    assert_eq!(pairs, routed);
}

#[test]
fn the_route_checks_catch_violations() {
    let laid_out = || {
        let mut g = random_graph(9, 4, 24, 60);
        g.layout_folder_tree();
        g
    };
    let fails =
        |g: Graph| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| assert_routes_obey_the_laws(&g))).is_err();
    assert!(!fails(laid_out()), "a fresh layout passes");

    // A card dropped onto the middle of a route.
    let mut g = laid_out();
    let route = g.flow.as_ref().unwrap().routes.iter().find(|r| r.points.len() >= 4).unwrap().clone();
    let mid = [(route.points[1][0] + route.points[2][0]) * 0.5, (route.points[1][1] + route.points[2][1]) * 0.5];
    let bystander = *g.nodes.keys().find(|&&id| id != route.provider && id != route.consumer).unwrap();
    g.nodes.get_mut(&bystander).unwrap().position = [mid[0] - 10.0, mid[1] - 10.0];
    assert!(fails(g), "a card on a wire is caught");

    // A diagonal segment.
    let mut g = laid_out();
    let r = &mut g.flow.as_mut().unwrap().routes[0];
    let n = r.points.len();
    r.points[n / 2][0] += 37.0;
    r.points[n / 2][1] += 23.0;
    assert!(fails(g), "a diagonal is caught");

    // A wire leaving a folder away from its gates.
    let mut g = laid_out();
    let gates = g.flow.as_ref().unwrap().gates.clone();
    let flow = g.flow.as_mut().unwrap();
    flow.gates.clear();
    let crossing = !gates.is_empty();
    assert!(crossing && fails(g), "a crossing without a gate is caught");

    // A card dropped onto a documentation route.
    let mut g = laid_out();
    let route = g.flow.as_ref().unwrap().doc_routes.iter().find(|r| r.points.len() >= 4).unwrap().clone();
    let mid = [(route.points[1][0] + route.points[2][0]) * 0.5, (route.points[1][1] + route.points[2][1]) * 0.5];
    let bystander = *g.nodes.keys().find(|&&id| id != route.provider && id != route.consumer).unwrap();
    g.nodes.get_mut(&bystander).unwrap().position = [mid[0] - 10.0, mid[1] - 10.0];
    assert!(fails(g), "a card on a documentation wire is caught");

    // Documentation wires crossing folders only at code gates.
    let mut g = laid_out();
    let flow = g.flow.as_mut().unwrap();
    let before = flow.gates.len();
    flow.gates.retain(|x| x.kind == GateKind::Code);
    assert!(flow.gates.len() < before && fails(g), "a documentation crossing without a documentation gate is caught");
}

/// One folder holding `cards` cards, laid out after `connect` adds wires.
fn one_folder(cards: usize, connect: impl Fn(&mut Graph, &[NodeId])) -> (Graph, Vec<NodeId>) {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut root = GroupCluster::new("root", "root", "Folder", 0);
    let ids: Vec<NodeId> = (0..cards).map(|i| card(&mut g, &format!("{i:03}.rs"), "root")).collect();
    root.node_ids = ids.clone();
    g.clusters = vec![root];
    connect(&mut g, &ids);
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    (g, ids)
}

fn aspect(r: Rect) -> f32 {
    (r[2] - r[0]) / (r[3] - r[1])
}

#[test]
fn a_long_loop_is_packed_into_a_block_not_a_row() {
    let (g, ids) = one_folder(24, |g, ids| {
        for i in 0..ids.len() {
            wire(g, ids[i], ids[(i + 1) % ids.len()], EdgeKind::Import);
        }
    });
    let boxes = cycle_rects(&g);
    assert_eq!(boxes.len(), 1);
    let columns: BTreeSet<i32> = ids.iter().map(|id| g.nodes[id].position[0] as i32).collect();
    assert!(columns.len() > 1 && columns.len() < ids.len(), "{} columns for {} cards", columns.len(), ids.len());
    let a = aspect(boxes[0]);
    assert!((0.5..=4.0).contains(&a), "loop box aspect {a}");
    assert_nested_without_overlap(&g);
    assert_routes_obey_the_laws(&g);
}

#[test]
fn a_very_tall_column_is_split_and_still_flows_left_to_right() {
    // Forty files all feed one: they share a layer, which used to make one column 40 cards tall.
    let (g, ids) = one_folder(41, |g, ids| {
        for &p in &ids[1..] {
            wire(g, p, ids[0], EdgeKind::Import);
        }
    });
    let providers: BTreeSet<i32> = ids[1..].iter().map(|id| g.nodes[id].position[0] as i32).collect();
    assert!(providers.len() > 1, "the layer is split into several columns");
    let root = cluster_rect(&g.clusters[0]);
    assert!(aspect(root) > 0.4, "folder aspect {}", aspect(root));
    assert_providers_left(&g);
    assert_nested_without_overlap(&g);
    assert_routes_obey_the_laws(&g);
}

#[test]
fn packing_picks_the_column_count_closest_to_the_target_aspect() {
    let sizes = vec![[220.0, 42.0]; 30];
    let seq: Vec<usize> = (0..30).collect();
    let columns = pack_columns(&seq, &sizes);
    assert_eq!(columns.len(), 30);
    assert!(columns.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1), "consecutive, in order");
    let n = columns.last().unwrap() + 1;
    let rows = 30_usize.div_ceil(n);
    let (w, h) = (n as f32 * 220.0 + (n - 1) as f32 * GAP_BASE, rows as f32 * (42.0 + ITEM_GAP));
    assert!((w / h / TARGET_ASPECT).ln().abs() < 0.6, "{n} columns: {w} x {h}");
    assert!(pack_columns(&[], &sizes).is_empty());
}
