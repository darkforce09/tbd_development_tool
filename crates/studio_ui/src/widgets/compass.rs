//! The compass: one button per district, laid out the way the districts lie on the map, with the
//! one the camera is at lit in its colour, and the zoom underneath.

use egui::{vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::colors::*;

/// A button of the compass.
#[derive(Debug, Clone, Copy)]
pub struct CompassStop<'a> {
    pub label: &'a str,
    pub color: Color32,
    /// Column and row in the compass grid.
    pub cell: [u8; 2],
    /// Shown on hover.
    pub tooltip: &'a str,
}

/// What was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompassClick {
    /// The stop at this index.
    Stop(usize),
    /// The zoom readout: back to 100%.
    ActualSize,
}

const CELL: Vec2 = vec2(66.0, 22.0);
const SPACING: f32 = 4.0;

/// Draws the compass; `current` is the index of the stop the camera is at, `zoom_percent` the
/// zoom to show under it.
pub fn compass(
    ui: &mut Ui,
    stops: &[CompassStop<'_>],
    current: Option<usize>,
    zoom_percent: f32,
) -> Option<CompassClick> {
    let cols = stops.iter().map(|s| s.cell[0] as usize + 1).max().unwrap_or(1);
    let rows = stops.iter().map(|s| s.cell[1] as usize + 1).max().unwrap_or(1);
    let grid = vec2(cols as f32 * (CELL.x + SPACING) - SPACING, rows as f32 * (CELL.y + SPACING) - SPACING);
    let padding = 8.0;
    let zoom_h = 16.0;
    let (frame, _) = ui.allocate_exact_size(grid + vec2(padding * 2.0, padding * 2.0 + zoom_h), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        frame,
        CornerRadius::from(12.0),
        with_alpha(FLOATING_TOOLBAR_BG, 236),
        Stroke::new(1.0, FLOATING_TOOLBAR_BORDER),
        StrokeKind::Inside,
    );

    let mut clicked = None;
    for (i, stop) in stops.iter().enumerate() {
        let min = frame.min
            + vec2(padding, padding)
            + vec2(stop.cell[0] as f32 * (CELL.x + SPACING), stop.cell[1] as f32 * (CELL.y + SPACING));
        let rect = Rect::from_min_size(min, CELL);
        let response = ui.interact(rect, ui.id().with(("compass", i)), Sense::click()).on_hover_text(stop.tooltip);
        let active = current == Some(i);
        let fill = if active {
            with_alpha(stop.color, 56)
        } else if response.hovered() {
            FLOATING_BTN_HOVER
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::from(7.0), fill, Stroke::NONE, StrokeKind::Inside);
        let dot = rect.left_center() + vec2(9.0, 0.0);
        painter.circle_filled(dot, 3.0, stop.color);
        let text = if active || response.hovered() { TEXT_HIGHLIGHT } else { TEXT_SECONDARY };
        painter.text(dot + vec2(8.0, 0.0), Align2::LEFT_CENTER, stop.label, FontId::proportional(11.5), text);
        if response.clicked() {
            clicked = Some(CompassClick::Stop(i));
        }
    }

    let zoom_rect = Rect::from_min_max(
        frame.left_bottom() + vec2(padding, -padding - zoom_h),
        frame.right_bottom() - vec2(padding, padding),
    );
    let zoom = ui.interact(zoom_rect, ui.id().with("compass_zoom"), Sense::click()).on_hover_text("Back to 100% (0)");
    let label = if zoom_percent < 1.0 { format!("{zoom_percent:.2}%") } else { format!("{zoom_percent:.0}%") };
    let color = if zoom.hovered() { TEXT_HIGHLIGHT } else { TEXT_DIM };
    ui.painter().text(zoom_rect.center(), Align2::CENTER_CENTER, label, FontId::monospace(10.5), color);
    if zoom.clicked() {
        clicked = Some(CompassClick::ActualSize);
    }
    clicked
}
