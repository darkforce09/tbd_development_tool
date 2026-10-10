use eframe::egui;
use egui::{Color32, CornerRadius, FontFamily, FontId, Key, RichText, Stroke};
use studio_canvas::CanvasAction;
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_spotlight_modal(&mut self, ctx: &egui::Context) {
        if !self.spotlight_open {
            return;
        }

        let spot_title = format!("{} Search", egui_phosphor::regular::MAGNIFYING_GLASS);
        egui::Window::new(spot_title)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0])
            .fixed_size([560.0, 260.0])
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(18, 20, 28))
                    .stroke(Stroke::new(1.5, Color32::from_rgb(99, 102, 241)))
                    .corner_radius(CornerRadius::from(10.0))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS)
                            .font(FontId::new(16.0, FontFamily::Proportional)),
                    );
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.spotlight_search)
                            .hint_text("Type a symbol to jump to it")
                            .desired_width(460.0)
                            .font(FontId::new(14.0, FontFamily::Proportional)),
                    );
                    edit.request_focus();
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // High-performance Trigram Symbol Search Results (Enso style)
                ui.label(
                    RichText::new("SYMBOLS").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong(),
                );
                ui.add_space(4.0);

                let results = self.search_index.search(&self.spotlight_search, 16);
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for item in results {
                        if self.graph.nodes.contains_key(&item.node_id) {
                            ui.horizontal(|ui| {
                                let arch_color = studio_canvas::archetype_color(item.archetype);
                                ui.label(
                                    RichText::new(format!(" {} ", item.archetype.label()))
                                        .font(FontId::new(9.0, FontFamily::Monospace))
                                        .color(arch_color)
                                        .background_color(Color32::from_rgba_premultiplied(
                                            arch_color.r(),
                                            arch_color.g(),
                                            arch_color.b(),
                                            35,
                                        )),
                                );

                                if ui
                                    .button(
                                        RichText::new(&item.title)
                                            .font(FontId::new(12.0, FontFamily::Proportional))
                                            .color(TEXT_PRIMARY),
                                    )
                                    .clicked()
                                {
                                    self.canvas_state.selected_nodes.clear();
                                    self.canvas_state.selected_nodes.insert(item.node_id);
                                    // The canvas centres it, in its own rectangle.
                                    self.canvas_state.action_request = Some(CanvasAction::CenterNode(item.node_id));
                                    self.spotlight_open = false;
                                }

                                if let Some(ref crate_name) = item.crate_name {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(crate_name)
                                                .font(FontId::new(9.5, FontFamily::Monospace))
                                                .color(TEXT_DIM),
                                        );
                                    });
                                }
                            });
                        }
                    }
                });

                ui.add_space(6.0);
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.spotlight_open = false;
                }
            });
    }

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
