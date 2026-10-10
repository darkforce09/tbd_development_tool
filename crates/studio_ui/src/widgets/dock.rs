//! The Dock at the bottom of the window: Build, Test and Trace, the pinned tools, then All tools
//! and Add a tool. Icons grow as the pointer nears them, the way the macOS Dock does, and each
//! says on hover exactly what it runs.

use egui::{vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui};

use crate::colors::*;

/// One icon of the Dock.
#[derive(Debug, Clone)]
pub struct DockItem {
    pub icon: &'static str,
    pub label: String,
    /// What it does, e.g. the exact command line.
    pub tooltip: String,
    pub color: Color32,
    /// A thin divider before this item.
    pub divider_before: bool,
    /// Something it started is still running.
    pub running: bool,
}

/// Size of an icon at rest, in points.
pub const ICON: f32 = 38.0;
/// How much the icon under the pointer grows.
const MAGNIFY: f32 = 0.5;
/// How far the growth reaches, in points.
const REACH: f32 = 70.0;
const SPACING: f32 = 8.0;
const DIVIDER: f32 = 14.0;

/// How much an icon `distance` points from the pointer grows: a bell curve, 1.5x on top of it.
pub fn magnification(distance: f32) -> f32 {
    1.0 + MAGNIFY * (-(distance / REACH).powi(2)).exp()
}

/// Draws the Dock and returns the index of the item clicked.
pub fn dock(ui: &mut Ui, items: &[DockItem]) -> Option<usize> {
    // Where each icon sits at rest, to measure the pointer against (so growing never feeds back).
    let rest: Vec<f32> = {
        let mut x = 0.0;
        items
            .iter()
            .map(|item| {
                if item.divider_before {
                    x += DIVIDER;
                }
                let centre = x + ICON * 0.5;
                x += ICON + SPACING;
                centre
            })
            .collect()
    };
    let rest_width = rest.last().map_or(0.0, |c| c + ICON * 0.5) + 2.0 * 12.0;
    let origin = ui.cursor().min;
    let pointer = ui.ctx().pointer_hover_pos();
    let band = Rect::from_min_size(origin - vec2(0.0, ICON), vec2(rest_width, ICON * 2.6 + 16.0));
    let near = pointer.filter(|p| band.contains(*p));
    let scales: Vec<f32> =
        rest.iter().map(|c| near.map_or(1.0, |p| magnification((p.x - (origin.x + 12.0 + c)).abs()))).collect();
    let width: f32 = scales.iter().map(|s| ICON * s + SPACING).sum::<f32>()
        + items.iter().filter(|i| i.divider_before).count() as f32 * DIVIDER
        + 24.0;
    let height = ICON + 22.0;
    let (frame, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    ui.painter().rect(
        frame,
        CornerRadius::from(18.0),
        with_alpha(FLOATING_TOOLBAR_BG, 240),
        Stroke::new(1.0, FLOATING_TOOLBAR_BORDER),
        StrokeKind::Inside,
    );

    let mut clicked = None;
    let mut x = frame.min.x + 12.0;
    for (i, (item, scale)) in items.iter().zip(&scales).enumerate() {
        if item.divider_before {
            let line_x = x + DIVIDER * 0.5 - SPACING * 0.5;
            ui.painter().line_segment(
                [egui::pos2(line_x, frame.min.y + 12.0), egui::pos2(line_x, frame.max.y - 12.0)],
                Stroke::new(1.0, FLOATING_TOOLBAR_BORDER),
            );
            x += DIVIDER;
        }
        let size = ICON * scale;
        // Icons stand on the Dock's floor and grow upwards, out of it.
        let rect = Rect::from_min_size(egui::pos2(x, frame.max.y - 8.0 - size), vec2(size, size));
        let response = ui.interact(rect, ui.id().with(("dock", i)), Sense::click());
        let hovered = response.hovered();
        let response = response.on_hover_ui_at_pointer(|ui| {
            ui.label(egui::RichText::new(&item.label).strong());
            if !item.tooltip.is_empty() {
                ui.label(egui::RichText::new(&item.tooltip).monospace().color(TEXT_SECONDARY));
            }
        });
        let painter = ui.painter();
        let fill = if hovered { with_alpha(item.color, 70) } else { with_alpha(item.color, 34) };
        painter.rect(
            rect,
            CornerRadius::from(10.0 * scale),
            fill,
            Stroke::new(1.0, with_alpha(item.color, 110)),
            StrokeKind::Inside,
        );
        painter.text(rect.center(), Align2::CENTER_CENTER, item.icon, FontId::proportional(18.0 * scale), item.color);
        if item.running {
            painter.circle_filled(egui::pos2(rect.center().x, frame.max.y - 3.0), 2.0, TEXT_HIGHLIGHT);
        }
        if response.clicked() {
            clicked = Some(i);
        }
        x += size + SPACING;
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_under_the_pointer_grows_most_and_far_ones_not_at_all() {
        assert!((magnification(0.0) - 1.5).abs() < 1e-6);
        assert!(magnification(40.0) > magnification(80.0));
        assert!((magnification(400.0) - 1.0).abs() < 1e-3);
    }
}
