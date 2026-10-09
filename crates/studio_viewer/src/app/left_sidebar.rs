use eframe::egui;
use egui::{Color32, FontFamily, FontId, Pos2, RichText, Stroke};
use studio_graph::NodeArchetype;
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_left_sidebar(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        if !self.left_sidebar_open {
            return;
        }

        egui::Panel::left("studio_left_sidebar")
            .resizable(true)
            .default_size(280.0)
            .min_size(220.0)
            .max_size(420.0)
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(root, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // Search Input
                    ui.label(
                        RichText::new("FILTER MAP")
                            .font(FontId::new(11.0, FontFamily::Monospace))
                            .color(TEXT_DIM)
                            .strong(),
                    );
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .hint_text("Search filters...")
                                .desired_width(190.0),
                        );
                        if ui.button(egui_phosphor::regular::X).clicked() {
                            self.search_query.clear();
                        }
                    });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Categories Checklist (CodeSee style)
                    ui.label(
                        RichText::new("CATEGORIES")
                            .font(FontId::new(11.0, FontFamily::Monospace))
                            .color(TEXT_DIM)
                            .strong(),
                    );
                    ui.add_space(4.0);

                    let archetypes = [
                        (NodeArchetype::File, "FILE", "Files", ARCHETYPE_FILE),
                        (NodeArchetype::Function, "FN", "Functions", Color32::from_rgb(168, 85, 247)),
                        (NodeArchetype::Struct, "STR", "Structs", Color32::from_rgb(251, 146, 60)),
                        (NodeArchetype::Enum, "ENM", "Enums", Color32::from_rgb(250, 204, 21)),
                        (NodeArchetype::Trait, "TRT", "Traits", Color32::from_rgb(52, 211, 153)),
                        (NodeArchetype::Module, "MOD", "Modules", Color32::from_rgb(56, 189, 248)),
                        (NodeArchetype::Ingress, "IN", "Ingress", ARCHETYPE_INGRESS),
                        (NodeArchetype::Compute, "CPU", "Compute", ARCHETYPE_COMPUTE),
                        (NodeArchetype::State, "MEM", "State Cache", ARCHETYPE_STATE),
                        (NodeArchetype::Egress, "OUT", "Egress Output", ARCHETYPE_EGRESS),
                    ];

                    for (arch, tag, label, color) in archetypes {
                        let count = self.graph.archetype_count(arch);
                        if count == 0 && self.project_stats.is_some() {
                            continue;
                        }

                        let checked = self.category_filters.entry(arch).or_insert(true);
                        ui.horizontal(|ui| {
                            ui.checkbox(checked, "");

                            // Badge Pill
                            let tag_rect = ui.label(
                                RichText::new(format!(" {} ", tag))
                                    .font(FontId::new(9.5, FontFamily::Monospace))
                                    .color(color)
                                    .background_color(Color32::from_rgba_premultiplied(
                                        color.r(),
                                        color.g(),
                                        color.b(),
                                        35,
                                    )),
                            );
                            let _ = tag_rect;

                            ui.label(
                                RichText::new(label)
                                    .font(FontId::new(11.5, FontFamily::Proportional))
                                    .color(TEXT_PRIMARY),
                            );

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{}", count))
                                        .font(FontId::new(10.5, FontFamily::Monospace))
                                        .color(TEXT_DIM),
                                );
                            });
                        });
                    }

                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Clusters & Subsystems List (CodeSee style)
                    ui.label(
                        RichText::new("CLUSTERS & SUBSYSTEMS")
                            .font(FontId::new(11.0, FontFamily::Monospace))
                            .color(TEXT_DIM)
                            .strong(),
                    );
                    ui.add_space(4.0);

                    for cluster in &self.graph.clusters {
                        let btn_text = format!(
                            "{}  {} ({})",
                            egui_phosphor::regular::FOLDER,
                            cluster.label,
                            cluster.node_ids.len()
                        );
                        if ui.button(btn_text).clicked() {
                            let center_world = Pos2::new(
                                cluster.position[0] + cluster.size[0] * 0.5,
                                cluster.position[1] + cluster.size[1] * 0.5,
                            );
                            self.canvas_state.transform.center_on_world_pos(center_world, ctx.content_rect(), None);
                        }
                    }
                });
            });
    }
}
