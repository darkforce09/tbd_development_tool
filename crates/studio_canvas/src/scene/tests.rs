use super::*;
use studio_graph::{DataType, EdgeKind, GroupCluster, NodeArchetype};

fn card(g: &mut Graph, name: &str, folder: usize) -> NodeId {
    let id = g.add_node(
        name,
        NodeArchetype::File,
        "",
        None,
        vec![("in".into(), DataType::RustFlow), ("docs".into(), DataType::Documentation)],
        vec![("out".into(), DataType::RustFlow)],
        [0.0, 0.0],
    );
    g.nodes.get_mut(&id).unwrap().size = [220.0, 42.0];
    g.clusters[folder].node_ids.push(id);
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

/// root/{a, b}: a.rs and lib.rs in `a` feed main.rs in `b`; readme.md documents main.rs.
fn project() -> (Graph, [NodeId; 4]) {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let mut root = GroupCluster::new("root", "root", "Folder", 0);
    root.child_cluster_ids = vec!["root/a".into(), "root/b".into()];
    let mut a = GroupCluster::new("root/a", "a", "Folder", 1);
    a.parent_id = Some("root".into());
    let mut b = GroupCluster::new("root/b", "b", "Folder", 2);
    b.parent_id = Some("root".into());
    g.clusters = vec![root, a, b];
    let lib = card(&mut g, "lib.rs", 1);
    let util = card(&mut g, "util.rs", 1);
    let main = card(&mut g, "main.rs", 2);
    let readme = card(&mut g, "readme.md", 0);
    wire(&mut g, lib, main, EdgeKind::Import);
    wire(&mut g, lib, main, EdgeKind::Import);
    wire(&mut g, util, main, EdgeKind::Call);
    wire(&mut g, readme, main, EdgeKind::Documentation);
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    (g, [lib, util, main, readme])
}

fn key(w: &WireInstance) -> ([i32; 4], [u8; 4]) {
    let q = |v: f32| (v * 10.0).round() as i32;
    ([q(w.p0[0]), q(w.p0[1]), q(w.p1[0]), q(w.p1[1])], w.color)
}

#[test]
fn every_segment_is_drawn_once() {
    let (g, _) = project();
    let scene = CanvasScene::build(&g, true);
    assert!(!scene.segments.is_empty());
    let distinct: FxHashSet<_> = scene.segments.iter().map(key).collect();
    assert_eq!(distinct.len(), scene.segments.len(), "a segment shared by several routes is drawn once");
    assert_eq!(scene.segments.len(), scene.segment_owner.len());
}

#[test]
fn code_and_documentation_wires_follow_their_routes() {
    let (g, [_, _, main, readme]) = project();
    let scene = CanvasScene::build(&g, true);
    assert!(scene.segments.iter().all(|w| !w.is_curve() && (w.p0[0] == w.p1[0] || w.p0[1] == w.p1[1])));
    assert!(scene.curves.is_empty(), "every wire here has a route");
    let doc = g.edges.iter().find(|e| e.kind == EdgeKind::Documentation).unwrap();
    assert_eq!((doc.from_node, doc.to_node), (readme, main));
    let doc_kind = EdgeKind::Documentation as u32;
    assert!(scene.segments.iter().any(|w| w.kind_index() == doc_kind));
    // The documentation wire ends at main.rs's documentation port, from the left.
    let port = Pos2::new(g.nodes[&main].position[0], g.nodes[&main].position[1] + studio_graph::DOC_PORT_OFFSET_Y);
    assert!(scene.segments.iter().any(|w| w.kind_index() == doc_kind && Pos2::from(w.p1).distance(port) < 0.1));
}

#[test]
fn wires_without_a_route_are_curves() {
    let (mut g, [lib, util, ..]) = project();
    // Same folder, no layout run since: no route for this pair.
    wire(&mut g, util, lib, EdgeKind::Asset);
    g.rebuild_fast_indices();
    let scene = CanvasScene::build(&g, true);
    assert_eq!(scene.curves.len(), 1);
    assert_eq!(scene.curves[0].kind_index(), EdgeKind::Asset as u32);
}

#[test]
fn wires_within_a_closed_card_are_not_drawn() {
    let (mut g, [lib, ..]) = project();
    wire(&mut g, lib, lib, EdgeKind::Call);
    g.rebuild_fast_indices();
    assert!(CanvasScene::build(&g, true).curves.is_empty(), "a closed card hides its inner calls");
    g.nodes.get_mut(&lib).unwrap().is_dropdown_expanded = true;
    assert_eq!(CanvasScene::build(&g, true).curves.len(), 1, "an opened card shows them");
}

#[test]
fn tiles_partition_the_wires_and_bound_them() {
    let (g, _) = project();
    let scene = CanvasScene::build(&g, true);
    for (tiles, wires) in [(&scene.tiles, &scene.segments), (&scene.curve_tiles, &scene.curves)] {
        let mut next = 0;
        for t in tiles {
            assert_eq!(t.start, next, "tiles are contiguous");
            assert!(t.end > t.start);
            for w in &wires[t.start as usize..t.end as usize] {
                assert!(t.bounds.expand(0.01).contains_rect(wire_bounds(w)));
            }
            next = t.end;
        }
        assert_eq!(next as usize, wires.len(), "every wire is in a tile");
    }
}

#[test]
fn visible_ranges_follow_the_view() {
    let (g, _) = project();
    let scene = CanvasScene::build(&g, true);
    let everything = Rect::from_min_max(Pos2::new(-1e6, -1e6), Pos2::new(1e6, 1e6));
    let all: u32 = scene.visible_ranges(everything).iter().map(|r| r.end - r.start).sum();
    assert_eq!(all as usize, scene.segments.len());
    let far = Rect::from_min_size(Pos2::new(5e6, 5e6), egui::vec2(100.0, 100.0));
    assert!(scene.visible_ranges(far).is_empty());
    assert!(scene.visible_curve_ranges(far).is_empty());
}

#[test]
fn hit_test_finds_wires_of_visible_kinds_only() {
    let (g, _) = project();
    let scene = CanvasScene::build(&g, true);
    let w = scene.segments.iter().position(|w| (Pos2::from(w.p0) - Pos2::from(w.p1)).length() > 4.0).unwrap();
    let mid = Pos2::from(scene.segments[w].p0).lerp(Pos2::from(scene.segments[w].p1), 0.5);
    let kind = scene.segments[w].kind_index();
    let hit = scene.hit_test(mid + egui::vec2(0.0, 2.0), 5.0, |_| true).expect("a wire under the pointer");
    assert_eq!(g.get_edge(hit).unwrap().kind as u32, kind);
    assert_eq!(scene.hit_test(mid, 5.0, |k| k != kind && k != EdgeKind::Documentation as u32), None);
    assert_eq!(scene.hit_test(Pos2::new(5e6, 5e6), 5.0, |_| true), None);
}

#[test]
fn a_collapsed_folder_hides_its_wires_inside() {
    let (mut g, _) = project();
    g.toggle_cluster_collapse("root/b");
    let scene = CanvasScene::build(&g, true);
    let b = &g.clusters[2];
    let inside = Rect::from_min_size(Pos2::from(b.position), egui::vec2(b.size[0], b.size[1])).shrink(1.0);
    for w in &scene.segments {
        assert!(!inside.contains(Pos2::from(w.p0)) && !inside.contains(Pos2::from(w.p1)), "{w:?} enters {inside:?}");
    }
    assert!(!scene.segments.is_empty(), "wires still reach the folder's gates");
    assert!(scene.folder_order.contains(&2), "the collapsed folder itself is drawn");
}

#[test]
fn folders_are_drawn_parents_first_then_cycle_boxes() {
    let (g, _) = project();
    let scene = CanvasScene::build(&g, true);
    assert_eq!(scene.folder_order[0], 0, "the root is drawn before its children");
    let depths: Vec<usize> = scene.folder_order.iter().map(|&i| g.clusters[i].depth).collect();
    assert!(depths.windows(2).all(|d| d[0] <= d[1]));
    assert_eq!(scene.cycle_start, scene.boxes.len() - scene.cycle_order.len());
    assert_eq!(scene.gates.len(), g.flow.as_ref().unwrap().gates.len());
}

#[test]
fn the_highlight_layer_holds_only_the_given_edges() {
    let (g, [lib, ..]) = project();
    let edges: Vec<EdgeId> = g.edges.iter().filter(|e| e.from_node == lib).map(|e| e.id).collect();
    let (overlay, curves_start) = build_overlay(&g, edges.iter().map(|&e| (e, WIRE_HIGHLIGHT)));
    assert!(!overlay.is_empty());
    assert_eq!(curves_start as usize, overlay.len(), "routed wires only, no curves");
    assert!(overlay.iter().all(|w| w.flags & WIRE_HIGHLIGHT != 0 && w.kind_index() == EdgeKind::Import as u32));
    assert!(build_overlay(&g, std::iter::empty()).0.is_empty());
}

#[test]
fn the_card_layer_skips_hidden_cards_and_marks_selection() {
    let (mut g, [lib, _, main, _]) = project();
    g.toggle_cluster_collapse("root/b");
    let selected = std::collections::BTreeSet::from([lib]);
    let boxes = build_card_layer(&g, |_| false, &selected);
    assert_eq!(boxes.len(), g.nodes.len() - 1, "main.rs is inside the collapsed folder");
    assert!(g.is_node_in_collapsed_cluster(main));
    let selected_fill = CARD_BORDER_SELECTED.to_srgba_unmultiplied();
    assert_eq!(boxes.iter().filter(|b| b.fill == selected_fill).count(), 1);
}

#[test]
fn every_build_gets_a_new_revision() {
    let (g, _) = project();
    let a = CanvasScene::build(&g, true);
    let b = CanvasScene::build(&g, true);
    assert_ne!(a.revision, b.revision);
    assert_ne!(a.revision, 0);
}
