use egui::{Color32, Pos2, Rect, Vec2};
use std::collections::BTreeSet;
use studio_graph::{DataType, EdgeId, EdgeKind, FolderDetail, Graph, NodeArchetype, NodeId};
use studio_ui::color_tokens::*;

use std::sync::Arc;

use crate::gpu::CardLayer;
use crate::interaction::{HoverState, InteractionMode};
use crate::scene::CanvasScene;
use crate::spatial::SpatialHashGrid;
use crate::transform::CanvasTransform;

/// Maps DataType to its designated UI accent color.
pub fn data_type_color(data_type: &DataType) -> Color32 {
    match data_type {
        DataType::Audio => TYPE_AUDIO,
        DataType::Vision => TYPE_VISION,
        DataType::Text => TYPE_TEXT,
        DataType::State => TYPE_STATE,
        DataType::Flow => TYPE_FLOW,
        DataType::Composite => TYPE_COMPOSITE,
        DataType::RustFlow => TYPE_FLOW,
        DataType::Documentation => KIND_DOCUMENTATION,
        DataType::RustType(name) => {
            let s = name.trim_start_matches('&').trim_start_matches("mut ").trim();
            match s {
                "bool" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
                | "isize" | "f32" | "f64" => {
                    Color32::from_rgb(250, 204, 21) // Amber/Yellow
                }
                "String" | "str" => Color32::from_rgb(56, 189, 248), // Sky blue
                s if s.starts_with("Option") || s.starts_with("Result") => Color32::from_rgb(52, 211, 153), // Emerald
                s if s.starts_with("Vec")
                    || s.starts_with("HashMap")
                    || s.starts_with("BTreeMap")
                    || s.starts_with("HashSet") =>
                {
                    Color32::from_rgb(167, 139, 250) // Purple
                }
                _ => {
                    let mut hash: u32 = 5381;
                    for b in s.bytes() {
                        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(b as u32);
                    }
                    let hue = (hash % 360) as f32 / 360.0;
                    hsv_to_rgb(hue, 0.65, 0.90)
                }
            }
        }
    }
}

pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color32 {
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Maps NodeArchetype to its designated UI accent color.
pub fn archetype_color(archetype: NodeArchetype) -> Color32 {
    match archetype {
        NodeArchetype::Ingress => ARCHETYPE_INGRESS,
        NodeArchetype::Compute => ARCHETYPE_COMPUTE,
        NodeArchetype::State => ARCHETYPE_STATE,
        NodeArchetype::Egress => ARCHETYPE_EGRESS,
        NodeArchetype::Module => KIND_IMPORT,
        NodeArchetype::Function => KIND_CALL,
        NodeArchetype::Struct => KIND_TYPE_USE,
        NodeArchetype::Enum => KIND_ENUM,
        NodeArchetype::Trait => KIND_IMPLEMENTS,
        NodeArchetype::File => ARCHETYPE_FILE,
        NodeArchetype::Link => KIND_DOCUMENTATION,
    }
}

/// Colour of a wire, by what it means (docs/VISUAL_LANGUAGE.md).
pub fn edge_kind_color(kind: EdgeKind) -> Color32 {
    match kind {
        EdgeKind::Call => KIND_CALL,
        EdgeKind::TypeUse => KIND_TYPE_USE,
        EdgeKind::Implements => KIND_IMPLEMENTS,
        EdgeKind::Import => KIND_IMPORT,
        EdgeKind::Documentation => KIND_DOCUMENTATION,
        EdgeKind::Asset => KIND_ASSET,
    }
}

/// High-level user interaction actions triggered on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasAction {
    CenterNode(NodeId),
    InspectNode(NodeId),
    ToggleFileDropdown(NodeId),
    ToggleMemberWires(NodeId),
    ToggleMemberDrawer(NodeId, String),
    InspectMember(NodeId, String),
    ToggleCodeExpand(NodeId),
    ToggleMarkdownPreview(NodeId),
    SetNodeTab(NodeId, usize),
    DeleteNode(NodeId),
    FitGraph,
    ResetGraph,
    /// Load a minimised folder whose contents are not in the graph yet (cluster id), then show it
    /// at the given detail level.
    ExpandFolder(String, FolderDetail),
}

pub use studio_ui::ContextMenuItem as ContextMenuAction;

#[derive(Debug, Clone)]
pub struct NodeContextMenu {
    pub node_id: NodeId,
    pub screen_pos: Pos2,
}

/// What the last rendered frame drew, for the debug panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanvasFrameStats {
    pub visible_nodes: usize,
    /// Wire instances submitted: straight segments in visible tiles, curves and highlights.
    pub visible_wires: usize,
}

/// Which kinds of wire are shown, one switch per kind (docs/VISUAL_LANGUAGE.md colours).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireKinds {
    pub calls: bool,
    pub type_uses: bool,
    pub implements: bool,
    pub imports: bool,
    pub documentation: bool,
    pub assets: bool,
}

impl Default for WireKinds {
    fn default() -> Self {
        Self { calls: true, type_uses: true, implements: true, imports: true, documentation: true, assets: true }
    }
}

impl WireKinds {
    /// Bit `EdgeKind as u32` set for every kind shown.
    pub fn mask(&self) -> u32 {
        [
            (EdgeKind::Call, self.calls),
            (EdgeKind::TypeUse, self.type_uses),
            (EdgeKind::Implements, self.implements),
            (EdgeKind::Import, self.imports),
            (EdgeKind::Documentation, self.documentation),
            (EdgeKind::Asset, self.assets),
        ]
        .into_iter()
        .filter(|&(_, shown)| shown)
        .fold(0, |mask, (kind, _)| mask | 1 << kind as u32)
    }
}

/// State of the canvas viewport, camera, and interactions.
#[derive(Debug, Clone)]
pub struct CanvasState {
    pub transform: CanvasTransform,
    pub interaction: InteractionMode,
    pub hover: HoverState,
    pub selected_nodes: BTreeSet<NodeId>,
    pub status_message: Option<String>,
    pub search_filter: String,
    pub category_filter: BTreeSet<NodeArchetype>,
    pub active_flow_edges: Option<BTreeSet<EdgeId>>,
    /// Master switch for every wire; hidden wires are neither drawn nor hit-tested.
    pub show_wires: bool,
    pub show_subnode_wires_globally: bool,
    pub collapsed_subnodes: BTreeSet<(NodeId, String)>,
    pub action_request: Option<CanvasAction>,
    pub context_menu: Option<NodeContextMenu>,
    pub spatial_grid: SpatialHashGrid,
    /// Static world-space contents (wires, folder frames, gates), rebuilt when geometry changes.
    pub scene: Arc<CanvasScene>,
    /// The layout or a card's size changed: rebuild the spatial grid and the scene next frame.
    pub scene_dirty: bool,
    /// Which kinds of wire are shown.
    pub wire_kinds: WireKinds,
    /// Cards as rects for far zoom, and the inputs they were built from.
    pub card_layer: Option<(u64, CardLayer)>,
    pub interactive_rects: Vec<Rect>,
    pub use_gpu_wires: bool,
    pub frame_stats: CanvasFrameStats,
    /// Frames drawn, for telling frames apart on the GPU.
    pub frame_counter: u64,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            transform: CanvasTransform::default(),
            interaction: InteractionMode::Idle,
            hover: HoverState::default(),
            selected_nodes: BTreeSet::new(),
            status_message: None,
            search_filter: String::new(),
            category_filter: BTreeSet::new(),
            active_flow_edges: None,
            show_wires: true,
            show_subnode_wires_globally: true,
            collapsed_subnodes: BTreeSet::new(),
            action_request: None,
            context_menu: None,
            spatial_grid: SpatialHashGrid::default(),
            scene: Arc::new(CanvasScene::default()),
            scene_dirty: true,
            wire_kinds: WireKinds::default(),
            card_layer: None,
            interactive_rects: Vec::new(),
            use_gpu_wires: true,
            frame_stats: CanvasFrameStats::default(),
            frame_counter: 0,
        }
    }
}

impl CanvasState {
    /// Marks the layout as changed: the spatial grid and the scene are rebuilt before the next
    /// frame draws.
    pub fn mark_scene_dirty(&mut self) {
        self.scene_dirty = true;
    }

    /// Rebuilds the spatial grid and the scene if the layout changed or the member-wire switch
    /// differs from the one the scene was built with.
    pub fn refresh_scene(&mut self, graph: &Graph) {
        if self.scene_dirty || self.scene.member_wires != self.show_subnode_wires_globally || self.scene.revision == 0 {
            self.spatial_grid.build_from_graph(graph);
            self.scene = Arc::new(CanvasScene::build(graph, self.show_subnode_wires_globally));
            // The hover index is only needed once the pointer is over a wire: build it off the
            // UI thread.
            let scene = self.scene.clone();
            rayon::spawn(move || scene.build_hit_index());
            self.scene_dirty = false;
        }
    }

    /// Whether wires of this kind (`EdgeKind as u32`) are shown.
    pub fn wire_kind_visible(&self, kind: u32) -> bool {
        self.wire_kinds.mask() & (1 << kind) != 0
    }

    pub fn reset_view(&mut self) {
        self.transform.reset();
    }

    pub fn zoom_to_fit(&mut self, graph: &Graph, screen_rect: Rect) {
        if graph.nodes.is_empty() {
            self.transform.reset();
            return;
        }

        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        if !graph.clusters.is_empty() {
            for cluster in &graph.clusters {
                if cluster.size[0] > 1.0 && cluster.size[1] > 1.0 && !graph.hidden_cluster_ids.contains(&cluster.id) {
                    min_x = min_x.min(cluster.position[0]);
                    min_y = min_y.min(cluster.position[1]);
                    max_x = max_x.max(cluster.position[0] + cluster.size[0]);
                    max_y = max_y.max(cluster.position[1] + cluster.size[1]);
                }
            }
        } else {
            for node in graph.nodes.values() {
                min_x = min_x.min(node.position[0]);
                min_y = min_y.min(node.position[1]);
                max_x = max_x.max(node.position[0] + node.size[0]);
                max_y = max_y.max(node.position[1] + node.size[1]);
            }
        }

        let padding = 80.0;
        let content_width = ((max_x - min_x) + padding * 2.0).max(100.0);
        let content_height = ((max_y - min_y) + padding * 2.0).max(100.0);

        let scale_x = screen_rect.width() / content_width;
        let scale_y = screen_rect.height() / content_height;
        let fit_zoom = scale_x.min(scale_y).clamp(self.transform.min_zoom, 2.0);

        let content_center_x = (min_x + max_x) * 0.5;
        let content_center_y = (min_y + max_y) * 0.5;

        self.transform.zoom = fit_zoom;
        self.transform.pan = Vec2::new(
            screen_rect.center().x - content_center_x * fit_zoom,
            screen_rect.center().y - content_center_y * fit_zoom,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomAction {
    ZoomIn,
    ZoomOut,
    Fit,
    Reset,
}

#[derive(Default)]
pub struct RenderEvents {
    /// A folder's detail-level button was clicked: (cluster id, level).
    pub folder_detail: Option<(String, FolderDetail)>,
    pub file_dropdown_toggle_clicked: Option<NodeId>,
    pub file_wires_toggle_clicked: Option<NodeId>,
    pub file_code_expand_clicked: Option<NodeId>,
    pub member_code_toggle_clicked: Option<(NodeId, String)>,
    pub member_row_clicked: Option<(NodeId, String)>,
    pub code_close_clicked: Option<NodeId>,
    pub markdown_preview_toggle_clicked: Option<NodeId>,
    pub node_tab_clicked: Option<(NodeId, usize)>,
    pub port_jump_clicked: Option<NodeId>,
    pub subnode_jump_clicked: Option<(NodeId, usize)>,
    pub member_fold_clicked: Option<(NodeId, String)>,
    pub context_menu_action: Option<(NodeId, ContextMenuAction)>,
    pub single_selected_action: Option<CanvasAction>,
    pub zoom_action: Option<ZoomAction>,
}
