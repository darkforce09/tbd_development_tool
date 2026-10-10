//! The title bar: the project and where you are on the left (project › district › folders), what
//! Studio is doing in the middle with the worktree and agent pills beside it, search and the View
//! menu on the right. Empty space drags the window. On macOS the native window buttons sit over
//! its left end; elsewhere Studio draws its own on the right.

use eframe::egui;
use egui::containers::menu::{MenuBar, MenuConfig};
use egui::{vec2, Align2, CornerRadius, FontId, PopupCloseBehavior, Rect, RichText, Sense, Stroke, StrokeKind};
use std::path::{Path, PathBuf};
use studio_canvas::{CameraTarget, Stop};
use studio_sources::{AgentIndex, GitHistory, StatusCounts};
use studio_ui::color_tokens::*;
use studio_ui::Pill;

use super::activity::{ActivityInputs, ActivityLine};
use super::commands::{run_command, AppCommand};
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
                let mut controls_left = bar.right();
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
                        let ticked = MenuBar::new()
                            .config(MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside))
                            .ui(ui, |ui| view_menu::view_menu(ui, &self.canvas_state))
                            .inner;
                        if let Some(i) = ticked {
                            run_command(self, AppCommand::ToggleView(i));
                        }
                        ui.add_space(6.0);
                        self.search_field(ui);
                        controls_left = ui.min_rect().left();
                    });
                });
                let activity = activity_box(ui, bar, &line);
                self.title_pills(ui, activity.right() + 8.0, controls_left - 8.0, bar.center().y);
            });
    }

    /// The worktree and agent pills, left to right from `left`; a pill that would reach past
    /// `right` is left out. A click flies to the Changes district.
    fn title_pills(&mut self, ui: &mut egui::Ui, left: f32, right: f32, center_y: f32) {
        let Some(sources) = &self.sources else { return };
        let pills = &sources.pills;
        let mut x = left;
        if let Some(worktree) = &pills.worktree {
            let pill = Pill::new(&worktree.text).dot(if worktree.clean { DISTRICT_RUN } else { ARCHETYPE_STATE });
            let rect = pill.rect_at(ui, x, center_y);
            if rect.right() <= right {
                x = rect.right() + 6.0;
                if pill
                    .show_at(ui, rect, egui::Id::new("title_pill_worktree"))
                    .on_hover_text(&worktree.tooltip)
                    .clicked()
                {
                    let target = self.worktree_row_target(&worktree.path);
                    self.canvas_state.fly_to(target);
                }
            } else {
                x = f32::INFINITY;
            }
        }
        if let Some(sessions) = &pills.sessions {
            let pill = Pill::new(&sessions.text).dot(DISTRICT_CHANGES);
            let rect = pill.rect_at(ui, x, center_y);
            if rect.right() <= right
                && pill
                    .show_at(ui, rect, egui::Id::new("title_pill_sessions"))
                    .on_hover_text(&sessions.tooltip)
                    .clicked()
            {
                self.canvas_state.fly_to(CameraTarget::Stop(Stop::Changes));
            }
        }
    }

    /// Where the camera goes for a worktree's pill: its row in the Changes district, or the
    /// district while it has no rows.
    fn worktree_row_target(&self, path: &Path) -> CameraTarget {
        let Some(view) = &self.canvas_state.districts.changes else { return CameraTarget::Stop(Stop::Changes) };
        let row = view.rows.iter().position(|r| r.path == path).or_else(|| view.rows.iter().position(|r| r.is_main));
        match row {
            Some(row) => CameraTarget::Rect(studio_canvas::districts::changes::row_world_rect(
                &self.canvas_state.world,
                view,
                row,
            )),
            None => CameraTarget::Stop(Stop::Changes),
        }
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

impl StudioApp {
    /// The search field, in its fixed 200×26 slot so the pills never move. Closed, it is a button
    /// that opens the palette; open, a text field in the same place, focused, whose Enter, arrows
    /// and Escape the palette has already taken.
    fn search_field(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(vec2(200.0, 26.0), Sense::click());
        self.palette.field_rect = Some(rect);
        let open = self.palette.open;
        let hovered = response.hovered() && !open;
        let fill = if hovered { FLOATING_BTN_HOVER } else { FLOATING_TOOLBAR_BG };
        let border = if open { CARD_BORDER_SELECTED } else { FLOATING_TOOLBAR_BORDER };
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::from(8.0), fill, Stroke::new(1.0, border), StrokeKind::Inside);
        let text = if hovered || open { TEXT_PRIMARY } else { TEXT_DIM };
        let left = rect.left_center() + vec2(10.0, 0.0);
        painter.text(
            left,
            Align2::LEFT_CENTER,
            egui_phosphor::regular::MAGNIFYING_GLASS,
            FontId::proportional(13.0),
            text,
        );
        if open {
            let field = Rect::from_min_max(rect.min + vec2(30.0, 4.0), rect.max - vec2(8.0, 4.0));
            let edit = ui.put(
                field,
                egui::TextEdit::singleline(&mut self.palette.text)
                    .id(egui::Id::new(SEARCH_FIELD_ID))
                    .hint_text("Go to anything")
                    .frame(egui::Frame::NONE)
                    .font(FontId::proportional(12.5))
                    .text_color(TEXT_PRIMARY)
                    .return_key(None)
                    .desired_width(field.width()),
            );
            // The field keeps the keyboard while the palette is open.
            edit.request_focus();
            return;
        }
        painter.text(left + vec2(20.0, 0.0), Align2::LEFT_CENTER, "Go to anything", FontId::proportional(12.5), text);
        let shortcut = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl K" };
        painter.text(
            rect.right_center() - vec2(10.0, 0.0),
            Align2::RIGHT_CENTER,
            shortcut,
            FontId::proportional(11.0),
            TEXT_DIM,
        );
        if response.on_hover_text("Files, symbols, commands and more (/ or ⌘K)").clicked() {
            self.open_palette();
        }
    }
}

/// The search field's text edit.
pub const SEARCH_FIELD_ID: &str = "palette_field";

/// The activity line, centred in the bar, with a thin progress bar under it while loading.
/// Returns where it is.
fn activity_box(ui: &mut egui::Ui, bar: Rect, line: &ActivityLine) -> Rect {
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
    rect
}

/// What the title bar's pills say, built when git or the agents index arrives (and again when
/// the day ends), never per frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TitlePills {
    /// The project's worktree: "main · 3 changes"; `None` until the history arrives.
    pub worktree: Option<WorktreePill>,
    /// "4 sessions today"; `None` without an index or with no session today.
    pub sessions: Option<SessionsPill>,
    /// When "today" ends, unix ms.
    pub valid_until_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorktreePill {
    pub text: String,
    pub tooltip: String,
    pub clean: bool,
    /// The worktree's folder, to find its row in the Changes district.
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionsPill {
    pub text: String,
    pub tooltip: String,
}

impl TitlePills {
    /// The pills for the project at `root` at `now_ms`, the local time being `offset_secs` ahead
    /// of UTC. The worktree is the one the project lies in (the main checkout when none holds it).
    pub fn build(
        root: &Path,
        history: Option<&GitHistory>,
        agents: Option<&AgentIndex>,
        now_ms: i64,
        offset_secs: i64,
    ) -> Self {
        let midnight = midnight_ms(now_ms, offset_secs);
        let worktree = history.and_then(|history| {
            let worktree = super::sources::project_worktree(root, &history.worktrees)
                .map(|(w, _)| w)
                .or_else(|| history.worktrees.iter().find(|w| w.is_main))?;
            let short: String = worktree.head.chars().take(7).collect();
            let branch = worktree.branch.clone().unwrap_or_else(|| format!("detached at {short}"));
            let total = worktree.changes.total();
            Some(WorktreePill {
                text: worktree_pill_text(&branch, total),
                tooltip: format!("{}\n{}", worktree.path.display(), change_counts_text(&worktree.changes)),
                clean: total == 0,
                path: worktree.path.clone(),
            })
        });
        let sessions = agents.and_then(|agents| {
            let today = agents.sessions.iter().filter(|s| s.ended >= midnight).count();
            let text = sessions_pill_text(today)?;
            let tooltip = format!("{today} of {} sessions ended today (local time)", agents.sessions.len());
            Some(SessionsPill { text, tooltip })
        });
        Self { worktree, sessions, valid_until_ms: midnight + DAY_MS }
    }
}

const DAY_MS: i64 = 86_400_000;

/// "main · 1 change", "main · 3 changes", "main · no changes".
pub fn worktree_pill_text(branch: &str, changes: usize) -> String {
    match changes {
        0 => format!("{branch} · no changes"),
        1 => format!("{branch} · 1 change"),
        n => format!("{branch} · {n} changes"),
    }
}

/// "1 session today", "4 sessions today"; nothing for none.
pub fn sessions_pill_text(today: usize) -> Option<String> {
    match today {
        0 => None,
        1 => Some("1 session today".to_string()),
        n => Some(format!("{n} sessions today")),
    }
}

/// Every count `git status` gave, the four main ones always.
fn change_counts_text(c: &StatusCounts) -> String {
    let mut parts = vec![
        format!("{} modified", c.modified),
        format!("{} added", c.added),
        format!("{} deleted", c.deleted),
        format!("{} untracked", c.untracked),
    ];
    if c.renamed > 0 {
        parts.push(format!("{} renamed", c.renamed));
    }
    if c.conflicted > 0 {
        parts.push(format!("{} conflicted", c.conflicted));
    }
    parts.join(" · ")
}

/// The start of the local day holding `now_ms`, unix ms, the local time being `offset_secs`
/// ahead of UTC.
pub fn midnight_ms(now_ms: i64, offset_secs: i64) -> i64 {
    let offset = offset_secs * 1000;
    (now_ms + offset).div_euclid(DAY_MS) * DAY_MS - offset
}

/// How far local time is ahead of UTC at unix time `secs`, in seconds (0 where unknown).
pub fn local_offset_secs(secs: i64) -> i64 {
    #[cfg(unix)]
    {
        let time = secs as libc::time_t;
        // SAFETY: `localtime_r` writes only into `tm`, which lives on this stack frame.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if !libc::localtime_r(&time, &mut tm).is_null() {
                return tm.tm_gmtoff as i64;
            }
        }
    }
    let _ = secs;
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_sources::{Session, Worktree};

    #[test]
    fn pill_texts_count_in_words() {
        assert_eq!(worktree_pill_text("main", 0), "main · no changes");
        assert_eq!(worktree_pill_text("main", 1), "main · 1 change");
        assert_eq!(worktree_pill_text("v4-s6", 13), "v4-s6 · 13 changes");
        assert_eq!(sessions_pill_text(0), None);
        assert_eq!(sessions_pill_text(1).as_deref(), Some("1 session today"));
        assert_eq!(sessions_pill_text(4).as_deref(), Some("4 sessions today"));
    }

    #[test]
    fn midnight_is_local() {
        let day = DAY_MS;
        let now = 20_000 * day + 3_600_000; // 01:00 UTC
        assert_eq!(midnight_ms(now, 0), 20_000 * day);
        // Two hours ahead: 03:00 local, the local day began at 22:00 UTC the day before.
        assert_eq!(midnight_ms(now, 7_200), 20_000 * day - 7_200_000);
        // Five hours behind: 20:00 local the day before.
        assert_eq!(midnight_ms(now, -18_000), 19_999 * day + 18_000_000);
    }

    fn session(ended: i64) -> Session {
        Session {
            id: format!("s{ended}"),
            slug: None,
            root: 0,
            branches: Vec::new(),
            started: ended - 1,
            ended,
            logs: Vec::new(),
            plan: None,
            tasks: Vec::new(),
            touch_count: 0,
        }
    }

    #[test]
    fn pills_show_the_project_worktree_and_todays_sessions() {
        let day = DAY_MS;
        let now = 20_000 * day + 10 * 3_600_000;
        let worktree = |path: &str, branch: Option<&str>, is_main: bool, modified: usize| Worktree {
            path: PathBuf::from(path),
            head: "abcdef0123".to_string() + &"0".repeat(30),
            branch: branch.map(str::to_string),
            is_main,
            changes: StatusCounts { modified, untracked: 1, ..Default::default() },
            files: Vec::new(),
        };
        let history = GitHistory {
            worktrees: vec![worktree("/r", Some("main"), true, 2), worktree("/wt/s6", None, false, 0)],
            ..Default::default()
        };
        let agents = AgentIndex {
            sessions: vec![session(now - day), session(20_000 * day), session(now)],
            ..Default::default()
        };
        let pills = TitlePills::build(Path::new("/r"), Some(&history), Some(&agents), now, 0);
        let w = pills.worktree.unwrap();
        assert_eq!(w.text, "main · 3 changes");
        assert_eq!(w.tooltip, "/r\n2 modified · 0 added · 0 deleted · 1 untracked");
        assert!(!w.clean);
        assert_eq!(pills.sessions.unwrap().text, "2 sessions today");
        assert_eq!(pills.valid_until_ms, 20_001 * day);

        let pills = TitlePills::build(Path::new("/wt/s6/app"), Some(&history), None, now, 0);
        assert_eq!(pills.worktree.unwrap().text, "detached at abcdef0 · 1 change");
        assert_eq!(pills.sessions, None);
        assert_eq!(TitlePills::build(Path::new("/r"), None, None, now, 0).worktree, None);
    }
}
