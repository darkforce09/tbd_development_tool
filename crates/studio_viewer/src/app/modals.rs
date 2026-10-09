use eframe::egui;
use egui::{Color32, FontFamily, FontId, Key, Pos2, ProgressBar, RichText, CornerRadius, Stroke};
use studio_graph::{DataType, NodeArchetype};
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_loading_hud(&mut self, ctx: &egui::Context) {
        if !self.is_loading {
            return;
        }

        egui::Window::new("streaming_loader_modal")
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([480.0, 130.0])
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(18, 20, 29))
                    .stroke(Stroke::new(1.5, Color32::from_rgb(99, 102, 241)))
                    .corner_radius(CornerRadius::from(12.0))
                    .inner_margin(egui::Margin::same(18)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("STREAMING AST GRAPH")
                                .font(FontId::new(13.0, FontFamily::Monospace))
                                .color(Color32::from_rgb(99, 102, 241))
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(&self.gpu_label)
                                    .font(FontId::new(10.0, FontFamily::Monospace))
                                    .color(Color32::from_rgb(52, 211, 153)),
                            );
                        });
                    });

                    ui.add_space(10.0);
                    let progress_bar = ProgressBar::new(self.loading_progress)
                        .show_percentage()
                        .animate(true);
                    ui.add(progress_bar);

                    ui.add_space(8.0);
                    let detail_text = if self.loading_total_files > 0 {
                        format!("{} ({}/{} files)", self.loading_stage, self.loading_files_done, self.loading_total_files)
                    } else {
                        self.loading_stage.clone()
                    };
                    ui.label(
                        RichText::new(detail_text)
                            .font(FontId::new(11.0, FontFamily::Proportional))
                            .color(TEXT_SECONDARY),
                    );
                });
            });
    }

    pub(crate) fn render_spotlight_modal(&mut self, ctx: &egui::Context) {
        if !self.spotlight_open {
            return;
        }

        let screen_rect = ctx.content_rect();
        let spot_title = format!("{} Enso Spotlight & Quick Actions", egui_phosphor::regular::MAGNIFYING_GLASS);
        egui::Window::new(spot_title)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0])
            .fixed_size([560.0, 380.0])
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
                    ui.label(RichText::new(egui_phosphor::regular::MAGNIFYING_GLASS).font(FontId::new(16.0, FontFamily::Proportional)));
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.spotlight_search)
                            .hint_text("Type symbol to jump or 'add' to create...")
                            .desired_width(460.0)
                            .font(FontId::new(14.0, FontFamily::Proportional)),
                    );
                    edit.request_focus();
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // Insert Actions (Enso style)
                ui.label(RichText::new("CREATE NEW NODE ON CANVAS").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                ui.add_space(4.0);

                ui.horizontal_wrapped(|ui| {
                    let center_world = self.canvas_state.transform.screen_to_world(screen_rect.center());

                    let fn_btn = format!("{} Function", egui_phosphor::regular::PLUS);
                    if ui.button(fn_btn).clicked() {
                        let id = self.graph.add_node("fn new_handler", NodeArchetype::Function, "Custom function node", Some("FN".to_string()), vec![("exec".to_string(), DataType::RustFlow)], vec![("call".to_string(), DataType::RustFlow)], [center_world.x, center_world.y]);
                        self.canvas_state.selected_nodes.clear();
                        self.canvas_state.selected_nodes.insert(id);
                        self.spotlight_open = false;
                    }

                    let struct_btn = format!("{} Struct", egui_phosphor::regular::PLUS);
                    if ui.button(struct_btn).clicked() {
                        let id = self.graph.add_node("struct Config", NodeArchetype::Struct, "Custom struct node", Some("STRUCT".to_string()), vec![], vec![("Self".to_string(), DataType::RustType("Config".to_string()))], [center_world.x, center_world.y]);
                        self.canvas_state.selected_nodes.clear();
                        self.canvas_state.selected_nodes.insert(id);
                        self.spotlight_open = false;
                    }

                    let mod_btn = format!("{} Module", egui_phosphor::regular::PLUS);
                    if ui.button(mod_btn).clicked() {
                        let id = self.graph.add_node("mod services", NodeArchetype::Module, "Module container", Some("MOD".to_string()), vec![], vec![("export".to_string(), DataType::RustFlow)], [center_world.x, center_world.y]);
                        self.canvas_state.selected_nodes.clear();
                        self.canvas_state.selected_nodes.insert(id);
                        self.spotlight_open = false;
                    }

                    let in_btn = format!("{} Ingress", egui_phosphor::regular::PLUS);
                    if ui.button(in_btn).clicked() {
                        let id = self.graph.add_node("Data Ingress", NodeArchetype::Ingress, "Stream input source", Some("IN".to_string()), vec![], vec![("packet".to_string(), DataType::Audio)], [center_world.x, center_world.y]);
                        self.canvas_state.selected_nodes.clear();
                        self.canvas_state.selected_nodes.insert(id);
                        self.spotlight_open = false;
                    }

                    let cpu_btn = format!("{} Compute", egui_phosphor::regular::PLUS);
                    if ui.button(cpu_btn).clicked() {
                        let id = self.graph.add_node("Transformer Worker", NodeArchetype::Compute, "Compute node", Some("CPU".to_string()), vec![("in".to_string(), DataType::Audio)], vec![("out".to_string(), DataType::Text)], [center_world.x, center_world.y]);
                        self.canvas_state.selected_nodes.clear();
                        self.canvas_state.selected_nodes.insert(id);
                        self.spotlight_open = false;
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // High-performance Trigram Symbol Search Results (Enso style)
                ui.label(RichText::new("JUMP TO GRAPH SYMBOLS").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                ui.add_space(4.0);

                let results = self.search_index.search(&self.spotlight_search, 16);
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    for item in results {
                        if let Some(node) = self.graph.nodes.get(&item.node_id) {
                            ui.horizontal(|ui| {
                                let arch_color = studio_canvas::archetype_color(item.archetype);
                                ui.label(
                                    RichText::new(format!(" {} ", item.archetype.label()))
                                        .font(FontId::new(9.0, FontFamily::Monospace))
                                        .color(arch_color)
                                        .background_color(Color32::from_rgba_premultiplied(arch_color.r(), arch_color.g(), arch_color.b(), 35)),
                                );

                                if ui.button(RichText::new(&item.title).font(FontId::new(12.0, FontFamily::Proportional)).color(TEXT_PRIMARY)).clicked() {
                                    self.canvas_state.selected_nodes.clear();
                                    self.canvas_state.selected_nodes.insert(item.node_id);
                                    let center_world = Pos2::new(node.position[0] + node.size[0] * 0.5, node.position[1] + node.size[1] * 0.5);
                                    self.canvas_state.transform.center_on_world_pos(center_world, screen_rect, None);
                                    self.spotlight_open = false;
                                }

                                if let Some(ref crate_name) = item.crate_name {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(crate_name).font(FontId::new(9.5, FontFamily::Monospace)).color(TEXT_DIM));
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
}
