use eframe::egui;
use egui::{Color32, FontFamily, FontId, Pos2, RichText, Rounding, Stroke, Vec2};
use studio_canvas::{archetype_color, data_type_color};
use studio_ui::{color_tokens::*, truncate_with_ellipsis};

use super::StudioApp;

impl StudioApp {
    pub(crate) fn render_right_inspector(&mut self, ctx: &egui::Context) {
        if !self.right_inspector_open {
            return;
        }

        let panel_width = if self.code_editor_expanded { 640.0 } else { 380.0 };
        egui::SidePanel::right("studio_right_inspector")
            .resizable(true)
            .default_width(panel_width)
            .min_width(320.0)
            .max_width(900.0)
            .frame(
                egui::Frame::none()
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // Check selected node
                    let selected_id = self.canvas_state.selected_nodes.iter().next().copied();

                    if let Some(node_id) = selected_id {
                        if let Some(node) = self.graph.nodes.get(&node_id).cloned() {
                            let lang = studio_ui::detect_language(node.file_path.as_deref(), node.badge.as_deref());
                            let is_markdown = lang == "md" || lang == "markdown" || node.badge.as_deref() == Some("MD");

                            // Header: Title + Archetype + Crate
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&node.title).font(FontId::new(15.0, FontFamily::Proportional)).strong().color(TEXT_HIGHLIGHT));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let arch_color = if is_markdown { Color32::from_rgb(0x2d, 0xd4, 0xbf) } else { archetype_color(node.archetype) };
                                    let arch_label = if is_markdown { "MARKDOWN".to_string() } else { node.archetype.label().to_string() };
                                    ui.label(
                                        RichText::new(format!(" {} ", arch_label))
                                            .font(FontId::new(10.0, FontFamily::Monospace))
                                            .color(arch_color)
                                            .background_color(Color32::from_rgba_premultiplied(arch_color.r(), arch_color.g(), arch_color.b(), 35)),
                                    );
                                });
                            });

                            // Location & Crate Badge
                            if let Some(path) = &node.file_path {
                                let line_str = node.line_number.map(|l| format!(":{}", l)).unwrap_or_default();
                                ui.label(RichText::new(format!("{} {}{}", egui_phosphor::regular::MAP_PIN, path, line_str)).font(FontId::new(10.5, FontFamily::Proportional)).color(TEXT_DIM));
                            }

                            ui.add_space(4.0);

                            // Focus button
                            let focus_label = format!("{} Focus on Canvas", egui_phosphor::regular::EYE);
                            if ui.button(focus_label).clicked() {
                                let center_world = Pos2::new(node.position[0] + node.size[0] * 0.5, node.position[1] + node.size[1] * 0.5);
                                self.canvas_state.transform.center_on_world_pos(center_world, ctx.screen_rect(), None);
                            }

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(6.0);

                            if is_markdown {
                                // Dedicated Full-Height Markdown Document Workspace
                                ui.horizontal(|ui| {
                                    let tab0 = format!("{} Document Preview", egui_phosphor::regular::EYE);
                                    let tab1 = format!("{} Outline ({})", egui_phosphor::regular::LIST_NUMBERS, node.member_nodes.len());
                                    let tab2 = format!("{} Source Editor", egui_phosphor::regular::PENCIL_SIMPLE);

                                    if ui.selectable_label(self.markdown_inspector_tab == 0, tab0).clicked() {
                                        self.markdown_inspector_tab = 0;
                                    }
                                    if ui.selectable_label(self.markdown_inspector_tab == 1, tab1).clicked() {
                                        self.markdown_inspector_tab = 1;
                                    }
                                    if ui.selectable_label(self.markdown_inspector_tab == 2, tab2).clicked() {
                                        self.markdown_inspector_tab = 2;
                                    }

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let split_label = if self.code_editor_expanded { "Collapse" } else { "Expand Split" };
                                        let split_btn_text = format!("{} {}", egui_phosphor::regular::CORNERS_OUT, split_label);
                                        if ui.small_button(split_btn_text).clicked() {
                                            self.code_editor_expanded = !self.code_editor_expanded;
                                        }
                                    });
                                });

                                ui.add_space(6.0);

                                match self.markdown_inspector_tab {
                                    0 => {
                                        // Tab 0: Document Preview (Full-Height Scrollable Markdown View)
                                        let avail_h = (ui.available_height() - 20.0).max(480.0);
                                        egui::Frame::none()
                                            .fill(CARD_BG)
                                            .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
                                            .rounding(Rounding::same(6.0))
                                            .inner_margin(egui::Margin::same(12.0))
                                            .show(ui, |ui| {
                                                egui::ScrollArea::vertical()
                                                    .id_source("md_sidebar_full_preview")
                                                    .max_height(avail_h)
                                                    .show(ui, |ui| {
                                                        studio_ui::render_markdown(ui, &self.code_editor_buffer);
                                                    });
                                            });
                                    }
                                    1 => {
                                        // Tab 1: Outline & Headings
                                        ui.label(RichText::new("DOCUMENT OUTLINE & HEADINGS").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                                        ui.add_space(4.0);
                                        egui::Frame::none()
                                            .fill(CARD_BG)
                                            .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
                                            .rounding(Rounding::same(6.0))
                                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                            .show(ui, |ui| {
                                                if node.member_nodes.is_empty() {
                                                    ui.label(RichText::new("(No headings extracted)").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_DIM));
                                                } else {
                                                    for member in &node.member_nodes {
                                                        let is_active = self.code_editor_member_id.as_deref() == Some(&member.id);
                                                        ui.horizontal(|ui| {
                                                            let tag = if member.visibility.is_empty() { "H" } else { &member.visibility };
                                                            ui.label(
                                                                RichText::new(format!(" {} ", tag))
                                                                    .font(FontId::new(9.5, FontFamily::Monospace))
                                                                    .color(Color32::from_rgb(0x2d, 0xd4, 0xbf))
                                                                    .background_color(Color32::from_rgba_premultiplied(45, 212, 191, 35)),
                                                            );
                                                            ui.label(
                                                                RichText::new(&member.name)
                                                                    .font(FontId::new(11.0, FontFamily::Proportional))
                                                                    .color(if is_active { TEXT_HIGHLIGHT } else { TEXT_PRIMARY }),
                                                            );
                                                            ui.label(RichText::new(format!(":{}", member.line_number)).font(FontId::new(9.5, FontFamily::Monospace)).color(TEXT_DIM));
                                                        });
                                                        ui.add_space(3.0);
                                                    }
                                                }
                                            });
                                    }
                                    _ => {
                                        // Tab 2: Live Markdown Source Editor with VS Code Syntax Highlighting
                                        ui.horizontal(|ui| {
                                            let save_label = format!("{} Save Changes (Ctrl+S)", egui_phosphor::regular::FLOPPY_DISK);
                                            if ui.button(save_label).clicked() {
                                                self.save_current_editor_code();
                                            }

                                            if self.code_editor_dirty {
                                                ui.label(RichText::new("● modified (unsaved)").font(FontId::new(10.5, FontFamily::Monospace)).color(Color32::from_rgb(251, 146, 60)));
                                            }

                                            if let Some(status) = &self.code_editor_status {
                                                ui.label(RichText::new(status).font(FontId::new(10.0, FontFamily::Monospace)).color(TEXT_SECONDARY));
                                            }
                                        });

                                        ui.add_space(4.0);
                                        let avail_h = (ui.available_height() - 20.0).max(480.0);
                                        let response = ui.add(
                                            egui::TextEdit::multiline(&mut self.code_editor_buffer)
                                                .font(FontId::new(11.0, FontFamily::Monospace))
                                                .code_editor()
                                                .layouter(&mut studio_ui::code_editor_layouter("md".to_string()))
                                                .desired_rows(24)
                                                .desired_width(f32::INFINITY)
                                                .min_size(Vec2::new(ui.available_width(), avail_h)),
                                        );

                                        if response.changed() {
                                            self.code_editor_dirty = true;
                                            self.code_editor_status = None;
                                        }
                                    }
                                }
                            } else {
                                // Standard Code File Inspection (Documentation, File Members Accordion, Haystack Ports, and Editor)

                                // Documentation / Description (Rich Markdown Renderer)
                                if let Some(docs) = &node.doc_comment {
                                    ui.label(RichText::new("DOCUMENTATION (MARKDOWN)").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                                    ui.add_space(2.0);
                                    studio_ui::render_markdown(ui, docs);
                                    ui.add_space(8.0);
                                } else if !node.description.is_empty() {
                                    ui.label(RichText::new("SUMMARY").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                                    ui.add_space(2.0);
                                    studio_ui::render_markdown(ui, &node.description);
                                    ui.add_space(8.0);
                                }

                                // FILE MEMBERS (Interactive Accordion Members for File Nodes)
                                if !node.member_nodes.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("FILE MEMBERS ({})", node.member_nodes.len()))
                                                .font(FontId::new(10.5, FontFamily::Monospace))
                                                .color(TEXT_DIM)
                                                .strong(),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let toggle_label = if node.is_dropdown_expanded {
                                                format!("{} Collapse", egui_phosphor::regular::CARET_DOWN)
                                            } else {
                                                format!("{} Expand", egui_phosphor::regular::CARET_RIGHT)
                                            };
                                            if ui.small_button(toggle_label).clicked() {
                                                if let Some(n) = self.graph.nodes.get_mut(&node.id) {
                                                    n.is_dropdown_expanded = !n.is_dropdown_expanded;
                                                    n.size = studio_canvas::calculate_file_node_size(n);
                                                    let bounds = [
                                                        n.position[0],
                                                        n.position[1],
                                                        n.position[0] + n.size[0],
                                                        n.position[1] + n.size[1],
                                                    ];
                                                    self.canvas_state.spatial_grid.update(node.id, bounds);
                                                }
                                                self.graph.update_cluster_bounds();
                                            }

                                            let wires_text = if node.show_member_wires {
                                                format!("{} Lines: ON", egui_phosphor::regular::PLUGS_CONNECTED)
                                            } else {
                                                format!("{} Lines: OFF", egui_phosphor::regular::PLUGS)
                                            };
                                            if ui.small_button(wires_text).on_hover_text("Toggle sub-node connection lines for this file").clicked() {
                                                if let Some(n) = self.graph.nodes.get_mut(&node.id) {
                                                    n.show_member_wires = !n.show_member_wires;
                                                }
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);

                                    egui::Frame::none()
                                        .fill(CARD_BG)
                                        .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                        .show(ui, |ui| {
                                            for member in &node.member_nodes {
                                                let is_active = self.code_editor_member_id.as_deref() == Some(&member.id);
                                                let arch_col = archetype_color(member.archetype);

                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(" {} ", member.archetype.label()))
                                                            .font(FontId::new(9.5, FontFamily::Monospace))
                                                            .color(arch_col)
                                                            .background_color(Color32::from_rgba_premultiplied(arch_col.r(), arch_col.g(), arch_col.b(), 35)),
                                                    );

                                                    if member.visibility == "pub" {
                                                        ui.label(RichText::new("pub").font(FontId::new(10.0, FontFamily::Monospace)).color(Color32::from_rgb(134, 239, 172)));
                                                    }

                                                    let display_text = if !member.signature.is_empty() {
                                                        &member.signature
                                                    } else {
                                                        &member.name
                                                    };
                                                    ui.label(
                                                        RichText::new(truncate_with_ellipsis(display_text, 22))
                                                            .font(FontId::new(10.5, FontFamily::Monospace))
                                                            .color(if is_active { TEXT_HIGHLIGHT } else { TEXT_PRIMARY }),
                                                    );

                                                    ui.label(RichText::new(format!(":{}", member.line_number)).font(FontId::new(9.5, FontFamily::Monospace)).color(TEXT_DIM));

                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        let btn_label = if is_active { "Active" } else { "Inspect" };
                                                        if ui.small_button(btn_label).clicked() {
                                                            self.request_open_member(node.id, member.id.clone());
                                                        }
                                                    });
                                                });
                                                ui.add_space(2.0);
                                            }
                                        });

                                    ui.add_space(10.0);
                                    ui.separator();
                                    ui.add_space(6.0);
                                }

                                // Haystack Ports Schema (Inputs & Outputs tables with jump-to links)
                                ui.label(RichText::new("PORTS & CONNECTIONS (HAYSTACK)").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());
                                ui.add_space(4.0);

                                // Inputs Table
                                ui.label(RichText::new(format!("Inputs ({})", node.inputs.len())).font(FontId::new(11.5, FontFamily::Proportional)).strong().color(TEXT_PRIMARY));
                                for port in &node.inputs {
                                    let connected_edge_ids = self.graph.get_port_edges(node_id, port.id);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("  ← {}", port.name)).font(FontId::new(11.0, FontFamily::Monospace)).color(data_type_color(&port.data_type)));
                                        ui.label(RichText::new(format!("({})", port.data_type.display_name())).font(FontId::new(10.0, FontFamily::Monospace)).color(TEXT_DIM));

                                        if let Some(&first_edge_id) = connected_edge_ids.first() {
                                            if let Some(edge) = self.graph.get_edge(first_edge_id) {
                                                if let Some(src_node) = self.graph.nodes.get(&edge.from_node) {
                                                    if ui.small_button(format!("Jump: {}", truncate_with_ellipsis(&src_node.title, 14))).clicked() {
                                                        self.canvas_state.selected_nodes.clear();
                                                        self.canvas_state.selected_nodes.insert(src_node.id);
                                                        let center_world = Pos2::new(src_node.position[0] + src_node.size[0] * 0.5, src_node.position[1] + src_node.size[1] * 0.5);
                                                        self.canvas_state.transform.center_on_world_pos(center_world, ctx.screen_rect(), None);
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }

                                ui.add_space(4.0);

                                // Outputs Table
                                ui.label(RichText::new(format!("Outputs ({})", node.outputs.len())).font(FontId::new(11.5, FontFamily::Proportional)).strong().color(TEXT_PRIMARY));
                                for port in &node.outputs {
                                    let connected_edge_ids = self.graph.get_port_edges(node_id, port.id);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("  → {}", port.name)).font(FontId::new(11.0, FontFamily::Monospace)).color(data_type_color(&port.data_type)));
                                        ui.label(RichText::new(format!("({})", port.data_type.display_name())).font(FontId::new(10.0, FontFamily::Monospace)).color(TEXT_DIM));

                                        if let Some(&first_edge_id) = connected_edge_ids.first() {
                                            if let Some(edge) = self.graph.get_edge(first_edge_id) {
                                                if let Some(dst_node) = self.graph.nodes.get(&edge.to_node) {
                                                    if ui.small_button(format!("Jump: {}", truncate_with_ellipsis(&dst_node.title, 14))).clicked() {
                                                        self.canvas_state.selected_nodes.clear();
                                                        self.canvas_state.selected_nodes.insert(dst_node.id);
                                                        let center_world = Pos2::new(dst_node.position[0] + dst_node.size[0] * 0.5, dst_node.position[1] + dst_node.size[1] * 0.5);
                                                        self.canvas_state.transform.center_on_world_pos(center_world, ctx.screen_rect(), None);
                                                    }
                                                }
                                            }
                                        }
                                    });
                                }

                                ui.add_space(12.0);
                                ui.separator();
                                ui.add_space(6.0);

                                // CODE CANVAS LIVE CODE EDITOR (See & Edit code with line numbers & disk save)
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("CODE CANVAS LIVE EDITOR").font(FontId::new(10.5, FontFamily::Monospace)).color(TEXT_DIM).strong());

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let split_label = if self.code_editor_expanded { "Collapse" } else { "Expand Split" };
                                        let split_btn_text = format!("{} {}", egui_phosphor::regular::CORNERS_OUT, split_label);
                                        if ui.small_button(split_btn_text).clicked() {
                                            self.code_editor_expanded = !self.code_editor_expanded;
                                        }
                                    });
                                });

                                ui.add_space(4.0);

                                // Editor Controls & Save Button
                                ui.horizontal(|ui| {
                                    let save_label = format!("{} Save Changes (Ctrl+S)", egui_phosphor::regular::FLOPPY_DISK);
                                    if ui.button(save_label).clicked() {
                                        self.save_current_editor_code();
                                    }

                                    if self.code_editor_dirty {
                                        ui.label(RichText::new("● modified (unsaved)").font(FontId::new(10.5, FontFamily::Monospace)).color(Color32::from_rgb(251, 146, 60)));
                                    }

                                    if let Some(status) = &self.code_editor_status {
                                        ui.label(RichText::new(status).font(FontId::new(10.0, FontFamily::Monospace)).color(TEXT_SECONDARY));
                                    }
                                });

                                let item_snippet = node.archetype != studio_graph::NodeArchetype::File
                                    && matches!(self.code_editor_origin, Some(studio_parser::EditOrigin::Snippet { .. }));
                                let snippet_label = self.code_editor_member_id.clone().or_else(|| item_snippet.then(|| node.title.clone()));
                                if let Some(snippet_label) = snippet_label {
                                    let mut clear_inspect = false;
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("Viewing snippet: {}", snippet_label))
                                                .font(FontId::new(10.0, FontFamily::Monospace))
                                                .color(Color32::from_rgb(56, 189, 248)),
                                        );
                                        let load_file_label = format!("{} Load Full File", egui_phosphor::regular::FILE_TEXT);
                                        if ui.small_button(load_file_label).clicked() {
                                            clear_inspect = true;
                                        }
                                    });
                                    if clear_inspect {
                                        self.load_full_file_in_editor();
                                    }
                                }

                                ui.add_space(4.0);

                                let editor_height = if self.code_editor_expanded { 460.0 } else { 240.0 };
                                // Multi-line code editor area with VS Code syntax highlighting layouter
                                let response = ui.add(
                                    egui::TextEdit::multiline(&mut self.code_editor_buffer)
                                        .font(FontId::new(11.0, FontFamily::Monospace))
                                        .code_editor()
                                        .layouter(&mut studio_ui::code_editor_layouter(lang))
                                        .desired_rows(16)
                                        .desired_width(f32::INFINITY)
                                        .min_size(Vec2::new(ui.available_width(), editor_height)),
                                );

                                if response.changed() {
                                    self.code_editor_dirty = true;
                                    self.code_editor_status = None;
                                }
                            }
                        }
                    } else {
                        // No node selected: Display Haystack Pipeline Diagnostics
                        ui.label(RichText::new("PIPELINE DIAGNOSTICS (HAYSTACK)").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                        ui.add_space(8.0);

                        egui::Frame::none()
                            .fill(CARD_BG)
                            .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(egui::Margin::same(12.0))
                            .show(ui, |ui| {
                                let diag_label = format!("{} Architecture Graph Valid", egui_phosphor::regular::CHECK_CIRCLE);
                                ui.label(RichText::new(diag_label).font(FontId::new(13.0, FontFamily::Proportional)).color(ARCHETYPE_EGRESS).strong());
                                ui.add_space(6.0);

                                let isolated_nodes = self.graph.isolated_nodes_count();

                                ui.label(RichText::new(format!("• Total Components: {}", self.graph.nodes.len())).font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_PRIMARY));
                                ui.label(RichText::new(format!("• Active Wires: {}", self.graph.edges.len())).font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_PRIMARY));
                                ui.label(RichText::new(format!("• Isolated Nodes: {}", isolated_nodes)).font(FontId::new(11.0, FontFamily::Monospace)).color(if isolated_nodes > 0 { ARCHETYPE_STATE } else { ARCHETYPE_EGRESS }));
                                ui.label(RichText::new(format!("• Subsystem Clusters: {}", self.graph.clusters.len())).font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_PRIMARY));
                            });

                        ui.add_space(14.0);
                        ui.separator();
                        ui.add_space(8.0);

                        ui.label(RichText::new("SHORTCUTS & COMMANDS").font(FontId::new(11.0, FontFamily::Monospace)).color(TEXT_DIM).strong());
                        ui.add_space(4.0);
                        ui.label(RichText::new("• [Ctrl+K] / [/] : Open Enso Spotlight").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                        ui.label(RichText::new("• [RMB Drag] / [Space + LMB] : Pan canvas smoothly").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                        ui.label(RichText::new("• [Mouse Wheel]  : Zoom in/out at pointer").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                        ui.label(RichText::new("• [Click Node]   : Inspect schema & code").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                        ui.label(RichText::new("• [Ctrl + S]     : Save code changes to disk").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                        ui.label(RichText::new("• [Del]          : Delete selected node").font(FontId::new(11.0, FontFamily::Proportional)).color(TEXT_SECONDARY));
                    }
                });
            });
    }
}
