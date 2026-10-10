//! A pill: a short rounded label with an optional coloured dot, the way macOS shows status in a
//! toolbar. It lights up under the pointer and answers clicks.

use egui::{
    vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui,
};

use crate::colors::*;

/// Height of a pill, the same as the title bar's activity box.
pub const PILL_HEIGHT: f32 = 26.0;
const PADDING: f32 = 11.0;
const DOT: f32 = 7.0;
const DOT_GAP: f32 = 7.0;

/// A rounded label with an optional coloured dot before it.
#[derive(Debug, Clone, Copy)]
pub struct Pill<'a> {
    pub text: &'a str,
    pub dot: Option<Color32>,
}

impl<'a> Pill<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, dot: None }
    }

    /// Puts a dot of `color` before the text.
    pub fn dot(mut self, color: Color32) -> Self {
        self.dot = Some(color);
        self
    }

    fn font() -> FontId {
        FontId::proportional(12.0)
    }

    /// The pill's width for its text, in points.
    pub fn width(&self, ui: &Ui) -> f32 {
        let text = ui.painter().layout_no_wrap(self.text.to_string(), Self::font(), TEXT_SECONDARY).size().x;
        let dot = if self.dot.is_some() { DOT + DOT_GAP } else { 0.0 };
        (PADDING * 2.0 + dot + text).ceil()
    }

    /// Draws the pill in `rect` (see [`Pill::width`] and [`PILL_HEIGHT`]) and returns its
    /// response: hover and click.
    pub fn show_at(self, ui: &mut Ui, rect: Rect, id: Id) -> Response {
        let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let hovered = response.hovered();
        let painter = ui.painter();
        let fill = if hovered { FLOATING_BTN_HOVER } else { FLOATING_TOOLBAR_BG };
        let radius = CornerRadius::from(rect.height() * 0.5);
        painter.rect(rect, radius, fill, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        let mut x = rect.left() + PADDING;
        if let Some(color) = self.dot {
            painter.circle_filled(egui::pos2(x + DOT * 0.5, rect.center().y), DOT * 0.5, color);
            x += DOT + DOT_GAP;
        }
        let color = if hovered { TEXT_PRIMARY } else { TEXT_SECONDARY };
        painter.text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, self.text, Self::font(), color);
        response
    }

    /// The pill's rectangle with its left edge at `left`, centred on `center_y`.
    pub fn rect_at(&self, ui: &Ui, left: f32, center_y: f32) -> Rect {
        Rect::from_min_size(egui::pos2(left, center_y - PILL_HEIGHT * 0.5), vec2(self.width(ui), PILL_HEIGHT))
    }
}
