use std::path::Path;
use studio_canvas::CanvasState;
use studio_graph::Graph;
use studio_parser::{spawn_load_project, SearchIndex};
use studio_ui::apply_theme;

use super::settings::PersistedSettings;
use super::StudioApp;

impl StudioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_path: Option<std::path::PathBuf>) -> Self {
        apply_theme(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let saved: PersistedSettings =
            cc.storage.and_then(|storage| eframe::get_value(storage, eframe::APP_KEY)).unwrap_or_default();

        let mut canvas_state = CanvasState::default();
        let gpu = cc
            .wgpu_render_state
            .as_ref()
            .map(|rs| crate::telemetry::GpuDeviceInfo::from_adapter_info(&rs.adapter.get_info()));
        if let Some(render_state) = &cc.wgpu_render_state {
            let gpu = studio_canvas::CanvasGpu::new(&render_state.device, render_state.target_format);
            render_state.renderer.write().callback_resources.insert(gpu);
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
            gpu,
            debug: Default::default(),
            current_project_path: None,
            path_input: String::new(),
            project_stats: None,
            is_from_cache: false,
            load_error: None,
            loader_rx: None,
            is_loading: false,
            loading_stage: String::new(),
            loading_files_done: 0,
            loading_total_files: 0,
            loading_progress: 0.0,
            search_index: SearchIndex::default(),
            folder_loads: Vec::new(),
            palette: Default::default(),
            shortcut_sheet_open: false,
            activity: Default::default(),
            desk_reader: Default::default(),
            sources: None,
            egui_ctx: cc.egui_ctx.clone(),
            show_tool_sources: false,
        };

        // Explicit path, else the last project, else a Cargo project in the cwd, else nothing:
        // the canvas then shows the open-folder prompt.
        let last_project = saved.last_project.filter(|p| p.is_dir());
        let cwd_project = std::env::current_dir().ok().filter(|dir| dir.join("Cargo.toml").exists());
        if let Some(path) = initial_path.or(last_project).or(cwd_project) {
            app.load_project(&path);
        }

        app
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
        let path_buf = path.to_path_buf();
        self.current_project_path = Some(path_buf.clone());
        self.canvas_state.desk.clear(path_buf.canonicalize().ok().or_else(|| Some(path_buf.clone())));
        self.canvas_state.session_lit = None;
        self.canvas_state.session_lit_nodes.clear();
        self.canvas_state.changes_selected = Default::default();
        self.path_input = path_buf.to_string_lossy().to_string();
        self.is_loading = true;
        self.load_error = None;
        self.loading_stage = "Checking user rkyv cache & manifests...".to_string();
        self.loading_files_done = 0;
        self.loading_total_files = 0;
        self.loading_progress = 0.05;
        self.canvas_state.status_message = Some(format!("Loading {}...", path.display()));

        self.sources = Some(super::sources::ProjectSources::new(path, &self.egui_ctx));
        let (tx, rx) = std::sync::mpsc::channel();
        spawn_load_project(path_buf, tx);
        self.loader_rx = Some(rx);
    }
}
