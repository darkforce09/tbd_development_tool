use eframe::egui;
use egui::{Color32, FontFamily, FontId, Pos2, RichText, Rounding, Stroke};
use studio_graph::NodeArchetype;
use studio_ui::color_tokens::*;

use super::types::StudioViewMode;
use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_left_sidebar(&mut self, ctx: &egui::Context) {
        if !self.left_sidebar_open {
            return;
        }

        egui::SidePanel::left("studio_left_sidebar")
            .resizable(true)
            .default_width(280.0)
            .min_width(220.0)
            .max_width(420.0)
            .frame(
                egui::Frame::none()
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    match self.view_mode {
                        StudioViewMode::CodebaseMaps | StudioViewMode::ArchitectureMap => {
                            // Search Input
                            ui.label(RichText::new("FILTER MAP").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                            ui.add_space(4.0);

                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text("Search filters...").desired_width(190.0));
                                if ui.button(egui_phosphor::regular::X).clicked() {
                                    self.search_query.clear();
                                }
                            });

                            ui.add_space(12.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // Categories Checklist (CodeSee style)
                            ui.label(RichText::new("CATEGORIES").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
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
                                            .background_color(Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 35)),
                                    );
                                    let _ = tag_rect;

                                    ui.label(RichText::new(label).font(FontId::new(11.5, FontFamily::Proportional)).color(TEXT_PRIMARY));

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(format!("{}", count)).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM));
                                    });
                                });
                            }

                            ui.add_space(14.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // Clusters & Subsystems List (CodeSee style)
                            ui.label(RichText::new("CLUSTERS & SUBSYSTEMS").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                            ui.add_space(4.0);

                            for cluster in &self.graph.clusters {
                                let btn_text = format!("{}  {} ({})", egui_phosphor::regular::FOLDER, cluster.label, cluster.node_ids.len());
                                if ui.button(btn_text).clicked() {
                                    let center_world = Pos2::new(
                                        cluster.position[0] + cluster.size[0] * 0.5,
                                        cluster.position[1] + cluster.size[1] * 0.5,
                                    );
                                    self.canvas_state.transform.center_on_world_pos(center_world, ctx.screen_rect(), None);
                                }
                            }
                        }

                        StudioViewMode::FlowsAndTraces => {
                            // CodeSee Flows & Variants Panel
                            ui.label(RichText::new("EXECUTION FLOWS").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                            ui.add_space(4.0);

                            if let Some(flow) = self.flows.get(self.active_flow_index) {
                                // Flow Header
                                ui.label(RichText::new(&flow.name).font(FontId::new(13.0, FontFamily::Proportional)).strong().color(TEXT_PRIMARY));
                                ui.label(RichText::new(format!("Entry: {}", flow.entry_point)).font(FontId::new(10.5, FontFamily::Monospace)).color(ARCHETYPE_INGRESS));
                                ui.add_space(2.0);
                                ui.label(RichText::new(&flow.description).font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));

                                ui.add_space(6.0);
                                ui.label(RichText::new(format!("Originators: {}", flow.originators)).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                                ui.label(RichText::new(format!("Stages: {}  •  Calls: {}", flow.service_count, flow.api_call_count)).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM));

                                ui.add_space(14.0);
                                ui.separator();
                                ui.add_space(8.0);

                                // Flow Variants
                                ui.label(RichText::new("VARIANTS").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                                ui.add_space(6.0);

                                let mut activate_variant: Option<usize> = None;
                                for (v_idx, variant) in flow.variants.iter().enumerate() {
                                    let is_selected = self.active_variant_index == v_idx;
                                    let card_bg = if is_selected { Color32::from_rgb(26, 32, 50) } else { CARD_BG };
                                    let border_stroke = if is_selected { Stroke::new(1.5, CARD_BORDER_SELECTED) } else { Stroke::new(1.0, CARD_BORDER_NORMAL) };

                                    egui::Frame::none()
                                        .fill(card_bg)
                                        .stroke(border_stroke)
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(egui::Margin::same(10.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(&variant.name).font(FontId::new(12.0, FontFamily::Proportional)).strong().color(if is_selected { TEXT_HIGHLIGHT } else { TEXT_PRIMARY }));
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    ui.label(RichText::new(format!("{}%", variant.coverage_pct)).font(FontId::new(10.0, FontFamily::Monospace)).color(ARCHETYPE_COMPUTE));
                                                });
                                            });
                                            ui.add_space(3.0);
                                            ui.label(RichText::new(&variant.summary).font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM));
                                            ui.label(RichText::new(&variant.description).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_SECONDARY));

                                            if !is_selected && ui.button("Activate Trace").clicked() {
                                                activate_variant = Some(v_idx);
                                            }
                                        });
                                    ui.add_space(6.0);
                                }

                                if let Some(v_idx) = activate_variant {
                                    self.active_variant_index = v_idx;
                                    self.sync_active_flow_edges();
                                }
                            }
                        }
                    }
                });
            });
    }
}
