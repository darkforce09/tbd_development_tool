pub mod activity;
pub mod debug_panel;
pub mod desk_io;
pub mod districts;
pub mod folders;
pub mod modals;
pub mod overlays;
pub mod settings;
pub mod shortcuts;
pub mod sources;
pub mod state;
pub mod title_bar;
pub mod view_menu;
pub mod window_frame;

use eframe::{egui, App, Frame};
use egui::Key;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use studio_canvas::{CameraTarget, CanvasAction, CanvasState, CanvasView, Stop};
use studio_graph::Graph;
use studio_parser::{LoaderMessage, ProjectStats, SymbolSearchIndex};

/// The Studio desktop application: one infinite canvas for the whole project.
pub struct StudioApp {
    pub graph: Graph,
    pub canvas_state: CanvasState,
    pub frame_counter: u64,
    pub last_frame_time: f64,
    pub fps: f32,
    /// GPU adapter rendering the canvas; `None` when wires fall back to the CPU.
    pub gpu: Option<crate::telemetry::GpuDeviceInfo>,
    pub debug: debug_panel::DebugState,

    // Project state
    pub current_project_path: Option<PathBuf>,
    pub path_input: String,
    pub project_stats: Option<ProjectStats>,
    pub is_from_cache: bool,
    /// Why the last project load failed, shown on the project pill until the next load.
    pub load_error: Option<String>,

    // Non-blocking Asynchronous Background Project Loader
    pub loader_rx: Option<Receiver<LoaderMessage>>,
    pub is_loading: bool,
    pub loading_stage: String,
    pub loading_files_done: usize,
    pub loading_total_files: usize,
    pub loading_progress: f32,

    pub search_index: SymbolSearchIndex,

    // Collapsed folders being loaded in the background
    pub folder_loads: Vec<folders::FolderLoad>,

    pub spotlight_open: bool,
    pub spotlight_search: String,

    /// What the title bar's activity line shows.
    pub activity: activity::Activity,
    /// Reads files for the Desk in the background.
    pub desk_reader: desk_io::DeskReader,
    /// The open project's sources besides its code.
    pub sources: Option<sources::ProjectSources>,
    /// For waking the UI from background work.
    pub egui_ctx: egui::Context,
    /// The "Add a tool" panel is open.
    pub show_tool_sources: bool,
}

impl App for StudioApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut Frame) {
        let ctx = &root.ctx().clone();
        ctx.request_repaint();

        // Drain asynchronous loader messages
        let mut graph_replaced = false;
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
                            crate::timing::event(|| format!("initial layout received ({} files)", stats.file_count));
                            self.graph = graph;
                            graph_replaced = true;
                            // A project opens at its top level, with nothing selected.
                            self.canvas_state.focus = None;
                            self.canvas_state.selected_nodes.clear();
                            self.project_stats = Some(stats.clone());
                            self.search_index = search_index;
                            self.canvas_state.mark_scene_dirty();
                            self.canvas_state.jump_to(CameraTarget::Stop(Stop::World));
                            self.is_loading = false;
                            self.canvas_state.status_message = Some(format!(
                                "Opened '{}': {} files displayed, parsing in the background",
                                stats.project_name, stats.file_count
                            ));
                        }
                        LoaderMessage::Complete { graph, stats, search_index, from_cache } => {
                            crate::timing::event(|| {
                                let how = if from_cache { "from cache" } else { "parsed" };
                                format!("parse complete ({} files, {how})", stats.file_count)
                            });
                            self.graph = graph;
                            graph_replaced = true;
                            self.project_stats = Some(stats.clone());
                            self.search_index = search_index;
                            self.is_from_cache = from_cache;
                            self.canvas_state.mark_scene_dirty();
                            if self.is_loading {
                                self.canvas_state.jump_to(CameraTarget::Stop(Stop::World));
                            }
                            self.canvas_state.status_message = Some(format!("Loaded '{}'", stats.project_name));
                            self.is_loading = false;
                            if from_cache {
                                should_clear_rx = true;
                                break;
                            }
                        }
                        LoaderMessage::FolderTotals(totals) => {
                            studio_parser::apply_folder_totals(&mut self.graph, &totals);
                            should_clear_rx = true;
                            break;
                        }
                        LoaderMessage::Error(err) => {
                            self.load_error = Some(err);
                            self.is_loading = false;
                            should_clear_rx = true;
                            break;
                        }
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        should_clear_rx = true;
                        if self.is_loading {
                            self.load_error = Some("Loader thread disconnected unexpectedly.".to_string());
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

        if graph_replaced {
            self.apply_sources();
        }
        self.poll_folder_loads();
        self.poll_desk_reads(ctx);
        self.poll_sources();

        // FPS calculation
        let now = ctx.input(|i| i.time);
        self.frame_counter += 1;
        let delta = (now - self.last_frame_time) as f32;
        if delta >= 0.5 {
            self.fps = (self.frame_counter as f32 / delta).round();
            self.frame_counter = 0;
            self.last_frame_time = now;
        }

        self.debug.push_frame_time(ctx.input(|i| i.unstable_dt) * 1000.0);

        // Global Keyboard Shortcuts
        if ctx.input(|i| i.key_pressed(Key::F3)) {
            self.debug.open = !self.debug.open;
        }
        if shortcuts::search_shortcut(ctx) {
            self.spotlight_open = !self.spotlight_open;
            if self.spotlight_open {
                self.spotlight_search.clear();
            }
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
                        node.expanded_member_id = Some(member_id);
                        if node.is_code_expanded {
                            node.expanded_tab = 2;
                        }
                    }
                }
                CanvasAction::ExpandFolder(cluster_id, detail) => self.start_folder_load(cluster_id, detail),
                CanvasAction::ToggleMemberWires(id) => {
                    if let Some(n) = self.graph.nodes.get_mut(&id) {
                        n.show_member_wires = !n.show_member_wires;
                    }
                }
                CanvasAction::LoadCommitFiles(id) => self.load_commit_files(id),
                CanvasAction::OpenSessionPlan(id) => self.open_session_plan(id),
                // B3 lights the session's files from this.
                CanvasAction::LightSession(id) => self.canvas_state.session_lit = id,
                other => {
                    self.canvas_state.action_request = Some(other);
                }
            }
        }

        self.render_title_bar(root);

        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(studio_ui::color_tokens::CANVAS_BG)).show(
            root,
            |ui| {
                let started = std::time::Instant::now();
                CanvasView::new(&mut self.canvas_state, &mut self.graph).show(ui);
                self.debug.canvas_ms = started.elapsed().as_secs_f32() * 1000.0;
            },
        );

        self.render_compass(ctx);
        self.render_dock(ctx);
        self.render_empty_state(ctx);
        self.render_spotlight_modal(ctx);
        self.render_debug_panel(ctx);
        // macOS keeps its native window frame, which resizes itself.
        if !cfg!(target_os = "macos") {
            window_frame::resize_edges(ctx);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let settings = settings::PersistedSettings { last_project: self.current_project_path.clone() };
        eframe::set_value(storage, eframe::APP_KEY, &settings);
    }
}
