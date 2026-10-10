//! The keyboard-shortcut sheet: every row of the SHORTCUTS table, by group, centred over the
//! canvas. Escape or a click outside closes it.

use eframe::egui;
use egui::{Align2, CornerRadius, Id, Order, RichText, Stroke};
use studio_canvas::shortcuts::{shortcut_text, ShortcutGroup, SHORTCUTS};
use studio_ui::color_tokens::*;

use super::StudioApp;

/// The sheet's rows, by group in the sheet's order: the keys as the platform spells them, and
/// what they do. Groups with no rows are left out.
pub fn sheet_rows(mac: bool) -> Vec<(ShortcutGroup, Vec<(String, &'static str)>)> {
    ShortcutGroup::ALL
        .into_iter()
        .map(|group| {
            let rows = SHORTCUTS
                .iter()
                .filter(|s| s.group == group)
                .map(|s| (shortcut_text(s, mac), s.label))
                .collect::<Vec<_>>();
            (group, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

impl StudioApp {
    /// The sheet, while it is open. A click outside it closes it, except on the frame it opens.
    pub(crate) fn render_shortcut_sheet(&mut self, ctx: &egui::Context) {
        let shown = Id::new("shortcut_sheet_shown");
        let was_shown = ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<bool>(shown), false));
        if !self.shortcut_sheet_open {
            return;
        }
        let area = egui::Area::new(Id::new("shortcut_sheet"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(FLOATING_TOOLBAR_BG)
                    .stroke(Stroke::new(1.0, FLOATING_TOOLBAR_BORDER))
                    .corner_radius(CornerRadius::from(12.0))
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, sheet_contents);
            });
        if was_shown && area.response.clicked_elsewhere() {
            self.shortcut_sheet_open = false;
        }
        ctx.data_mut(|d| d.insert_temp(shown, self.shortcut_sheet_open));
    }
}

fn sheet_contents(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Keyboard shortcuts").size(15.0).strong().color(TEXT_PRIMARY));
        ui.add_space(24.0);
        ui.label(RichText::new("Esc closes").size(11.0).color(TEXT_DIM));
    });
    ui.add_space(8.0);
    for (group, rows) in sheet_rows(cfg!(target_os = "macos")) {
        ui.add_space(6.0);
        ui.label(RichText::new(group.label()).size(11.0).strong().color(TEXT_DIM));
        egui::Grid::new(("shortcut_sheet", group)).num_columns(2).spacing([18.0, 5.0]).show(ui, |ui| {
            for (keys, label) in rows {
                ui.label(RichText::new(keys).monospace().color(TEXT_PRIMARY));
                ui.label(RichText::new(label).color(TEXT_SECONDARY));
                ui.end_row();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Rect, Vec2};
    use studio_canvas::CanvasView;

    #[test]
    fn the_sheet_lists_every_row_in_its_group() {
        let rows = sheet_rows(false);
        assert_eq!(rows.iter().map(|(_, r)| r.len()).sum::<usize>(), SHORTCUTS.len());
        let groups: Vec<ShortcutGroup> = rows.iter().map(|(g, _)| *g).collect();
        assert_eq!(groups, ShortcutGroup::ALL);
        assert_eq!(rows[0].1[0], ("Ctrl K".to_string(), "Search"));
        assert_eq!(sheet_rows(true)[0].1[0].0, "⌘K");
    }

    /// Runs frames of the canvas with the sheet over it, the way the app draws them.
    fn frames(app: &mut StudioApp, ctx: &egui::Context, time: &mut f64, events: Vec<egui::Event>, idle: usize) {
        for events in std::iter::once(events).chain((0..idle).map(|_| Vec::new())) {
            *time += 1.0 / 60.0;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                time: Some(*time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                app.handle_shortcuts(ui.ctx());
                CanvasView::new(&mut app.canvas_state, &mut app.graph).show(ui);
                app.render_shortcut_sheet(ui.ctx());
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_wheel_over_the_sheet_never_zooms_the_map_and_a_click_outside_closes_it() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut app = StudioApp::for_test();
        app.canvas_state.use_gpu_wires = false;
        app.shortcut_sheet_open = true;
        let middle = Pos2::new(600.0, 400.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        };
        frames(&mut app, &ctx, &mut time, vec![egui::Event::PointerMoved(middle)], 3);
        let zoom = app.canvas_state.transform.zoom;
        frames(&mut app, &ctx, &mut time, vec![wheel.clone()], 30);
        assert_eq!(app.canvas_state.transform.zoom, zoom, "the sheet took the wheel");
        assert!(app.shortcut_sheet_open);

        let outside = Pos2::new(40.0, 760.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: outside,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frames(&mut app, &ctx, &mut time, vec![egui::Event::PointerMoved(outside)], 1);
        frames(&mut app, &ctx, &mut time, vec![button(true)], 0);
        frames(&mut app, &ctx, &mut time, vec![button(false)], 2);
        assert!(!app.shortcut_sheet_open, "a click outside closes it");

        frames(&mut app, &ctx, &mut time, vec![egui::Event::PointerMoved(middle)], 2);
        frames(&mut app, &ctx, &mut time, vec![wheel], 30);
        assert!(app.canvas_state.transform.zoom > zoom, "with the sheet gone, the map zooms");
    }
}
