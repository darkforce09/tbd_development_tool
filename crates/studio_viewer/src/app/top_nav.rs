use eframe::egui;
use egui::containers::menu::{MenuBar, MenuConfig};
use egui::{FontFamily, FontId, PopupCloseBehavior, Rect, RichText, Stroke, Vec2};
use studio_ui::color_tokens::*;

use super::{view_menu, window_frame, StudioApp};

impl StudioApp {
    /// The integrated title bar: actions and menus on the left, the project name in the
    /// centre, window buttons on the right. Empty space drags the window.
    pub(crate) fn render_top_nav(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("studio_top_nav")
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin { left: 10, right: 4, top: 3, bottom: 3 })
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(root, |ui| {
                window_frame::title_bar_drag(ui);
                let bar = ui.max_rect();

                ui.horizontal(|ui| {
                    let search_label = format!("{} Search (Ctrl+K)", egui_phosphor::regular::MAGNIFYING_GLASS);
                    if ui.button(search_label).clicked() {
                        self.spotlight_open = true;
                    }

                    let open_label = format!("{} Open...", egui_phosphor::regular::FOLDER_OPEN);
                    if ui.button(open_label).clicked() {
                        self.open_folder_dialog();
                    }

                    ui.separator();

                    MenuBar::new()
                        .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
                        .ui(ui, |ui| view_menu::view_menu(ui, &mut self.canvas_state));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), window_frame::window_controls);
                });

                self.render_title(ui, bar);
            });
    }

    /// Project name centred in the bar. After a failed load it shows a warning, with the
    /// error on hover.
    /// The breadcrumb, centred in the bar: the project, then each folder down to the one in
    /// focus. Clicking a crumb closes everything opened inside it and fits it in view. After a
    /// failed load the project shows a warning, with the error on hover.
    fn render_title(&mut self, ui: &mut egui::Ui, bar: Rect) {
        let name = self.project_stats.as_ref().map_or("No project", |s| s.project_name.as_str()).to_string();
        let path: Vec<(String, String)> = self
            .canvas_state
            .focus
            .as_deref()
            .map(|id| self.graph.cluster_path(id))
            .unwrap_or_default()
            .into_iter()
            .skip(1)
            .filter_map(|id| self.graph.clusters.iter().find(|c| c.id == id).map(|c| (id, c.label.clone())))
            .collect();
        let font = FontId::new(12.5, FontFamily::Proportional);
        let title_rect = Rect::from_center_size(bar.center(), Vec2::new(bar.width() * 0.5, bar.height()));
        let mut picked: Option<Option<String>> = None;
        let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_align(egui::Align::Center);
        ui.scope_builder(egui::UiBuilder::new().max_rect(title_rect).layout(layout), |ui| {
            let (text, color) = match &self.load_error {
                Some(_) => (format!("{} {name}", egui_phosphor::regular::WARNING), ARCHETYPE_STATE),
                None => (name.clone(), if path.is_empty() { TEXT_PRIMARY } else { TEXT_SECONDARY }),
            };
            let project =
                ui.add(egui::Button::new(RichText::new(text).font(font.clone()).color(color).strong()).frame(false));
            if project.clicked() {
                picked = Some(None);
            }
            match (&self.load_error, &self.current_project_path) {
                (Some(err), _) => {
                    project.on_hover_text(format!("Load failed: {err}"));
                }
                (None, Some(p)) => {
                    project.on_hover_text(format!("{}\nClick to go back to the top level", p.display()));
                }
                (None, None) => {}
            }
            for (i, (id, label)) in path.iter().enumerate() {
                ui.label(RichText::new(egui_phosphor::regular::CARET_RIGHT).font(font.clone()).color(TEXT_DIM));
                let last = i + 1 == path.len();
                let color = if last { TEXT_PRIMARY } else { TEXT_SECONDARY };
                let crumb =
                    ui.add(egui::Button::new(RichText::new(label).font(font.clone()).color(color)).frame(false));
                if crumb.clicked() {
                    picked = Some(Some(id.clone()));
                }
            }
        });
        if let Some(target) = picked {
            self.canvas_state.focus_folder(&mut self.graph, target.as_deref());
        }
    }
}
