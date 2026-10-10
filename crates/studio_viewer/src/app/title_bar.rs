//! The title bar: the project and where you are on the left (project › district › folders), what
//! Studio is doing in the middle, search and the View menu on the right. Empty space drags the
//! window. On macOS the native window buttons sit over its left end; elsewhere Studio draws its
//! own on the right.

use eframe::egui;
use egui::containers::menu::{MenuBar, MenuConfig};
use egui::{vec2, Align2, CornerRadius, FontId, PopupCloseBehavior, Rect, RichText, Sense, Stroke, StrokeKind};
use studio_canvas::{CameraTarget, Stop};
use studio_ui::color_tokens::*;

use super::activity::{ActivityInputs, ActivityLine};
use super::{view_menu, window_frame, StudioApp};

/// Height of the bar, in points.
pub const TITLE_BAR_HEIGHT: f32 = 40.0;
/// Room left for the native window buttons on macOS.
const MACOS_BUTTONS_WIDTH: f32 = 76.0;

impl StudioApp {
    pub(crate) fn render_title_bar(&mut self, root: &mut egui::Ui) {
        let line = self.activity_line(root.ctx());
        egui::Panel::top("studio_title_bar")
            .exact_size(TITLE_BAR_HEIGHT)
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .inner_margin(egui::Margin { left: 10, right: 4, top: 0, bottom: 0 })
                    .stroke(Stroke::new(1.0, PANEL_BORDER)),
            )
            .show(root, |ui| {
                window_frame::title_bar_drag(ui);
                let bar = ui.max_rect();
                ui.horizontal_centered(|ui| {
                    if cfg!(target_os = "macos") {
                        ui.add_space(MACOS_BUTTONS_WIDTH);
                    }
                    self.breadcrumb(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !cfg!(target_os = "macos") {
                            window_frame::window_controls(ui);
                            ui.add_space(6.0);
                        }
                        MenuBar::new()
                            .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
                            .ui(ui, |ui| view_menu::view_menu(ui, &mut self.canvas_state));
                        ui.add_space(6.0);
                        if search_field(ui).clicked() {
                            self.spotlight_open = true;
                            self.spotlight_search.clear();
                        }
                    });
                });
                activity_box(ui, bar, &line);
            });
    }

    /// What the activity line says this frame.
    fn activity_line(&mut self, ctx: &egui::Context) -> ActivityLine {
        let folders: Vec<String> = self.folder_loads.iter().map(|l| l.label.clone()).collect();
        let packages = self.project_stats.as_ref().map(|s| (s.file_count, s.crate_count));
        self.activity.line(ActivityInputs {
            now: ctx.input(|i| i.time),
            loading: self.is_loading.then_some((
                self.loading_stage.as_str(),
                self.loading_files_done,
                self.loading_total_files,
                self.loading_progress,
            )),
            folder_loads: &folders,
            message: self.canvas_state.status_message.as_deref(),
            error: self.load_error.as_deref(),
            project: packages,
        })
    }

    /// Project › district › folders. The project goes to the world view (its menu opens another
    /// folder); a district goes to that district; a folder closes what is open inside it.
    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let font = FontId::proportional(13.0);
        let name = self.project_stats.as_ref().map_or("Studio", |s| s.project_name.as_str()).to_string();
        let color = if self.load_error.is_some() { ARCHETYPE_STATE } else { TEXT_PRIMARY };
        let project =
            ui.add(egui::Button::new(RichText::new(&name).font(font.clone()).color(color).strong()).frame(false));
        let hover = match (&self.load_error, &self.current_project_path) {
            (Some(err), _) => format!("Could not open: {err}"),
            (None, Some(path)) => format!("{}\nClick for the whole project (Home)", path.display()),
            (None, None) => "Open a folder to start".to_string(),
        };
        if project.on_hover_text(hover).clicked() {
            self.canvas_state.fly_to(CameraTarget::Stop(Stop::World));
        }
        ui.menu_button(RichText::new(egui_phosphor::regular::CARET_DOWN).color(TEXT_DIM), |ui| {
            if ui.button(format!("{} Open folder…", egui_phosphor::regular::FOLDER_OPEN)).clicked() {
                ui.close();
                self.open_folder_dialog();
            }
        });

        let stop = self.canvas_state.stop;
        if stop == Stop::World {
            return;
        }
        let caret = |ui: &mut egui::Ui| {
            ui.label(
                RichText::new(egui_phosphor::regular::CARET_RIGHT).font(FontId::proportional(11.0)).color(TEXT_DIM),
            );
        };
        caret(ui);
        let district = ui.add(
            egui::Button::new(RichText::new(format!("● {}", stop.label())).font(font.clone()).color(stop.color()))
                .frame(false),
        );
        if district.on_hover_text(stop.description()).clicked() {
            self.canvas_state.fly_to(CameraTarget::Stop(stop));
        }
        if stop != Stop::Code {
            return;
        }
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
        let mut picked = None;
        for (i, (id, label)) in path.iter().enumerate() {
            caret(ui);
            let color = if i + 1 == path.len() { TEXT_PRIMARY } else { TEXT_SECONDARY };
            if ui.add(egui::Button::new(RichText::new(label).font(font.clone()).color(color)).frame(false)).clicked() {
                picked = Some(id.clone());
            }
        }
        if let Some(id) = picked {
            self.canvas_state.focus_folder(&mut self.graph, Some(&id));
        }
    }
}

/// A button that looks like a search field and opens the search.
fn search_field(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(200.0, 26.0), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    let fill = if hovered { FLOATING_BTN_HOVER } else { FLOATING_TOOLBAR_BG };
    painter.rect(rect, CornerRadius::from(8.0), fill, Stroke::new(1.0, FLOATING_TOOLBAR_BORDER), StrokeKind::Inside);
    let text = if hovered { TEXT_PRIMARY } else { TEXT_DIM };
    let left = rect.left_center() + vec2(10.0, 0.0);
    painter.text(left, Align2::LEFT_CENTER, egui_phosphor::regular::MAGNIFYING_GLASS, FontId::proportional(13.0), text);
    painter.text(left + vec2(20.0, 0.0), Align2::LEFT_CENTER, "Search", FontId::proportional(12.5), text);
    let shortcut = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl K" };
    painter.text(
        rect.right_center() - vec2(10.0, 0.0),
        Align2::RIGHT_CENTER,
        shortcut,
        FontId::proportional(11.0),
        TEXT_DIM,
    );
    response.on_hover_text("Search files and symbols (/ or ⌘K)")
}

/// The activity line, centred in the bar, with a thin progress bar under it while loading.
fn activity_box(ui: &mut egui::Ui, bar: Rect, line: &ActivityLine) {
    let width = (bar.width() * 0.3).clamp(240.0, 440.0);
    let rect = Rect::from_center_size(bar.center(), vec2(width, 26.0));
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::from(8.0),
        FLOATING_TOOLBAR_BG,
        Stroke::new(1.0, PANEL_BORDER),
        StrokeKind::Inside,
    );
    let color = if line.warning { ARCHETYPE_STATE } else { TEXT_SECONDARY };
    let max_chars = ((width - 40.0) / 6.5) as usize;
    let text = studio_ui::truncate_with_ellipsis(&line.text, max_chars);
    painter.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.0), color);
    if line.busy {
        let spinner = Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(12.0, 12.0));
        egui::Spinner::new().size(12.0).color(TEXT_DIM).paint_at(ui, spinner);
    }
    if let Some(progress) = line.progress {
        let track = Rect::from_min_max(rect.left_bottom() + vec2(8.0, -3.0), rect.right_bottom() - vec2(8.0, 1.0));
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::from(1.0), PANEL_BORDER);
        let done = Rect::from_min_max(track.min, egui::pos2(track.min.x + track.width() * progress, track.max.y));
        painter.rect_filled(done, CornerRadius::from(1.0), Stop::Code.color());
    }
}
