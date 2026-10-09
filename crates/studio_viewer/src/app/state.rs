use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use studio_canvas::CanvasState;
use studio_graph::{
    create_showcase_files_graph, create_showcase_graph, EdgeId, Graph, NodeArchetype,
};
use studio_parser::{
    spawn_load_project_opt, ProjectStats, SymbolSearchIndex, ViewGranularity,
};
use studio_ui::apply_theme;

use super::settings::{granularity_from_key, PersistedSettings};
use super::types::{FlowDefinition, FlowVariant, StudioViewMode};
use super::StudioApp;

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);

        let mut category_filters = BTreeMap::new();
        category_filters.insert(NodeArchetype::File, true);
        category_filters.insert(NodeArchetype::Function, true);
        category_filters.insert(NodeArchetype::Struct, true);
        category_filters.insert(NodeArchetype::Enum, true);
        category_filters.insert(NodeArchetype::Trait, true);
        category_filters.insert(NodeArchetype::Module, true);
        category_filters.insert(NodeArchetype::Ingress, true);
        category_filters.insert(NodeArchetype::Compute, true);
        category_filters.insert(NodeArchetype::State, true);
        category_filters.insert(NodeArchetype::Egress, true);

        let saved: PersistedSettings = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY))
            .unwrap_or_default();

        let mut canvas_state = CanvasState::default();
        let gpu_label = cc.wgpu_render_state.as_ref().map_or_else(
            || "CPU wire rendering".to_string(),
            |rs| crate::telemetry::GpuDeviceInfo::from_adapter_info(&rs.adapter.get_info()).name,
        );
        if let Some(render_state) = &cc.wgpu_render_state {
            let pipeline = studio_canvas::GpuWirePipeline::new(
                &render_state.device,
                render_state.target_format,
            );
            render_state
                .renderer
                .write()
                .callback_resources
                .insert(pipeline);
            canvas_state.use_gpu_wires = true;
        } else {
            canvas_state.use_gpu_wires = false;
        }

        let mut app = Self {
            graph: Graph::new(),
            canvas_state,
            frame_counter: 0,
            last_frame_time: 0.0,
            fps: 60.0,
            gpu_label,
            current_project_path: None,
            path_input: String::new(),
            granularity: ViewGranularity::FilesAndFolders,
            project_stats: None,
            pending_fit_view: false,
            is_from_cache: false,
            loader_rx: None,
            is_loading: false,
            loading_stage: String::new(),
            loading_files_done: 0,
            loading_total_files: 0,
            loading_progress: 0.0,
            search_index: SymbolSearchIndex::default(),
            view_mode: StudioViewMode::CodebaseMaps,
            left_sidebar_open: true,
            right_inspector_open: false,
            search_query: String::new(),
            category_filters,
            code_editor_expanded: false,
            code_editor_buffer: String::new(),
            code_editor_node_id: None,
            code_editor_member_id: None,
            code_editor_dirty: false,
            code_editor_status: None,
            code_editor_path: None,
            code_editor_origin: None,
            pending_editor_switch: None,
            markdown_preview_mode: true,
            markdown_inspector_tab: 0,
            spotlight_open: false,
            spotlight_search: String::new(),
            flows: Vec::new(),
            active_flow_index: 0,
            active_variant_index: 0,
        };

        if let Some(g) = saved.granularity.as_deref().and_then(granularity_from_key) {
            app.granularity = g;
        }
        app.left_sidebar_open = saved.left_sidebar_open.unwrap_or(app.left_sidebar_open);
        app.right_inspector_open = saved.right_inspector_open.unwrap_or(app.right_inspector_open);

        // Explicit path, else the last project, else a Cargo project in the cwd, else the showcase.
        let last_project = saved.last_project.filter(|p| p.is_dir());
        if let Some(path) = initial_path.or(last_project) {
            app.load_project(&path);
        } else if let Ok(dir) = std::env::current_dir() {
            if dir.join("Cargo.toml").exists() {
                app.load_project(&dir);
            } else {
                app.load_showcase();
            }
        } else {
            app.load_showcase();
        }

        app
    }

    pub fn load_showcase(&mut self) {
        self.current_project_path = None;
        self.project_stats = None;
        self.is_loading = false;
        self.is_from_cache = false;
        self.loader_rx = None;
        self.graph = if self.granularity == ViewGranularity::FilesAndFolders {
            create_showcase_files_graph()
        } else {
            create_showcase_graph()
        };
        self.search_index = SymbolSearchIndex::build(&self.graph);
        self.canvas_state.spatial_grid.build_from_graph(&self.graph);
        self.canvas_state.spatial_grid_dirty = false;
        self.pending_fit_view = true;
        self.build_showcase_flows();
        self.canvas_state.status_message = Some("Showcase architecture pipeline loaded".to_string());
    }

    pub(crate) fn build_showcase_flows(&mut self) {
        let edge_ids: Vec<EdgeId> = self.graph.edges.iter().map(|e| e.id).collect();
        let regular_edges = edge_ids.clone();
        let cached_edges = if edge_ids.len() >= 3 {
            vec![edge_ids[1], edge_ids[2]]
        } else {
            edge_ids.clone()
        };

        self.flows = vec![
            FlowDefinition {
                name: "Realtime Transcribe & Inpaint".to_string(),
                entry_point: "AudioDemuxer::poll_next_packet()".to_string(),
                description: "Full audio ingestion, Whisper speech inference, subtitle rasterization, and DRM composite.".to_string(),
                originators: "Audio Demuxer, Video Ring Buffer".to_string(),
                service_count: 5,
                api_call_count: 4,
                variants: vec![
                    FlowVariant {
                        name: "Regular Pipeline (Full Path)".to_string(),
                        coverage_pct: 68,
                        summary: "100% of pipeline nodes, 4 active stages".to_string(),
                        description: "Standard end-to-end path executed on uncompressed video frames.".to_string(),
                        edge_ids: regular_edges,
                    },
                    FlowVariant {
                        name: "Cached Frame Overlay".to_string(),
                        coverage_pct: 32,
                        summary: "Cached VRAM hit, bypasses demuxer stage".to_string(),
                        description: "Fast-path frame inpainting without audio re-transcription.".to_string(),
                        edge_ids: cached_edges,
                    },
                ],
            },
        ];
        self.active_flow_index = 0;
        self.active_variant_index = 0;
    }

    pub fn open_folder_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Select Project Folder (Code, Markdown, Scripts)");
        if let Some(ref current) = self.current_project_path {
            dialog = dialog.set_directory(current);
        } else if let Ok(current_dir) = std::env::current_dir() {
            dialog = dialog.set_directory(&current_dir);
        }

        if let Some(folder) = dialog.pick_folder() {
            self.load_project(&folder);
        }
    }

    pub fn load_project(&mut self, path: &Path) {
        self.load_project_opt(path, false);
    }

    pub fn load_project_opt(&mut self, path: &Path, force_reparse: bool) {
        let path_buf = path.to_path_buf();
        self.current_project_path = Some(path_buf.clone());
        self.path_input = path_buf.to_string_lossy().to_string();
        self.is_loading = true;
        self.loading_stage = if force_reparse {
            "Force re-parsing workspace from disk...".to_string()
        } else {
            "Checking user rkyv cache & manifests...".to_string()
        };
        self.loading_files_done = 0;
        self.loading_total_files = 0;
        self.loading_progress = 0.05;
        self.canvas_state.status_message = Some(format!("Loading {}...", path.display()));

        let (tx, rx) = std::sync::mpsc::channel();
        spawn_load_project_opt(path_buf, self.granularity, force_reparse, tx);
        self.loader_rx = Some(rx);
    }

    pub fn force_reparse_current_project(&mut self) {
        if let Some(path) = self.current_project_path.clone() {
            self.load_project_opt(&path, true);
        } else {
            self.load_showcase();
        }
    }

    pub(crate) fn build_project_flows(&mut self, stats: &ProjectStats) {
        let edge_ids: Vec<EdgeId> = self.graph.edges.iter().map(|e| e.id).collect();
        let count = edge_ids.len();
        let half = count / 2;

        self.flows = vec![
            FlowDefinition {
                name: format!("{} Ingestion & Parse Flow", stats.project_name),
                entry_point: "studio_parser::load_rust_project".to_string(),
                description: "Scans project directories, extracts syn AST items, and constructs visual architecture graph.".to_string(),
                originators: "CLI / File Dialog".to_string(),
                service_count: stats.crate_count,
                api_call_count: stats.wire_count,
                variants: vec![
                    FlowVariant {
                        name: "All Items Execution".to_string(),
                        coverage_pct: 75,
                        summary: format!("{} active connection wires", count),
                        description: "Complete syntax and reference graph flow.".to_string(),
                        edge_ids: edge_ids.clone(),
                    },
                    FlowVariant {
                        name: "Primary Call Chain".to_string(),
                        coverage_pct: 25,
                        summary: format!("{} core wires", half),
                        description: "High-priority direct execution and constructor path.".to_string(),
                        edge_ids: edge_ids.into_iter().take(half).collect(),
                    },
                ],
            },
        ];
        self.active_flow_index = 0;
        self.active_variant_index = 0;
    }

    pub fn reload_current_project(&mut self) {
        if let Some(path) = self.current_project_path.clone() {
            self.load_project(&path);
        } else {
            self.load_showcase();
        }
    }

    pub fn set_granularity(&mut self, granularity: ViewGranularity) {
        if self.granularity != granularity {
            self.granularity = granularity;
            self.reload_current_project();
        }
    }

    pub fn set_view_mode(&mut self, mode: StudioViewMode) {
        self.view_mode = mode;
        if mode == StudioViewMode::FlowsAndTraces {
            self.sync_active_flow_edges();
        } else {
            self.canvas_state.active_flow_edges = None;
        }
    }

    pub(crate) fn sync_active_flow_edges(&mut self) {
        if let Some(flow) = self.flows.get(self.active_flow_index) {
            if let Some(variant) = flow.variants.get(self.active_variant_index) {
                let set: BTreeSet<EdgeId> = variant.edge_ids.iter().copied().collect();
                self.canvas_state.active_flow_edges = Some(set);
            }
        }
    }

    pub(crate) fn sync_category_filters(&mut self) {
        let active: BTreeSet<NodeArchetype> = self
            .category_filters
            .iter()
            .filter(|(_, &enabled)| enabled)
            .map(|(&arch, _)| arch)
            .collect();
        self.canvas_state.category_filter = active;
    }
}
