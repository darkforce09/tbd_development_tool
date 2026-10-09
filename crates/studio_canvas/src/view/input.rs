use std::collections::HashSet;
use egui::{Key, PointerButton, Pos2, Rect, Ui, Vec2};
use studio_graph::{can_connect, Graph, NodeId, PortDirection};

use crate::interaction::{HoverState, InteractionMode};
use crate::wire::{compute_bezier_control_points, distance_to_bezier};

use super::layout::port_world_position;
use super::types::{CanvasAction, CanvasState, NodeContextMenu};

pub fn handle_canvas_input(
    state: &mut CanvasState,
    graph: &mut Graph,
    ui: &mut Ui,
    rect: Rect,
) {
    // 1. Process Panning & Zooming Inputs
    let pointer_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
    let is_space_pressed = ui.input(|i| i.key_down(Key::Space));

    // Mouse wheel: Disambiguate zoom vs card code scrolling
    let is_ctrl_or_cmd = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
    let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
    let zoom_delta = ui.input(|i| i.zoom_delta());
    let is_pinch = (zoom_delta - 1.0).abs() > 0.001;

    // Intentional zoom always zooms canvas, NEVER scrolls code card
    let mut did_zoom = false;
    if is_ctrl_or_cmd || is_pinch {
        let factor = if is_pinch {
            zoom_delta
        } else {
            1.0 + (scroll_delta * 0.002).clamp(-0.25, 0.25)
        };
        state.transform.zoom_at_pointer(pointer_pos, factor);
        did_zoom = true;
    } else if scroll_delta.abs() > 0.5 {
        // Plain wheel scroll: only scroll code if cursor is strictly inside an expanded card's code body
        let mut card_scrolled = false;
        if let Some(hovered_id) = state.hover.hovered_node {
            if let Some(hovered_node) = graph.nodes.get_mut(&hovered_id) {
                if hovered_node.is_code_expanded {
                    let min_screen = state.transform.world_to_screen(Pos2::new(hovered_node.position[0], hovered_node.position[1]));
                    let max_screen = state.transform.world_to_screen(Pos2::new(
                        hovered_node.position[0] + hovered_node.size[0],
                        hovered_node.position[1] + hovered_node.size[1],
                    ));
                    let card_rect = Rect::from_min_max(min_screen, max_screen);
                    let header_tab_h = (60.0 * state.transform.zoom).max(34.0);
                    let body_rect = Rect::from_min_max(
                        Pos2::new(card_rect.min.x, card_rect.min.y + header_tab_h),
                        card_rect.max,
                    );

                    if body_rect.contains(pointer_pos) {
                        let line_count = hovered_node.source_code.as_deref().unwrap_or(&hovered_node.description).lines().count();
                        let max_scroll = (line_count as f32 * 18.0 - (hovered_node.size[1] - 80.0)).max(0.0);
                        hovered_node.scroll_offset_y = (hovered_node.scroll_offset_y - scroll_delta * 0.8).clamp(0.0, max_scroll);
                        card_scrolled = true;
                    }
                }
            }
        }

        if !card_scrolled {
            let factor = 1.0 + (scroll_delta * 0.002).clamp(-0.25, 0.25);
            state.transform.zoom_at_pointer(pointer_pos, factor);
            did_zoom = true;
        }
    }

    // Panning via right mouse drag or space + left drag
    let is_secondary_down = ui.input(|i| i.pointer.button_down(PointerButton::Secondary));
    let is_secondary_pressed = ui.input(|i| i.pointer.secondary_pressed());
    let is_primary_down = ui.input(|i| i.pointer.button_down(PointerButton::Primary));
    let pointer_delta = ui.input(|i| i.pointer.delta());

    let is_space_pan = is_space_pressed && is_primary_down;
    if is_space_pan {
        state.transform.pan_by(pointer_delta);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }

    if (is_secondary_pressed || is_secondary_down) && state.interaction == InteractionMode::Idle {
        state.interaction = InteractionMode::Panning {
            start_pointer: pointer_pos,
            has_panned: false,
        };
    }

    if let InteractionMode::Panning {
        start_pointer,
        has_panned,
    } = &mut state.interaction
    {
        if is_secondary_down {
            if did_zoom {
                *has_panned = true;
                state.context_menu = None;
            }
            if !*has_panned && pointer_pos.distance(*start_pointer) >= 4.0 {
                *has_panned = true;
                state.context_menu = None;
            }
            if *has_panned {
                state.transform.pan_by(pointer_delta);
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
    }

    let is_panning = matches!(state.interaction, InteractionMode::Panning { has_panned: true, .. });
    let wants_pan = is_space_pan || is_secondary_down || is_panning;

    // Reset hover state before hit-testing
    let mut new_hover = HoverState::default();

    // 2. Ensure spatial grid is initialized
    if state.spatial_grid_dirty
        || (state.spatial_grid.get_bounds(graph.nodes.keys().next().copied().unwrap_or(NodeId(0))).is_none()
            && !graph.nodes.is_empty())
    {
        state.spatial_grid.build_from_graph(graph);
        state.spatial_grid_dirty = false;
    }

    // 3. Fast Spatial Hit-Testing: Query only nodes near the mouse cursor
    let pointer_world = state.transform.screen_to_world(pointer_pos);
    let socket_hit_radius = (12.0 * state.transform.zoom).clamp(7.0, 24.0);
    let socket_hit_world = socket_hit_radius / state.transform.zoom.max(0.1);
    let candidate_rect = Rect::from_center_size(pointer_world, Vec2::splat(socket_hit_world * 2.0 + 40.0));
    let candidate_nodes = state.spatial_grid.query_rect(candidate_rect);

    // Test sockets on candidate nodes
    for &node_id in &candidate_nodes {
        if graph.is_node_in_collapsed_cluster(node_id) {
            continue;
        }
        if let Some(node) = graph.nodes.get(&node_id) {
            for port in node.inputs.iter().chain(node.outputs.iter()) {
                let is_member_port = node.member_nodes.iter().any(|m| m.in_port_id == Some(port.id) || m.out_port_id == Some(port.id));
                if is_member_port && (!node.is_dropdown_expanded || (!node.show_member_wires && !state.show_subnode_wires_globally)) {
                    continue;
                }
                if let Some(wpos) = port_world_position(node, port.id) {
                    let spos = state.transform.world_to_screen(wpos);
                    if spos.distance(pointer_pos) <= socket_hit_radius {
                        new_hover.hovered_port = Some((node_id, port.id));
                        break;
                    }
                }
            }
            if new_hover.hovered_port.is_some() {
                break;
            }
        }
    }

    // Test candidate node cards
    if new_hover.hovered_port.is_none() {
        for &node_id in candidate_nodes.iter().rev() {
            if graph.is_node_in_collapsed_cluster(node_id) {
                continue;
            }
            if let Some(node) = graph.nodes.get(&node_id) {
                let min_screen = state.transform.world_to_screen(Pos2::new(node.position[0], node.position[1]));
                let max_screen = state.transform.world_to_screen(Pos2::new(
                    node.position[0] + node.size[0],
                    node.position[1] + node.size[1],
                ));
                let card_rect = Rect::from_min_max(min_screen, max_screen);
                if card_rect.contains(pointer_pos) {
                    new_hover.hovered_node = Some(node_id);
                    break;
                }
            }
        }
    }

    // Test edges touching candidate nodes
    if new_hover.hovered_port.is_none() && new_hover.hovered_node.is_none() {
        let mut checked_edges = HashSet::new();
        for &node_id in &candidate_nodes {
            if let Some(cand_node) = graph.nodes.get(&node_id) {
                let mut node_edge_ids = Vec::new();
                for p in cand_node.inputs.iter().chain(cand_node.outputs.iter()) {
                    node_edge_ids.extend(graph.get_port_edges(node_id, p.id));
                }
                for edge_id in node_edge_ids {
                    if !checked_edges.insert(edge_id) {
                        continue;
                    }
                    if let Some(edge) = graph.get_edge(edge_id) {
                        if let (Some(from_node), Some(to_node)) = (
                            graph.nodes.get(&edge.from_node),
                            graph.nodes.get(&edge.to_node),
                        ) {
                            let from_is_member = from_node.member_nodes.iter().any(|m| m.in_port_id == Some(edge.from_port) || m.out_port_id == Some(edge.from_port));
                            let to_is_member = to_node.member_nodes.iter().any(|m| m.in_port_id == Some(edge.to_port) || m.out_port_id == Some(edge.to_port));
                            if from_is_member || to_is_member {
                                let from_active = from_is_member && from_node.is_dropdown_expanded && (from_node.show_member_wires || state.show_subnode_wires_globally);
                                let to_active = to_is_member && to_node.is_dropdown_expanded && (to_node.show_member_wires || state.show_subnode_wires_globally);
                                if !from_active && !to_active {
                                    continue;
                                }
                            }
                            let p0_w = port_world_position(from_node, edge.from_port).unwrap_or_else(|| {
                                Pos2::new(from_node.position[0] + from_node.size[0], from_node.position[1] + from_node.size[1] * 0.5)
                            });
                            let p3_w = port_world_position(to_node, edge.to_port).unwrap_or_else(|| {
                                Pos2::new(to_node.position[0], to_node.position[1] + to_node.size[1] * 0.5)
                            });

                            let p0 = state.transform.world_to_screen(p0_w);
                            let p3 = state.transform.world_to_screen(p3_w);
                            let (c1, c2) = compute_bezier_control_points(
                                p0,
                                p3,
                                state.transform.zoom,
                            );
                            let dist = distance_to_bezier(pointer_pos, p0, c1, c2, p3);
                            if dist <= 7.0 {
                                new_hover.hovered_edge = Some(edge.id);
                                break;
                            }
                        }
                    }
                }
            }
            if new_hover.hovered_edge.is_some() {
                break;
            }
        }
    }

    state.hover = new_hover;

    // 4. Interaction State Machine Handling
    let pointer_pressed = ui.input(|i| i.pointer.primary_pressed());
    let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());
    let pointer_released = ui.input(|i| i.pointer.primary_released() || i.pointer.any_released());
    let is_over_interactive = state
        .interactive_rects
        .iter()
        .any(|r| r.contains(pointer_pos));

    // Context menu dismissal on Escape or click outside
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        state.context_menu = None;
    }

    if pointer_clicked && !is_over_interactive {
        state.context_menu = None;
    }

    // Handle active interaction mode
    match &mut state.interaction {
        InteractionMode::Idle => {
            if pointer_pressed && !wants_pan {
                if let Some((node_id, port_id)) = state.hover.hovered_port {
                    // Start wire connection drag
                    if let Some(port) = graph.find_port(node_id, port_id) {
                        let start_screen = graph
                            .nodes
                            .get(&node_id)
                            .and_then(|n| port_world_position(n, port_id))
                            .map(|w| state.transform.world_to_screen(w))
                            .unwrap_or(pointer_pos);

                        state.interaction = InteractionMode::Connecting {
                            from_node: node_id,
                            from_port: port_id,
                            is_output: port.direction == PortDirection::Output,
                            start_screen_pos: start_screen,
                            current_screen_pos: pointer_pos,
                            snapped_target: None,
                        };
                    }
                } else if let Some(node_id) = state.hover.hovered_node {
                    // Double-click to toggle code expand on the hovered card
                    if ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary)) && !is_over_interactive {
                        state.action_request = Some(CanvasAction::ToggleCodeExpand(node_id));
                    } else if !is_over_interactive {
                        // Only enter pending drag if not pressing an interactive control inside the card
                        let is_shift = ui.input(|i| i.modifiers.shift);
                        let press_start_world = state.transform.screen_to_world(pointer_pos);
                        state.interaction = InteractionMode::PendingNodeDrag {
                            node_id,
                            press_start_screen: pointer_pos,
                            press_start_world,
                            is_shift,
                        };
                    }
                }
            } else if pointer_clicked && !wants_pan && !is_over_interactive && state.hover.hovered_node.is_none() && state.hover.hovered_port.is_none()
                && !ui.input(|i| i.modifiers.shift) {
                state.selected_nodes.clear();
            }
        }

        InteractionMode::PendingNodeDrag {
            node_id,
            press_start_screen,
            press_start_world,
            is_shift,
        } => {
            let n_id = *node_id;
            let s_screen = *press_start_screen;
            let s_world = *press_start_world;
            let shift = *is_shift;

            if pointer_released || !is_primary_down {
                // Single click: released before exceeding drag threshold!
                // Select node without moving it
                if shift {
                    if state.selected_nodes.contains(&n_id) {
                        state.selected_nodes.remove(&n_id);
                    } else {
                        state.selected_nodes.insert(n_id);
                    }
                } else {
                    state.selected_nodes.clear();
                    state.selected_nodes.insert(n_id);
                }
                state.interaction = InteractionMode::Idle;
            } else if is_primary_down && pointer_pos.distance(s_screen) >= 4.0 {
                // User held click and dragged >= 4px at the same time: ACTIVATE DRAG!
                if !state.selected_nodes.contains(&n_id) {
                    if !shift {
                        state.selected_nodes.clear();
                    }
                    state.selected_nodes.insert(n_id);
                }

                let node_ids: Vec<NodeId> = state.selected_nodes.iter().copied().collect();
                let initial_positions: Vec<[f32; 2]> = node_ids
                    .iter()
                    .filter_map(|id| graph.nodes.get(id).map(|n| n.position))
                    .collect();

                // Apply first movement delta immediately for smooth response
                let current_world = state.transform.screen_to_world(pointer_pos);
                let delta_x = current_world.x - s_world.x;
                let delta_y = current_world.y - s_world.y;

                for (i, id) in node_ids.iter().enumerate() {
                    if let Some(node) = graph.nodes.get_mut(id) {
                        let orig = initial_positions[i];
                        node.position = [orig[0] + delta_x, orig[1] + delta_y];
                    }
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);

                state.interaction = InteractionMode::DraggingNodes {
                    node_ids,
                    drag_start_world: s_world,
                    initial_positions,
                };
            }
        }

        InteractionMode::DraggingNodes {
            node_ids,
            drag_start_world,
            initial_positions,
        } => {
            if is_primary_down {
                let current_world = state.transform.screen_to_world(pointer_pos);
                let delta_x = current_world.x - drag_start_world.x;
                let delta_y = current_world.y - drag_start_world.y;

                for (i, id) in node_ids.iter().enumerate() {
                    if let Some(node) = graph.nodes.get_mut(id) {
                        let orig = initial_positions[i];
                        node.position = [orig[0] + delta_x, orig[1] + delta_y];
                    }
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }

            if pointer_released || !is_primary_down {
                for id in node_ids {
                    if let Some(node) = graph.nodes.get(id) {
                        let bounds = [
                            node.position[0],
                            node.position[1],
                            node.position[0] + node.size[0],
                            node.position[1] + node.size[1],
                        ];
                        state.spatial_grid.update(*id, bounds);
                    }
                }
                graph.update_cluster_bounds();
                state.interaction = InteractionMode::Idle;
            }
        }

        InteractionMode::Connecting {
            from_node,
            from_port,
            start_screen_pos: _,
            current_screen_pos,
            snapped_target,
            ..
        } => {
            *current_screen_pos = pointer_pos;
            let mut snapped = None;

            let pointer_world = state.transform.screen_to_world(pointer_pos);
            let snap_box = Rect::from_center_size(pointer_world, Vec2::splat(52.0 / state.transform.zoom.max(0.1)));
            let snap_candidates = state.spatial_grid.query_rect(snap_box);

            for &cand_id in &snap_candidates {
                if cand_id != *from_node {
                    if let Some(cand_node) = graph.nodes.get(&cand_id) {
                        for port in cand_node.inputs.iter().chain(cand_node.outputs.iter()) {
                            let is_member_port = cand_node.member_nodes.iter().any(|m| m.in_port_id == Some(port.id) || m.out_port_id == Some(port.id));
                            if is_member_port && (!cand_node.is_dropdown_expanded || (!cand_node.show_member_wires && !state.show_subnode_wires_globally)) {
                                continue;
                            }
                            if let Some(wpos) = port_world_position(cand_node, port.id) {
                                let spos = state.transform.world_to_screen(wpos);
                                if spos.distance(pointer_pos) <= 26.0
                                    && can_connect(graph, *from_node, *from_port, cand_id, port.id).is_ok()
                                {
                                    snapped = Some((cand_id, port.id));
                                    *current_screen_pos = spos;
                                    break;
                                }
                            }
                        }
                    }
                }
                if snapped.is_some() {
                    break;
                }
            }
            *snapped_target = snapped;

            if pointer_released || !is_primary_down {
                if let Some((target_node, target_port)) = snapped {
                    if let Ok((src_n, src_p, dst_n, dst_p)) =
                        can_connect(graph, *from_node, *from_port, target_node, target_port)
                    {
                        graph.connect(src_n, src_p, dst_n, dst_p);
                        state.status_message = Some("Connected nodes".to_string());
                    }
                }
                state.interaction = InteractionMode::Idle;
            }
        }

        InteractionMode::Panning {
            start_pointer,
            has_panned,
        } => {
            let panned = *has_panned;
            let start_pt = *start_pointer;
            if !is_secondary_down {
                state.interaction = InteractionMode::Idle;
                if !panned && pointer_pos.distance(start_pt) < 4.0 {
                    // Click without drag: execute right-click context menu and sever logic
                    if let Some((n_id, p_id)) = state.hover.hovered_port {
                        let severed = graph.disconnect_port(n_id, p_id);
                        if !severed.is_empty() {
                            state.status_message =
                                Some(format!("Disconnected {} wire(s)", severed.len()));
                        }
                    } else if let Some(e_id) = state.hover.hovered_edge {
                        if graph.disconnect_edge(e_id) {
                            state.status_message = Some("Severed connection wire".to_string());
                        }
                    } else if let Some(n_id) = state.hover.hovered_node {
                        state.selected_nodes.clear();
                        state.selected_nodes.insert(n_id);
                        state.context_menu = Some(NodeContextMenu {
                            node_id: n_id,
                            screen_pos: pointer_pos,
                        });
                    } else {
                        state.context_menu = None;
                    }
                }
            }
        }
    }

    // Delete key removes selected nodes
    if ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
        let to_remove: Vec<NodeId> = state.selected_nodes.iter().copied().collect();
        for id in to_remove {
            graph.remove_node(id);
            state.spatial_grid.remove(id);
        }
        state.selected_nodes.clear();
    }
}
