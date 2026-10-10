//! Who owns the pointer and the keyboard this frame.
//!
//! The canvas reacts to the pointer only while it is the top-most widget under it: a window,
//! menu or panel drawn over the canvas takes the pointer, and so does a press or drag that
//! started on another widget. Keys reach the canvas only while no widget has keyboard focus, so
//! typing in a text field never pans, zooms or changes the map.

use egui::{Context, Pos2, Response, Vec2};

/// The pointer and keys the canvas may use this frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputGate {
    /// The pointer, while the canvas owns it: over the canvas with nothing on top, or in a press
    /// or drag that started on the canvas.
    pub pointer: Option<Pos2>,
    /// No widget has keyboard focus, so keys are for the canvas.
    pub keyboard: bool,
    /// A primary click on the canvas ended this frame.
    pub clicked: bool,
    /// The second primary click of a double click ended this frame.
    pub double_clicked: bool,
    /// A secondary (right) click on the canvas ended this frame.
    pub secondary_clicked: bool,
    /// The canvas is being dragged, with any button.
    pub dragging: bool,
    /// How far the drag moved this frame, in points. The frame a drag starts it covers the whole
    /// way from the press, so the point grabbed stays under the pointer.
    pub drag_delta: Vec2,
}

impl InputGate {
    /// Reads the gate from the canvas widget's response, after it was allocated this frame.
    pub fn new(ctx: &Context, response: &Response) -> Self {
        let dragging = response.dragged();
        let owns_pointer = response.hovered() || dragging;
        let (pointer, press_origin) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.press_origin()));
        let drag_delta = match (dragging, response.drag_started(), pointer, press_origin) {
            (true, true, Some(now), Some(origin)) => now - origin,
            (true, _, _, _) => response.drag_delta(),
            _ => Vec2::ZERO,
        };
        Self {
            pointer: if owns_pointer { pointer } else { None },
            keyboard: !ctx.egui_wants_keyboard_input(),
            clicked: response.clicked(),
            double_clicked: response.double_clicked(),
            secondary_clicked: response.secondary_clicked(),
            dragging,
            drag_delta,
        }
    }

    /// Where a primary click on the canvas landed this frame.
    pub fn click(&self) -> Option<Pos2> {
        self.pointer.filter(|_| self.clicked)
    }

    /// Whether the pointer is over `rect`, as far as the canvas can tell.
    pub fn hovers(&self, rect: egui::Rect) -> bool {
        self.pointer.is_some_and(|p| rect.contains(p))
    }
}
