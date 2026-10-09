use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use studio_canvas::CanvasState;
use studio_graph::{create_showcase_files_graph, Graph, NodeArchetype};
use studio_parser::{spawn_load_project_opt, SymbolSearchIndex, ViewGranularity};
use studio_ui::apply_theme;

use super::settings::PersistedSettings;
use super::StudioApp;

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<std::path::PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

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

        let saved: PersistedSettings =
            cc.storage.and_then(|storage| eframe::get_value(storage, eframe::APP_KEY)).unwrap_or_default();

        let mut canvas_state = CanvasState::default();
        let gpu_label = cc.wgpu_render_state.as_ref().map_or_else(
            || "CPU wire rendering".to_string(),
            |rs| crate::telemetry::GpuDeviceInfo::from_adapter_info(&rs.adapter.get_info()).name,
        );
        if let Some(render_state) = &cc.wgpu_render_state {
            let pipeline = studio_canvas::GpuWirePipeline::new(&render_state.device, render_state.target_format);
            render_state.renderer.write().callback_resources.insert(pipeline);
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
            project_stats: None,
            pending_fit_view: false,
            is_from_cache: false,
            load_error: None,
            loader_rx: None,
            is_loading: false,
            loading_stage: String::new(),
            loading_files_done: 0,
            loading_total_files: 0,
            loading_progress: 0.0,
            search_index: SymbolSearchIndex::default(),
            left_sidebar_open: false,
            search_query: String::new(),
            category_filters,
            folder_loads: Vec::new(),
            spotlight_open: false,
            spotlight_search: String::new(),
        };

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
        self.load_error = None;
        self.loader_rx = None;
        self.graph = create_showcase_files_graph();
        self.search_index = SymbolSearchIndex::build(&self.graph);
        self.canvas_state.spatial_grid.build_from_graph(&self.graph);
        self.canvas_state.spatial_grid_dirty = false;
        self.pending_fit_view = true;
        self.canvas_state.status_message = Some("Showcase loaded".to_string());
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
        self.load_error = None;
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
        spawn_load_project_opt(path_buf, ViewGranularity::FilesAndFolders, force_reparse, tx);
        self.loader_rx = Some(rx);
    }

    pub fn force_reparse_current_project(&mut self) {
        if let Some(path) = self.current_project_path.clone() {
            self.load_project_opt(&path, true);
        } else {
            self.load_showcase();
        }
    }

    pub(crate) fn sync_category_filters(&mut self) {
        let active: BTreeSet<NodeArchetype> =
            self.category_filters.iter().filter(|(_, &enabled)| enabled).map(|(&arch, _)| arch).collect();
        self.canvas_state.category_filter = active;
    }
}
