use std::sync::Arc;

use egui::{Color32, Painter, Pos2, Rect, Stroke};
use studio_graph::{EdgeId, EvidenceTier, FolderDetail, Graph, Provenance};
use studio_ui::{paint_group_cluster, paint_pin_socket, with_alpha, GroupClusterProps, SocketVisualState};

use crate::gpu::{CanvasFrame, CanvasLayer, CanvasPaint, CardLayer, SceneUniforms};
use crate::scene::{
    self, build_card_layer, build_overlay, curve_ends, world_control_points, WireInstance, CARD_LAYER_ZOOM, CYCLE_BOX,
    FOLDER_LAYER_ZOOM, GATE_MIN_ZOOM, WIRE_ACTIVE, WIRE_HIGHLIGHT,
};
use crate::wire::paint_wire_badge_and_label;

use super::gate::InputGate;
use super::render_nodes::is_dimmed;
use super::types::CanvasState;

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

/// The far-zoom card layer, rebuilt only when the scene, the selection or the lit session change.
fn card_layer(state: &mut CanvasState, graph: &Graph) -> CardLayer {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.scene.revision.hash(&mut hasher);
    state.selected_nodes.hash(&mut hasher);
    state.session_lit_nodes.hash(&mut hasher);
    let key = hasher.finish();
    if let Some((k, layer)) = &state.card_layer {
        if *k == key {
            return layer.clone();
        }
    }
    let dimmed = |n: &studio_graph::Node| is_dimmed(n, state.trace_nodes.as_ref());
    let boxes = build_card_layer(graph, dimmed, &state.selected_nodes, &state.session_lit_nodes);
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
/// Returns the folder detail level picked by a click, if any, and the number of wire instances
/// drawn. The folders' detail buttons go into `interactive_rects`.
#[allow(clippy::too_many_arguments)]
pub fn render_background_and_wires(
    painter: &Painter,
    state: &CanvasState,
    graph: &Graph,
    rect: Rect,
    visible_world_rect: Rect,
    gate: &InputGate,
    interactive_rects: &mut Vec<Rect>,
    frame: Option<&Arc<CanvasFrame>>,
) -> (Option<(String, FolderDetail)>, usize) {
    let zoom = state.transform.zoom;
    let to_screen = |p: [f32; 2], size: [f32; 2]| {
        Rect::from_min_max(
            state.transform.world_to_screen(Pos2::from(p)),
            state.transform.world_to_screen(Pos2::new(p[0] + size[0], p[1] + size[1])),
        )
    };
    let scene = &state.scene;

    // Layer A-2: Folders. From far away the GPU draws the frames and egui only the names that fit.
    let gpu_folders = frame.is_some_and(|f| f.folders);
    if let Some(f) = frame {
        gpu_layer(painter, f, CanvasLayer::Folders, rect);
    }
    let mut folder_detail = None;
    // Once zoomed names are too small to read, names are drawn at a constant size instead.
    let map_labels = 11.5 * zoom < studio_ui::FOLDER_TEXT_MIN_PX;
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
                detail: FolderDetail::ALL.iter().position(|&d| d == cluster.detail).unwrap_or(2),
                depth: cluster.depth,
                zoom,
                frame: !gpu_folders,
                text: !map_labels,
            },
        );
        if map_labels {
            studio_ui::paint_folder_map_label(painter, r, &cluster.label, cluster.depth, cluster.is_collapsed());
        }
        interactive_rects.extend(layout.detail_buttons.iter().copied());
        if let Some(click) = gate.click() {
            if let Some(i) = layout.detail_buttons.iter().position(|r| r.contains(click)) {
                folder_detail = Some((cluster.id.clone(), FolderDetail::ALL[i]));
            }
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
        paint_tier_chip(painter, state, graph, rect);
    }

    // How many card pairs each bundled wire carries, and the hub and documentation chips.
    if state.show_wires && zoom >= BUNDLE_LABEL_MIN_ZOOM {
        let mut taken: Vec<Rect> = Vec::new();
        for label in &scene.bundle_labels {
            let p = state.transform.world_to_screen(Pos2::from(label.at));
            if rect.contains(p) && taken.len() < MAX_SCREEN_LABELS {
                let color = super::types::edge_kind_color(label.kind);
                if let Some(r) =
                    paint_chip(painter, p, egui::Align2::CENTER_CENTER, &label.pairs.to_string(), color, &taken)
                {
                    taken.push(r);
                }
            }
        }
    }
    paint_chips(painter, state, scene, rect, zoom);

    // Pin names of folders in node view, once they are legible.
    if zoom >= 0.5 {
        let font = egui::FontId::proportional((10.0 * zoom).clamp(7.0, 14.0));
        for label in &scene.pin_labels {
            let p = state.transform.world_to_screen(Pos2::from(label.at));
            if !rect.expand(200.0).contains(p) {
                continue;
            }
            let (offset, align) = if label.input {
                (egui::vec2(10.0 * zoom, 0.0), egui::Align2::LEFT_CENTER)
            } else {
                (egui::vec2(-10.0 * zoom, 0.0), egui::Align2::RIGHT_CENTER)
            };
            let text = studio_ui::truncate_with_ellipsis(&label.text, 18);
            painter.text(p + offset, align, text, font.clone(), studio_ui::color_tokens::TEXT_DIM);
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
                    let socket = SocketVisualState { is_hovered: false, is_connected: true };
                    let [r, g, b, a] = gate.fill;
                    paint_pin_socket(painter, p, Color32::from_rgba_unmultiplied(r, g, b, a), socket, zoom * 0.8);
                }
            }
        }
        None => {}
    }

    (folder_detail, drawn_wires)
}

/// Below this zoom, bundle counts are not drawn.
const BUNDLE_LABEL_MIN_ZOOM: f32 = 0.03;
/// Most counts or chips drawn in one frame; beyond that they would only cover each other.
const MAX_SCREEN_LABELS: usize = 400;

/// A rounded chip with `text`, anchored at `at`, unless it would overlap one already drawn.
/// Returns its rectangle when drawn.
fn paint_chip(
    painter: &Painter,
    at: Pos2,
    align: egui::Align2,
    text: &str,
    accent: Color32,
    taken: &[Rect],
) -> Option<Rect> {
    let font = egui::FontId::proportional(11.0);
    let galley = painter.layout_no_wrap(text.to_string(), font, accent);
    let size = galley.size() + egui::vec2(10.0, 4.0);
    let r = align.anchor_size(at, size);
    if taken.iter().any(|t| t.intersects(r)) {
        return None;
    }
    painter.rect(
        r,
        egui::CornerRadius::from(6.0),
        studio_ui::color_tokens::PANEL_BG,
        Stroke::new(1.0, with_alpha(accent, 160)),
        egui::StrokeKind::Inside,
    );
    painter.galley(r.min + egui::vec2(5.0, 2.0), galley, accent);
    Some(r)
}

/// Hub badges ("used by N") under hubs and documentation chips ("docs N") on cards and folders,
/// once their owner is big enough on screen to tell what they belong to.
fn paint_chips(painter: &Painter, state: &CanvasState, scene: &scene::CanvasScene, rect: Rect, zoom: f32) {
    if zoom < CHIP_MIN_ZOOM {
        return;
    }
    let mut taken: Vec<Rect> = Vec::new();
    for chip in &scene.chips {
        if taken.len() >= MAX_SCREEN_LABELS {
            break;
        }
        let p = state.transform.world_to_screen(Pos2::from(chip.at));
        if !rect.expand(40.0).contains(p) {
            continue;
        }
        let (text, accent, align) = match chip.kind {
            scene::ChipKind::Hub => {
                (format!("used by {}", chip.count), studio_ui::color_tokens::TEXT_SECONDARY, egui::Align2::LEFT_TOP)
            }
            scene::ChipKind::Docs => {
                (format!("docs {}", chip.count), studio_ui::color_tokens::KIND_DOCUMENTATION, egui::Align2::RIGHT_TOP)
            }
        };
        if let Some(r) = paint_chip(painter, p, align, &text, accent, &taken) {
            taken.push(r);
        }
    }
}

/// Below this zoom, chips are not drawn: their cards are too small to tell apart.
const CHIP_MIN_ZOOM: f32 = 0.08;

/// Width of a wire on screen, as the wire shader computes it.
fn wire_width(zoom: f32, highlighted: bool, flags: u32) -> f32 {
    let base = if highlighted {
        (3.0 * zoom).clamp(2.0, 5.0)
    } else if zoom < 0.35 {
        (1.8 * zoom).clamp(1.1, 2.1)
    } else {
        (1.9 * zoom).clamp(1.4, 3.0)
    };
    let weight = ((flags & scene::WIRE_WEIGHT_MASK) >> scene::WIRE_WEIGHT_SHIFT) as f32;
    base * (1.0 + 0.45 * weight)
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
    let clip = painter.clip_rect().expand(16.0);
    let mut shapes: Vec<egui::Shape> = Vec::new();
    let mut paint = |w: &WireInstance, highlighted: bool| {
        let [r, g, b, a] = w.color;
        let alpha = match (highlighted, dim) {
            (true, _) => a,
            (false, true) => (a as u16 * 45 / 255) as u8,
            // Wires are quieter than the structure until highlighted, as in the shader.
            (false, false) => (a as f32 * 0.72) as u8,
        };
        let alpha = tier_alpha(alpha, w.tier());
        let stroke =
            Stroke::new(wire_width(zoom, highlighted, w.flags), Color32::from_rgba_unmultiplied(r, g, b, alpha));
        let path = if w.is_curve() {
            let (c1, c2) = world_control_points(Pos2::from(w.p0), Pos2::from(w.p1));
            WirePath::Curve([
                screen(w.p0),
                state.transform.world_to_screen(c1),
                state.transform.world_to_screen(c2),
                screen(w.p1),
            ])
        } else {
            WirePath::Segment([screen(w.p0), screen(w.p1)])
        };
        wire_shapes(path, w.tier(), stroke, clip, &mut shapes);
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
    painter.extend(shapes);
    drawn
}

/// Dash sizes of the evidence tiers on screen, in points, as in wire.wgsl: (dash, gap) for
/// possible set and unresolved, the dot spacing for observed.
const POSSIBLE_DASH: (f32, f32) = (14.0, 8.0);
const UNRESOLVED_DASH: (f32, f32) = (6.0, 6.0);
const OBSERVED_SPACING: f32 = 7.0;
/// Unresolved wires are drawn at this share of their alpha.
const UNRESOLVED_ALPHA: f32 = 0.6;

/// `alpha` lowered for a wire matched by name only.
fn tier_alpha(alpha: u8, tier: EvidenceTier) -> u8 {
    match tier {
        EvidenceTier::Unresolved => (alpha as f32 * UNRESOLVED_ALPHA) as u8,
        _ => alpha,
    }
}

/// A wire on screen: a straight segment, or a cubic curve (start, two control points, end).
#[derive(Clone, Copy, Debug)]
pub(crate) enum WirePath {
    Segment([Pos2; 2]),
    Curve([Pos2; 4]),
}

/// The shapes of one wire in the line style of its evidence tier: one shape when proven, one per
/// dash or dot otherwise. Only the part of a segment inside `clip` is dashed; it starts on a whole
/// period of the pattern, so the dashes stay put while the camera pans.
pub(crate) fn wire_shapes(path: WirePath, tier: EvidenceTier, stroke: Stroke, clip: Rect, out: &mut Vec<egui::Shape>) {
    let points: Vec<Pos2> = match (path, tier) {
        (WirePath::Segment(ends), EvidenceTier::Proven) => {
            out.push(egui::Shape::line_segment(ends, stroke));
            return;
        }
        (WirePath::Curve(points), EvidenceTier::Proven) => {
            let curve = egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke);
            out.push(curve.into());
            return;
        }
        (WirePath::Segment([a, b]), _) => {
            let period = match tier {
                EvidenceTier::PossibleSet => POSSIBLE_DASH.0 + POSSIBLE_DASH.1,
                EvidenceTier::Observed => OBSERVED_SPACING,
                _ => UNRESOLVED_DASH.0 + UNRESOLVED_DASH.1,
            };
            match clip_segment(a, b, clip, period) {
                Some((a, b)) => vec![a, b],
                None => return,
            }
        }
        (WirePath::Curve(points), _) => {
            if !Rect::from_points(&points).intersects(clip) {
                return;
            }
            egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, stroke)
                .flatten(None)
        }
    };
    match tier {
        EvidenceTier::Observed => {
            out.extend(egui::Shape::dotted_line(&points, stroke.color, OBSERVED_SPACING, (stroke.width * 0.5).max(1.0)))
        }
        EvidenceTier::PossibleSet => {
            out.extend(egui::Shape::dashed_line(&points, stroke, POSSIBLE_DASH.0, POSSIBLE_DASH.1))
        }
        _ => out.extend(egui::Shape::dashed_line(&points, stroke, UNRESOLVED_DASH.0, UNRESOLVED_DASH.1)),
    }
}

/// The part of segment `a`-`b` inside `clip`, its start moved back to a whole multiple of
/// `period` from `a`. `None` when the segment misses `clip`.
fn clip_segment(a: Pos2, b: Pos2, clip: Rect, period: f32) -> Option<(Pos2, Pos2)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-d.x, a.x - clip.min.x), (d.x, clip.max.x - a.x), (-d.y, a.y - clip.min.y), (d.y, clip.max.y - a.y)]
    {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    if t0 > t1 {
        return None;
    }
    let len = d.length();
    if len < 1e-3 {
        return Some((a, b));
    }
    let start = (t0 * len / period).floor() * period / len;
    Some((a + d * start, a + d * t1))
}

/// What the chip on a hovered wire says about its evidence: nothing when proven, "1 of n" for one
/// of a set of candidates, otherwise the tier's name.
pub(crate) fn tier_chip_text(provenance: &Provenance) -> Option<String> {
    match provenance.tier {
        EvidenceTier::Proven => None,
        EvidenceTier::PossibleSet => Some(format!("1 of {}", provenance.candidates.max(1))),
        tier => Some(tier.label().to_string()),
    }
}

/// The accent of a tier chip.
fn tier_chip_color(tier: EvidenceTier) -> Color32 {
    use studio_ui::color_tokens::{TIER_OBSERVED, TIER_POSSIBLE, TIER_PROVEN, TIER_UNRESOLVED};
    match tier {
        EvidenceTier::Proven => TIER_PROVEN,
        EvidenceTier::PossibleSet => TIER_POSSIBLE,
        EvidenceTier::Observed => TIER_OBSERVED,
        EvidenceTier::Unresolved => TIER_UNRESOLVED,
    }
}

/// The hovered wire's evidence chip, at the middle of its longest straight piece or of its curve.
fn paint_tier_chip(painter: &Painter, state: &CanvasState, graph: &Graph, rect: Rect) {
    let Some(edge) = state.hover.hovered_edge.and_then(|id| graph.get_edge(id)) else { return };
    let Some(text) = tier_chip_text(&edge.provenance) else { return };
    let mut points = Vec::new();
    let at = if crate::view::routed_wire_points_into(graph, edge, &mut points) {
        let Some(w) = points.windows(2).max_by(|a, b| (a[1] - a[0]).length().total_cmp(&(b[1] - b[0]).length())) else {
            return;
        };
        w[0].lerp(w[1], 0.5)
    } else {
        let (a, b) = curve_ends(graph, edge);
        let (c1, c2) = world_control_points(a, b);
        crate::wire::eval_cubic_bezier(a, c1, c2, b, 0.5)
    };
    let p = state.transform.world_to_screen(at) - egui::vec2(0.0, 12.0);
    if rect.contains(p) {
        paint_chip(painter, p, egui::Align2::CENTER_BOTTOM, &text, tier_chip_color(edge.provenance.tier), &[]);
    }
}
