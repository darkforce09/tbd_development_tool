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
    fn render_title(&self, ui: &mut egui::Ui, bar: Rect) {
        let name = self.project_stats.as_ref().map_or("No project", |s| s.project_name.as_str());
        let (text, color) = match &self.load_error {
            Some(_) => (format!("{} {name}", egui_phosphor::regular::WARNING), ARCHETYPE_STATE),
            None => (name.to_string(), TEXT_PRIMARY),
        };
        let title_rect = Rect::from_center_size(bar.center(), Vec2::new(bar.width() * 0.3, bar.height()));
        let label = ui.put(
            title_rect,
            egui::Label::new(
                RichText::new(text).font(FontId::new(12.5, FontFamily::Proportional)).color(color).strong(),
            )
            .selectable(false),
        );
        match (&self.load_error, &self.current_project_path) {
            (Some(err), _) => {
                label.on_hover_text(format!("Load failed: {err}"));
            }
            (None, Some(path)) => {
                label.on_hover_text(path.display().to_string());
            }
            (None, None) => {}
        }
    }
}
