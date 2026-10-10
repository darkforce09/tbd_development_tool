use egui::{CursorIcon, Key, Pos2, Rect, Ui, Vec2};
use studio_graph::Graph;

use crate::camera::{CameraTarget, Stop};
use crate::interaction::{HoverState, InteractionMode};

use super::gate::InputGate;
use super::layout::port_world_position;
use super::types::{CanvasAction, CanvasState, NodeContextMenu};

/// Pans, zooms, hovers, selects and opens the context menu, with only the input the canvas owns
/// this frame, on a canvas filling `rect`. Nothing here changes what the map says: wires come
/// from the code, so the canvas never draws, cuts or deletes them, and cards stay where the
/// layout puts them. Any wheel, pinch, drag or zoom key stops a camera flight.
pub fn handle_canvas_input(state: &mut CanvasState, graph: &mut Graph, ui: &Ui, gate: &InputGate, rect: Rect) {
    // 1. Zoom and pan. A drag with any button moves the view.
    if gate.pointer.is_some_and(|pointer| wheel(state, graph, ui, pointer)) {
        state.camera.cancel();
    }
    if gate.keyboard {
        keys(state, ui, rect);
    }
    if gate.dragging {
        state.camera.cancel();
        if state.interaction == InteractionMode::Idle {
            state.context_menu = None;
        }
        state.interaction = InteractionMode::Panning;
        state.transform.pan_by(gate.drag_delta);
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else {
        state.interaction = InteractionMode::Idle;
    }

    // 2. Spatial grid and scene follow the layout; then what is under the pointer.
    state.refresh_scene(graph);
    state.hover = match gate.pointer {
        Some(pointer) if state.interaction == InteractionMode::Idle => hit_test(state, graph, pointer),
        _ => HoverState::default(),
    };

    let Some(pointer) = gate.pointer else { return };
    // Buttons painted on the canvas (card controls, the context menu) take their own clicks.
    let over_button = state.interactive_rects.iter().any(|r| r.contains(pointer));

    // 3. Clicks: select, open, and the context menu.
    if gate.clicked && !over_button {
        state.context_menu = None;
    }
    if gate.double_clicked && !over_button {
        if let Some(node_id) = state.hover.hovered_node {
            state.action_request = Some(CanvasAction::OpenOnDesk(node_id));
        } else if let Some(id) = folder_at(graph, state.transform.screen_to_world(pointer)) {
            // A closed folder opens in place; an open one fills the view.
            if graph.is_cluster_collapsed(&id) {
                state.open_folder(graph, &id);
            } else {
                state.focus = Some(id.clone());
                state.fly_to(CameraTarget::Folder(id));
            }
        }
    } else if let Some(district) = gate
        .clicked
        .then(|| state.world.region_at(state.transform.screen_to_world(pointer)))
        .flatten()
        .filter(|&d| d != state.stop && !over_button)
    {
        // A click on another district (or on any, from the world view) goes there.
        state.fly_to(CameraTarget::Stop(district));
    } else if gate.clicked && !over_button {
        let shift = ui.input(|i| i.modifiers.shift);
        match state.hover.hovered_node {
            // Shift adds a card to the selection, or takes it out again.
            Some(id) if shift => {
                let added = state.selected_nodes.insert(id);
                if !added {
                    state.selected_nodes.remove(&id);
                }
            }
            Some(id) => {
                state.selected_nodes.clear();
                state.selected_nodes.insert(id);
            }
            None if !shift => state.selected_nodes.clear(),
            None => {}
        }
    }
    if gate.secondary_clicked {
        state.context_menu = state.hover.hovered_node.map(|node_id| {
            state.selected_nodes.clear();
            state.selected_nodes.insert(node_id);
            NodeContextMenu { node_id, screen_pos: pointer }
        });
    }
}

/// Mouse wheel and pinch at `pointer`. Pinch and Ctrl/⌘ + wheel always zoom the map; a plain
/// wheel scrolls the code of an open card under the pointer and zooms everywhere else. Returns
/// whether the view zoomed.
fn wheel(state: &mut CanvasState, graph: &mut Graph, ui: &Ui, pointer: Pos2) -> bool {
    let (zoom_key, scroll, zoom_delta) =
        ui.input(|i| (i.modifiers.ctrl || i.modifiers.command, i.smooth_scroll_delta.y, i.zoom_delta()));
    let pinch = (zoom_delta - 1.0).abs() > 0.001;
    let wheel_factor = 1.0 + (scroll * 0.002).clamp(-0.25, 0.25);
    if (zoom_key && scroll.abs() > 0.0) || pinch {
        state.transform.zoom_at_pointer(pointer, if pinch { zoom_delta } else { wheel_factor });
        true
    } else if scroll.abs() > 0.5 && !scroll_open_card(state, graph, pointer, scroll) {
        state.transform.zoom_at_pointer(pointer, wheel_factor);
        true
    } else {
        false
    }
}

/// Home goes to the world view; `=` and `-` zoom about the middle; `0` goes to 100% (where a
/// district's text is crisp); Escape closes the menu, then clears the selection.
fn keys(state: &mut CanvasState, ui: &Ui, rect: Rect) {
    let (home, zoom_in, zoom_out, actual, escape) = ui.input(|i| {
        let plain = !(i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt);
        (
            i.key_pressed(Key::Home),
            plain && (i.key_pressed(Key::Equals) || i.key_pressed(Key::Plus)),
            plain && i.key_pressed(Key::Minus),
            plain && i.key_pressed(Key::Num0),
            i.key_pressed(Key::Escape),
        )
    });
    if home {
        state.fly_to(CameraTarget::Stop(Stop::World));
    }
    if actual {
        state.actual_size();
    } else if zoom_in || zoom_out {
        state.camera.cancel();
        state.transform.zoom_at_pointer(rect.center(), if zoom_in { 1.25 } else { 0.8 });
    }
    if escape {
        if state.context_menu.is_some() {
            state.context_menu = None;
        } else {
            state.selected_nodes.clear();
        }
    }
}

/// Scrolls the code of the open card under the pointer, if the pointer is on its code. Returns
/// whether it did.
fn scroll_open_card(state: &CanvasState, graph: &mut Graph, pointer: Pos2, scroll: f32) -> bool {
    let Some(node) = state.hover.hovered_node.and_then(|id| graph.nodes.get_mut(&id)) else { return false };
    if !node.is_code_expanded {
        return false;
    }
    let min = state.transform.world_to_screen(Pos2::new(node.position[0], node.position[1]));
    let max =
        state.transform.world_to_screen(Pos2::new(node.position[0] + node.size[0], node.position[1] + node.size[1]));
    let header = (60.0 * state.transform.zoom).max(34.0);
    if !Rect::from_min_max(Pos2::new(min.x, min.y + header), max).contains(pointer) {
        return false;
    }
    let line_count = node.source_code.as_deref().unwrap_or(&node.description).lines().count();
    let max_scroll = (line_count as f32 * 18.0 - (node.size[1] - 80.0)).max(0.0);
    node.scroll_offset_y = (node.scroll_offset_y - scroll * 0.8).clamp(0.0, max_scroll);
    true
}

/// What is under the pointer: a socket first (with its card), then the top-most card, then a wire.
fn hit_test(state: &CanvasState, graph: &Graph, pointer: Pos2) -> HoverState {
    let mut hover = HoverState::default();
    let zoom = state.transform.zoom;
    let pointer_world = state.transform.screen_to_world(pointer);
    let socket_radius = (12.0 * zoom).clamp(7.0, 24.0);
    let socket_world = socket_radius / zoom.max(0.1);
    let candidates =
        state.spatial_grid.query_rect(Rect::from_center_size(pointer_world, Vec2::splat(socket_world * 2.0 + 40.0)));
    let visible = |id| !graph.is_node_in_collapsed_cluster(id);

    for &node_id in candidates.iter().filter(|&&id| visible(id)) {
        let Some(node) = graph.nodes.get(&node_id) else { continue };
        let hit = node.inputs.iter().chain(&node.outputs).find(|port| {
            let member_port =
                node.member_nodes.iter().any(|m| m.in_port_id == Some(port.id) || m.out_port_id == Some(port.id));
            let hidden = member_port
                && (!node.is_dropdown_expanded || (!node.show_member_wires && !state.show_subnode_wires_globally));
            !hidden
                && port_world_position(node, port.id)
                    .is_some_and(|w| state.transform.world_to_screen(w).distance(pointer) <= socket_radius)
        });
        if let Some(port) = hit {
            hover.hovered_port = Some((node_id, port.id));
            hover.hovered_node = Some(node_id);
            return hover;
        }
    }

    hover.hovered_node = candidates.iter().rev().copied().filter(|&id| visible(id)).find(|id| {
        graph.nodes.get(id).is_some_and(|node| {
            let min = state.transform.world_to_screen(Pos2::new(node.position[0], node.position[1]));
            let max = state
                .transform
                .world_to_screen(Pos2::new(node.position[0] + node.size[0], node.position[1] + node.size[1]));
            Rect::from_min_max(min, max).contains(pointer)
        })
    });

    // Wires near the pointer, wherever their cards are.
    if state.show_wires && hover.hovered_node.is_none() {
        let reach = 7.0 / zoom.max(1e-4);
        hover.hovered_edge = state.scene.hit_test(pointer_world, reach, |kind| state.wire_kind_visible(kind));
    }
    hover
}

/// The innermost visible folder under a world point.
fn folder_at(graph: &Graph, world: Pos2) -> Option<String> {
    graph
        .clusters
        .iter()
        .filter(|c| !graph.hidden_cluster_ids.contains(&c.id) && c.size[0] > 1.0)
        .filter(|c| Rect::from_min_size(Pos2::from(c.position), Vec2::new(c.size[0], c.size[1])).contains(world))
        .max_by_key(|c| c.depth)
        .map(|c| c.id.clone())
}
