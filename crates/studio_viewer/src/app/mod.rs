pub mod editor;
pub mod folders;
pub mod left_sidebar;
pub mod modals;
pub mod right_inspector;
pub mod settings;
pub mod state;
pub mod top_nav;
pub mod types;

use eframe::{egui, App, Frame};
use egui::{Key, Pos2};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use studio_canvas::{CanvasAction, CanvasState, CanvasView};
use studio_graph::{Graph, NodeArchetype, NodeId};
use studio_parser::{LoaderMessage, ProjectStats, SymbolSearchIndex, ViewGranularity};

#[allow(unused_imports)]
pub use types::{FlowDefinition, FlowVariant, StudioViewMode};

/// The primary Studio Viewer desktop application combining CodeSee, Enso, Haystack, and Code Canvas.
pub struct StudioApp {
    pub graph: Graph,
    pub canvas_state: CanvasState,
    pub frame_counter: u64,
    pub last_frame_time: f64,
    pub fps: f32,
    /// Name of the GPU adapter rendering the canvas, or why there is none.
    pub gpu_label: String,

    // Project state
    pub current_project_path: Option<PathBuf>,
    pub path_input: String,
    pub granularity: ViewGranularity,
    pub project_stats: Option<ProjectStats>,
    pub pending_fit_view: bool,
    pub is_from_cache: bool,

    // Non-blocking Asynchronous Background Project Loader
    pub loader_rx: Option<Receiver<LoaderMessage>>,
    pub is_loading: bool,
    pub loading_stage: String,
    pub loading_files_done: usize,
    pub loading_total_files: usize,
    pub loading_progress: f32,

    // High-performance DDR5 In-Memory Symbol Index
    pub search_index: SymbolSearchIndex,

    // CodeSee View Modes & Panels
    pub view_mode: StudioViewMode,
    pub left_sidebar_open: bool,
    pub right_inspector_open: bool,

    // Search & Filter State
    pub search_query: String,
    pub category_filters: BTreeMap<NodeArchetype, bool>,

    // Code Canvas Live Code Editor State
    pub code_editor_expanded: bool,
    pub code_editor_buffer: String,
    pub code_editor_node_id: Option<NodeId>,
    pub code_editor_member_id: Option<String>,
    pub code_editor_dirty: bool,
    pub code_editor_status: Option<String>,
    /// File the buffer is saved to; `None` for in-memory (mock) nodes.
    pub code_editor_path: Option<PathBuf>,
    /// Which part of `code_editor_path` the buffer holds.
    pub code_editor_origin: Option<studio_parser::EditOrigin>,
    pub pending_editor_switch: Option<editor::PendingEditorSwitch>,

    // Collapsed folders being loaded in the background
    pub folder_loads: Vec<folders::FolderLoad>,
    pub markdown_preview_mode: bool,
    pub markdown_inspector_tab: usize,

    // Enso Spotlight Palette
    pub spotlight_open: bool,
    pub spotlight_search: String,

    // CodeSee Flows & Traces
    pub flows: Vec<FlowDefinition>,
    pub active_flow_index: usize,
    pub active_variant_index: usize,
}

impl App for StudioApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut Frame) {
        let ctx = &root.ctx().clone();
        ctx.request_repaint();

        // Drain asynchronous loader messages
        if let Some(ref rx) = self.loader_rx {
            let mut should_clear_rx = false;
            loop {
                match rx.try_recv() {
                    Ok(msg) => match msg {
                        LoaderMessage::Progress { stage, files_done, total_files, percentage } => {
                            self.loading_stage = stage.clone();
                            self.loading_files_done = files_done;
                            self.loading_total_files = total_files;
                            self.loading_progress = percentage;
                            if !self.is_loading {
                                let detail = if total_files > 0 {
                                    format!(
                                        "{} ({}/{} files, {:.0}%)",
                                        stage,
                                        files_done,
                                        total_files,
                                        percentage * 100.0
                                    )
                                } else {
                                    format!("{} ({:.0}%)", stage, percentage * 100.0)
                                };
                                self.canvas_state.status_message = Some(detail);
                            }
                        }
                        LoaderMessage::InitialLayoutReady { graph, stats, search_index } => {
                            self.graph = graph;
                            self.project_stats = Some(stats.clone());
                            self.search_index = search_index;
                            self.canvas_state.spatial_grid.build_from_graph(&self.graph);
                            self.canvas_state.spatial_grid_dirty = false;
                            self.pending_fit_view = true;
                            self.is_loading = false;
                            self.canvas_state.status_message = Some(format!(
                                "Opened '{}' instantly: {} files displayed (enriching source AST & wires in background...)",
                                stats.project_name, stats.file_count
                            ));
                        }
                        LoaderMessage::Complete { graph, stats, search_index, from_cache } => {
                            self.graph = graph;
                            self.project_stats = Some(stats.clone());
                            self.search_index = search_index;
                            self.is_from_cache = from_cache;
                            self.canvas_state.spatial_grid.build_from_graph(&self.graph);
                            self.canvas_state.spatial_grid_dirty = false;
                            if self.is_loading {
                                self.pending_fit_view = true;
                            }
                            self.build_project_flows(&stats);
                            let cache_tag = if from_cache { " [rkyv user cache]" } else { "" };
                            self.canvas_state.status_message = Some(format!(
                                "Loaded '{}'{}: {} nodes, {} wires across {} files",
                                stats.project_name, cache_tag, stats.node_count, stats.wire_count, stats.file_count
                            ));
                            self.is_loading = false;
                            should_clear_rx = true;
                            break;
                        }
                        LoaderMessage::Error(err) => {
                            self.canvas_state.status_message = Some(format!("Error: {}", err));
                            self.is_loading = false;
                            should_clear_rx = true;
                            break;
                        }
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        should_clear_rx = true;
                        if self.is_loading {
                            self.canvas_state.status_message =
                                Some("Loader thread disconnected unexpectedly.".to_string());
                            self.is_loading = false;
                        }
                        break;
                    }
                }
            }
            if should_clear_rx {
                self.loader_rx = None;
            }
        }

        self.poll_folder_loads();

        // FPS calculation
        let now = ctx.input(|i| i.time);
        self.frame_counter += 1;
        let delta = (now - self.last_frame_time) as f32;
        if delta >= 0.5 {
            self.fps = (self.frame_counter as f32 / delta).round();
            self.frame_counter = 0;
            self.last_frame_time = now;
        }

        // Global Keyboard Shortcuts
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::K)) || ctx.input(|i| i.key_pressed(Key::Slash)) {
            self.spotlight_open = !self.spotlight_open;
            if self.spotlight_open {
                self.spotlight_search.clear();
            }
        }

        // Save shortcut in editor
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::S)) {
            self.save_current_editor_code();
        }

        // Handle canvas external actions
        if let Some(action) = self.canvas_state.action_request.take() {
            match action {
                CanvasAction::InspectNode(id) => {
                    self.canvas_state.selected_nodes.clear();
                    self.canvas_state.selected_nodes.insert(id);
                }
                CanvasAction::InspectMember(node_id, member_id) => {
                    self.canvas_state.selected_nodes.clear();
                    self.canvas_state.selected_nodes.insert(node_id);
                    if let Some(node) = self.graph.nodes.get_mut(&node_id) {
                        node.expanded_member_id = Some(member_id.clone());
                        if node.is_code_expanded {
                            node.expanded_tab = 2;
                        }
                    }
                    self.request_open_member(node_id, member_id);
                }
                CanvasAction::CenterNode(id) => {
                    if let Some(n) = self.graph.nodes.get(&id) {
                        let center_world = Pos2::new(n.position[0] + n.size[0] * 0.5, n.position[1] + n.size[1] * 0.5);
                        self.canvas_state.transform.center_on_world_pos(center_world, ctx.content_rect(), None);
                    }
                }
                CanvasAction::ExpandFolder(cluster_id) => self.start_folder_load(cluster_id),
                CanvasAction::ToggleMemberWires(id) => {
                    if let Some(n) = self.graph.nodes.get_mut(&id) {
                        n.show_member_wires = !n.show_member_wires;
                    }
                }
                other => {
                    self.canvas_state.action_request = Some(other);
                }
            }
        }

        // Synchronize search filter to canvas
        self.canvas_state.search_filter = self.search_query.clone();
        self.sync_category_filters();

        // 1. Top Navigation Bar
        self.render_top_nav(root);

        // 2. Bottom Status Strip
        self.render_bottom_panel(root);

        // 3. Left Sidebar
        self.render_left_sidebar(root);

        // 4. Right Inspector
        self.sync_editor_with_selection();
        self.render_right_inspector(root);

        // 5. Central Infinite Canvas Viewport
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(studio_ui::color_tokens::CANVAS_BG)).show(
            root,
            |ui| {
                if self.pending_fit_view {
                    let screen_rect = ui.ctx().content_rect();
                    self.canvas_state.zoom_to_fit(&self.graph, screen_rect);
                    self.pending_fit_view = false;
                }

                CanvasView::new(&mut self.canvas_state, &mut self.graph).show(ui);
            },
        );

        // 6. Non-blocking Async Loading HUD
        self.render_loading_hud(ctx);

        // 7. Enso Spotlight / Command Palette Modal
        self.render_spotlight_modal(ctx);

        // 8. Unsaved editor changes prompt
        self.render_unsaved_changes_modal(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = settings::PersistedSettings {
            last_project: self.current_project_path.clone(),
            granularity: Some(settings::granularity_key(self.granularity).to_string()),
            left_sidebar_open: Some(self.left_sidebar_open),
            right_inspector_open: Some(self.right_inspector_open),
        };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }
}
