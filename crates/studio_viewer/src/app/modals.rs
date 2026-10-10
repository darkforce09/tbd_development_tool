//! The prompt on the empty canvas when no project is open.

use eframe::egui;
use egui::{FontFamily, FontId, RichText};
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    /// Centered prompt on the empty canvas when no project is open.
    pub(crate) fn render_empty_state(&mut self, ctx: &egui::Context) {
        if self.current_project_path.is_some() || self.is_loading {
            return;
        }

        egui::Area::new(egui::Id::new("empty_state")).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(egui_phosphor::regular::FOLDER_OPEN)
                        .font(FontId::new(40.0, FontFamily::Proportional))
                        .color(TEXT_DIM),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("No project open")
                        .font(FontId::new(16.0, FontFamily::Proportional))
                        .color(TEXT_PRIMARY)
                        .strong(),
                );
                ui.add_space(12.0);
                let open_label = format!("{} Open folder...", egui_phosphor::regular::FOLDER_OPEN);
                if ui.button(RichText::new(open_label).font(FontId::new(13.0, FontFamily::Proportional))).clicked() {
                    self.open_folder_dialog();
                }
            });
        });
    }
}
