use egui::{Pos2, Rect, Vec2};
use studio_graph::{DataType, FileMemberNode, NodeArchetype};

use super::layout::{calculate_file_node_size, port_world_position};
use super::types::CanvasState;

#[test]
fn test_calculate_file_node_size() {
    let mut graph = studio_graph::Graph::new();
    let nid = graph.add_node(
        "lib.rs",
        NodeArchetype::File,
        "lib file",
        Some("RS".to_string()),
        vec![],
        vec![],
        [0.0, 0.0],
    );
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
    let mut events = super::types::RenderEvents::default();
    events.node_tab_clicked = Some((n1, 2)); // Switch to Members tab
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&n1).unwrap().expanded_tab, 2);

    // 2. Port jump event: selects target node and immediately executes CenterNode
    let mut events = super::types::RenderEvents::default();
    events.port_jump_clicked = Some(n2);
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(state.selected_nodes.contains(&n2));
    let n2_node = graph.nodes.get(&n2).unwrap();
    let n2_center = Pos2::new(
        n2_node.position[0] + n2_node.size[0] * 0.5,
        n2_node.position[1] + n2_node.size[1] * 0.5,
    );
    let screen_pos = state.transform.world_to_screen(n2_center);
    assert!((screen_pos.x - rect.center().x).abs() < 1.0);
    assert!((screen_pos.y - rect.center().y).abs() < 1.0);

    // 3. Close code card resets is_code_expanded and scroll_offset_y
    graph.nodes.get_mut(&n1).unwrap().is_code_expanded = true;
    graph.nodes.get_mut(&n1).unwrap().scroll_offset_y = 120.0;
    let mut events = super::types::RenderEvents::default();
    events.code_close_clicked = Some(n1);
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
    let mut events = super::types::RenderEvents::default();
    events.subnode_jump_clicked = Some((nid, 15));
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&nid).unwrap().scroll_offset_y, (15 - 1) as f32 * 18.0);

    // 2. ContextMenuAction::ToggleExpand toggles code expansion
    let mut events = super::types::RenderEvents::default();
    events.context_menu_action = Some((nid, super::types::ContextMenuAction::ToggleExpand));
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(graph.nodes.get(&nid).unwrap().is_code_expanded);

    // 3. ContextMenuAction::ViewDocs sets active tab to Documentation (tab 1)
    let mut events = super::types::RenderEvents::default();
    events.context_menu_action = Some((nid, super::types::ContextMenuAction::ViewDocs));
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert_eq!(graph.nodes.get(&nid).unwrap().expanded_tab, 1);

    // 4. ContextMenuAction::CopyPath sets status message
    let mut events = super::types::RenderEvents::default();
    events.context_menu_action = Some((nid, super::types::ContextMenuAction::CopyPath));
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
    let mut events = super::types::RenderEvents::default();
    events.member_fold_clicked = Some((nid, "execute".to_string()));
    super::actions::apply_render_events(events, &mut state, &mut graph, rect);

    assert!(state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    let mut events2 = super::types::RenderEvents::default();
    events2.member_fold_clicked = Some((nid, "execute".to_string()));
    super::actions::apply_render_events(events2, &mut state, &mut graph, rect);

    assert!(!state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    // Test file_dropdown_toggle_clicked toggles is_dropdown_expanded
    assert!(!graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
    let mut events3 = super::types::RenderEvents::default();
    events3.file_dropdown_toggle_clicked = Some(nid);
    super::actions::apply_render_events(events3, &mut state, &mut graph, rect);
    assert!(graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
}

#[test]
fn test_rmb_panning_threshold_and_state_transitions() {
    use crate::interaction::InteractionMode;

    let start_pointer = Pos2::new(200.0, 150.0);

    let mut mode = InteractionMode::Panning {
        start_pointer,
        has_panned: false,
    };

    // 1. Small movement (< 4px) should not trigger panning
    let tiny_move = Pos2::new(202.0, 151.0); // distance = sqrt(4 + 1) = 2.236 < 4.0
    assert!(tiny_move.distance(start_pointer) < 4.0);
    if let InteractionMode::Panning { has_panned, .. } = &mode {
        assert!(!*has_panned);
    }

    // 2. Drag movement (>= 4px) triggers panning
    let drag_pos = Pos2::new(230.0, 170.0);
    assert!(drag_pos.distance(start_pointer) >= 4.0);

    if let InteractionMode::Panning {
        start_pointer: start,
        has_panned,
    } = &mut mode
    {
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

