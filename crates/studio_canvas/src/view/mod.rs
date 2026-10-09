use egui::{Response, Sense, Ui};
use studio_graph::Graph;

pub mod actions;
pub mod hud;
pub mod input;
pub mod layout;
pub mod render_nodes;
pub mod render_wires;
pub mod types;

#[cfg(test)]
mod tests;

pub use actions::*;
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

    pub fn show(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let anim_time = ui.ctx().input(|i| i.time);
        let pointer_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
        let pointer_clicked = ui.input(|i| i.pointer.primary_clicked());

        // 1. Process Input
        handle_canvas_input(self.state, self.graph, ui, rect);

        // 2. Render Canvas Layers
        let visible_world_rect = self.state.transform.screen_to_world_rect(rect);
        // At galaxy zoom (< 0.005), individual file cards are < 0.44px (completely sub-pixel).
        // Pruning them avoids querying and iterating hundreds of thousands of invisible items,
        // while the parent folder clusters (rendered in Layer A-2) provide the galactic overview.
        let visible_node_ids = if self.state.transform.zoom < 0.005 {
            Vec::new()
        } else {
            self.state.spatial_grid.query_rect(visible_world_rect)
        };

        // Layer A, A-2, B, C: Background, Clusters, Wires, Pending wire
        let toggle_cluster_id = render_background_and_wires(
            &painter,
            self.state,
            self.graph,
            rect,
            visible_world_rect,
            pointer_pos,
            pointer_clicked,
            anim_time,
        );

        let mut events = RenderEvents { toggle_cluster_id, ..Default::default() };

        // Layer D, E: Node cards & sockets
        let mut interactive_rects = Vec::new();
        let single_selected_rect = render_nodes_and_sockets(
            &painter,
            self.state,
            self.graph,
            &visible_node_ids,
            pointer_pos,
            pointer_clicked,
            &mut interactive_rects,
            &mut events,
        );

        // Layer E-2, F: Floating Toolbar & Zoom HUD
        render_hud(
            &painter,
            self.state,
            self.graph,
            rect,
            single_selected_rect,
            pointer_pos,
            pointer_clicked,
            &mut interactive_rects,
            &mut events,
        );

        self.state.interactive_rects = interactive_rects;

        // 3. Apply Render Events & Actions
        apply_render_events(events, self.state, self.graph, rect);

        response
    }
}
