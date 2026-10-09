use std::sync::Arc;

use egui::{Color32, Painter, Pos2, Rect, Stroke};
use studio_graph::{EdgeId, Graph};
use studio_ui::{paint_group_cluster, paint_pin_socket, with_alpha, GroupClusterProps, SocketVisualState};

use crate::gpu::{CanvasFrame, CanvasLayer, CanvasPaint, CardLayer, SceneUniforms};
use crate::grid::paint_infinite_grid;
use crate::interaction::InteractionMode;
use crate::scene::{
    self, build_card_layer, build_overlay, curve_ends, world_control_points, WireInstance, CARD_LAYER_ZOOM, CYCLE_BOX,
    FOLDER_LAYER_ZOOM, GATE_MIN_ZOOM, WIRE_ACTIVE, WIRE_HIGHLIGHT,
};
use crate::wire::{paint_pending_wire, paint_wire_badge_and_label};

use super::render_nodes::is_dimmed;
use super::types::{data_type_color, CanvasState};

/// Smallest on-screen folder, in points, that gets its name drawn while the GPU draws frames.
const FOLDER_LABEL_MIN: [f32; 2] = [80.0, 12.0];

/// What this frame draws on the GPU: the camera, the visible wire tiles, the highlighted wires
/// and, when zoomed far out, the card layer. Only the highlighted edges are looked at per frame.
pub fn prepare_frame(state: &mut CanvasState, graph: &Graph, view_world: Rect, anim_time: f64) -> Arc<CanvasFrame> {
    state.frame_counter += 1;
    let zoom = state.transform.zoom;
    let margin = 12.0 / zoom.max(1e-4);
    let view = view_world.expand(margin);
    let (wire_ranges, curve_ranges, (overlay, overlay_curves_start)) = if state.show_wires {
        (
            state.scene.visible_ranges(view),
            state.scene.visible_curve_ranges(view),
            build_overlay(graph, highlighted_edges(state, graph)),
        )
    } else {
        (Vec::new(), Vec::new(), (Vec::new(), 0))
    };
    let cards = (zoom < CARD_LAYER_ZOOM).then(|| card_layer(state, graph));
    Arc::new(CanvasFrame {
        id: state.frame_counter,
        uniforms: SceneUniforms {
            screen_size: [0.0; 2],
            pan: [state.transform.pan.x, state.transform.pan.y],
            zoom,
            time: (anim_time % 3600.0) as f32,
            kind_mask: state.wire_kinds.mask(),
            flags: state.active_flow_edges.is_some() as u32,
        },
        scene: state.scene.clone(),
        wire_ranges,
        curve_ranges,
        show_wires: state.show_wires,
        overlay,
        overlay_curves_start,
        folders: zoom < FOLDER_LAYER_ZOOM,
        gates: state.show_wires && zoom >= GATE_MIN_ZOOM,
        cards,
    })
}

/// Hovered edge, the edges of selected cards, and the active flow.
fn highlighted_edges(state: &CanvasState, graph: &Graph) -> Vec<(EdgeId, u32)> {
    let mut out: Vec<(EdgeId, u32)> = state.hover.hovered_edge.map(|e| (e, WIRE_HIGHLIGHT)).into_iter().collect();
    for id in &state.selected_nodes {
        let Some(node) = graph.nodes.get(id) else { continue };
        for port in node.inputs.iter().chain(&node.outputs) {
            if let Some(edges) = graph.port_edges.get(&(*id, port.id)) {
                out.extend(edges.iter().map(|&e| (e, WIRE_HIGHLIGHT)));
            }
        }
    }
    if let Some(active) = &state.active_flow_edges {
        out.extend(active.iter().map(|&e| (e, WIRE_ACTIVE)));
    }
    out
}

/// The far-zoom card layer, rebuilt only when the scene, search, filters or selection change.
fn card_layer(state: &mut CanvasState, graph: &Graph) -> CardLayer {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.scene.revision.hash(&mut hasher);
    state.search_filter.hash(&mut hasher);
    state.category_filter.hash(&mut hasher);
    state.selected_nodes.hash(&mut hasher);
    let key = hasher.finish();
    if let Some((k, layer)) = &state.card_layer {
        if *k == key {
            return layer.clone();
        }
    }
    let query = state.search_filter.trim().to_lowercase();
    let dimmed = |n: &studio_graph::Node| is_dimmed(n, &query, &state.category_filter);
    let boxes = build_card_layer(graph, dimmed, &state.selected_nodes);
    let layer = CardLayer { revision: scene::next_revision(), boxes: Arc::new(boxes) };
    state.card_layer = Some((key, layer.clone()));
    layer
}

/// Draws one layer of the frame on the GPU, in egui's paint order.
fn gpu_layer(painter: &Painter, frame: &Arc<CanvasFrame>, layer: CanvasLayer, rect: Rect) {
    painter.add(CanvasPaint { layer, frame: frame.clone() }.into_paint_callback(rect));
}

/// Paints the grid, folders, cycle boxes, wires and gates. With a GPU `frame` the static parts are
/// drawn by the GPU and egui only adds legible text; without one everything is painted here.
/// Returns the folder whose collapse button was clicked and the number of wire instances drawn.
#[allow(clippy::too_many_arguments)]
pub fn render_background_and_wires(
    painter: &Painter,
    state: &CanvasState,
    graph: &Graph,
    rect: Rect,
    visible_world_rect: Rect,
    pointer_pos: Pos2,
    pointer_clicked: bool,
    frame: Option<&Arc<CanvasFrame>>,
) -> (Option<String>, usize) {
    let zoom = state.transform.zoom;
    let to_screen = |p: [f32; 2], size: [f32; 2]| {
        Rect::from_min_max(
            state.transform.world_to_screen(Pos2::from(p)),
            state.transform.world_to_screen(Pos2::new(p[0] + size[0], p[1] + size[1])),
        )
    };
    let scene = &state.scene;

    // Layer A: Infinite dot grid
    paint_infinite_grid(painter, rect, &state.transform);

    // Layer A-2: Folders. From far away the GPU draws the frames and egui only the names that fit.
    let gpu_folders = frame.is_some_and(|f| f.folders);
    if let Some(f) = frame {
        gpu_layer(painter, f, CanvasLayer::Folders, rect);
    }
    let mut toggle_cluster_id = None;
    for &i in &scene.folder_order {
        let Some(cluster) = graph.clusters.get(i) else { continue };
        let r = to_screen(cluster.position, cluster.size);
        if !r.intersects(rect) || (gpu_folders && (r.width() < FOLDER_LABEL_MIN[0] || r.height() < FOLDER_LABEL_MIN[1]))
        {
            continue;
        }
        let layout = paint_group_cluster(
            painter,
            GroupClusterProps {
                rect: r,
                label: &cluster.label,
                category: &cluster.category,
                subtitle: cluster.subtitle.as_deref(),
                color_index: cluster.color_index,
                item_count: cluster.node_ids.len(),
                is_collapsed: cluster.is_collapsed,
                depth: cluster.depth,
                zoom,
                frame: !gpu_folders,
            },
        );
        if pointer_clicked && layout.collapse_button_rect.contains(pointer_pos) {
            toggle_cluster_id = Some(cluster.id.clone());
        }
    }

    // Layer A-3: Cycle boxes, around items that depend on each other in a loop.
    if let Some(f) = frame {
        gpu_layer(painter, f, CanvasLayer::CycleBoxes, rect);
    }
    if let Some(flow) = &graph.flow {
        for b in scene.cycle_order.iter().filter_map(|&i| flow.cycle_boxes.get(i)) {
            let r = to_screen(b.position, b.size);
            if !r.intersects(rect) {
                continue;
            }
            if frame.is_none() {
                painter.rect(
                    r,
                    egui::CornerRadius::from(8.0 * zoom),
                    with_alpha(CYCLE_BOX, 14),
                    Stroke::new((1.5 * zoom).clamp(1.0, 2.0), with_alpha(CYCLE_BOX, 170)),
                    egui::StrokeKind::Inside,
                );
            }
            if zoom >= 0.3 {
                painter.text(
                    r.min + egui::vec2(10.0 * zoom, 8.0 * zoom),
                    egui::Align2::LEFT_TOP,
                    format!("{} loop", egui_phosphor::regular::ARROWS_CLOCKWISE),
                    egui::FontId::proportional((11.0 * zoom).clamp(8.0, 16.0)),
                    with_alpha(CYCLE_BOX, 220),
                );
            }
        }
    }

    // Layer B: Wires
    let drawn_wires = match frame {
        Some(f) => {
            gpu_layer(painter, f, CanvasLayer::Wires, rect);
            if f.show_wires {
                f.wire_ranges.iter().chain(&f.curve_ranges).map(|r| (r.end - r.start) as usize).sum::<usize>()
                    + f.overlay.len()
            } else {
                0
            }
        }
        None => paint_wires_cpu(painter, state, graph, visible_world_rect),
    };
    if state.show_wires && zoom >= 0.35 {
        for edge in scene.labeled.iter().filter_map(|&id| graph.get_edge(id)) {
            let (a, b) = curve_ends(graph, edge);
            let (p0, p3) = (state.transform.world_to_screen(a), state.transform.world_to_screen(b));
            if Rect::from_two_pos(p0, p3).intersects(rect) {
                paint_wire_badge_and_label(painter, p0, p3, zoom, edge.step_number, edge.label.as_deref());
            }
        }
    }

    // Layer B-2: Gates, where wires cross folder and cycle box edges.
    match frame {
        Some(f) => gpu_layer(painter, f, CanvasLayer::Gates, rect),
        None if state.show_wires && zoom >= GATE_MIN_ZOOM => {
            let visible = rect.expand(10.0);
            for gate in &scene.gates {
                let p = state.transform.world_to_screen(Pos2::from(gate.center));
                if visible.contains(p) {
                    let socket = SocketVisualState { is_hovered: false, is_connected: true, is_snapped: false };
                    let [r, g, b, a] = gate.fill;
                    paint_pin_socket(painter, p, Color32::from_rgba_unmultiplied(r, g, b, a), socket, zoom * 0.8);
                }
            }
        }
        None => {}
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
            .unwrap_or(studio_ui::color_tokens::WIRE_ACTIVE);
        paint_pending_wire(painter, *start_screen_pos, *current_screen_pos, wire_color, snapped_target.is_some(), zoom);
    }

    (toggle_cluster_id, drawn_wires)
}

/// Width of a wire on screen, as the wire shader computes it.
fn wire_width(zoom: f32, highlighted: bool) -> f32 {
    if highlighted {
        (3.0 * zoom).clamp(2.0, 5.0)
    } else if zoom < 0.35 {
        (2.2 * zoom).clamp(1.4, 2.6)
    } else {
        (2.4 * zoom).clamp(1.8, 3.8)
    }
}

/// Paints the scene's wires with egui, for when no GPU is available. Correct, not fast.
fn paint_wires_cpu(painter: &Painter, state: &CanvasState, graph: &Graph, view_world: Rect) -> usize {
    if !state.show_wires {
        return 0;
    }
    let zoom = state.transform.zoom;
    let scene = &state.scene;
    let dim = state.active_flow_edges.is_some();
    let screen = |p: [f32; 2]| state.transform.world_to_screen(Pos2::from(p));
    let paint = |w: &WireInstance, highlighted: bool| {
        let [r, g, b, a] = w.color;
        let alpha = if dim && !highlighted { (a as u16 * 45 / 255) as u8 } else { a };
        let stroke = Stroke::new(wire_width(zoom, highlighted), Color32::from_rgba_unmultiplied(r, g, b, alpha));
        if w.is_curve() {
            let (c1, c2) = world_control_points(Pos2::from(w.p0), Pos2::from(w.p1));
            let points =
                [screen(w.p0), state.transform.world_to_screen(c1), state.transform.world_to_screen(c2), screen(w.p1)];
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
        } else {
            painter.line_segment([screen(w.p0), screen(w.p1)], stroke);
        }
    };
    let mut drawn = 0;
    let view = view_world.expand(12.0 / zoom.max(1e-4));
    let segments =
        scene.visible_ranges(view).into_iter().flat_map(|r| &scene.segments[r.start as usize..r.end as usize]);
    let curves =
        scene.visible_curve_ranges(view).into_iter().flat_map(|r| &scene.curves[r.start as usize..r.end as usize]);
    for w in segments.chain(curves) {
        if state.wire_kind_visible(w.kind_index()) {
            paint(w, false);
            drawn += 1;
        }
    }
    let (overlay, _) = build_overlay(graph, highlighted_edges(state, graph));
    for w in overlay.iter().filter(|w| state.wire_kind_visible(w.kind_index())) {
        paint(w, true);
        drawn += 1;
    }
    drawn
}
