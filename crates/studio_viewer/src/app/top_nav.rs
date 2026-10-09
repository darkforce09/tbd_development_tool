use eframe::egui;
use egui::{FontFamily, FontId, PopupCloseBehavior, RichText, Stroke};
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_top_nav(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("studio_top_nav")
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(14, 7))
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let spot_label = format!("{} Spotlight / Add (Ctrl+K)", egui_phosphor::regular::MAGNIFYING_GLASS);
                    if ui.button(spot_label).clicked() {
                        self.spotlight_open = true;
                    }

                    ui.separator();

                    let open_label = format!("{} Open...", egui_phosphor::regular::FOLDER_OPEN);
                    if ui.button(open_label).clicked() {
                        self.open_folder_dialog();
                    }

                    ui.separator();

                    // Global Sub-node Wires toggle button
                    let wires_btn_text = if self.canvas_state.show_subnode_wires_globally {
                        format!("{} Sub-node Wires: ON", egui_phosphor::regular::PLUGS_CONNECTED)
                    } else {
                        format!("{} Sub-node Wires: OFF", egui_phosphor::regular::PLUGS)
                    };
                    if ui.selectable_label(self.canvas_state.show_subnode_wires_globally, wires_btn_text)
                        .on_hover_text("Toggle visibility of dependency & call connection wires between internal file sub-nodes across the entire canvas")
                        .clicked()
                    {
                        self.canvas_state.show_subnode_wires_globally = !self.canvas_state.show_subnode_wires_globally;
                    }

                    // Right-aligned sidebar toggle, project pill and FPS
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let fps_color = if self.fps >= 55.0 { ARCHETYPE_EGRESS } else { ARCHETYPE_STATE };
                        ui.label(
                            RichText::new(format!("{:.0} FPS", self.fps))
                                .font(FontId::new(10.5, FontFamily::Monospace))
                                .color(fps_color),
                        );

                        ui.separator();

                        let left_label = format!("{} Filters", egui_phosphor::regular::FUNNEL);
                        if ui.selectable_label(self.left_sidebar_open, left_label).clicked() {
                            self.left_sidebar_open = !self.left_sidebar_open;
                        }

                        ui.separator();

                        self.render_project_pill(ui);
                    });
                });
            });
    }

    /// Project name button; clicking it opens the project stats popover.
    fn render_project_pill(&mut self, ui: &mut egui::Ui) {
        let (icon, color) = if self.load_error.is_some() {
            (egui_phosphor::regular::WARNING, ARCHETYPE_STATE)
        } else {
            (egui_phosphor::regular::PACKAGE, TEXT_SECONDARY)
        };
        let name = self.project_stats.as_ref().map_or("No project", |s| s.project_name.as_str());
        let pill = ui.button(
            RichText::new(format!("{icon} {name} {}", egui_phosphor::regular::CARET_DOWN))
                .font(FontId::new(11.0, FontFamily::Proportional))
                .color(color),
        );

        egui::Popup::from_toggle_button_response(&pill).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(
            |ui| {
                ui.set_min_width(240.0);
                self.render_project_stats(ui);
            },
        );
    }

    fn render_project_stats(&self, ui: &mut egui::Ui) {
        let row = |ui: &mut egui::Ui, label: &str, value: String| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(value).font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_PRIMARY));
                });
            });
        };

        if let Some(err) = &self.load_error {
            ui.label(
                RichText::new(format!("{} Load failed", egui_phosphor::regular::WARNING))
                    .font(FontId::new(12.0, FontFamily::Proportional))
                    .color(ARCHETYPE_STATE)
                    .strong(),
            );
            ui.label(RichText::new(err).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_SECONDARY));
            ui.separator();
        }

        let Some(stats) = &self.project_stats else {
            ui.label(RichText::new("No project open").color(TEXT_SECONDARY));
            return;
        };
        ui.label(
            RichText::new(&stats.project_name)
                .font(FontId::new(12.5, FontFamily::Proportional))
                .color(TEXT_HIGHLIGHT)
                .strong(),
        );
        if let Some(path) = &self.current_project_path {
            ui.label(
                RichText::new(path.display().to_string())
                    .font(FontId::new(10.0, FontFamily::Monospace))
                    .color(TEXT_DIM),
            );
        }
        ui.add_space(4.0);
        row(ui, "Files", stats.file_count.to_string());
        row(ui, "Crates", stats.crate_count.to_string());
        row(ui, "Nodes", self.graph.nodes.len().to_string());
        row(ui, "Wires", self.graph.edges.len().to_string());
        row(ui, "Folders", self.graph.clusters.len().to_string());
        row(ui, "Nodes without wires", self.graph.isolated_nodes_count().to_string());
        row(ui, "Loaded from cache", if self.is_from_cache { "yes" } else { "no" }.to_string());

        if let Some(msg) = &self.canvas_state.status_message {
            ui.separator();
            ui.label(RichText::new(msg).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
        }
    }
}
