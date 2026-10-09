//! Everything the canvas draws that only changes when the layout changes, in world space.
//!
//! The scene is built once per geometry change (layout, folder collapse, card expansion) and
//! uploaded to the GPU once; a frame then only updates the camera uniforms. Highlights (hover,
//! selection, active flow) are a small per-frame layer drawn on top, and cards seen from far away
//! are a separate layer rebuilt when the search, filters or selection change.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use bytemuck::{Pod, Zeroable};
use egui::{Color32, Pos2, Rect};
use rustc_hash::{FxHashMap, FxHashSet};
use studio_graph::{Edge, EdgeId, FolderDetail, GateKind, GateSide, Graph, NodeId, VisibleEnd};
use studio_ui::{cluster_tint, color_tokens::*, with_alpha};

use crate::view::{archetype_color, edge_kind_color, port_world_position, routed_wire_points_into};

/// Bits 0-3 of [`WireInstance::flags`]: the edge kind (`EdgeKind as u32`).
pub const WIRE_KIND_MASK: u32 = 0xF;
/// A Bezier curve from `p0` to `p1` instead of a straight segment.
pub const WIRE_CURVE: u32 = 1 << 4;
/// Hovered or selected: thicker, with a glow and moving pulses.
pub const WIRE_HIGHLIGHT: u32 = 1 << 5;
/// Part of the active flow: a soft glow and moving pulses.
pub const WIRE_ACTIVE: u32 = 1 << 6;
/// Bits 7–9: how many card pairs the segment carries, as `floor(log2(pairs))` capped at 7. The
/// shader draws heavier wires thicker.
pub const WIRE_WEIGHT_SHIFT: u32 = 7;
pub const WIRE_WEIGHT_MASK: u32 = 0x7 << WIRE_WEIGHT_SHIFT;

/// Weight bits for a segment carrying `pairs` card pairs.
pub fn wire_weight(pairs: u32) -> u32 {
    (31 - pairs.max(1).leading_zeros()).min(7) << WIRE_WEIGHT_SHIFT
}

/// Below this zoom, cards are drawn as plain rects by the GPU card layer.
pub const CARD_LAYER_ZOOM: f32 = 0.12;
/// Below this zoom, folder frames are drawn by the GPU; above it egui draws the few on screen.
pub const FOLDER_LAYER_ZOOM: f32 = 0.35;
/// Below this zoom, gates are not drawn.
pub const GATE_MIN_ZOOM: f32 = 0.2;

/// World size of a wire tile, the unit of per-frame culling.
const TILE: f32 = 4096.0;
/// World size of a hit-test cell. Coarse, because long wires are listed in every cell they cross.
const HIT_CELL: f32 = 8192.0;

/// One straight wire segment or one curve, in world coordinates (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct WireInstance {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    /// Unmultiplied RGBA.
    pub color: [u8; 4],
    pub flags: u32,
}

impl WireInstance {
    pub fn kind_index(&self) -> u32 {
        self.flags & WIRE_KIND_MASK
    }

    pub fn is_curve(&self) -> bool {
        self.flags & WIRE_CURVE != 0
    }
}

/// A rounded rect in world coordinates with a fill and a border (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BoxInstance {
    pub min: [f32; 2],
    pub size: [f32; 2],
    /// Unmultiplied RGBA.
    pub fill: [u8; 4],
    pub border: [u8; 4],
    /// Corner radius in world units.
    pub radius: f32,
    /// Border width in world units; on screen it is clamped to 1-2.5 px. A box with no border is
    /// skipped when it is under half a pixel on screen.
    pub border_width: f32,
}

/// A gate pin: a filled circle with a ring (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PinInstance {
    pub center: [f32; 2],
    pub fill: [u8; 4],
    pub ring: [u8; 4],
}

/// The file name next to a pin of a folder in node view, inside the folder's edge.
#[derive(Debug, Clone, PartialEq)]
pub struct PinLabel {
    pub at: [f32; 2],
    pub text: String,
    /// On the left edge (an input); otherwise the right edge.
    pub input: bool,
}

/// The number of card pairs a bundle stands for, shown on its wire.
#[derive(Clone, Debug, PartialEq)]
pub struct BundleLabel {
    pub at: [f32; 2],
    pub pairs: u32,
    pub kind: studio_graph::EdgeKind,
}

/// What a chip says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipKind {
    /// "used by N": a hub's wires are hidden behind it.
    Hub,
    /// "docs N": documentation files linking to the card or into the closed folder.
    Docs,
}

/// A small label on a card or folder, anchored in world space.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    /// Top-left of the chip for hubs (under the owner), top-right for docs (on the owner).
    pub at: [f32; 2],
    pub count: u32,
    pub kind: ChipKind,
}

/// A run of `segments` whose midpoints fall in one world tile, with the bounds of the run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireTile {
    pub bounds: Rect,
    pub start: u32,
    pub end: u32,
}

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

/// Unique number for each scene or layer built, so the GPU knows when to upload.
pub fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// The static, world-space contents of the canvas. See the module docs.
#[derive(Debug, Default)]
pub struct CanvasScene {
    pub revision: u64,
    /// Straight wire segments, each drawn once however many routes share it, sorted by tile.
    pub segments: Vec<WireInstance>,
    /// An edge each segment belongs to, for hover.
    pub segment_owner: Vec<EdgeId>,
    pub tiles: Vec<WireTile>,
    /// Wires without a route, drawn as curves.
    pub curves: Vec<WireInstance>,
    pub curve_owner: Vec<EdgeId>,
    pub curve_tiles: Vec<WireTile>,
    /// Folder frames (parents before children), then cycle boxes from `cycle_start`.
    pub boxes: Vec<BoxInstance>,
    pub cycle_start: usize,
    /// Indices into `graph.clusters` of the folders drawn, parents before children.
    pub folder_order: Vec<usize>,
    /// Indices into `graph.flow.cycle_boxes` of the cycle boxes drawn.
    pub cycle_order: Vec<usize>,
    pub gates: Vec<PinInstance>,
    /// Names next to the pins of folders in node view.
    pub pin_labels: Vec<PinLabel>,
    /// Wires without a route that carry a label or step badge.
    pub labeled: Vec<EdgeId>,
    /// Counts on wires that stand for more than one card pair.
    pub bundle_labels: Vec<BundleLabel>,
    /// Hub badges and documentation chips.
    pub chips: Vec<Chip>,
    /// Whether member wires were drawn individually when the scene was built.
    pub member_wires: bool,
    /// How long the build took, in ms.
    pub build_ms: f32,
    /// (hit cell, wire) pairs, sorted: every cell a wire passes lists it. Built on first use or
    /// by `build_hit_index`.
    hit: std::sync::OnceLock<Vec<((i32, i32), u32)>>,
    tile_keys: Vec<(i32, i32)>,
}

/// A route segment while building: tile, quantized ends (tenths of a world unit), kind, the order
/// of the edge it came from, and the card pairs it carries.
type SegmentKey = ((i32, i32), [i32; 4], u8, u32, u32);

/// Index into `hit` cells: curves are marked with the top bit.
const HIT_CURVE: u32 = 1 << 31;

impl CanvasScene {
    /// Builds the scene from the graph's current layout. `member_wires` is
    /// `CanvasState::show_subnode_wires_globally`.
    pub fn build(graph: &Graph, member_wires: bool) -> Self {
        let started = std::time::Instant::now();
        let mut scene = CanvasScene { revision: next_revision(), member_wires, ..Default::default() };
        scene.build_wires(graph, member_wires);
        scene.build_folders(graph);
        scene.build_gates(graph);
        scene.build_chips(graph);
        scene.build_ms = started.elapsed().as_secs_f32() * 1000.0;
        scene
    }

    fn build_wires(&mut self, graph: &Graph, member_wires: bool) {
        use rayon::prelude::*;
        // What each edge needs, worked out in parallel; which of them are drawn is then decided in
        // edge order, so the result never depends on thread timing. Pairs already drawn into or
        // out of a collapsed folder, between two closed cards per kind, and as one bundled member
        // wire, are skipped.
        enum Plan {
            Skip,
            /// Route ends in a collapsed folder: drawn once per bundle.
            Hidden,
            Wire {
                bundle: bool,
                routed: bool,
                dedup: bool,
            },
        }
        let opened = |n: &studio_graph::Node| n.is_dropdown_expanded || n.is_code_expanded;
        let plans: Vec<Plan> = graph
            .edges
            .par_iter()
            .map(|edge| {
                // A hub's badge stands in for its wires.
                if graph.is_behind_hub(edge) {
                    return Plan::Skip;
                }
                if graph.is_node_in_collapsed_cluster(edge.from_node)
                    || graph.is_node_in_collapsed_cluster(edge.to_node)
                {
                    return if graph.has_route(edge) { Plan::Hidden } else { Plan::Skip };
                }
                let (Some(from), Some(to)) = (graph.nodes.get(&edge.from_node), graph.nodes.get(&edge.to_node)) else {
                    return Plan::Skip;
                };
                // A wire within one card only means something once the card shows its members.
                if from.id == to.id && !opened(from) {
                    return Plan::Skip;
                }
                let is_member = |n: &studio_graph::Node, port| {
                    n.member_nodes.iter().any(|m| m.in_port_id == Some(port) || m.out_port_id == Some(port))
                };
                // Member wires between two closed cards are bundled into one wire per pair.
                let both_closed = !from.is_dropdown_expanded && !to.is_dropdown_expanded;
                let bundle =
                    !member_wires && both_closed && (is_member(from, edge.from_port) || is_member(to, edge.to_port));
                let routed = graph.has_route(edge);
                // Between two closed cards every wire of a pair and kind has the same path.
                let dedup = routed && !opened(from) && !opened(to);
                Plan::Wire { bundle, routed, dedup }
            })
            .collect();

        // Routed wires to draw: the edge, the kind its colour comes from, and the card pairs it
        // carries.
        let mut routed: Vec<(&Edge, studio_graph::EdgeKind, u32)> = Vec::new();
        let mut curves: Vec<(WireInstance, EdgeId)> = Vec::new();
        let mut hidden_bundles: FxHashSet<u32> = FxHashSet::default();
        let mut drawn_pairs: FxHashSet<(NodeId, NodeId, u32)> = FxHashSet::default();
        let mut bundled_pairs: FxHashSet<(NodeId, NodeId)> = FxHashSet::default();
        for (edge, plan) in graph.edges.iter().zip(plans) {
            let pair = (edge.from_node, edge.to_node);
            match plan {
                Plan::Skip => {}
                Plan::Hidden => {
                    // One wire per bundle, in the colour of the kind it carries most.
                    let Some(route) = graph.route_of(edge) else { continue };
                    if hidden_bundles.insert(route.bundle) {
                        match graph.bundle_of(edge) {
                            Some(b) => {
                                routed.push((edge, b.main_kind(), b.pairs));
                                if b.pairs > 1 {
                                    if let Some(at) = label_anchor(&route.points) {
                                        self.bundle_labels.push(BundleLabel {
                                            at,
                                            pairs: b.pairs,
                                            kind: b.main_kind(),
                                        });
                                    }
                                }
                            }
                            None => routed.push((edge, edge.kind, 1)),
                        }
                    }
                }
                Plan::Wire { bundle, .. } if bundle && !bundled_pairs.insert(pair) => {}
                Plan::Wire { routed: true, dedup, .. } => {
                    if !dedup || drawn_pairs.insert((pair.0, pair.1, edge.kind as u32)) {
                        routed.push((edge, edge.kind, 1));
                    }
                }
                Plan::Wire { routed: false, .. } => {
                    let (p0, p1) = curve_ends(graph, edge);
                    let color = edge_kind_color(edge.kind).to_srgba_unmultiplied();
                    let flags = edge.kind as u32 | WIRE_CURVE;
                    curves.push((WireInstance { p0: p0.into(), p1: p1.into(), color, flags }, edge.id));
                    if edge.step_number.is_some() || edge.label.is_some() {
                        self.labeled.push(edge.id);
                    }
                }
            }
        }

        // Every segment of every route, keyed by (tile, quantized ends, kind, edge order). Most
        // sharing is one provider fanning out, so each provider's routes of one kind are
        // deduplicated together first; sorting then puts any remaining copies side by side, the
        // first belonging to the earliest edge, so each segment is drawn once and the result is
        // deterministic.
        let mut groups: Vec<Vec<u32>> = Vec::new();
        let mut group_of: FxHashMap<(NodeId, u32), usize> = FxHashMap::default();
        for (i, (edge, kind, _)) in routed.iter().enumerate() {
            let g = *group_of.entry((edge.from_node, *kind as u32)).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[g].push(i as u32);
        }
        let q = |v: f32| (v * 10.0).round() as i32;
        let mut keys: Vec<SegmentKey> = groups
            .par_iter()
            .map_init(
                || (Vec::new(), FxHashMap::default()),
                |(points, seen): &mut (Vec<Pos2>, FxHashMap<[i32; 4], usize>), group| {
                    seen.clear();
                    let mut out: Vec<SegmentKey> = Vec::new();
                    for &order in group {
                        let (edge, kind, pairs) = routed[order as usize];
                        routed_wire_points_into(graph, edge, points);
                        for w in points.windows(2).filter(|w| (w[0] - w[1]).length_sq() >= 1e-6) {
                            let (a, b) = if (w[0].x, w[0].y) <= (w[1].x, w[1].y) { (w[0], w[1]) } else { (w[1], w[0]) };
                            let ends = [q(a.x), q(a.y), q(b.x), q(b.y)];
                            // A stretch shared by several wires carries all their pairs.
                            match seen.get(&ends) {
                                Some(&i) => out[i].4 += pairs,
                                None => {
                                    let mid = a.lerp(b, 0.5);
                                    let tile = ((mid.x / TILE).floor() as i32, (mid.y / TILE).floor() as i32);
                                    seen.insert(ends, out.len());
                                    out.push((tile, ends, kind as u8, order, pairs));
                                }
                            }
                        }
                    }
                    out
                },
            )
            .flatten_iter()
            .collect();
        keys.par_sort_unstable_by_key(|k| (k.0, k.1, k.2, k.3));
        // Copies of one segment sit side by side: keep the first, adding up what they carry.
        let mut merged: Vec<SegmentKey> = Vec::with_capacity(keys.len());
        for k in keys {
            match merged.last_mut() {
                Some(m) if (m.0, m.1, m.2) == (k.0, k.1, k.2) => m.4 += k.4,
                _ => merged.push(k),
            }
        }

        self.segments.reserve(merged.len());
        self.segment_owner.reserve(merged.len());
        for (tile, ends, kind, order, pairs) in merged {
            let (edge, main_kind, _) = routed[order as usize];
            let instance = WireInstance {
                p0: [ends[0] as f32 / 10.0, ends[1] as f32 / 10.0],
                p1: [ends[2] as f32 / 10.0, ends[3] as f32 / 10.0],
                color: edge_kind_color(main_kind).to_srgba_unmultiplied(),
                flags: kind as u32 | wire_weight(pairs),
            };
            let bounds = wire_bounds(&instance);
            let i = self.segments.len() as u32;
            match self.tiles.last_mut() {
                Some(t) if self.tile_keys.last() == Some(&tile) => {
                    t.bounds = t.bounds.union(bounds);
                    t.end = i + 1;
                }
                _ => {
                    self.tiles.push(WireTile { bounds, start: i, end: i + 1 });
                    self.tile_keys.push(tile);
                }
            }
            self.segments.push(instance);
            self.segment_owner.push(edge.id);
        }
        self.tile_keys = Vec::new();
        (self.curves, self.curve_owner, self.curve_tiles) = tiled(curves);
    }

    fn build_folders(&mut self, graph: &Graph) {
        let mut order: Vec<usize> = (0..graph.clusters.len()).collect();
        order.sort_by_key(|&i| graph.clusters[i].depth);
        for i in order {
            let c = &graph.clusters[i];
            if c.size[0] <= 1.0 || c.size[1] <= 1.0 || graph.hidden_cluster_ids.contains(&c.id) {
                continue;
            }
            self.folder_order.push(i);
            let (fill, border) = cluster_tint(c.color_index, c.depth);
            let rect = Rect::from_min_size(Pos2::from(c.position), egui::vec2(c.size[0], c.size[1]));
            if c.detail == FolderDetail::Minimised {
                let shadow = rect.translate(egui::vec2(0.0, 2.0));
                self.boxes.push(boxed(shadow, Color32::from_black_alpha(70), Color32::TRANSPARENT, 8.0, 0.0));
                self.boxes.push(boxed(rect, CLUSTER_HEADER_BG, border, 8.0, 1.2));
            } else {
                self.boxes.push(boxed(rect, fill, border, 10.0, 1.5));
                let has_subtitle = c.subtitle.as_deref().is_some_and(|s| !s.is_empty());
                let header = Rect::from_min_size(
                    rect.min,
                    egui::vec2(
                        (rect.width() * 0.75).clamp(200.0, 420.0).min(rect.width()),
                        if has_subtitle { 42.0 } else { 32.0 },
                    ),
                );
                self.boxes.push(boxed(header, CLUSTER_HEADER_BG, border, 6.0, 1.0));
            }
        }
        self.cycle_start = self.boxes.len();
        let collapsed: HashSet<&str> =
            graph.clusters.iter().filter(|c| c.is_collapsed()).map(|c| c.id.as_str()).collect();
        if let Some(flow) = &graph.flow {
            for (i, b) in flow.cycle_boxes.iter().enumerate() {
                if !container_visible(graph, &collapsed, &b.id) {
                    continue;
                }
                self.cycle_order.push(i);
                let rect = Rect::from_min_size(Pos2::from(b.position), egui::vec2(b.size[0], b.size[1]));
                self.boxes.push(boxed(rect, with_alpha(CYCLE_BOX, 14), with_alpha(CYCLE_BOX, 170), 8.0, 1.5));
            }
        }
    }

    /// Hub badges under hubs, and documentation chips on cards and on closed folders that hide
    /// documented cards.
    fn build_chips(&mut self, graph: &Graph) {
        let index: FxHashMap<&str, usize> =
            graph.clusters.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
        let folder_rect = |id: &str| {
            let c = &graph.clusters[*index.get(id)?];
            (!graph.hidden_cluster_ids.contains(&c.id) && c.size[0] > 1.0).then_some((c.position, c.size))
        };
        let card_rect = |n: NodeId| {
            let node = graph.nodes.get(&n)?;
            (!graph.is_node_in_collapsed_cluster(n)).then_some((node.position, node.size))
        };
        if let Some(flow) = &graph.flow {
            for hub in &flow.hubs {
                let owner = match &hub.item {
                    VisibleEnd::Card(n) => card_rect(*n),
                    VisibleEnd::Folder(id) => folder_rect(id),
                };
                if let Some((pos, size)) = owner {
                    self.chips.push(Chip {
                        at: [pos[0], pos[1] + size[1] + 4.0],
                        count: hub.used_by,
                        kind: ChipKind::Hub,
                    });
                }
            }
        }

        // Documentation files linking to each card, counted on the card or on the outermost
        // closed folder hiding it.
        let outermost_closed = |n: NodeId| -> Option<usize> {
            let mut at = graph.nodes.get(&n)?.group_id.as_deref().and_then(|id| index.get(id).copied());
            let mut found = None;
            while let Some(i) = at {
                let c = &graph.clusters[i];
                if c.is_collapsed() {
                    found = Some(i);
                }
                at = c.parent_id.as_deref().and_then(|p| index.get(p).copied());
            }
            found
        };
        let mut on_card: std::collections::BTreeMap<NodeId, FxHashSet<NodeId>> = Default::default();
        let mut on_folder: std::collections::BTreeMap<usize, FxHashSet<(NodeId, NodeId)>> = Default::default();
        for e in graph.edges.iter().filter(|e| e.kind == studio_graph::EdgeKind::Documentation) {
            match outermost_closed(e.to_node) {
                Some(f) => {
                    on_folder.entry(f).or_default().insert((e.from_node, e.to_node));
                }
                None => {
                    on_card.entry(e.to_node).or_default().insert(e.from_node);
                }
            }
        }
        for (n, docs) in on_card {
            if let Some((pos, size)) = card_rect(n) {
                self.chips.push(Chip {
                    at: [pos[0] + size[0], pos[1] - 9.0],
                    count: docs.len() as u32,
                    kind: ChipKind::Docs,
                });
            }
        }
        for (f, docs) in on_folder {
            if let Some((pos, size)) = folder_rect(&graph.clusters[f].id) {
                self.chips.push(Chip {
                    at: [pos[0] + size[0], pos[1] - 9.0],
                    count: docs.len() as u32,
                    kind: ChipKind::Docs,
                });
            }
        }
    }

    fn build_gates(&mut self, graph: &Graph) {
        let Some(flow) = &graph.flow else { return };
        let collapsed: HashSet<&str> =
            graph.clusters.iter().filter(|c| c.is_collapsed()).map(|c| c.id.as_str()).collect();
        let node_view: HashSet<&str> = graph
            .clusters
            .iter()
            .filter(|c| c.detail == FolderDetail::NodeView && !graph.hidden_cluster_ids.contains(&c.id))
            .map(|c| c.id.as_str())
            .collect();
        let ring = SOCKET_RING_IDLE.to_srgba_unmultiplied();
        for gate in flow.gates.iter().filter(|g| container_visible(graph, &collapsed, &g.container)) {
            let fill = match gate.kind {
                GateKind::Code => KIND_IMPORT,
                GateKind::Documentation => KIND_DOCUMENTATION,
            };
            self.gates.push(PinInstance { center: gate.position, fill: fill.to_srgba_unmultiplied(), ring });
            // A folder in node view names the file or closed folder behind each of its pins.
            if node_view.contains(gate.container.as_str()) {
                let title = match &gate.source {
                    VisibleEnd::Card(n) => graph.nodes.get(n).map_or_else(String::new, |n| n.title.clone()),
                    VisibleEnd::Folder(id) => {
                        graph.clusters.iter().find(|c| &c.id == id).map_or_else(String::new, |c| c.label.clone())
                    }
                };
                self.pin_labels.push(PinLabel { at: gate.position, text: title, input: gate.side == GateSide::Input });
            }
        }
    }

    /// Builds the hover index now if it is not built yet. `CanvasState::refresh_scene` starts this
    /// on a worker thread; a hover before it finishes waits for it.
    pub fn build_hit_index(&self) {
        self.hit.get_or_init(|| self.hit_index());
    }

    fn hit_index(&self) -> Vec<((i32, i32), u32)> {
        use rayon::prelude::*;
        let cell = |p: Pos2| ((p.x / HIT_CELL).floor() as i32, (p.y / HIT_CELL).floor() as i32);
        let span = |a: (i32, i32), b: (i32, i32)| {
            (a.0.min(b.0)..=a.0.max(b.0)).flat_map(move |cx| (a.1.min(b.1)..=a.1.max(b.1)).map(move |cy| (cx, cy)))
        };
        let segments =
            self.segments.par_iter().enumerate().flat_map_iter(|(i, w)| {
                span(cell(Pos2::from(w.p0)), cell(Pos2::from(w.p1))).map(move |c| (c, i as u32))
            });
        let curves = self.curves.par_iter().enumerate().flat_map_iter(|(i, w)| {
            // Cells along the curve, sampled in twelve steps.
            let (a, b) = (Pos2::from(w.p0), Pos2::from(w.p1));
            let (c1, c2) = world_control_points(a, b);
            let mut cells: Vec<(i32, i32)> = (0..12)
                .flat_map(|step| {
                    let p = |t: f32| crate::wire::eval_cubic_bezier(a, c1, c2, b, t);
                    span(cell(p(step as f32 / 12.0)), cell(p((step + 1) as f32 / 12.0)))
                })
                .collect();
            cells.sort_unstable();
            cells.dedup();
            cells.into_iter().map(move |c| (c, i as u32 | HIT_CURVE))
        });
        let mut hit: Vec<((i32, i32), u32)> = segments.chain(curves).collect();
        hit.par_sort_unstable();
        hit
    }

    /// The wire nearest to `world` within `reach` (world units) whose kind `visible` accepts.
    pub fn hit_test(&self, world: Pos2, reach: f32, visible: impl Fn(u32) -> bool) -> Option<EdgeId> {
        let hit = self.hit.get_or_init(|| self.hit_index());
        let lo = ((world.x - reach) / HIT_CELL).floor() as i32..=((world.x + reach) / HIT_CELL).floor() as i32;
        let mut best: Option<(f32, EdgeId)> = None;
        for cx in lo {
            for cy in ((world.y - reach) / HIT_CELL).floor() as i32..=((world.y + reach) / HIT_CELL).floor() as i32 {
                let from = hit.partition_point(|&(c, _)| c < (cx, cy));
                let to = hit.partition_point(|&(c, _)| c <= (cx, cy));
                for &(_, i) in &hit[from..to] {
                    let (w, owner) = if i & HIT_CURVE != 0 {
                        let i = (i & !HIT_CURVE) as usize;
                        (&self.curves[i], self.curve_owner[i])
                    } else {
                        (&self.segments[i as usize], self.segment_owner[i as usize])
                    };
                    if !visible(w.kind_index()) {
                        continue;
                    }
                    let (a, b) = (Pos2::from(w.p0), Pos2::from(w.p1));
                    let d = if w.is_curve() {
                        let (c1, c2) = world_control_points(a, b);
                        crate::wire::distance_to_bezier(world, a, c1, c2, b)
                    } else {
                        distance_to_segment(world, a, b)
                    };
                    if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, owner));
                    }
                }
            }
        }
        best.map(|(_, e)| e)
    }

    /// Ranges of `segments` in tiles that touch `view` (world), adjacent runs merged.
    pub fn visible_ranges(&self, view: Rect) -> Vec<std::ops::Range<u32>> {
        visible_runs(&self.tiles, view)
    }

    /// Ranges of `curves` in tiles that touch `view` (world), adjacent runs merged.
    pub fn visible_curve_ranges(&self, view: Rect) -> Vec<std::ops::Range<u32>> {
        visible_runs(&self.curve_tiles, view)
    }
}

fn visible_runs(tiles: &[WireTile], view: Rect) -> Vec<std::ops::Range<u32>> {
    let mut ranges: Vec<std::ops::Range<u32>> = Vec::new();
    for t in tiles.iter().filter(|t| t.bounds.intersects(view)) {
        match ranges.last_mut() {
            Some(r) if r.end == t.start => r.end = t.end,
            _ => ranges.push(t.start..t.end),
        }
    }
    ranges
}

/// World bounds of a wire instance, including a curve's control points.
fn wire_bounds(w: &WireInstance) -> Rect {
    let (a, b) = (Pos2::from(w.p0), Pos2::from(w.p1));
    if w.is_curve() {
        let (c1, c2) = world_control_points(a, b);
        Rect::from_points(&[a, c1, c2, b])
    } else {
        Rect::from_two_pos(a, b)
    }
}

/// Sorts wires by the tile of their midpoint and groups them into tiles, so each frame draws whole
/// runs of visible tiles.
fn tiled(mut wires: Vec<(WireInstance, EdgeId)>) -> (Vec<WireInstance>, Vec<EdgeId>, Vec<WireTile>) {
    let tile_of = |w: &WireInstance| {
        let mid = [(w.p0[0] + w.p1[0]) * 0.5, (w.p0[1] + w.p1[1]) * 0.5];
        ((mid[0] / TILE).floor() as i32, (mid[1] / TILE).floor() as i32)
    };
    wires.sort_by_key(|(w, _)| tile_of(w));
    let mut tiles: Vec<WireTile> = Vec::new();
    let mut current = None;
    for (i, (w, _)) in wires.iter().enumerate() {
        let key = tile_of(w);
        let bounds = wire_bounds(w);
        match tiles.last_mut() {
            Some(tile) if current == Some(key) => {
                tile.bounds = tile.bounds.union(bounds);
                tile.end = i as u32 + 1;
            }
            _ => {
                tiles.push(WireTile { bounds, start: i as u32, end: i as u32 + 1 });
                current = Some(key);
            }
        }
    }
    let (wires, owners) = wires.into_iter().unzip();
    (wires, owners, tiles)
}

/// The highlight layer: wires that are hovered, belong to a selected card, or are part of the
/// active flow. Built every frame from those few edges only. Returns the instances (straight
/// segments, then curves) and where the curves start.
pub fn build_overlay(graph: &Graph, edges: impl IntoIterator<Item = (EdgeId, u32)>) -> (Vec<WireInstance>, u32) {
    let mut straight = Vec::new();
    let mut curves = Vec::new();
    let mut points = Vec::new();
    let mut done: FxHashSet<EdgeId> = FxHashSet::default();
    for (id, flag) in edges {
        let Some(edge) = graph.get_edge(id) else { continue };
        if !done.insert(id) {
            continue;
        }
        let color = edge_kind_color(edge.kind).to_srgba_unmultiplied();
        let flags = edge.kind as u32 | flag;
        if routed_wire_points_into(graph, edge, &mut points) {
            straight.extend(points.windows(2).map(|w| WireInstance { p0: w[0].into(), p1: w[1].into(), color, flags }));
        } else if !graph.is_node_in_collapsed_cluster(edge.from_node)
            && !graph.is_node_in_collapsed_cluster(edge.to_node)
        {
            let (p0, p1) = curve_ends(graph, edge);
            curves.push(WireInstance { p0: p0.into(), p1: p1.into(), color, flags: flags | WIRE_CURVE });
        }
    }
    let curves_start = straight.len() as u32;
    straight.extend(curves);
    (straight, curves_start)
}

/// Cards as plain rects, for zoom levels where their contents cannot be read.
pub fn build_card_layer(
    graph: &Graph,
    dimmed: impl Fn(&studio_graph::Node) -> bool,
    selected: &std::collections::BTreeSet<NodeId>,
) -> Vec<BoxInstance> {
    graph
        .nodes
        .values()
        .filter(|n| !graph.is_node_in_collapsed_cluster(n.id))
        .map(|n| {
            let fill = if selected.contains(&n.id) {
                CARD_BORDER_SELECTED
            } else {
                with_alpha(archetype_color(n.archetype), if dimmed(n) { 35 } else { 175 })
            };
            let rect = Rect::from_min_size(Pos2::from(n.position), egui::vec2(n.size[0], n.size[1]));
            boxed(rect, fill, Color32::TRANSPARENT, 0.0, 0.0)
        })
        .collect()
}

fn boxed(rect: Rect, fill: Color32, border: Color32, radius: f32, border_width: f32) -> BoxInstance {
    BoxInstance {
        min: rect.min.into(),
        size: rect.size().into(),
        fill: fill.to_srgba_unmultiplied(),
        border: border.to_srgba_unmultiplied(),
        radius,
        border_width,
    }
}

/// Ends of a wire drawn as a curve: its ports, or the card edges when a port is unknown.
pub(crate) fn curve_ends(graph: &Graph, edge: &Edge) -> (Pos2, Pos2) {
    let from = graph.nodes.get(&edge.from_node);
    let to = graph.nodes.get(&edge.to_node);
    let p0 = from
        .and_then(|n| {
            port_world_position(n, edge.from_port)
                .or(Some(Pos2::new(n.position[0] + n.size[0], n.position[1] + n.size[1] * 0.5)))
        })
        .unwrap_or_default();
    let p1 = to
        .and_then(|n| {
            port_world_position(n, edge.to_port).or(Some(Pos2::new(n.position[0], n.position[1] + n.size[1] * 0.5)))
        })
        .unwrap_or_default();
    (p0, p1)
}

/// Control points of a curve in world space, as the wire shader draws it.
pub fn world_control_points(p0: Pos2, p1: Pos2) -> (Pos2, Pos2) {
    let tangent = ((p1.x - p0.x).abs() * 0.38 + (p1.y - p0.y).abs() * 0.12).clamp(40.0, 320.0);
    (p0 + egui::vec2(tangent, 0.0), p1 - egui::vec2(tangent, 0.0))
}

pub fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq < 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Outline colour of cycle boxes.
pub const CYCLE_BOX: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24);

/// Whether a folder (cluster id) or a cycle box (`cycle:<folder>:<n>`) is on screen: not inside
/// a collapsed folder, and for a cycle box, its folder is not collapsed either.
pub fn container_visible(graph: &Graph, collapsed: &HashSet<&str>, id: &str) -> bool {
    let folder = match id.strip_prefix("cycle:").and_then(|rest| rest.rsplit_once(':')) {
        Some((folder, _)) => folder,
        None => return !graph.hidden_cluster_ids.contains(id),
    };
    !graph.hidden_cluster_ids.contains(folder) && !collapsed.contains(folder)
}

#[cfg(test)]
mod tests;

/// Where a bundle's count goes: the middle of its longest horizontal segment.
fn label_anchor(points: &[[f32; 2]]) -> Option<[f32; 2]> {
    points
        .windows(2)
        .filter(|w| (w[0][1] - w[1][1]).abs() < 0.01)
        .max_by(|a, b| (a[1][0] - a[0][0]).abs().total_cmp(&(b[1][0] - b[0][0]).abs()))
        .map(|w| [(w[0][0] + w[1][0]) * 0.5, w[0][1]])
}
