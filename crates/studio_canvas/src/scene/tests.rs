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
    let boxes = build_card_layer(&g, |_| false, &selected, &Default::default());
    assert_eq!(boxes.len(), g.nodes.len() - 1, "main.rs is inside the collapsed folder");
    assert!(g.is_node_in_collapsed_cluster(main));
    let selected_fill = CARD_BORDER_SELECTED.to_srgba_unmultiplied();
    assert_eq!(boxes.iter().filter(|b| b.fill == selected_fill).count(), 1);
}

#[test]
fn the_card_layer_tints_the_lit_session_files() {
    let (g, [lib, a, _, _]) = project();
    let lit = std::collections::BTreeSet::from([lib, a]);
    let boxes = build_card_layer(&g, |_| false, &std::collections::BTreeSet::from([lib]), &lit);
    let fill = |c: egui::Color32| boxes.iter().filter(|b| b.fill == c.to_srgba_unmultiplied()).count();
    assert_eq!(fill(CARD_BORDER_SELECTED), 1, "the selection wins");
    assert_eq!(fill(DISTRICT_CHANGES), 1);
}

#[test]
fn every_build_gets_a_new_revision() {
    let (g, _) = project();
    let a = CanvasScene::build(&g, true);
    let b = CanvasScene::build(&g, true);
    assert_ne!(a.revision, b.revision);
    assert_ne!(a.revision, 0);
}

#[test]
fn a_closed_folder_draws_one_labelled_heavier_wire() {
    let (mut g, _) = project();
    g.set_folder_detail("root/a", FolderDetail::NodeView);
    let scene = CanvasScene::build(&g, true);
    assert_eq!(scene.bundle_labels.len(), 1, "one count for the bundle from the closed folder");
    assert_eq!(scene.bundle_labels[0].pairs, 2);
    let weights: Vec<u32> = scene
        .segments
        .iter()
        .filter(|s| s.kind_index() != EdgeKind::Documentation as u32)
        .map(|s| (s.flags & WIRE_WEIGHT_MASK) >> WIRE_WEIGHT_SHIFT)
        .collect();
    assert!(!weights.is_empty());
    assert!(weights.iter().all(|&w| w == 1), "two pairs weigh log2(2) = 1: {weights:?}");
}

#[test]
fn weights_grow_with_the_log_of_the_pairs() {
    assert_eq!(wire_weight(1) >> WIRE_WEIGHT_SHIFT, 0);
    assert_eq!(wire_weight(2) >> WIRE_WEIGHT_SHIFT, 1);
    assert_eq!(wire_weight(23) >> WIRE_WEIGHT_SHIFT, 4);
    assert_eq!(wire_weight(1_000_000) >> WIRE_WEIGHT_SHIFT, 7, "capped");
}

#[test]
fn hubs_get_a_badge_and_no_wires_and_documentation_a_chip() {
    let mut g = Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    g.clusters = vec![GroupCluster::new("root", "root", "Folder", 0)];
    let hub = card(&mut g, "types.rs", 0);
    let users: Vec<NodeId> = (0..9).map(|i| card(&mut g, &format!("user{i}.rs"), 0)).collect();
    for &u in &users {
        wire(&mut g, hub, u, EdgeKind::Import);
    }
    let readme = card(&mut g, "readme.md", 0);
    wire(&mut g, readme, users[0], EdgeKind::Documentation);
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    let scene = CanvasScene::build(&g, true);
    let hubs: Vec<&Chip> = scene.chips.iter().filter(|c| c.kind == ChipKind::Hub).collect();
    assert_eq!(hubs.len(), 1);
    assert_eq!(hubs[0].count, 9, "used by 9");
    let hub_edges: std::collections::BTreeSet<EdgeId> =
        g.edges.iter().filter(|e| e.from_node == hub).map(|e| e.id).collect();
    assert!(scene.segment_owner.iter().all(|id| !hub_edges.contains(id)), "the hub's wires are not drawn");
    assert!(scene.curves.is_empty() || scene.curve_owner.iter().all(|id| !hub_edges.contains(id)));
    let docs: Vec<&Chip> = scene.chips.iter().filter(|c| c.kind == ChipKind::Docs).collect();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].count, 1);
}

const TIERS: [EvidenceTier; 4] =
    [EvidenceTier::Proven, EvidenceTier::PossibleSet, EvidenceTier::Observed, EvidenceTier::Unresolved];

#[test]
fn every_tier_round_trips_through_the_wire_flags() {
    assert_eq!(WIRE_TIER_MASK & (WIRE_KIND_MASK | WIRE_CURVE | WIRE_HIGHLIGHT | WIRE_ACTIVE | WIRE_WEIGHT_MASK), 0);
    for (i, &tier) in TIERS.iter().enumerate() {
        assert_eq!(wire_tier(tier), (i as u32) << WIRE_TIER_SHIFT);
        let flags = EdgeKind::Call as u32 | WIRE_CURVE | WIRE_HIGHLIGHT | wire_weight(9) | wire_tier(tier);
        let w = WireInstance { flags, ..Default::default() };
        assert_eq!(w.tier(), tier);
        assert_eq!(w.kind_index(), EdgeKind::Call as u32);
        assert!(w.is_curve());
        assert_eq!(flags & WIRE_WEIGHT_MASK, wire_weight(9));
    }
    // Raising keeps the stronger tier and leaves the other bits alone.
    let mut flags = EdgeKind::Import as u32 | wire_weight(4) | wire_tier(EvidenceTier::Unresolved);
    strengthen(&mut flags, EvidenceTier::Observed);
    assert_eq!(tier_of_flags(flags), EvidenceTier::Observed);
    strengthen(&mut flags, EvidenceTier::Unresolved);
    assert_eq!(tier_of_flags(flags), EvidenceTier::Observed);
    strengthen(&mut flags, EvidenceTier::Proven);
    assert_eq!(tier_of_flags(flags), EvidenceTier::Proven);
    assert_eq!(flags & !WIRE_TIER_MASK, EdgeKind::Import as u32 | wire_weight(4));
    for &a in &TIERS {
        for &b in &TIERS {
            assert_eq!(strongest_tier(a, b), TIERS[(a as usize).min(b as usize)]);
        }
    }
}

/// Owners and tiers of every instance in the scene, segments then curves.
fn tiers_by_owner(scene: &CanvasScene) -> Vec<(EdgeId, EvidenceTier)> {
    let segments = scene.segments.iter().zip(&scene.segment_owner);
    let curves = scene.curves.iter().zip(&scene.curve_owner);
    segments.chain(curves).map(|(w, &owner)| (owner, w.tier())).collect()
}

#[test]
fn a_wire_drawn_once_for_two_edges_shows_the_stronger_tier() {
    let (mut g, [lib, _, main, _]) = project();
    let imports: Vec<usize> =
        (0..g.edges.len()).filter(|&i| (g.edges[i].from_node, g.edges[i].to_node) == (lib, main)).collect();
    assert_eq!(imports.len(), 2, "lib.rs imports main.rs twice");
    let base = CanvasScene::build(&g, true);
    assert!(tiers_by_owner(&base).iter().all(|&(_, t)| t == EvidenceTier::Unresolved), "edges are name matches");
    // Both copies share every segment; the first copy owns them. Proving only the second one
    // proves the wire, and adds no instance.
    g.edges[imports[1]].provenance = studio_graph::Provenance::proven(studio_graph::Basis::PathResolution);
    let scene = CanvasScene::build(&g, true);
    assert_eq!((scene.segments.len(), scene.curves.len()), (base.segments.len(), base.curves.len()));
    let first = g.edges[imports[0]].id;
    let owned: Vec<EvidenceTier> =
        tiers_by_owner(&scene).into_iter().filter(|&(o, _)| o == first).map(|(_, t)| t).collect();
    assert!(!owned.is_empty());
    assert!(owned.iter().all(|&t| t == EvidenceTier::Proven), "{owned:?}");
    let second = g.edges[imports[1]].id;
    assert!(tiers_by_owner(&scene).iter().all(|&(o, _)| o != second), "the second copy draws nothing of its own");
    // A possible set beats unresolved, and loses to proven.
    g.edges[imports[0]].provenance = studio_graph::Provenance::possible(studio_graph::Basis::Tag, 3);
    let scene = CanvasScene::build(&g, true);
    assert_eq!(scene.segments.len(), base.segments.len());
    assert!(tiers_by_owner(&scene).iter().filter(|&&(o, _)| o == first).all(|&(_, t)| t == EvidenceTier::Proven));
    g.edges[imports[1]].provenance = studio_graph::Provenance::default();
    let scene = CanvasScene::build(&g, true);
    assert!(tiers_by_owner(&scene).iter().filter(|&&(o, _)| o == first).all(|&(_, t)| t == EvidenceTier::PossibleSet));
}

/// root/{a, b}: lib.rs in `a` imports main.rs and other.rs in `b`. The route to main.rs is
/// redrawn to leave lib.rs along exactly the first stretch of the route to other.rs.
fn fan_out() -> (Graph, [NodeId; 3]) {
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
    let main = card(&mut g, "main.rs", 2);
    let other = card(&mut g, "other.rs", 2);
    wire(&mut g, lib, main, EdgeKind::Import);
    wire(&mut g, lib, other, EdgeKind::Import);
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    let drawn = |g: &Graph, to: NodeId| {
        let mut points = Vec::new();
        assert!(routed_wire_points_into(g, g.edges.iter().find(|e| e.to_node == to).unwrap(), &mut points));
        points
    };
    let (to_main, to_other) = (drawn(&g, main), drawn(&g, other));
    let (start, split, end) = (to_other[0], to_other[1], to_main[to_main.len() - 1]);
    assert!(split.x > start.x && end.x > split.x);
    let route = g.route_index[&(lib, main)];
    g.flow.as_mut().unwrap().routes[route].points = vec![start.into(), split.into(), [split.x, end.y], end.into()];
    (g, [lib, main, other])
}

#[test]
fn a_segment_shared_by_two_wires_shows_the_stronger_tier() {
    let (mut g, [_, main, other]) = fan_out();
    let to = |g: &Graph, n: NodeId| g.edges.iter().position(|e| e.to_node == n).unwrap();
    let (to_main, to_other) = (to(&g, main), to(&g, other));
    let points = |g: &Graph, i: usize| {
        let mut points = Vec::new();
        assert!(routed_wire_points_into(g, &g.edges[i], &mut points));
        let q = |p: Pos2| ((p.x * 10.0).round() as i32, (p.y * 10.0).round() as i32);
        points
            .windows(2)
            .map(|w| if q(w[0]) <= q(w[1]) { (q(w[0]), q(w[1])) } else { (q(w[1]), q(w[0])) })
            .collect::<FxHashSet<_>>()
    };
    let shared: FxHashSet<_> = points(&g, to_main).intersection(&points(&g, to_other)).copied().collect();
    assert!(!shared.is_empty(), "the two routes share a stretch");
    let is_shared = |w: &WireInstance| {
        let q = |v: f32| (v * 10.0).round() as i32;
        shared.contains(&((q(w.p0[0]), q(w.p0[1])), (q(w.p1[0]), q(w.p1[1]))))
    };
    let base = CanvasScene::build(&g, true);
    for (proven, unresolved) in [(to_main, to_other), (to_other, to_main)] {
        for e in g.edges.iter_mut() {
            e.provenance = studio_graph::Provenance::default();
        }
        g.edges[proven].provenance = studio_graph::Provenance::proven(studio_graph::Basis::PathResolution);
        let scene = CanvasScene::build(&g, true);
        assert_eq!(scene.segments.len(), base.segments.len(), "tiers never add instances");
        for (w, owner) in scene.segments.iter().zip(&scene.segment_owner) {
            let expected = if is_shared(w) || *owner == g.edges[proven].id {
                EvidenceTier::Proven
            } else {
                assert_eq!(*owner, g.edges[unresolved].id);
                EvidenceTier::Unresolved
            };
            assert_eq!(w.tier(), expected, "{w:?}");
        }
        assert!(scene.segments.iter().any(|w| w.tier() == EvidenceTier::Unresolved));
    }
}

#[test]
fn a_segment_shared_by_two_providers_shows_the_stronger_tier() {
    // lib.rs and util.rs both import main.rs, and util.rs's route is redrawn to end along the
    // last stretch of lib.rs's: two providers, so the copies meet in the sorted merge.
    let (mut g, [lib, util, main, _]) = project();
    for e in g.edges.iter_mut().filter(|e| e.from_node == util) {
        e.kind = EdgeKind::Import;
    }
    let drawn = |g: &Graph, from: NodeId| {
        let mut points = Vec::new();
        let edge = g.edges.iter().find(|e| (e.from_node, e.to_node) == (from, main)).unwrap();
        assert!(routed_wire_points_into(g, edge, &mut points));
        points
    };
    let (from_lib, from_util) = (drawn(&g, lib), drawn(&g, util));
    let n = from_lib.len();
    let (corner, end) = (from_lib[n - 2], from_lib[n - 1]);
    assert_eq!(corner.y, end.y);
    let route = g.route_index[&(util, main)];
    g.flow.as_mut().unwrap().routes[route].points =
        vec![from_util[0].into(), [corner.x, from_util[0].y], corner.into(), end.into()];
    let shared = |w: &WireInstance| Pos2::from(w.p0) == corner && Pos2::from(w.p1) == end;
    let import = EdgeKind::Import as u32;
    let base = CanvasScene::build(&g, true);
    assert_eq!(base.segments.iter().filter(|w| shared(w)).count(), 1, "the shared stretch is drawn once");
    let util_edge = g.edges.iter().position(|e| e.from_node == util).unwrap();
    g.edges[util_edge].provenance = studio_graph::Provenance::observed();
    let scene = CanvasScene::build(&g, true);
    assert_eq!(scene.segments.len(), base.segments.len(), "tiers never add instances");
    let owner = scene.segments.iter().position(shared).map(|i| scene.segment_owner[i]);
    assert_ne!(owner, Some(g.edges[util_edge].id), "lib.rs's earlier edge owns the shared stretch");
    for (w, owner) in scene.segments.iter().zip(&scene.segment_owner).filter(|(w, _)| w.kind_index() == import) {
        let expected = if shared(w) || *owner == g.edges[util_edge].id {
            EvidenceTier::Observed
        } else {
            EvidenceTier::Unresolved
        };
        assert_eq!(w.tier(), expected, "{w:?}");
    }
}

#[test]
fn highlighted_wires_keep_their_tier() {
    let (mut g, _) = project();
    for (e, &tier) in g.edges.iter_mut().zip(TIERS.iter().cycle()) {
        e.provenance.tier = tier;
    }
    for e in &g.edges {
        let (overlay, _) = build_overlay(&g, [(e.id, WIRE_HIGHLIGHT)]);
        assert!(!overlay.is_empty());
        assert!(overlay.iter().all(|w| w.tier() == e.provenance.tier && w.flags & WIRE_HIGHLIGHT != 0));
    }
}
