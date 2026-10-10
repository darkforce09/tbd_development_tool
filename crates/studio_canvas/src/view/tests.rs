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
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert_eq!(graph.nodes.get(&n1).unwrap().expanded_tab, 2);

    // 2. Port jump event: selects the target node and flies the camera to it
    let events = super::types::RenderEvents { port_jump_clicked: Some(n2), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph);
    state.tick_camera(&graph, rect, 0.0, 1.0);
    state.tick_camera(&graph, rect, 10.0, 1.0);

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
    super::actions::apply_render_events(events, &mut state, &mut graph);

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

    // 1. Subnode jump scrolls card to line
    let events = super::types::RenderEvents { subnode_jump_clicked: Some((nid, 15)), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert_eq!(graph.nodes.get(&nid).unwrap().scroll_offset_y, (15 - 1) as f32 * 18.0);

    // 2. ContextMenuAction::ToggleExpand toggles code expansion
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::ToggleExpand)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert!(graph.nodes.get(&nid).unwrap().is_code_expanded);

    // 3. ContextMenuAction::ViewDocs sets active tab to Documentation (tab 1)
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::ViewDocs)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert_eq!(graph.nodes.get(&nid).unwrap().expanded_tab, 1);

    // 4. ContextMenuAction::CopyPath sets status message
    let events = super::types::RenderEvents {
        context_menu_action: Some((nid, super::types::ContextMenuAction::CopyPath)),
        ..Default::default()
    };
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert!(state.status_message.as_ref().unwrap().contains("handler.rs"));
    assert_eq!(state.copy_request.as_deref(), Some("handler.rs"), "the path goes on the clipboard");
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

    // Test member_fold_clicked inserts then removes from collapsed_subnodes
    let events =
        super::types::RenderEvents { member_fold_clicked: Some((nid, "execute".to_string())), ..Default::default() };
    super::actions::apply_render_events(events, &mut state, &mut graph);

    assert!(state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    let events2 =
        super::types::RenderEvents { member_fold_clicked: Some((nid, "execute".to_string())), ..Default::default() };
    super::actions::apply_render_events(events2, &mut state, &mut graph);

    assert!(!state.collapsed_subnodes.contains(&(nid, "execute".to_string())));

    // Test file_dropdown_toggle_clicked toggles is_dropdown_expanded
    assert!(!graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
    let events3 = super::types::RenderEvents { file_dropdown_toggle_clicked: Some(nid), ..Default::default() };
    super::actions::apply_render_events(events3, &mut state, &mut graph);
    assert!(graph.nodes.get(&nid).unwrap().is_dropdown_expanded);
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
        totals: Some(studio_graph::FolderTotals { file_count: 3, dir_count: 1, total_bytes: 10 }),
        reason: "dependencies".into(),
    });
    let mut normal = studio_graph::GroupCluster::new("dir:src", "src", "Folder", 0);
    normal.detail = studio_graph::FolderDetail::Minimised;
    graph.clusters = vec![lazy, normal];
    let mut state = CanvasState::default();

    use studio_graph::FolderDetail;
    let pick = |id: &str, detail| super::types::RenderEvents {
        folder_detail: Some((id.to_string(), detail)),
        ..Default::default()
    };
    super::actions::apply_render_events(pick("dir:node_modules", FolderDetail::NodeView), &mut state, &mut graph);
    assert!(matches!(
        state.action_request.take(),
        Some(super::types::CanvasAction::ExpandFolder(id, FolderDetail::NodeView)) if id == "dir:node_modules"
    ));
    assert!(graph.clusters[0].is_collapsed(), "stays minimised until its contents arrive");

    super::actions::apply_render_events(pick("dir:src", FolderDetail::NodeView), &mut state, &mut graph);
    assert!(state.action_request.is_none());
    assert_eq!(graph.clusters[1].detail, FolderDetail::NodeView, "loaded folders switch directly");
    super::actions::apply_render_events(pick("dir:src", FolderDetail::Open), &mut state, &mut graph);
    assert_eq!(graph.clusters[1].detail, FolderDetail::Open);
}

#[test]
fn test_wires_toggle_hides_every_wire() {
    let mut graph = studio_graph::create_sample_graph();
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
                &super::gate::InputGate::default(),
                &mut Vec::new(),
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
    state.tick_camera(&g, screen, 0.0, 1.0);
    state.tick_camera(&g, screen, 10.0, 1.0);
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
    assert!(super::render_nodes::is_dimmed(&g.nodes[&other], state.trace_nodes.as_ref()));
    assert!(!super::render_nodes::is_dimmed(&g.nodes[&a], state.trace_nodes.as_ref()));

    state.selected_nodes.clear();
    state.refresh_trace(&g);
    assert!(state.active_flow_edges.is_none() && state.trace_nodes.is_none());
}

/// Drives the real canvas through egui frames on a 1200×800 screen, with an optional window on
/// top of it, the way the app draws it.
struct Harness {
    ctx: egui::Context,
    time: f64,
    state: CanvasState,
    graph: studio_graph::Graph,
    /// A clickable window covering this screen rectangle, drawn over the canvas.
    overlay: Option<Rect>,
    /// A focused text field in a window at the top left, drawn over the canvas.
    text_field: Option<String>,
}

impl Harness {
    /// Two file cards with an input and an output each, at world x 100..320 and 600..820,
    /// y 100..142, with the camera at the Code district (zoom 1).
    fn new(connected: bool) -> Self {
        let mut graph = studio_graph::Graph::new();
        let card = |g: &mut studio_graph::Graph, name: &str, x: f32| {
            let id = g.add_node(
                name,
                NodeArchetype::File,
                "",
                Some("RS".to_string()),
                vec![("in".into(), DataType::RustFlow)],
                vec![("out".into(), DataType::RustFlow)],
                [x, 100.0],
            );
            let node = g.nodes.get_mut(&id).unwrap();
            node.size = calculate_file_node_size(node);
            node.file_path = Some(format!("/p/{name}"));
            id
        };
        let a = card(&mut graph, "a.rs", 100.0);
        let b = card(&mut graph, "b.rs", 600.0);
        if connected {
            let (out, inp) = (graph.nodes[&a].outputs[0].id, graph.nodes[&b].inputs[0].id);
            graph.connect_kind(a, out, b, inp, studio_graph::EdgeKind::Import);
        }
        graph.rebuild_fast_indices();
        let mut state = CanvasState::default();
        state.use_gpu_wires = false;
        state.jump_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Code));
        let mut h = Self { ctx: egui::Context::default(), time: 0.0, state, graph, overlay: None, text_field: None };
        h.idle(2);
        assert_eq!(h.state.stop, crate::camera::Stop::Code);
        h
    }

    /// Where a world point is on screen.
    fn at(&self, world: Pos2) -> Pos2 {
        self.state.transform.world_to_screen(world)
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (state, graph, overlay, text_field) =
            (&mut self.state, &mut self.graph, self.overlay, &mut self.text_field);
        let mut output = self.ctx.run_ui(raw, |ui| {
            super::CanvasView::new(&mut *state, &mut *graph).show(ui);
            if let Some(r) = overlay {
                egui::Area::new(egui::Id::new("overlay")).fixed_pos(r.min).show(ui.ctx(), |ui| {
                    ui.allocate_exact_size(r.size(), egui::Sense::click());
                });
            }
            if let Some(text) = text_field.as_mut() {
                egui::Area::new(egui::Id::new("field")).fixed_pos(Pos2::new(10.0, 10.0)).show(ui.ctx(), |ui| {
                    ui.add(egui::TextEdit::singleline(text)).request_focus();
                });
            }
        });
        output.textures_delta.clear();
    }

    fn idle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame(Vec::new());
        }
    }

    fn move_to(&mut self, pos: Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.idle(2);
    }

    fn button(&mut self, pos: Pos2, button: egui::PointerButton, pressed: bool) {
        self.frame(vec![egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }]);
    }

    fn click(&mut self, pos: Pos2, button: egui::PointerButton) {
        self.move_to(pos);
        self.button(pos, button, true);
        self.button(pos, button, false);
        self.idle(2);
    }

    fn key(&mut self, key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![event(true)]);
        self.frame(vec![event(false)]);
    }

    fn card(&self, name: &str) -> studio_graph::NodeId {
        self.graph.nodes.values().find(|n| n.title == name).unwrap().id
    }
}

/// World points on the harness cards.
const CARD_A: Pos2 = Pos2::new(210.0, 121.0);
const A_OUT: Pos2 = Pos2::new(320.0, 121.0);
const B_IN: Pos2 = Pos2::new(600.0, 121.0);
/// Card A's open-members button, painted by the canvas at the right of its header.
const A_DROPDOWN: Pos2 = Pos2::new(286.0, 119.0);

#[test]
fn backspace_and_delete_never_remove_cards() {
    let mut h = Harness::new(true);
    h.click(h.at(CARD_A), egui::PointerButton::Primary);
    assert!(h.state.selected_nodes.contains(&h.card("a.rs")), "the click selects the card");
    h.key(egui::Key::Backspace);
    h.key(egui::Key::Delete);
    assert_eq!(h.graph.nodes.len(), 2);
    assert_eq!(h.graph.edges.len(), 1);
}

#[test]
fn keys_typed_into_a_text_field_never_move_the_map() {
    let mut h = Harness::new(false);
    h.text_field = Some(String::new());
    h.idle(3);
    let before = (h.state.transform.pan, h.state.transform.zoom);
    for key in
        [egui::Key::Space, egui::Key::Home, egui::Key::ArrowLeft, egui::Key::Minus, egui::Key::Equals, egui::Key::Num0]
    {
        h.key(key);
    }
    // Space held while dragging out of the text field and across the map.
    let space = |pressed| egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    let field = Pos2::new(30.0, 20.0);
    h.move_to(field);
    h.frame(vec![space(true)]);
    h.button(field, egui::PointerButton::Primary, true);
    for x in [60.0, 200.0, 400.0, 600.0] {
        h.frame(vec![egui::Event::PointerMoved(Pos2::new(x, 300.0))]);
    }
    h.button(Pos2::new(600.0, 300.0), egui::PointerButton::Primary, false);
    h.frame(vec![space(false)]);
    h.idle(60);
    assert_eq!((h.state.transform.pan, h.state.transform.zoom), before);
}

#[test]
fn zoom_keys_work_when_nothing_has_focus() {
    let mut h = Harness::new(false);
    let middle = Pos2::new(600.0, 400.0);
    let world = h.state.transform.screen_to_world(middle);
    h.key(egui::Key::Equals);
    assert!((h.state.transform.zoom - 1.25).abs() < 1e-5);
    assert!((h.at(world) - middle).length() < 0.01, "zoomed about the middle");
    h.key(egui::Key::Num0);
    assert!((h.state.transform.zoom - 1.0).abs() < 1e-5, "0 goes back to 100%");
    h.key(egui::Key::Home);
    h.idle(70);
    assert_eq!(h.state.stop, crate::camera::Stop::World, "Home flies to the world view");
}

#[test]
fn the_wheel_over_a_window_never_zooms_the_map() {
    let wheel = || egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, 120.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    };
    let mut h = Harness::new(false);
    h.overlay = Some(Rect::from_min_size(Pos2::new(500.0, 400.0), Vec2::new(300.0, 300.0)));
    h.move_to(Pos2::new(600.0, 500.0));
    h.frame(vec![wheel()]);
    h.idle(30);
    assert_eq!(h.state.transform.zoom, 1.0, "the window took the wheel");

    h.move_to(Pos2::new(100.0, 500.0));
    h.frame(vec![wheel()]);
    h.idle(30);
    assert!(h.state.transform.zoom > 1.0, "the bare map zooms");
}

#[test]
fn right_clicking_a_socket_or_wire_keeps_the_wire() {
    let mut h = Harness::new(true);
    for at in [A_OUT, B_IN, Pos2::new(460.0, 121.0)] {
        h.click(h.at(at), egui::PointerButton::Secondary);
    }
    assert_eq!(h.graph.edges.len(), 1);
}

#[test]
fn dragging_from_a_socket_draws_no_wire_and_pans_instead() {
    let mut h = Harness::new(false);
    let pan = h.state.transform.pan;
    let (from, to) = (h.at(A_OUT), h.at(B_IN));
    h.move_to(from);
    h.button(from, egui::PointerButton::Primary, true);
    for i in 1..=10 {
        h.frame(vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 10.0))]);
    }
    h.button(to, egui::PointerButton::Primary, false);
    h.idle(2);
    assert!(h.graph.edges.is_empty(), "no wire was drawn");
    assert_eq!(h.state.transform.pan - pan, to - from, "the point grabbed stayed under the pointer");
    let a = h.card("a.rs");
    assert_eq!(h.graph.nodes[&a].position, [100.0, 100.0], "cards never move");
}

#[test]
fn a_click_on_a_window_never_reaches_the_buttons_painted_below() {
    let mut h = Harness::new(false);
    let a = h.card("a.rs");
    let button = h.at(A_DROPDOWN);
    h.overlay = Some(Rect::from_center_size(button, Vec2::splat(80.0)));
    h.click(button, egui::PointerButton::Primary);
    assert!(!h.graph.nodes[&a].is_dropdown_expanded, "the window took the click");

    h.overlay = None;
    h.click(button, egui::PointerButton::Primary);
    assert!(h.graph.nodes[&a].is_dropdown_expanded, "the button works when nothing covers it");
}

#[test]
fn a_pointer_over_a_window_hovers_nothing_below() {
    let mut h = Harness::new(false);
    h.overlay = Some(Rect::from_center_size(h.at(CARD_A), Vec2::splat(120.0)));
    h.move_to(h.at(CARD_A));
    assert_eq!(h.state.hover.hovered_node, None);

    h.overlay = None;
    h.idle(2);
    assert_eq!(h.state.hover.hovered_node, Some(h.card("a.rs")));
}

#[test]
fn clicking_another_district_flies_there_and_dims_the_rest() {
    let mut h = Harness::new(false);
    let desk = h.state.world.desk;
    h.click(h.at(desk.center() - Vec2::new(0.0, desk.height() * 0.45)), egui::PointerButton::Primary);
    assert!(h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Desk, "on its way, the destination counts");
    h.idle(70);
    assert!(!h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Desk);
    assert!((h.state.transform.zoom - h.state.world.local_zoom_one()).abs() < 1e-6, "the desk reads at 100%");
    assert!(h.state.selected_nodes.is_empty(), "the click only flew");
}

#[test]
fn any_drag_stops_a_flight_where_it_is() {
    let mut h = Harness::new(false);
    h.state.fly_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Run));
    h.idle(10);
    assert!(h.state.camera.is_flying());
    let p = Pos2::new(600.0, 400.0);
    h.move_to(p);
    h.button(p, egui::PointerButton::Secondary, true);
    h.frame(vec![egui::Event::PointerMoved(p + Vec2::new(40.0, 0.0))]);
    assert!(!h.state.camera.is_flying(), "the user took over");
    h.button(p + Vec2::new(40.0, 0.0), egui::PointerButton::Secondary, false);
}

#[test]
fn double_clicking_a_file_opens_it_on_the_desk_at_100_percent() {
    let mut h = Harness::new(false);
    h.state.desk.root = Some(std::path::PathBuf::from("/p"));
    let at = h.at(CARD_A);
    h.move_to(at);
    for _ in 0..2 {
        h.button(at, egui::PointerButton::Primary, true);
        h.button(at, egui::PointerButton::Primary, false);
    }
    assert_eq!(h.state.desk.cards.len(), 1);
    assert_eq!(h.state.desk.cards[0].rel, "a.rs");
    assert!(
        matches!(&h.state.desk.requests[..], [crate::desk::DeskRequest::Read { .. }]),
        "the host is asked to read it"
    );
    h.idle(1);
    assert_eq!(h.state.stop, crate::camera::Stop::Desk, "on its way to the Desk");
    h.idle(70);
    let one = h.state.world.local_zoom_one();
    assert!((h.state.transform.zoom - one).abs() < 1e-6, "the Desk reads at 100%");
    let origin = h.at(h.state.world.desk.min);
    assert_eq!(origin, origin.round(), "on whole pixels, so its text is crisp");

    // Filling the card draws its text without trouble.
    let id = h.state.desk.cards[0].id;
    let content = crate::desk::content_for_text(std::path::Path::new("/p/a.rs"), "fn a() {}\n/* one\n two */\n".into());
    h.state.desk.fill(id, content);
    h.idle(3);
    assert!(matches!(h.state.desk.cards[0].content, crate::desk::CardContent::Code(_)));
}

#[test]
fn painter_draws_a_proven_wire_whole_and_others_in_dashes() {
    use super::render_wires::{wire_shapes, WirePath};
    use studio_graph::EvidenceTier;
    let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(1000.0));
    let segment = WirePath::Segment([Pos2::new(10.0, 50.0), Pos2::new(400.0, 50.0)]);
    let curve = WirePath::Curve([
        Pos2::new(10.0, 50.0),
        Pos2::new(200.0, 50.0),
        Pos2::new(200.0, 300.0),
        Pos2::new(400.0, 300.0),
    ]);
    for path in [segment, curve] {
        let shapes = |tier| {
            let mut out = Vec::new();
            wire_shapes(path, tier, stroke, clip, &mut out);
            out.len()
        };
        assert_eq!(shapes(EvidenceTier::Proven), 1, "{path:?}");
        for tier in [EvidenceTier::PossibleSet, EvidenceTier::Observed, EvidenceTier::Unresolved] {
            assert!(shapes(tier) > 1, "{tier:?} {path:?}");
        }
        // Short dashes are more numerous than long ones.
        assert!(shapes(EvidenceTier::Unresolved) > shapes(EvidenceTier::PossibleSet));
    }
}

#[test]
fn painter_dashes_only_what_is_on_screen_and_keeps_them_in_place() {
    use super::render_wires::{wire_shapes, WirePath};
    use studio_graph::EvidenceTier;
    let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
    let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
    // A wire starting far off screen: only dashes near the screen are made.
    let dashes = |start: f32| {
        let mut out = Vec::new();
        wire_shapes(
            WirePath::Segment([Pos2::new(start, 10.0), Pos2::new(150.0, 10.0)]),
            EvidenceTier::Unresolved,
            stroke,
            clip,
            &mut out,
        );
        out.iter()
            .map(|s| match s {
                egui::Shape::LineSegment { points, .. } => points[0].x,
                other => panic!("{other:?}"),
            })
            .collect::<Vec<f32>>()
    };
    let far = dashes(-100_000.0);
    assert!(!far.is_empty() && far.len() <= 16, "{}", far.len());
    // Dash starts sit on the wire's own 12 px grid, wherever the clip cut it.
    for x in &far {
        let from_start = x + 100_000.0;
        let off = from_start - (from_start / 12.0).round() * 12.0;
        assert!(off.abs() < 0.05, "{x} is {off} px off the pattern");
    }
    // A wire missing the screen makes nothing.
    let mut out = Vec::new();
    wire_shapes(
        WirePath::Segment([Pos2::new(-500.0, 900.0), Pos2::new(-10.0, 900.0)]),
        EvidenceTier::Unresolved,
        stroke,
        clip,
        &mut out,
    );
    assert!(out.is_empty());
}

/// A Pipeline with one open group of one flow (two Rust steps) and one closed group.
fn pipeline_view() -> crate::districts::pipeline::PipelineView {
    use crate::districts::pipeline::*;
    let step = |title: &str, line| PipelineStep {
        title: title.into(),
        detail: String::new(),
        language: "Rust".into(),
        path: std::path::PathBuf::from(format!("src/{title}.rs")),
        line,
    };
    let lane = PipelineLane {
        name: "GET /a".into(),
        crossing: false,
        layers: vec![vec![step("route", 4)], vec![step("handler", 7)]],
        links: vec![PipelineLink {
            from: StepSlot { layer: 0, index: 0 },
            to: StepSlot { layer: 1, index: 0 },
            tier: studio_graph::EvidenceTier::Proven,
            candidates: 1,
            conditional: Some("dev".into()),
            crossing: false,
            label: String::new(),
        }],
        truncated: 0,
    };
    PipelineView {
        groups: vec![
            PipelineGroup {
                key: "open".into(),
                title: "Open".into(),
                count: 1,
                expanded: true,
                lanes: vec![lane.clone()],
            },
            PipelineGroup {
                key: "closed".into(),
                title: "Closed".into(),
                count: 3,
                expanded: false,
                lanes: vec![lane; 3],
            },
        ],
    }
}

/// The harness at the Pipeline stop with [`pipeline_view`] arrived.
fn harness_at_pipeline() -> Harness {
    let mut h = Harness::new(false);
    h.state.desk.root = Some(std::path::PathBuf::from("/p"));
    h.state.districts.pipeline = Some(std::sync::Arc::new(pipeline_view()));
    h.state.refresh_world();
    h.state.jump_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Pipeline));
    h.idle(3);
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
    h
}

/// Where a point of the Pipeline district, in its own units, is on screen.
fn pipeline_point(h: &Harness, local: Pos2) -> Pos2 {
    h.at(h.state.world.pipeline.min + local.to_vec2() * h.state.world.scale)
}

fn pipeline_layout(h: &Harness) -> crate::districts::pipeline::PipelineLayout {
    let view = h.state.districts.pipeline.clone().unwrap();
    let width = h.state.world.pipeline.width() / h.state.world.scale;
    crate::districts::pipeline::layout(&view, width, &h.state.pipeline_open)
}

#[test]
fn the_pipeline_stop_lands_at_100_percent_on_its_top_left() {
    let h = harness_at_pipeline();
    let one = h.state.world.local_zoom_one();
    assert!((h.state.transform.zoom - one).abs() < 1e-6, "Pipeline reads at 100%");
    let corner = h.at(h.state.world.pipeline.min);
    assert_eq!(corner, Pos2::new(48.0, 48.0), "its top left corner, header first");
    let first_card = pipeline_layout(&h).groups[0].lanes[0].cards[0].1;
    assert!(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0)).contains(pipeline_point(&h, first_card.center())));
}

#[test]
fn clicking_a_pipeline_step_opens_its_file_on_the_desk_at_its_line() {
    let mut h = harness_at_pipeline();
    let card = pipeline_layout(&h).groups[0].lanes[0].cards[1].1;
    h.click(pipeline_point(&h, card.center()), egui::PointerButton::Primary);
    assert_eq!(h.state.desk.cards.len(), 1);
    assert_eq!(h.state.desk.cards[0].path, std::path::PathBuf::from("/p/src/handler.rs"));
    assert_eq!(h.state.desk.cards[0].scroll_to, Some(7));
    assert_eq!(h.state.stop, crate::camera::Stop::Desk, "on its way to the Desk");
}

#[test]
fn opening_a_pipeline_group_grows_the_district_north_and_keeps_its_header_in_place() {
    let mut h = harness_at_pipeline();
    let (code, bottom) = (h.state.world.code, h.state.world.pipeline.bottom());
    let (top, desk) = (h.state.world.pipeline.top(), h.state.world.desk);
    let top_on_screen = h.at(h.state.world.pipeline.min);
    let header = pipeline_layout(&h).groups[1].header;
    let clicked = pipeline_point(&h, header.left_center() + Vec2::new(120.0, 0.0));
    let zoom = h.state.transform.zoom;
    h.click(clicked, egui::PointerButton::Primary);
    assert!(h.state.pipeline_open.contains("closed"));
    assert_eq!(pipeline_layout(&h).groups[1].lanes.len(), 3);
    assert_eq!(h.state.world.code, code, "the map never moves");
    assert_eq!(h.state.world.desk, desk);
    assert_eq!(h.state.world.pipeline.bottom(), bottom);
    assert!(h.state.world.pipeline.top() < top, "it grew north");
    assert_eq!(h.at(h.state.world.pipeline.min), top_on_screen, "its top edge keeps its place on screen");
    let header = pipeline_layout(&h).groups[1].header;
    let header_now = pipeline_point(&h, header.left_center() + Vec2::new(120.0, 0.0));
    assert!((header_now - clicked).length() < 1e-3, "the clicked header did not move: {header_now:?} vs {clicked:?}");
    assert_eq!(h.state.transform.zoom, zoom);
    assert!(!h.state.camera.is_flying());
    assert!(h.state.desk.cards.is_empty(), "a header click opens nothing");

    // And closed again, it is as it was, the header still under the pointer.
    h.click(header_now, egui::PointerButton::Primary);
    assert!(h.state.pipeline_open.is_empty());
    assert_eq!(h.state.world.pipeline.top(), top);
    assert_eq!(h.at(h.state.world.pipeline.min), top_on_screen);
}

#[test]
fn an_empty_pipeline_is_drawn_without_trouble() {
    let mut h = Harness::new(false);
    h.state.districts.pipeline = Some(std::sync::Arc::new(Default::default()));
    h.state.refresh_world();
    h.state.jump_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Pipeline));
    h.idle(3);
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
    assert_eq!(h.state.world.pipeline.height(), 900.0 * h.state.world.scale, "nothing to show keeps the fixed height");
}

#[test]
fn clicking_the_pipeline_from_the_world_flies_there_and_opens_no_step() {
    let mut h = harness_at_pipeline();
    h.state.jump_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::World));
    h.idle(3);
    assert_eq!(h.state.stop, crate::camera::Stop::World);
    // Right on a step: the click that starts the flight is not also a click on the step.
    let card = pipeline_layout(&h).groups[0].lanes[0].cards[1].1;
    h.click(pipeline_point(&h, card.center()), egui::PointerButton::Primary);
    assert!(h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
    h.idle(70);
    assert!(!h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
    assert!(h.state.desk.cards.is_empty(), "the click only flew");
}

/// [`pipeline_view`] with its closed group open by default and more flows: taller than the
/// fixed height, so it grows the district north when it arrives.
fn taller_pipeline_view() -> crate::districts::pipeline::PipelineView {
    let mut view = pipeline_view();
    let lane = view.groups[1].lanes[0].clone();
    view.groups[1].expanded = true;
    view.groups[1].lanes = vec![lane; 8];
    view.groups[1].count = 8;
    view
}

#[test]
fn pipeline_data_arriving_while_viewing_it_keeps_its_header_in_place() {
    let mut h = harness_at_pipeline();
    let top = h.state.world.pipeline.top();
    let top_on_screen = h.at(h.state.world.pipeline.min);
    assert_eq!(top_on_screen, Pos2::new(48.0, 48.0));
    let header = pipeline_layout(&h).groups[0].header;
    let header_on_screen = pipeline_point(&h, header.min);

    h.state.districts.pipeline = Some(std::sync::Arc::new(taller_pipeline_view()));
    h.state.refresh_world();
    h.idle(3);
    assert!(h.state.world.pipeline.top() < top, "it grew north");
    assert_eq!(h.at(h.state.world.pipeline.min), top_on_screen, "its top edge keeps its place on screen");
    assert!((pipeline_point(&h, pipeline_layout(&h).groups[0].header.min) - header_on_screen).length() < 1e-3);
    assert!(!h.state.camera.is_flying(), "the camera did not fly");
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
}

#[test]
fn a_flight_to_the_pipeline_lands_on_its_new_top_left_when_it_grows_on_the_way() {
    let mut h = Harness::new(false);
    h.state.districts.pipeline = Some(std::sync::Arc::new(pipeline_view()));
    h.state.refresh_world();
    h.state.fly_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Pipeline));
    h.idle(10);
    assert!(h.state.camera.is_flying());
    let top = h.state.world.pipeline.top();

    h.state.districts.pipeline = Some(std::sync::Arc::new(taller_pipeline_view()));
    h.state.refresh_world();
    assert!(h.state.world.pipeline.top() < top, "it grew north on the way");
    h.idle(80);
    assert!(!h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Pipeline);
    assert!((h.state.transform.zoom - h.state.world.local_zoom_one()).abs() < 1e-6, "Pipeline reads at 100%");
    assert_eq!(h.at(h.state.world.pipeline.min), Pos2::new(48.0, 48.0), "on its new top left corner");
}

#[test]
fn the_pipeline_is_laid_out_on_arrival_and_toggle_never_per_frame() {
    use crate::districts::pipeline::LAYOUTS_BUILT;
    let built = || LAYOUTS_BUILT.with(|n| n.get());
    let mut h = harness_at_pipeline();
    let header = pipeline_layout(&h).groups[1].header;
    let cached = h.state.pipeline_layout_cache.as_ref().map(|c| c.3.clone()).unwrap();
    let before = built();
    h.idle(10);
    assert_eq!(built(), before, "idle frames reuse the layout");
    assert!(std::sync::Arc::ptr_eq(&cached, &h.state.pipeline_layout_cache.as_ref().unwrap().3));

    // A toggle lays it out again, once; so does a new view.
    h.click(pipeline_point(&h, header.left_center() + Vec2::new(120.0, 0.0)), egui::PointerButton::Primary);
    assert!(h.state.pipeline_open.contains("closed"));
    assert!(!std::sync::Arc::ptr_eq(&cached, &h.state.pipeline_layout_cache.as_ref().unwrap().3));
    let after_toggle = built();
    assert_eq!(after_toggle, before + 1, "one layout for the toggle");
    h.idle(10);
    assert_eq!(built(), after_toggle);
    h.state.districts.pipeline = Some(std::sync::Arc::new(taller_pipeline_view()));
    h.state.refresh_world();
    h.idle(10);
    assert_eq!(built(), after_toggle + 1);
}

#[test]
fn clicking_the_run_district_from_the_world_flies_there_and_picks_no_tile() {
    use crate::districts::run::{tile_world_rect, RunView, ToolTile};
    let mut h = Harness::new(false);
    let tile = ToolTile {
        name: "build".into(),
        kind: "script",
        icon: "",
        invocation: "make build".into(),
        description: None,
        file: std::path::PathBuf::from("/p/Makefile"),
        line: 1,
        children: Vec::new(),
    };
    let view = RunView { tools: vec![tile], ..Default::default() };
    h.state.districts.run = Some(std::sync::Arc::new(view.clone()));
    h.state.jump_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::World));
    h.idle(3);
    assert_eq!(h.state.stop, crate::camera::Stop::World);
    // Right on the tile: the click that starts the flight is not also a click on the tile.
    let r = tile_world_rect(&h.state.world, &view, 0).unwrap();
    h.click(h.at(r.center()), egui::PointerButton::Primary);
    assert!(h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Run);
    h.idle(70);
    assert!(!h.state.camera.is_flying());
    assert_eq!(h.state.run_selected, None, "the click only flew");

    // There, the same tile picks it.
    let r = tile_world_rect(&h.state.world, &view, 0).unwrap();
    h.click(h.at(r.center()), egui::PointerButton::Primary);
    assert_eq!(h.state.run_selected, Some(0));
}

#[test]
fn clicking_the_files_district_from_the_map_flies_there_and_opens_no_folder() {
    let mut h = Harness::new(false);
    h.graph = folders_graph();
    h.state.mark_scene_dirty();
    h.state.districts.files = Some(std::sync::Arc::new(Default::default()));
    h.idle(2);
    // The tree's first row (folder "a"), in the district's units: tree at (36, 112), rows from
    // 64 below its top, 22 tall and 330 wide.
    let row = |h: &Harness| {
        h.at(h.state.world.files.min + Vec2::new(36.0 + 14.0 + 165.0, 112.0 + 64.0 + 11.0) * h.state.world.scale)
    };
    // On the map, zoomed out just enough to read the tree's rows at its left.
    let world = h.state.world;
    h.state.transform.zoom = 0.36 * world.local_zoom_one();
    let centre = Pos2::new(world.code_cell.left() + 10.0, world.code_cell.center().y);
    h.state.transform.pan = Pos2::new(600.0, 400.0).to_vec2() - centre.to_vec2() * h.state.transform.zoom;
    h.idle(2);
    assert_eq!(h.state.stop, crate::camera::Stop::Code);
    assert!(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0)).contains(row(&h)), "the row is on screen");
    // Right on the row: the click that starts the flight is not also a click on the row.
    h.click(row(&h), egui::PointerButton::Primary);
    assert!(h.state.camera.is_flying());
    assert_eq!(h.state.stop, crate::camera::Stop::Files);
    h.idle(70);
    assert!(!h.state.camera.is_flying());
    assert!(h.state.files_open.is_empty(), "the click only flew");

    // There, the same row opens the folder.
    h.click(row(&h), egui::PointerButton::Primary);
    assert!(h.state.files_open.contains("root/a"));
}

#[test]
fn a_flight_stopped_by_a_drag_leaves_the_camera_where_it_is() {
    let mut h = Harness::new(false);
    h.state.fly_to(crate::camera::CameraTarget::Stop(crate::camera::Stop::Run));
    h.idle(3);
    assert_eq!(h.state.stop, crate::camera::Stop::Run, "on its way, the destination counts");
    let p = Pos2::new(600.0, 400.0);
    h.move_to(p);
    h.button(p, egui::PointerButton::Secondary, true);
    h.frame(vec![egui::Event::PointerMoved(p + Vec2::new(40.0, 0.0))]);
    h.button(p + Vec2::new(40.0, 0.0), egui::PointerButton::Secondary, false);
    h.idle(2);
    assert!(!h.state.camera.is_flying());
    let here = h.state.world.stop_for_view(
        h.state.transform.screen_to_world_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
    );
    assert_eq!(h.state.stop, here, "the stop is where the camera is, not where it was going");
    assert_ne!(here, crate::camera::Stop::Run, "stopped early, it is not at Run yet");
}
