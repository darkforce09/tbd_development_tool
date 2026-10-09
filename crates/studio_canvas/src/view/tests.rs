use egui::{Pos2, Rect, Vec2};
use studio_graph::{DataType, FileMemberNode, NodeArchetype};

use super::layout::{calculate_file_node_size, port_world_position};
use super::types::CanvasState;

#[test]
fn test_calculate_file_node_size() {
    let mut graph = studio_graph::Graph::new();
    let nid =
        graph.add_node("lib.rs", NodeArchetype::File, "lib file", Some("RS".to_string()), vec![], vec![], [0.0, 0.0]);
    let node = graph.nodes.get(&nid).unwrap();
    assert_eq!(calculate_file_node_size(node), [220.0, 42.0]);

    let node_mut = graph.nodes.get_mut(&nid).unwrap();
    node_mut.is_dropdown_expanded = true;
    // Expanded with 0 members: 38 (hdr) + 2 (div) + 24 ("(no members)") + 8 (pad) = 72.0
    assert_eq!(calculate_file_node_size(node_mut), [280.0, 72.0]);

    // Add 2 members
    node_mut.member_nodes.push(FileMemberNode::new(
        "m1",
        "init",
        NodeArchetype::Function,
        "pub",
        "pub fn init()",
        10,
        "pub fn init() {}",
        None,
    ));
    node_mut.member_nodes.push(FileMemberNode::new(
        "m2",
        "Config",
        NodeArchetype::Struct,
        "pub",
        "pub struct Config",
        25,
        "pub struct Config {\n    pub port: u16,\n}",
        None,
    ));

    // 38 header + 2 divider + 2 * 26 (52) rows + 8 padding = 100.0
    assert_eq!(calculate_file_node_size(node_mut), [280.0, 100.0]);

    // Expand drawer for m2 (3 lines in source): drawer_h = 3 * 14.0 + 18.0 = 60.0
    node_mut.expanded_member_id = Some("m2".into());
    // 100.0 + 60.0 = 160.0
    assert_eq!(calculate_file_node_size(node_mut), [280.0, 160.0]);
}

#[test]
fn test_member_port_world_position_and_toggle() {
    let mut graph = studio_graph::Graph::new();
    let nid = graph.add_node(
        "worker.rs",
        NodeArchetype::File,
        "worker file",
        Some("RS".to_string()),
        vec![("in".into(), DataType::RustFlow)],
        vec![("out".into(), DataType::RustFlow)],
        [100.0, 200.0],
    );
    let in_p = graph.nodes.get(&nid).unwrap().inputs[0].id;
    let out_p = graph.nodes.get(&nid).unwrap().outputs[0].id;

    let node_mut = graph.nodes.get_mut(&nid).unwrap();
    node_mut.size = calculate_file_node_size(node_mut);
    node_mut.member_nodes.push(
        FileMemberNode::new(
            "fn1",
            "process",
            NodeArchetype::Function,
            "pub",
            "pub fn process()",
            12,
            "pub fn process() {}",
            None,
        )
        .with_ports(Some(in_p), Some(out_p)),
    );

    // When collapsed: ports collapse to center Y: 200.0 + 42.0 * 0.5 = 221.0
    let node = graph.nodes.get(&nid).unwrap();
    assert!(!node.show_member_wires);
    let pos_collapsed = port_world_position(node, in_p).unwrap();
    assert_eq!(pos_collapsed.x, 100.0);
    assert_eq!(pos_collapsed.y, 221.0);

    // When expanded: dynamic Y matching member row 0 center:
    // 200.0 + 38.0 (hdr) + 2.0 (div) + 4.0 + 12.0 = 256.0
    let node_mut = graph.nodes.get_mut(&nid).unwrap();
    node_mut.is_dropdown_expanded = true;
    let pos_expanded = port_world_position(node_mut, in_p).unwrap();
    assert_eq!(pos_expanded.x, 100.0);
    assert_eq!(pos_expanded.y, 256.0);

    // Output port is at x = 100.0 + 280.0 = 380.0
    node_mut.size = calculate_file_node_size(node_mut);
    let pos_out = port_world_position(node_mut, out_p).unwrap();
    assert_eq!(pos_out.x, 380.0);
    assert_eq!(pos_out.y, 256.0);
}

#[test]
fn test_pending_node_drag_threshold_logic() {
    let press_start = Pos2::new(100.0, 100.0);
    let small_move = Pos2::new(102.0, 102.0); // dist = sqrt(4 + 4) = 2.83 < 4.0
    assert!(small_move.distance(press_start) < 4.0);

    let drag_move = Pos2::new(104.0, 103.0); // dist = 5.0 >= 4.0
    assert!(drag_move.distance(press_start) >= 4.0);
}

#[test]
fn test_canvas_state_interactive_rects_default_and_detection() {
    let mut state = CanvasState::default();
    assert!(state.interactive_rects.is_empty());

    let btn_rect = Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(20.0, 20.0));
    state.interactive_rects.push(btn_rect);

    // Point inside interactive button
    let hit = Pos2::new(55.0, 55.0);
    assert!(state.interactive_rects.iter().any(|r| r.contains(hit)));

    // Point outside interactive button (on node card body)
    let miss = Pos2::new(20.0, 20.0);
    assert!(!state.interactive_rects.iter().any(|r| r.contains(miss)));
}

#[test]
fn test_node_tab_clicked_and_port_jump_action() {
    let mut graph = studio_graph::Graph::new();
    let n1 = graph.add_node(
        "parser.rs",
        NodeArchetype::File,
        "parser file",
        Some("RS".to_string()),
        vec![],
        vec![],
        [0.0, 0.0],
    );
    let n2 = graph.add_node(
        "ast.rs",
        NodeArchetype::File,
        "ast file",
        Some("RS".to_string()),
        vec![],
        vec![],
        [400.0, 200.0],
    );

    let mut state = CanvasState::default();
    let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1000.0, 800.0));

    // Initially node expanded_tab is 0 and scroll_offset_y is 0
    let node = graph.nodes.get(&n1).unwrap();
    assert_eq!(node.expanded_tab, 0);
    assert_eq!(node.scroll_offset_y, 0.0);

    // 1. Tab switch event
    let events = super::types::RenderEvents { node_tab_clicked: Some((n1, 2)), ..Default::default() }; // Switch to Members tab
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&n1).unwrap().expanded_tab, 2);

    // 2. Port jump event: selects target node and immediately executes CenterNode
    let events = super::types::RenderEvents { port_jump_clicked: Some(n2), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(state.selected_nodes.contains(&n2));
    let n2_node = graph.nodes.get(&n2).unwrap();
    let n2_center = Pos2::new(n2_node.position[0] + n2_node.size[0] * 0.5, n2_node.position[1] + n2_node.size[1] * 0.5);
    let screen_pos = state.transform.world_to_screen(n2_center);
    assert!((screen_pos.x - rect.center().x).abs() < 1.0);
    assert!((screen_pos.y - rect.center().y).abs() < 1.0);

    // 3. Close code card resets is_code_expanded and scroll_offset_y
    graph.nodes.get_mut(&n1).unwrap().is_code_expanded = true;
    graph.nodes.get_mut(&n1).unwrap().scroll_offset_y = 120.0;
    let events = super::types::RenderEvents { code_close_clicked: Some(n1), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    let node = graph.nodes.get(&n1).unwrap();
    assert!(!node.is_code_expanded);
    assert_eq!(node.scroll_offset_y, 0.0);
}

#[test]
fn test_expanded_code_card_line_anchored_ports() {
    let mut graph = studio_graph::Graph::new();
    let nid = graph.add_node(
        "service.rs",
        NodeArchetype::File,
        "service file",
        Some("RS".to_string()),
        vec![("in".into(), DataType::RustFlow)],
        vec![("out".into(), DataType::RustFlow)],
        [100.0, 100.0],
    );
    let in_p = graph.nodes.get(&nid).unwrap().inputs[0].id;
    let out_p = graph.nodes.get(&nid).unwrap().outputs[0].id;

    let node_mut = graph.nodes.get_mut(&nid).unwrap();
    node_mut.is_code_expanded = true;
    node_mut.size = [540.0, 400.0];
    node_mut.line_number = Some(1);
    node_mut.member_nodes.push(
        FileMemberNode::new(
            "m1",
            "execute",
            NodeArchetype::Function,
            "pub",
            "pub fn execute()",
            10,
            "pub fn execute() {}",
            None,
        )
        .with_ports(Some(in_p), Some(out_p)),
    );

    // Anchoring calculation:
    // header_h (32) + tab_bar_h (28) = content_top = 100 + 60 = 160
    // line_offset = 10 - 1 = 9; 160 + 8 + 9 * 18 + 9 = 339
    let pos_in = port_world_position(node_mut, in_p).unwrap();
    assert_eq!(pos_in.x, 100.0);
    assert_eq!(pos_in.y, 339.0);

    let pos_out = port_world_position(node_mut, out_p).unwrap();
    assert_eq!(pos_out.x, 640.0);
    assert_eq!(pos_out.y, 339.0);

    // When scrolled by 50px, socket shifts up by 50px: 339 - 50 = 289
    node_mut.scroll_offset_y = 50.0;
    let pos_scrolled = port_world_position(node_mut, in_p).unwrap();
    assert_eq!(pos_scrolled.y, 289.0);
}

#[test]
fn test_context_menu_actions_and_subnode_jump() {
    let mut graph = studio_graph::Graph::new();
    let nid = graph.add_node(
        "handler.rs",
        NodeArchetype::File,
        "handler file",
        Some("RS".to_string()),
        vec![],
        vec![],
        [100.0, 100.0],
    );
    let mut state = CanvasState::default();
    let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(1000.0, 800.0));

    // 1. Subnode jump scrolls card to line
    let events = super::types::RenderEvents { subnode_jump_clicked: Some((nid, 15)), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&nid).unwrap().scroll_offset_y, (15 - 1) as f32 * 18.0);

    // 2. ContextMenuAction::ToggleExpand toggles code expansion
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::ToggleExpand)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(graph.nodes.get(&nid).unwrap().is_code_expanded);

    // 3. ContextMenuAction::ViewDocs sets active tab to Documentation (tab 1)
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::ViewDocs)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&nid).unwrap().expanded_tab, 1);

    // 4. ContextMenuAction::CopyPath sets status message
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::CopyPath)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(state.status_message.as_ref().unwrap().contains("handler.rs"));
}

#[test]
fn test_member_fold_clicked_and_dropdown_toggle() {
    let mut graph = studio_graph::Graph::new();
    let nid = graph.add_node(
        "handler.rs",
        studio_graph::NodeArchetype::File,
        "handler file",
        Some("RS".to_string()),
        vec![],
        vec![],
        [100.0, 100.0],
    );

    let mut state = super::types::CanvasState::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(800.0, 600.0));

    // Test member_fold_clicked inserts then removes from collapsed_subnodes
    let events =
        super::types::RenderEvents { member_fold_clicked: Some((nid, "execute".to_string())), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    let events2 =
        super::types::RenderEvents { member_fold_clicked: Some((nid, "execute".to_string())), ..Default::default() };
    super::actions::apply_render_events(events2, &mut state, &mut graph, rect);

    assert!(!state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    // Test file_dropdown_toggle_clicked toggles is_dropdown_expanded
    assert!(!graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
    let events3 = super::types::RenderEvents { file_dropdown_toggle_clicked: Some(nid), ..Default::default() };
    super::actions::apply_render_events(events3, &mut state, &mut graph, rect);
    assert!(graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
}

#[test]
fn test_rmb_panning_threshold_and_state_transitions() {
    use crate::interaction::InteractionMode;

    let start_pointer = Pos2::new(200.0, 150.0);

    let mut mode = InteractionMode::Panning { start_pointer, has_panned: false };

    // 1. Small movement (< 4px) should not trigger panning
    let tiny_move = Pos2::new(202.0, 151.0); // distance = sqrt(4 + 1) = 2.236 < 4.0
    assert!(tiny_move.distance(start_pointer) < 4.0);
    if let InteractionMode::Panning { has_panned, .. } = &mode {
        assert!(!*has_panned);
    }

    // 2. Drag movement (>= 4px) triggers panning
    let drag_pos = Pos2::new(230.0, 170.0);
    assert!(drag_pos.distance(start_pointer) >= 4.0);

    if let InteractionMode::Panning { start_pointer: start, has_panned } = &mut mode {
        if drag_pos.distance(*start) >= 4.0 {
            *has_panned = true;
        }
    }

    assert!(matches!(mode, InteractionMode::Panning { has_panned: true, .. }));
}

#[test]
fn test_zoom_while_panning_maintains_cursor_anchor() {
    use crate::transform::CanvasTransform;

    let mut transform = CanvasTransform::new(Vec2::new(120.0, 100.0), 1.0);
    let cursor = Pos2::new(500.0, 400.0);

    // Initial world point under cursor
    let world_before = transform.screen_to_world(cursor);

    // Simulate mouse wheel zoom in by 25% while hovering at cursor
    transform.zoom_at_pointer(cursor, 1.25);

    // Verify world point under cursor is exactly invariant
    let world_after = transform.screen_to_world(cursor);
    assert!((world_before.x - world_after.x).abs() < 1e-4);
    assert!((world_before.y - world_after.y).abs() < 1e-4);

    // Incremental pan movement (e.g. user moved mouse by (10.0, 5.0))
    let pointer_delta = Vec2::new(10.0, 5.0);
    transform.pan_by(pointer_delta);

    // At the new pointer position, world point matches
    let new_cursor = cursor + pointer_delta;
    let world_at_new_cursor = transform.screen_to_world(new_cursor);
    assert!((world_after.x - world_at_new_cursor.x).abs() < 1e-4);
    assert!((world_after.y - world_at_new_cursor.y).abs() < 1e-4);
}

#[test]
fn test_clicking_unloaded_folder_requests_load_instead_of_toggling() {
    let mut graph = studio_graph::Graph::new();
    graph.tree_layout = true;
    let mut lazy = studio_graph::GroupCluster::new("dir:node_modules", "node_modules", "Folder", 0);
    lazy.detail = studio_graph::FolderDetail::Minimised;
    lazy.lazy = Some(studio_graph::LazyFolder {
        abs_path: "/p/node_modules".into(),
        file_count: 3,
        dir_count: 1,
        total_bytes: 10,
        reason: "dependencies".into(),
    });
    let mut normal = studio_graph::GroupCluster::new("dir:src", "src", "Folder", 0);
    normal.detail = studio_graph::FolderDetail::Minimised;
    graph.clusters = vec![lazy, normal];
    let mut state = CanvasState::default();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));

    use studio_graph::FolderDetail;
    let pick = |id: &str, detail| super::types::RenderEvents {
        folder_detail: Some((id.to_string(), detail)),
        ..Default::default()
    };
    super::actions::apply_render_events(pick("dir:node_modules", FolderDetail::NodeView), &mut state, &mut graph, rect);
    assert!(matches!(
        state.action_request.take(),
        Some(super::types::CanvasAction::ExpandFolder(id, FolderDetail::NodeView)) if id == "dir:node_modules"
    ));
    assert!(graph.clusters[0].is_collapsed(), "stays minimised until its contents arrive");

    super::actions::apply_render_events(pick("dir:src", FolderDetail::NodeView), &mut state, &mut graph, rect);
    assert!(state.action_request.is_none());
    assert_eq!(graph.clusters[1].detail, FolderDetail::NodeView, "loaded folders switch directly");
    super::actions::apply_render_events(pick("dir:src", FolderDetail::Open), &mut state, &mut graph, rect);
    assert_eq!(graph.clusters[1].detail, FolderDetail::Open);
}

#[test]
fn test_wires_toggle_hides_every_wire() {
    let mut graph = studio_graph::create_showcase_graph();
    graph.rebuild_fast_indices();
    assert!(!graph.edges.is_empty());
    let mut state = CanvasState::default();
    state.refresh_scene(&graph);
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0));
    state.zoom_to_fit(&graph, rect);
    state.use_gpu_wires = false;

    let ctx = egui::Context::default();
    let drawn = |state: &CanvasState| {
        let mut wires = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            let world = state.transform.screen_to_world_rect(rect);
            wires = super::render_wires::render_background_and_wires(
                &painter,
                state,
                &graph,
                rect,
                world,
                rect.center(),
                false,
                None,
            )
            .1;
        });
        output.textures_delta.clear();
        wires
    };

    assert!(drawn(&state) > 0, "wires draw by default");
    state.show_wires = false;
    assert_eq!(drawn(&state), 0, "hidden wires are not drawn");
}

/// root/{a/{a1/}, b/}, every folder but the root closed, one card in each leaf.
fn folders_graph() -> studio_graph::Graph {
    use studio_graph::{FolderDetail, GroupCluster};
    let mut g = studio_graph::Graph::new();
    g.tree_layout = true;
    g.flow_layout = true;
    let folder = |id: &str, parent: Option<&str>, kids: &[&str]| {
        let mut c = GroupCluster::new(id, id.rsplit('/').next().unwrap(), "Folder", 0);
        c.parent_id = parent.map(str::to_string);
        c.child_cluster_ids = kids.iter().map(|k| k.to_string()).collect();
        if parent.is_some() {
            c.detail = FolderDetail::NodeView;
        }
        c
    };
    g.clusters = vec![
        folder("root", None, &["root/a", "root/b"]),
        folder("root/a", Some("root"), &["root/a/a1"]),
        folder("root/a/a1", Some("root/a"), &[]),
        folder("root/b", Some("root"), &[]),
    ];
    for (i, f) in [2usize, 3].into_iter().enumerate() {
        let id = g.add_node(format!("f{i}.rs"), NodeArchetype::File, "", None, vec![], vec![], [0.0, 0.0]);
        g.nodes.get_mut(&id).unwrap().size = [220.0, 42.0];
        g.clusters[f].node_ids.push(id);
    }
    g.rebuild_fast_indices();
    g.layout_folder_tree();
    g
}

#[test]
fn opening_a_folder_focuses_and_fits_it_and_the_breadcrumb_closes_back_up() {
    use studio_graph::FolderDetail;
    let mut g = folders_graph();
    let mut state = CanvasState::default();
    let detail = |g: &studio_graph::Graph, id: &str| g.clusters.iter().find(|c| c.id == id).unwrap().detail;

    state.open_folder(&mut g, "root/a");
    state.open_folder(&mut g, "root/a/a1");
    assert_eq!(detail(&g, "root/a"), FolderDetail::Open);
    assert_eq!(detail(&g, "root/a/a1"), FolderDetail::Open);
    assert_eq!(state.focus.as_deref(), Some("root/a/a1"));
    assert_eq!(g.cluster_path("root/a/a1"), ["root", "root/a", "root/a/a1"]);

    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
    state.apply_zoom_request(&g, screen);
    let c = g.clusters.iter().find(|c| c.id == "root/a/a1").unwrap();
    let centre =
        state.transform.world_to_screen(Pos2::new(c.position[0] + c.size[0] * 0.5, c.position[1] + c.size[1] * 0.5));
    assert!((centre - screen.center()).length() < 1.0, "the opened folder is centred");

    // Back up to `a`: what was opened inside it closes, `a` stays open.
    state.focus_folder(&mut g, Some("root/a"));
    assert_eq!(detail(&g, "root/a"), FolderDetail::Open);
    assert_eq!(detail(&g, "root/a/a1"), FolderDetail::NodeView);
    assert_eq!(state.focus.as_deref(), Some("root/a"));

    // The project crumb returns to the top level.
    state.focus_folder(&mut g, None);
    assert_eq!(detail(&g, "root/a"), FolderDetail::NodeView);
    assert_eq!(detail(&g, "root"), FolderDetail::Open);
    assert_eq!(state.focus, None);
}

#[test]
fn selecting_a_card_traces_and_dims_the_rest() {
    let mut g = studio_graph::Graph::new();
    let mk = |g: &mut studio_graph::Graph, n: &str| {
        g.add_node(
            n,
            NodeArchetype::File,
            "",
            None,
            vec![("in".into(), DataType::RustFlow)],
            vec![("out".into(), DataType::RustFlow)],
            [0.0, 0.0],
        )
    };
    let [a, b, c, other] = ["a", "b", "c", "other"].map(|n| mk(&mut g, n));
    for (from, to) in [(a, b), (b, c)] {
        let (o, i) = (g.nodes[&from].outputs[0].id, g.nodes[&to].inputs[0].id);
        g.connect_kind(from, o, to, i, studio_graph::EdgeKind::Import);
    }
    g.rebuild_fast_indices();
    let mut state = CanvasState::default();
    state.selected_nodes.insert(b);
    state.refresh_trace(&g);
    assert_eq!(state.active_flow_edges.as_ref().map(|e| e.len()), Some(2));
    let trace = state.trace_nodes.clone().unwrap();
    assert!(trace.contains(&a) && trace.contains(&c) && !trace.contains(&other));
    assert!(super::render_nodes::is_dimmed(&g.nodes[&other], "", &Default::default(), state.trace_nodes.as_ref()));
    assert!(!super::render_nodes::is_dimmed(&g.nodes[&a], "", &Default::default(), state.trace_nodes.as_ref()));

    state.selected_nodes.clear();
    state.refresh_trace(&g);
    assert!(state.active_flow_edges.is_none() && state.trace_nodes.is_none());
}
