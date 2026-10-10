use egui::{Response, Sense, Ui};
use studio_graph::Graph;

use crate::camera::Stop;

pub mod actions;
pub mod gate;
pub mod hud;
pub mod input;
pub mod layout;
pub mod render_nodes;
pub mod render_wires;
pub mod types;

#[cfg(test)]
mod tests;

pub use actions::*;
pub use gate::*;
pub use hud::*;
pub use input::*;
pub use layout::*;
pub use render_nodes::*;
pub use render_wires::*;
pub use types::*;

/// Main Canvas View widget.
pub struct CanvasView<'a> {
    pub state: &'a mut CanvasState,
    pub graph: &'a mut Graph,
}

impl<'a> CanvasView<'a> {
    pub fn new(state: &'a mut CanvasState, graph: &'a mut Graph) -> Self {
        Self { state, graph }
    }

    pub fn show(mut self, ui: &mut Ui) -> Response {
        // Clicks and drags, but never keyboard focus: keys reach the canvas only while no widget
        // has focus (see `InputGate`).
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::CLICK | Sense::DRAG);
        let painter = ui.painter_at(rect);
        let anim_time = ui.ctx().input(|i| i.time);
        let gate = InputGate::new(ui.ctx(), &response);
        self.state.last_rect = rect;

        // 1. Process Input, then move the camera
        handle_canvas_input(self.state, self.graph, ui, &gate, rect);
        self.state.refresh_trace(self.graph);
        let ppp = ui.ctx().pixels_per_point();
        self.state.tick_camera(self.graph, rect, anim_time, ppp);

        // 2. Render Canvas Layers
        let visible_world_rect = self.state.transform.screen_to_world_rect(rect);
        let zoom = self.state.transform.zoom;
        let frame =
            self.state.use_gpu_wires.then(|| prepare_frame(self.state, self.graph, visible_world_rect, anim_time));
        // From far away the GPU card layer draws every card; egui draws only cards close enough
        // to read. With no GPU, cards under half a pixel (< 0.005) are skipped and the folders
        // give the overview.
        let gpu_cards = frame.as_ref().and_then(|f| f.cards.as_ref().map(|c| c.boxes.len()));
        let visible_node_ids = if gpu_cards.is_some() || zoom < 0.005 {
            Vec::new()
        } else {
            self.state.spatial_grid.query_rect(visible_world_rect)
        };

        // Layer A: the dot grid, then the districts around the map
        crate::grid::paint_infinite_grid(&painter, rect, &self.state.transform);
        let world = self.state.world;
        let districts = self.state.districts.clone();
        crate::world::paint::paint_frames(&painter, &world, &self.state.transform, rect, &|stop| match stop {
            Stop::Code | Stop::Desk => None,
            Stop::Run if districts.run.is_some() => None,
            Stop::Run => Some("Reading the project's tools…".to_string()),
            Stop::Files if districts.files.is_some() => None,
            Stop::Files => Some("Reading the disk…".to_string()),
            Stop::Changes if districts.changes.is_some() => None,
            Stop::Changes => Some("Reading git…".to_string()),
            _ => Some("Nothing read here yet".to_string()),
        });
        if let Some(files) = &districts.files {
            let rows = self.state.files_tree(self.graph);
            let action = crate::districts::files::paint_files(
                &painter,
                &world,
                &self.state.transform,
                rect,
                files,
                &rows,
                &gate,
            );
            match action {
                Some(crate::districts::files::FilesAction::Toggle(id)) => {
                    let opened = self.state.files_open.insert(id.clone());
                    if !opened {
                        self.state.files_open.remove(&id);
                    }
                }
                Some(crate::districts::files::FilesAction::Open(file, line)) => {
                    self.state.desk.open(&file, line);
                    self.state.fly_to(crate::camera::CameraTarget::Stop(Stop::Desk));
                }
                Some(crate::districts::files::FilesAction::OpenNode(id)) => {
                    self.state.action_request = Some(CanvasAction::OpenOnDesk(id));
                }
                None => {}
            }
        }
        if let Some(run) = &districts.run {
            let selected = self.state.run_selected;
            let action =
                crate::districts::run::paint_run(&painter, &world, &self.state.transform, rect, run, selected, &gate);
            match action {
                Some(crate::districts::run::RunAction::Select(i)) => self.state.run_selected = i,
                Some(crate::districts::run::RunAction::Open(file, line)) => {
                    self.state.desk.open(&file, Some(line));
                    self.state.fly_to(crate::camera::CameraTarget::Stop(Stop::Desk));
                }
                None => {}
            }
        }
        if let Some(changes) = &districts.changes {
            use crate::districts::changes::{self as cd, ChangesAction};
            let selection = cd::ChangesSelection {
                lit_session: self.state.session_lit.clone(),
                ..self.state.changes_selected.clone()
            };
            // A click that starts a flight here (from another district) is not also a click on the
            // district's contents.
            let gate = if self.state.camera.is_flying() { InputGate { clicked: false, ..gate } } else { gate };
            let action = cd::paint_changes(&painter, &world, &self.state.transform, rect, changes, &selection, &gate);
            match action {
                Some(ChangesAction::SelectRow(row)) => {
                    self.state.changes_selected = cd::ChangesSelection { row: Some(row), ..Default::default() };
                }
                Some(ChangesAction::SelectCommit(row, bead)) => {
                    self.state.changes_selected =
                        cd::ChangesSelection { row: Some(row), commit: Some((row, bead)), ..Default::default() };
                    // The commit's files are read once, on the first pick.
                    let bead = changes.rows.get(row).and_then(|r| r.commits.get(bead));
                    if let Some(b) = bead.filter(|b| b.files.is_none()) {
                        self.state.action_request = Some(CanvasAction::LoadCommitFiles(b.id.clone()));
                    }
                }
                Some(ChangesAction::LoadCommitFiles(id)) => {
                    self.state.action_request = Some(CanvasAction::LoadCommitFiles(id));
                }
                Some(ChangesAction::LightSession(id)) => {
                    self.state.action_request = Some(CanvasAction::LightSession(id));
                }
                Some(ChangesAction::OpenSessionPlan(id)) => {
                    self.state.action_request = Some(CanvasAction::OpenSessionPlan(id));
                }
                Some(ChangesAction::OpenFile(file)) => {
                    self.state.desk.open(&file, None);
                    self.state.fly_to(crate::camera::CameraTarget::Stop(Stop::Desk));
                }
                Some(ChangesAction::FlyToRow(row)) => {
                    let target = cd::row_world_rect(&world, changes, row);
                    self.state.fly_to(crate::camera::CameraTarget::Rect(target));
                }
                None => {}
            }
        }

        // Layer A-2, B: Clusters, Wires
        let mut interactive_rects = Vec::new();
        let (folder_detail, drawn_wires) = render_background_and_wires(
            &painter,
            self.state,
            self.graph,
            rect,
            visible_world_rect,
            &gate,
            &mut interactive_rects,
            frame.as_ref(),
        );
        if let (Some(f), Some(_)) = (&frame, gpu_cards) {
            painter.add(
                crate::gpu::CanvasPaint { layer: crate::gpu::CanvasLayer::Cards, frame: f.clone() }
                    .into_paint_callback(rect),
            );
        }

        self.state.frame_stats =
            CanvasFrameStats { visible_nodes: gpu_cards.unwrap_or(visible_node_ids.len()), visible_wires: drawn_wires };

        let mut events = RenderEvents { folder_detail, ..Default::default() };

        // Layer D, E: Node cards & sockets
        render_nodes_and_sockets(
            &painter,
            self.state,
            self.graph,
            &visible_node_ids,
            &gate,
            &mut interactive_rects,
            &mut events,
        );

        // Layer E-2: the veils over the districts the camera is not at and the far labels; then
        // the Desk in its own layer above, and the context menu over everything.
        crate::world::paint::paint_veils(&painter, &world, &self.state.transform, rect, self.state.stop);
        crate::world::paint::paint_far_labels(&painter, &world, &self.state.transform, rect);
        self.state.interactive_rects = interactive_rects;

        let transform = self.state.transform;
        let desk = crate::desk::view::show_desk(
            ui,
            &mut self.state.desk,
            self.graph,
            &world,
            &transform,
            rect,
            self.state.stop,
        );
        self.apply_desk(desk, ui.ctx(), rect);
        render_context_menu(ui.ctx(), self.state, self.graph, &mut events);

        // 3. Apply Render Events & Actions
        apply_render_events(events, self.state, self.graph);
        if let Some(text) = self.state.copy_request.take() {
            ui.ctx().copy_text(text);
        }

        response
    }

    /// Acts on what happened on the Desk, and lets a pinch or Ctrl/⌘ + scroll over its cards zoom
    /// the canvas as it does everywhere else.
    fn apply_desk(&mut self, out: crate::desk::view::DeskOutput, ctx: &egui::Context, rect: egui::Rect) {
        let state = &mut *self.state;
        if let Some(id) = out.open {
            if let Some(path) = self.graph.nodes.get(&id).and_then(|n| n.file_path.clone()) {
                state.desk.open(std::path::Path::new(&path), None);
            }
        }
        if let Some(path) = out.show_on_map {
            let wanted = path.to_string_lossy();
            if let Some(node) = self.graph.nodes.values().find(|n| n.file_path.as_deref() == Some(&*wanted)) {
                state.selected_nodes.clear();
                state.selected_nodes.insert(node.id);
                state.fly_to(crate::camera::CameraTarget::Node(node.id));
            }
        }
        if let Some(folder) = out.load_folder {
            state.action_request = Some(CanvasAction::ExpandFolder(folder, studio_graph::FolderDetail::Open));
        }
        if let Some((from, target)) = out.link {
            if target.starts_with("http://") || target.starts_with("https://") {
                ctx.open_url(egui::OpenUrl::new_tab(target));
            } else {
                let base = from.parent().unwrap_or(std::path::Path::new(""));
                if let Some(link) = crate::desk::parse_link(&target, base, state.desk.root.as_deref()) {
                    // One look at the disk, on a click, so a broken link says so instead of
                    // opening an empty card.
                    if !link.path.is_file() {
                        state.status_message = Some(format!("Not found: {}", link.path.display()));
                    } else if let Some(lines) = link.lines {
                        use crate::desk::{LitKind, LitRange, LINK_SOURCE};
                        state.desk.open_lit(&link.path, vec![LitRange::new(lines, LitKind::Lit, LINK_SOURCE)]);
                    } else {
                        state.desk.open(&link.path, None);
                    }
                }
            }
        }
        // An edit picked on a plan card: its file, with the edit lit.
        if let Some((path, lights)) = out.open_lit {
            state.desk.open_lit(&path, lights);
        }
        let over_desk = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| rect.contains(*p))
            .filter(|&p| ctx.layer_id_at(p).is_some_and(|l| l.id == egui::Id::new(crate::desk::view::DESK_LAYER)));
        if let Some(pointer) = over_desk {
            let zoom = ctx.input(|i| i.zoom_delta());
            if (zoom - 1.0).abs() > 1e-3 {
                state.camera.cancel();
                state.transform.zoom_at_pointer(pointer, zoom);
            }
        }
    }
}
