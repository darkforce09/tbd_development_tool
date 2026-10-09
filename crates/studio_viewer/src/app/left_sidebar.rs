use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Pos2, RichText, Sense, Stroke, Vec2};
use studio_graph::NodeArchetype;
use studio_ui::color_tokens::*;

use super::StudioApp;

impl StudioApp {
    /// Left sidebar, Eden style: closed, a small arrow tab sits at the left edge under the
    /// title bar; open, the same tab sits on the sidebar's right edge and closes it.
    pub(crate) fn render_left_sidebar(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let tab_y = root.available_rect_before_wrap().top() + 8.0;
        if !self.left_sidebar_open {
            let left = root.max_rect().left();
            if sidebar_tab(&ctx, Pos2::new(left, tab_y), egui_phosphor::regular::CARET_RIGHT, "Show sidebar") {
                self.left_sidebar_open = true;
            }
            return;
        }

        let panel = egui::Panel::left("studio_left_sidebar")
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

        let edge = panel.response.rect.right();
        if sidebar_tab(&ctx, Pos2::new(edge, tab_y), egui_phosphor::regular::CARET_LEFT, "Hide sidebar") {
            self.left_sidebar_open = false;
        }
    }
}

/// Arrow tab attached to a vertical edge at `pos` (top-left of the tab). Returns true when clicked.
fn sidebar_tab(ctx: &egui::Context, pos: Pos2, icon: &str, tooltip: &str) -> bool {
    egui::Area::new(egui::Id::new("left_sidebar_tab"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(16.0, 40.0), Sense::click());
            let response = response.on_hover_text(tooltip);
            let fill = if response.hovered() { FLOATING_BTN_HOVER } else { PANEL_BG };
            let radius = CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 };
            ui.painter().rect(rect, radius, fill, Stroke::new(1.0, PANEL_BORDER), egui::StrokeKind::Inside);
            let color = if response.hovered() { TEXT_HIGHLIGHT } else { TEXT_SECONDARY };
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon,
                FontId::new(12.0, FontFamily::Proportional),
                color,
            );
            response.clicked()
        })
        .inner
}
