use egui::{vec2, Color32, Pos2, Rect, Vec2};
use std::collections::BTreeSet;
use studio_graph::{DataType, EdgeId, EdgeKind, FolderDetail, Graph, NodeArchetype, NodeId};
use studio_ui::color_tokens::*;

use std::sync::Arc;

use crate::camera::{self, Camera, CameraTarget, Stop, View};
use crate::gpu::CardLayer;
use crate::interaction::{HoverState, InteractionMode};
use crate::scene::CanvasScene;
use crate::spatial::SpatialHashGrid;
use crate::transform::CanvasTransform;
use crate::world::WorldLayout;

/// Maps DataType to its designated UI accent color.
pub fn data_type_color(data_type: &DataType) -> Color32 {
    match data_type {
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
    /// Open a file card's file on the Desk and go there.
    OpenOnDesk(NodeId),
    FitGraph,
    ResetGraph,
    /// Load a minimised folder whose contents are not in the graph yet (cluster id), then show it
    /// at the given detail level.
    ExpandFolder(String, FolderDetail),
    /// Read the files this commit (full id) changed, for the Changes district.
    LoadCommitFiles(String),
    /// Open this agent session's plan (session id) on the Desk.
    OpenSessionPlan(String),
    /// Light what this agent session (id) touched on the map, or nothing.
    LightSession(Option<String>),
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
        // Documentation shows as chips on cards until its wires are turned on in the View menu.
        Self { calls: true, type_uses: true, implements: true, imports: true, documentation: false, assets: true }
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
    /// Text to put on the clipboard once the frame ends.
    pub copy_request: Option<String>,
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
    /// The folder last opened or picked in the breadcrumb (cluster id); `None` for the project.
    pub focus: Option<String>,
    /// Cards upstream and downstream of the selection; everything else is dimmed.
    pub trace_nodes: Option<BTreeSet<NodeId>>,
    /// The selection `trace_nodes` was worked out for.
    trace_for: BTreeSet<NodeId>,
    /// Every move of the view that is not the user's own goes through the camera.
    pub camera: Camera,
    /// Where the districts are, rebuilt with the scene.
    pub world: WorldLayout,
    /// The stop the camera is at, or flying to.
    pub stop: Stop,
    /// The stop the current flight is heading for.
    flight_stop: Option<Stop>,
    /// What the current flight was asked to reach, so a flight to a district that moves on the
    /// way can be sent on to where it is now.
    flight_target: Option<CameraTarget>,
    /// The canvas rectangle of the last frame, for what is drawn over it.
    pub last_rect: Rect,
    /// The files open on the Desk.
    pub desk: crate::desk::DeskState,
    /// What the districts around the map show.
    pub districts: crate::districts::DistrictViews,
    /// The tool picked in the Run district.
    pub run_selected: Option<usize>,
    /// Folders open in the Files district's disk tree (cluster ids).
    pub files_open: BTreeSet<String>,
    /// The disk tree's rows, and what they were worked out from (scene revision, open folders).
    pub files_rows: Option<(u64, Arc<Vec<crate::districts::files::TreeRow>>)>,
    /// What is picked in the Changes district.
    pub changes_selected: crate::districts::changes::ChangesSelection,
    /// The agent session whose touches are lit on the map (session id).
    pub session_lit: Option<String>,
    /// The cards of the files that session touched, found once per change of session or map.
    pub session_lit_nodes: BTreeSet<NodeId>,
    /// Pipeline groups the user opened or closed against their default (group keys).
    pub pipeline_open: BTreeSet<String>,
    /// The Pipeline district's layout, worked out once per view, open groups and width.
    pub pipeline_layout_cache: Option<PipelineLayoutCache>,
}

/// A Pipeline layout and what it was worked out from: the view (kept, so a new view never
/// matches by address), the open groups, and the width in local units as `f32` bits.
pub type PipelineLayoutCache = (
    Arc<crate::districts::pipeline::PipelineView>,
    BTreeSet<String>,
    u32,
    Arc<crate::districts::pipeline::PipelineLayout>,
);

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            transform: CanvasTransform::default(),
            interaction: InteractionMode::Idle,
            hover: HoverState::default(),
            selected_nodes: BTreeSet::new(),
            status_message: None,
            copy_request: None,
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
            focus: None,
            trace_nodes: None,
            trace_for: BTreeSet::new(),
            camera: Camera::default(),
            world: WorldLayout::default(),
            stop: Stop::World,
            flight_stop: None,
            flight_target: None,
            last_rect: Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)),
            desk: Default::default(),
            districts: Default::default(),
            run_selected: None,
            files_open: BTreeSet::new(),
            files_rows: None,
            changes_selected: Default::default(),
            session_lit: None,
            session_lit_nodes: BTreeSet::new(),
            pipeline_open: BTreeSet::new(),
            pipeline_layout_cache: None,
        }
    }
}

impl CanvasState {
    /// Opens a folder in place (L5 level 3), makes it the focus and fits it in view. A folder
    /// whose contents are not loaded yet is loaded first by the host.
    pub fn open_folder(&mut self, graph: &mut Graph, cluster_id: &str) {
        let Some(cluster) = graph.clusters.iter().find(|c| c.id == cluster_id) else { return };
        if cluster.is_collapsed() && cluster.lazy.is_some() {
            self.action_request = Some(CanvasAction::ExpandFolder(cluster_id.to_string(), FolderDetail::Open));
        } else if cluster.is_collapsed() {
            graph.set_folder_detail(cluster_id, FolderDetail::Open);
            self.mark_scene_dirty();
        }
        self.focus = Some(cluster_id.to_string());
        self.camera.fly_to(CameraTarget::Folder(cluster_id.to_string()));
    }

    /// Goes back up to a folder (or, with `None`, the whole project): every folder open inside it
    /// closes to node view, and it becomes the focus and fills the view.
    pub fn focus_folder(&mut self, graph: &mut Graph, cluster_id: Option<&str>) {
        let root = graph.clusters.iter().find(|c| c.parent_id.is_none()).map(|c| c.id.clone());
        let Some(top) = cluster_id.map(str::to_string).or(root) else { return };
        let changes: Vec<(String, FolderDetail)> = graph
            .descendant_clusters(&top)
            .into_iter()
            .filter(|id| graph.clusters.iter().any(|c| &c.id == id && c.detail == FolderDetail::Open))
            .map(|id| (id, FolderDetail::NodeView))
            .collect();
        graph.set_folder_details(&changes);
        self.mark_scene_dirty();
        self.focus = cluster_id.map(str::to_string);
        self.camera.fly_to(match cluster_id {
            Some(id) => CameraTarget::Folder(id.to_string()),
            None => CameraTarget::Stop(Stop::Code),
        });
    }

    /// Works out the trace when the selection changed: the cards and code wires upstream and
    /// downstream of the selected cards. The wires are drawn as the active flow; other cards dim.
    pub fn refresh_trace(&mut self, graph: &Graph) {
        if self.selected_nodes == self.trace_for {
            return;
        }
        self.trace_for = self.selected_nodes.clone();
        if self.selected_nodes.is_empty() {
            self.active_flow_edges = None;
            self.trace_nodes = None;
        } else {
            let (wires, cards) = graph.trace(&self.selected_nodes);
            self.active_flow_edges = Some(wires);
            self.trace_nodes = Some(cards);
        }
    }

    /// The zoom as the user sees it: 100% where text is drawn at its natural size (in a
    /// district's own units for the districts around the map).
    pub fn zoom_percent(&self) -> f32 {
        let local = if matches!(self.stop, Stop::Code | Stop::World) { 1.0 } else { self.world.scale };
        self.transform.zoom * local * 100.0
    }

    /// Back to 100% about the middle of the view.
    pub fn actual_size(&mut self) {
        self.camera.cancel();
        let one = if matches!(self.stop, Stop::Code | Stop::World) { 1.0 } else { self.world.local_zoom_one() };
        self.transform.zoom_at_pointer(self.last_rect.center(), one / self.transform.zoom);
    }

    /// The Files district's disk tree, worked out again only when the map or the open folders
    /// change.
    pub fn files_tree(&mut self, graph: &Graph) -> Arc<Vec<crate::districts::files::TreeRow>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.scene.revision.hash(&mut hasher);
        self.files_open.hash(&mut hasher);
        let key = hasher.finish();
        if let Some((k, rows)) = &self.files_rows {
            if *k == key {
                return rows.clone();
            }
        }
        let rows = Arc::new(crate::districts::files::tree_rows(graph, &self.files_open));
        self.files_rows = Some((key, rows.clone()));
        rows
    }

    /// Flies the camera to `target`.
    pub fn fly_to(&mut self, target: CameraTarget) {
        self.camera.fly_to(target);
    }

    /// Moves the camera to `target` at once.
    pub fn jump_to(&mut self, target: CameraTarget) {
        self.camera.jump_to(target);
    }

    /// Moves the camera for this frame: starts the flight to a pending target (now that the canvas
    /// knows its size), then follows the flight. Also works out which stop the camera is at.
    pub fn tick_camera(&mut self, graph: &Graph, screen: Rect, now: f64, pixels_per_point: f32) {
        let here = View { pan: self.transform.pan, zoom: self.transform.zoom };
        if let Some((target, fly)) = self.camera.pending.take() {
            if let Some((to, snap_anchor)) = self.resolve(&target, graph, screen) {
                self.flight_target = Some(target.clone());
                self.flight_stop = Some(match target {
                    CameraTarget::Stop(stop) => stop,
                    CameraTarget::Rect(r) => self.world.stop_for_view(r),
                    CameraTarget::Folder(_) | CameraTarget::Node(_) => Stop::Code,
                });
                if fly {
                    self.camera.flight = Some(camera::Flight::new(here, to, screen, now, snap_anchor));
                } else {
                    self.camera.flight =
                        Some(camera::Flight::new(to, to, screen, now - camera::FLIGHT_SECONDS, snap_anchor));
                }
            }
        }
        if let Some(flight) = self.camera.flight {
            let (mut view, landed) = flight.at(now);
            if landed {
                if let Some(anchor) = flight.snap_anchor() {
                    view = camera::snap(view, anchor, pixels_per_point);
                }
                self.camera.flight = None;
                self.flight_stop = None;
                self.flight_target = None;
            }
            self.transform.pan = view.pan;
            self.transform.zoom = view.zoom.clamp(self.transform.min_zoom, self.transform.max_zoom);
        }
        if self.camera.flight.is_none() {
            // A flight the user stopped (a drag) heads nowhere any more.
            self.flight_stop = None;
            self.flight_target = None;
        }
        self.stop = match self.flight_stop {
            Some(stop) => stop,
            None => self.world.stop_for_view(self.transform.screen_to_world_rect(screen)),
        };
    }

    /// The view a target asks for, and the world point to snap to a pixel if it lands at 100%.
    fn resolve(&self, target: &CameraTarget, graph: &Graph, screen: Rect) -> Option<(View, Option<Pos2>)> {
        let zooms = |max: f32| (self.transform.min_zoom, max.min(self.transform.max_zoom));
        let one = self.world.local_zoom_one();
        let resolved = match target {
            CameraTarget::Stop(Stop::World) => (camera::fit(self.world.bounds, screen, 48.0, zooms(1.0)), None),
            CameraTarget::Stop(Stop::Code) => (camera::fit(self.world.code, screen, 48.0, zooms(1.0)), None),
            CameraTarget::Stop(Stop::Pipeline) => {
                // At 100% on its top left corner, so its header and first group read however
                // tall it grew; centred across when it fits.
                let r = self.world.pipeline;
                let x = if r.width() * one <= screen.width() - 96.0 {
                    screen.center().x - r.center().x * one
                } else {
                    screen.min.x + 48.0 - r.min.x * one
                };
                (View { pan: vec2(x, screen.min.y + 48.0 - r.min.y * one), zoom: one }, Some(r.min))
            }
            CameraTarget::Stop(Stop::Desk) => {
                // At 100%, so its text is crisp, with its top left corner in view.
                let desk = self.world.desk;
                let fits = desk.width() * one <= screen.width() - 96.0;
                let view = if fits {
                    camera::centre_on(desk.center(), one, screen)
                } else {
                    let pan = (screen.min + vec2(48.0, 48.0)).to_vec2() - desk.min.to_vec2() * one;
                    View { pan, zoom: one }
                };
                (view, Some(desk.min))
            }
            CameraTarget::Stop(stop) => {
                let r = self.world.rect(*stop);
                let view = camera::fit(r, screen, 48.0, zooms(one));
                (view, ((view.zoom - one).abs() < 1e-6).then_some(r.min))
            }
            CameraTarget::Folder(id) => {
                let c = graph.clusters.iter().find(|c| &c.id == id)?;
                let r = Rect::from_min_size(Pos2::from(c.position), Vec2::new(c.size[0], c.size[1]));
                (camera::fit(r, screen, 60.0, zooms(2.0)), None)
            }
            CameraTarget::Node(id) => {
                let n = graph.nodes.get(id)?;
                let centre = Pos2::new(n.position[0] + n.size[0] * 0.5, n.position[1] + n.size[1] * 0.5);
                // Keep the zoom unless the card would be too small to read.
                let zoom = if self.transform.zoom < 0.35 { 0.8 } else { self.transform.zoom };
                (camera::centre_on(centre, zoom, screen), None)
            }
            CameraTarget::Rect(r) => (camera::fit(*r, screen, 60.0, zooms(2.0)), None),
        };
        Some(resolved)
    }

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
            let world = WorldLayout::for_graph(graph, |width| self.district_extents(width));
            let before = std::mem::replace(&mut self.world, world);
            // The map grew or shrank and moved the districts: the camera stays on the one it is
            // at, so what is on screen does not move.
            if self.world != before && !matches!(self.stop, Stop::Code | Stop::World) && !self.camera.is_flying() {
                self.camera.jump_to(CameraTarget::Stop(self.stop));
            } else if self.world != before {
                self.retarget_flight(&before);
            }
            // The hover index is only needed once the pointer is over a wire: build it off the
            // UI thread.
            let scene = self.scene.clone();
            rayon::spawn(move || scene.build_hit_index());
            self.scene_dirty = false;
        }
    }

    /// How tall the districts' content is, laid out in a centre column `width` local units wide.
    pub fn district_extents(&mut self, width: f32) -> crate::world::DistrictExtents {
        crate::world::DistrictExtents { pipeline: self.pipeline_layout(width).map(|l| l.height), changes: None }
    }

    /// The Pipeline district's layout in a centre column `width` local units wide (see
    /// [`WorldLayout::centre_width_local`]); `None` until its view arrives. Worked out again only
    /// when the view, the open groups or the width change, never per frame.
    pub fn pipeline_layout(&mut self, width: f32) -> Option<Arc<crate::districts::pipeline::PipelineLayout>> {
        let view = self.districts.pipeline.clone()?;
        if let Some((v, open, w, layout)) = &self.pipeline_layout_cache {
            if Arc::ptr_eq(v, &view) && *open == self.pipeline_open && *w == width.to_bits() {
                return Some(layout.clone());
            }
        }
        let layout = Arc::new(crate::districts::pipeline::layout(&view, width, &self.pipeline_open));
        self.pipeline_layout_cache = Some((view, self.pipeline_open.clone(), width.to_bits(), layout.clone()));
        Some(layout)
    }

    /// Lays the districts out again around the same map, after a district's content changed size
    /// (its view arrived, a group opened). Run then, never per frame. The map and everything
    /// anchored on it stay where they are. On Pipeline the camera keeps the district's top edge
    /// in its place on screen, so its header and the row just clicked do not move; a flight to a
    /// district that moved is sent on to where it is now.
    pub fn refresh_world(&mut self) {
        let code = self.world.code;
        let extents = self.district_extents(WorldLayout::centre_width_local(code));
        let before = std::mem::replace(&mut self.world, WorldLayout::compute(code, &extents));
        if self.world == before {
            return;
        }
        if self.camera.is_flying() {
            self.retarget_flight(&before);
        } else if self.stop == Stop::Pipeline {
            // `screen = world * zoom + pan`: the top edge stays where it was on screen.
            self.transform.pan.y -= (self.world.pipeline.min.y - before.pipeline.min.y) * self.transform.zoom;
        }
    }

    /// Sends a flight under way to a district that moved (`before` is the old world) on to where
    /// the district is now. The flight starts again from where the camera is.
    fn retarget_flight(&mut self, before: &WorldLayout) {
        if self.camera.flight.is_none() {
            return;
        }
        if let Some(CameraTarget::Stop(stop)) = self.flight_target {
            if before.rect(stop) != self.world.rect(stop) || before.scale != self.world.scale {
                self.camera.fly_to(CameraTarget::Stop(stop));
            }
        }
    }

    /// Whether wires of this kind (`EdgeKind as u32`) are shown.
    pub fn wire_kind_visible(&self, kind: u32) -> bool {
        self.wire_kinds.mask() & (1 << kind) != 0
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
}
