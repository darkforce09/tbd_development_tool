use eframe::egui;
use egui::{Color32, FontFamily, FontId, RichText, Stroke};
use studio_parser::ViewGranularity;
use studio_ui::color_tokens::*;

use super::types::StudioViewMode;
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
                    // CodeSee Brand Mark
                    ui.label(
                        RichText::new(format!("{} STUDIO", egui_phosphor::regular::CODE))
                            .font(FontId::new(13.0, FontFamily::Proportional))
                            .color(Color32::from_rgb(99, 102, 241))
                            .strong(),
                    );

                    ui.separator();

                    // CodeSee Primary Mode Tabs
                    let is_maps = self.view_mode == StudioViewMode::CodebaseMaps;
                    let is_arch = self.view_mode == StudioViewMode::ArchitectureMap;
                    let is_flows = self.view_mode == StudioViewMode::FlowsAndTraces;

                    let maps_label = format!("{} Codebase Maps", egui_phosphor::regular::MAP_TRIFOLD);
                    if ui.selectable_label(is_maps, maps_label).clicked() {
                        self.set_view_mode(StudioViewMode::CodebaseMaps);
                    }
                    let arch_label = format!("{} Architecture Map", egui_phosphor::regular::GRAPH);
                    if ui.selectable_label(is_arch, arch_label).clicked() {
                        self.set_view_mode(StudioViewMode::ArchitectureMap);
                    }
                    let flows_label = format!("{} Flows & Traces", egui_phosphor::regular::LIGHTNING);
                    if ui.selectable_label(is_flows, flows_label).clicked() {
                        self.set_view_mode(StudioViewMode::FlowsAndTraces);
                    }

                    ui.separator();

                    // Enso Spotlight Search Launcher Button
                    let spot_label = format!("{} Spotlight / Add (Ctrl+K)", egui_phosphor::regular::MAGNIFYING_GLASS);
                    if ui.button(spot_label).clicked() {
                        self.spotlight_open = true;
                    }

                    ui.separator();

                    // Quick Project Actions
                    let open_label = format!("{} Open...", egui_phosphor::regular::FOLDER_OPEN);
                    if ui.button(open_label).clicked() {
                        self.open_folder_dialog();
                    }

                    let repo_label = format!("{} Repo", egui_phosphor::regular::GIT_BRANCH);
                    if ui.button(repo_label).clicked() {
                        if let Ok(dir) = std::env::current_dir() {
                            self.load_project(&dir);
                        }
                    }

                    let show_label = format!("{} Showcase", egui_phosphor::regular::ARROWS_CLOCKWISE);
                    if ui.button(show_label).clicked() {
                        self.load_showcase();
                    }

                    if self.current_project_path.is_some() {
                        let reparse_label = format!("{} Force Reparse", egui_phosphor::regular::LIGHTNING);
                        if ui.button(reparse_label)
                            .on_hover_text("Bypass user rkyv cache and re-parse all files from scratch")
                            .clicked()
                        {
                            self.force_reparse_current_project();
                        }
                    }

                    // View Granularity Dropdown
                    let current_granularity_label = match self.granularity {
                        ViewGranularity::FilesAndFolders => format!("{} Files & Folders", egui_phosphor::regular::FOLDER),
                        ViewGranularity::AllItems => format!("{} Items: All", egui_phosphor::regular::TREE_STRUCTURE),
                        ViewGranularity::PublicApi => format!("{} Items: Public API", egui_phosphor::regular::GLOBE),
                        ViewGranularity::Modules => format!("{} Items: Modules", egui_phosphor::regular::PUZZLE_PIECE),
                    };
                    egui::ComboBox::from_id_salt("top_granularity_selector")
                        .selected_text(current_granularity_label)
                        .show_ui(ui, |ui| {
                            let files_item = format!("{} Files & Folders (Default)", egui_phosphor::regular::FOLDER);
                            if ui.selectable_label(self.granularity == ViewGranularity::FilesAndFolders, files_item).clicked() {
                                self.set_granularity(ViewGranularity::FilesAndFolders);
                            }
                            let all_item = format!("{} All Items", egui_phosphor::regular::TREE_STRUCTURE);
                            if ui.selectable_label(self.granularity == ViewGranularity::AllItems, all_item).clicked() {
                                self.set_granularity(ViewGranularity::AllItems);
                            }
                            let pub_item = format!("{} Public API", egui_phosphor::regular::GLOBE);
                            if ui.selectable_label(self.granularity == ViewGranularity::PublicApi, pub_item).clicked() {
                                self.set_granularity(ViewGranularity::PublicApi);
                            }
                            let mod_item = format!("{} Modules & Functions", egui_phosphor::regular::PUZZLE_PIECE);
                            if ui.selectable_label(self.granularity == ViewGranularity::Modules, mod_item).clicked() {
                                self.set_granularity(ViewGranularity::Modules);
                            }
                        });

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

                    // Right-aligned sidebar toggles and FPS
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // FPS pill
                        let fps_color = if self.fps >= 55.0 { ARCHETYPE_EGRESS } else { ARCHETYPE_STATE };
                        ui.label(
                            RichText::new(format!("{:.0} FPS", self.fps))
                                .font(FontId::new(10.5, FontFamily::Monospace))
                                .color(fps_color),
                        );

                        ui.separator();

                        // Right Inspector toggle button
                        let insp_label = format!("{} Inspector", egui_phosphor::regular::SIDEBAR_SIMPLE);
                        if ui.selectable_label(self.right_inspector_open, insp_label).clicked() {
                            self.right_inspector_open = !self.right_inspector_open;
                        }

                        // Code Editor width toggle button
                        let code_toggle_label = format!("{} Split Editor", if self.code_editor_expanded { egui_phosphor::regular::COLUMNS } else { egui_phosphor::regular::CODE });
                        if ui.selectable_label(self.code_editor_expanded, code_toggle_label).clicked() {
                            self.code_editor_expanded = !self.code_editor_expanded;
                            if self.code_editor_expanded {
                                self.right_inspector_open = true;
                            }
                        }

                        // Left Sidebar toggle button
                        let left_label = format!("{} Filters", egui_phosphor::regular::FUNNEL);
                        if ui.selectable_label(self.left_sidebar_open, left_label).clicked() {
                            self.left_sidebar_open = !self.left_sidebar_open;
                        }

                        ui.separator();

                        // Project summary pill
                        if let Some(stats) = &self.project_stats {
                            ui.label(
                                RichText::new(format!("{} {} ({} crates, {} files)", egui_phosphor::regular::PACKAGE, stats.project_name, stats.crate_count, stats.file_count))
                                    .font(FontId::new(11.0, FontFamily::Proportional))
                                    .color(TEXT_SECONDARY),
                            );
                        } else {
                            ui.label(
                                RichText::new("Showcase Pipeline")
                                    .font(FontId::new(11.0, FontFamily::Monospace))
                                    .color(TEXT_DIM),
                            );
                        }
                    });
                });
            });
    }

    pub(crate) fn render_bottom_panel(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("studio_bottom_panel")
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin::symmetric(14, 5))
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    if let Some(msg) = &self.canvas_state.status_message {
                        ui.label(RichText::new(format!("● {}", msg)).font(FontId::new(11.0, FontFamily::Proportional)).color(ARCHETYPE_INGRESS));
                        ui.separator();
                    }

                    if self.is_from_cache {
                        ui.label(
                            RichText::new(format!("{} rkyv user cache: active", egui_phosphor::regular::LIGHTNING))
                                .font(FontId::new(10.5, FontFamily::Proportional))
                                .color(Color32::from_rgb(52, 211, 153)),
                        );
                        ui.separator();
                    }

                    ui.label(
                        RichText::new("Pan: [MMB / Space+LMB]  •  Zoom: [Mouse Wheel]  •  Connect: [Drag Pin]  •  Sever: [RMB Pin/Wire]  •  Spotlight: [Ctrl+K]  •  Save: [Ctrl+S]")
                            .font(FontId::new(10.5, FontFamily::Proportional))
                            .color(TEXT_DIM),
                    );
                });
            });
    }
}
