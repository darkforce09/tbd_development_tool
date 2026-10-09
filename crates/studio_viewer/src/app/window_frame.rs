//! Client-side window frame: the window has no OS decorations, so the title bar moves and
//! maximises the window, draws its own window buttons, and the window edges resize it.

use eframe::egui;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, PointerButton, Pos2, Rect, ResizeDirection, Sense,
    Vec2, ViewportCommand,
};
use studio_ui::color_tokens::*;

/// Width of the grab zone along each window edge, in points.
const RESIZE_MARGIN: f32 = 5.0;
/// Corner zones are larger so diagonal resizing is easy to hit.
const RESIZE_CORNER: f32 = 12.0;

const CLOSE_HOVER: Color32 = Color32::from_rgb(196, 43, 28);

fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn is_fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

/// Makes the empty parts of the title bar drag the window and double-click toggle maximise.
/// Call before adding the bar's widgets so the widgets win the hit test.
pub fn title_bar_drag(ui: &mut egui::Ui) {
    let bar = ui.interact(ui.max_rect(), ui.id().with("title_bar_drag"), Sense::click_and_drag());
    if bar.double_clicked() {
        let maximized = is_maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    } else if bar.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Minimise, maximise/restore and close buttons. Expects a right-to-left layout, so close
/// ends up rightmost.
pub fn window_controls(ui: &mut egui::Ui) {
    let maximized = is_maximized(ui.ctx());
    if frame_button(ui, egui_phosphor::regular::X, "Close", CLOSE_HOVER) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }
    let (icon, tip) = if maximized {
        (egui_phosphor::regular::CORNERS_IN, "Restore")
    } else {
        (egui_phosphor::regular::SQUARE, "Maximise")
    };
    if frame_button(ui, icon, tip, FLOATING_BTN_HOVER) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if frame_button(ui, egui_phosphor::regular::MINUS, "Minimise", FLOATING_BTN_HOVER) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn frame_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, hover_fill: Color32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(36.0, 26.0), Sense::click());
    let response = response.on_hover_text(tooltip);
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::from(4.0), hover_fill);
    }
    let color = if response.hovered() { TEXT_HIGHLIGHT } else { TEXT_SECONDARY };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::new(14.0, FontFamily::Proportional), color);
    response.clicked()
}

/// Which edge or corner of `window` the pointer is over, if any.
pub fn resize_direction(window: Rect, pos: Pos2) -> Option<ResizeDirection> {
    if !window.contains(pos) {
        return None;
    }
    let near = |d: f32, zone: f32| d <= zone;
    let (left, right) = (pos.x - window.min.x, window.max.x - pos.x);
    let (top, bottom) = (pos.y - window.min.y, window.max.y - pos.y);

    let corner = |a: f32, b: f32| near(a, RESIZE_CORNER) && near(b, RESIZE_CORNER);
    if corner(top, left) {
        return Some(ResizeDirection::NorthWest);
    }
    if corner(top, right) {
        return Some(ResizeDirection::NorthEast);
    }
    if corner(bottom, left) {
        return Some(ResizeDirection::SouthWest);
    }
    if corner(bottom, right) {
        return Some(ResizeDirection::SouthEast);
    }
    if near(top, RESIZE_MARGIN) {
        Some(ResizeDirection::North)
    } else if near(bottom, RESIZE_MARGIN) {
        Some(ResizeDirection::South)
    } else if near(left, RESIZE_MARGIN) {
        Some(ResizeDirection::West)
    } else if near(right, RESIZE_MARGIN) {
        Some(ResizeDirection::East)
    } else {
        None
    }
}

fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// Lets the window edges resize the window. Call last in the frame so its cursor wins.
pub fn resize_edges(ctx: &egui::Context) {
    if is_maximized(ctx) || is_fullscreen(ctx) {
        return;
    }
    let (hover, pressed) = ctx.input(|i| (i.pointer.hover_pos(), i.pointer.primary_pressed()));
    let Some(direction) = hover.and_then(|pos| resize_direction(ctx.viewport_rect(), pos)) else { return };
    ctx.set_cursor_icon(resize_cursor(direction));
    if pressed {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners_map_to_resize_directions() {
        let w = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        assert_eq!(resize_direction(w, Pos2::new(500.0, 400.0)), None);
        assert_eq!(resize_direction(w, Pos2::new(500.0, 2.0)), Some(ResizeDirection::North));
        assert_eq!(resize_direction(w, Pos2::new(500.0, 798.0)), Some(ResizeDirection::South));
        assert_eq!(resize_direction(w, Pos2::new(1.0, 400.0)), Some(ResizeDirection::West));
        assert_eq!(resize_direction(w, Pos2::new(999.0, 400.0)), Some(ResizeDirection::East));
        assert_eq!(resize_direction(w, Pos2::new(4.0, 4.0)), Some(ResizeDirection::NorthWest));
        assert_eq!(resize_direction(w, Pos2::new(995.0, 3.0)), Some(ResizeDirection::NorthEast));
        assert_eq!(resize_direction(w, Pos2::new(3.0, 795.0)), Some(ResizeDirection::SouthWest));
        assert_eq!(resize_direction(w, Pos2::new(998.0, 790.0)), Some(ResizeDirection::SouthEast));
        assert_eq!(resize_direction(w, Pos2::new(1200.0, 400.0)), None);
    }
}
