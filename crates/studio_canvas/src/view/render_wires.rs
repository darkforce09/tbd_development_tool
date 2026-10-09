use egui::{Painter, Pos2, Rect};
use studio_graph::Graph;
use studio_ui::{color_tokens::*, paint_group_cluster, with_alpha, GroupClusterProps};

use crate::gpu::GpuWireBatch;
use crate::grid::paint_infinite_grid;
use crate::interaction::InteractionMode;
use crate::wire::{
    paint_bezier_wire, paint_pending_wire, paint_wire_badge_and_label, WireRenderProps,
};

use super::layout::port_world_position;
use super::types::{data_type_color, CanvasState};

#[allow(clippy::too_many_arguments)]
pub fn render_background_and_wires(
    painter: &Painter,
    state: &CanvasState,
    graph: &Graph,
    rect: Rect,
    visible_world_rect: Rect,
    pointer_pos: Pos2,
    pointer_clicked: bool,
    anim_time: f64,
) -> Option<String> {
    let mut toggle_cluster_id = None;
    let zoom = state.transform.zoom;

    // Layer A: Infinite dot grid
    paint_infinite_grid(painter, rect, &state.transform);

    // Layer A-2: Group Clusters (CodeSee Crate & Directory Clusters)
    let mut sorted_clusters: Vec<&studio_graph::GroupCluster> = graph.clusters.iter().collect();
    sorted_clusters.sort_by_key(|c| c.depth);

    for cluster in sorted_clusters {
        if cluster.size[0] <= 1.0 || cluster.size[1] <= 1.0 {
            continue;
        }
        let min_world = Pos2::new(cluster.position[0], cluster.position[1]);
        let max_world = Pos2::new(
            cluster.position[0] + cluster.size[0],
            cluster.position[1] + cluster.size[1],
        );
        // Frustum cull clusters
        if min_world.x > visible_world_rect.max.x
            || max_world.x < visible_world_rect.min.x
            || min_world.y > visible_world_rect.max.y
            || max_world.y < visible_world_rect.min.y
        {
            continue;
        }
        let min_screen = state.transform.world_to_screen(min_world);
        let max_screen = state.transform.world_to_screen(max_world);
        let cluster_rect = Rect::from_min_max(min_screen, max_screen);

        let cluster_layout = paint_group_cluster(
            painter,
            GroupClusterProps {
                rect: cluster_rect,
                label: &cluster.label,
                category: &cluster.category,
                subtitle: cluster.subtitle.as_deref(),
                color_index: cluster.color_index,
                item_count: cluster.node_ids.len(),
                is_collapsed: cluster.is_collapsed,
                depth: cluster.depth,
                zoom,
            },
        );

        if pointer_clicked && cluster_layout.collapse_button_rect.contains(pointer_pos) {
            toggle_cluster_id = Some(cluster.id.clone());
        }
    }

    // Layer B: Graph Connection Wires (Always Bezier curves, rendered at every zoom level)
    let wire_cull_rect = visible_world_rect.expand(200.0);
    let visible_edge_ids = state.spatial_grid.query_edges_rect(wire_cull_rect);

    // Track bundled connections between collapsed cards to avoid redundant overdraw
    let mut bundled_pairs: std::collections::HashSet<(studio_graph::NodeId, studio_graph::NodeId)> = std::collections::HashSet::new();

    let mut gpu_batch = if state.use_gpu_wires {
        Some(GpuWireBatch::new([rect.width(), rect.height()], zoom, anim_time as f32))
    } else {
        None
    };

    for edge_id in visible_edge_ids {
        let Some(edge) = graph.get_edge(edge_id) else { continue };
        if graph.is_node_in_collapsed_cluster(edge.from_node)
            || graph.is_node_in_collapsed_cluster(edge.to_node)
        {
            continue;
        }
        let Some(from_node) = graph.nodes.get(&edge.from_node) else { continue };
        let Some(to_node) = graph.nodes.get(&edge.to_node) else { continue };

        let from_is_member = from_node.member_nodes.iter().any(|m| m.in_port_id == Some(edge.from_port) || m.out_port_id == Some(edge.from_port));
        let to_is_member = to_node.member_nodes.iter().any(|m| m.in_port_id == Some(edge.to_port) || m.out_port_id == Some(edge.to_port));

        // When both cards are collapsed and global subnode wire mode is not active:
        // Bundle connections between the same pair into a single visible card-level line
        let both_collapsed = !from_node.is_dropdown_expanded && !to_node.is_dropdown_expanded;
        if (from_is_member || to_is_member) && both_collapsed && !state.show_subnode_wires_globally {
            if !bundled_pairs.insert((from_node.id, to_node.id)) {
                continue;
            }
        }

        let p0_w = port_world_position(from_node, edge.from_port).unwrap_or_else(|| {
            Pos2::new(from_node.position[0] + from_node.size[0], from_node.position[1] + from_node.size[1] * 0.5)
        });
        let p3_w = port_world_position(to_node, edge.to_port).unwrap_or_else(|| {
            Pos2::new(to_node.position[0], to_node.position[1] + to_node.size[1] * 0.5)
        });

        let is_flow_active = if let Some(ref active_edges) = state.active_flow_edges {
            active_edges.contains(&edge.id)
        } else {
            false
        };

        let is_hovered = state.hover.hovered_edge == Some(edge.id);
        let is_selected = state.selected_nodes.contains(&edge.from_node)
            || state.selected_nodes.contains(&edge.to_node);

        let p0 = state.transform.world_to_screen(p0_w);
        let p3 = state.transform.world_to_screen(p3_w);

        let base_color = from_node
            .find_port(edge.from_port)
            .map(|p| data_type_color(&p.data_type))
            .unwrap_or(WIRE_DEFAULT);

        let edge_color = if state.active_flow_edges.is_none() || is_flow_active {
            base_color
        } else {
            with_alpha(base_color, 45)
        };

        if let Some(ref mut batch) = gpu_batch {
            let core_width = if is_hovered || is_selected {
                (3.0 * zoom).clamp(2.0, 5.0)
            } else if zoom < 0.35 {
                (2.2 * zoom).clamp(1.4, 2.6)
            } else {
                (2.4 * zoom).clamp(1.8, 3.8)
            };

            let (glow_width, glow_color) = if is_hovered || is_flow_active || is_selected {
                let gw = (if is_hovered || is_selected { 6.0 } else { 4.5 } * zoom).clamp(2.5, 8.0);
                let gc = with_alpha(edge_color, if is_hovered || is_selected { 90 } else { 40 });
                (gw, Some(gc))
            } else {
                (0.0, None)
            };

            batch.push_wire(
                p0,
                p3,
                edge_color,
                glow_color,
                core_width,
                glow_width,
                is_flow_active || is_hovered || is_selected,
            );

            if zoom >= 0.35 && (edge.step_number.is_some() || edge.label.is_some()) {
                paint_wire_badge_and_label(
                    painter,
                    p0,
                    p3,
                    zoom,
                    edge.step_number,
                    edge.label.as_deref(),
                );
            }
        } else {
            paint_bezier_wire(
                painter,
                WireRenderProps {
                    p0,
                    p3,
                    color: edge_color,
                    is_active: is_flow_active,
                    is_hovered: is_hovered || is_selected,
                    zoom,
                    anim_time,
                    label: if zoom >= 0.35 { edge.label.as_deref() } else { None },
                    step_number: if zoom >= 0.35 { edge.step_number } else { None },
                },
            );
        }
    }

    // Layer C: Pending connection wire preview
    if let InteractionMode::Connecting {
        from_node,
        from_port,
        start_screen_pos,
        current_screen_pos,
        snapped_target,
        ..
    } = &state.interaction
    {
        let wire_color = graph
            .find_port(*from_node, *from_port)
            .map(|p| data_type_color(&p.data_type))
            .unwrap_or(WIRE_ACTIVE);

        if let Some(ref mut batch) = gpu_batch {
            let core_width = (2.4 * zoom).clamp(1.2, 5.0);
            let glow_width = (5.0 * zoom).clamp(2.0, 9.0);
            let glow_color = Some(with_alpha(wire_color, 60));
            batch.push_wire(
                *start_screen_pos,
                *current_screen_pos,
                wire_color,
                glow_color,
                core_width,
                glow_width,
                false,
            );

            // Target indicator dot
            painter.add(egui::epaint::CircleShape {
                center: *current_screen_pos,
                radius: (if snapped_target.is_some() { 6.0 } else { 4.0 } * zoom).clamp(2.0, 8.0),
                fill: if snapped_target.is_some() { wire_color } else { egui::Color32::WHITE },
                stroke: egui::Stroke::new((1.5 * zoom).clamp(1.0, 3.0), egui::Color32::BLACK),
            });
        } else {
            paint_pending_wire(
                painter,
                *start_screen_pos,
                *current_screen_pos,
                wire_color,
                snapped_target.is_some(),
                zoom,
            );
        }
    }

    // Submit GPU batch callback to egui painter
    if let Some(batch) = gpu_batch {
        if !batch.is_empty() {
            painter.add(batch.into_paint_callback(rect));
        }
    }

    toggle_cluster_id
}
