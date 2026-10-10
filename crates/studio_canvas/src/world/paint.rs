//! Drawing the districts: frames and names, what each holds until it has content, the labels and
//! links that read from far away, and the veil that dims the districts the camera is not at.

use egui::{vec2, Align2, CornerRadius, FontId, Id, Painter, Pos2, Rect, Stroke, StrokeKind};
use studio_ui::color_tokens::*;
use studio_ui::with_alpha;

use super::WorldLayout;
use crate::camera::Stop;
use crate::transform::CanvasTransform;

/// Below this many screen points per local unit, districts show far labels instead of their
/// own text.
pub const FAR_LOCAL_ZOOM: f32 = 0.35;

/// The districts with a frame of their own; Code's folders are its frame.
const FRAMED: [Stop; 5] = [Stop::Pipeline, Stop::Files, Stop::Desk, Stop::Run, Stop::Changes];

fn screen_rect(transform: &CanvasTransform, world: Rect) -> Rect {
    Rect::from_min_max(transform.world_to_screen(world.min), transform.world_to_screen(world.max))
}

/// Frames every district but Code with its name and what it is for. `note` gives the line a
/// district shows while it has no content, e.g. "No git repository here".
pub fn paint_frames(
    painter: &Painter,
    world: &WorldLayout,
    transform: &CanvasTransform,
    screen: Rect,
    note: &dyn Fn(Stop) -> Option<String>,
) {
    let local = transform.zoom * world.scale;
    for stop in FRAMED {
        let r = screen_rect(transform, world.rect(stop));
        if !r.intersects(screen) {
            continue;
        }
        let color = stop.color();
        let radius = (26.0 * local).clamp(4.0, 40.0);
        painter.rect(
            r,
            CornerRadius::from(radius),
            with_alpha(color, 6),
            Stroke::new(1.0, with_alpha(color, 70)),
            StrokeKind::Inside,
        );
        if local < FAR_LOCAL_ZOOM {
            continue;
        }
        let text_size = |pt: f32| FontId::proportional((pt * local).min(160.0));
        let inset = vec2(36.0, 30.0) * local;
        painter.text(r.min + inset, Align2::LEFT_TOP, stop.label(), text_size(26.0), color);
        painter.text(
            r.min + inset + vec2(0.0, 38.0 * local),
            Align2::LEFT_TOP,
            stop.description(),
            text_size(15.0),
            TEXT_SECONDARY,
        );
        if let Some(text) = note(stop) {
            painter.text(r.center(), Align2::CENTER_CENTER, text, text_size(17.0), TEXT_DIM);
        }
    }
}

/// From far away: each district's name (and what it is, if there is room) at a constant size,
/// and the links between them. Code's name sits in the gap above the map.
pub fn paint_far_labels(painter: &Painter, world: &WorldLayout, transform: &CanvasTransform, screen: Rect) {
    if transform.zoom * world.scale >= FAR_LOCAL_ZOOM {
        return;
    }
    for stop in FRAMED {
        let r = screen_rect(transform, world.rect(stop));
        if !r.intersects(screen) || r.width() < 48.0 {
            continue;
        }
        let roomy = r.width() > 180.0 && r.height() > 64.0;
        let name_at = if roomy { r.center() - vec2(0.0, 9.0) } else { r.center() };
        painter.text(name_at, Align2::CENTER_CENTER, stop.label(), FontId::proportional(18.0), stop.color());
        if roomy {
            let description = studio_ui::truncate_with_ellipsis(stop.description(), (r.width() / 7.0) as usize);
            painter.text(
                r.center() + vec2(0.0, 12.0),
                Align2::CENTER_CENTER,
                description,
                FontId::proportional(12.0),
                TEXT_SECONDARY,
            );
        }
    }
    let code = screen_rect(transform, world.code_cell);
    let gap = code.top() - screen_rect(transform, world.pipeline).bottom();
    if gap >= 20.0 && code.intersects(screen) {
        let at = Pos2::new(code.center().x, code.top() - gap * 0.5);
        painter.text(at, Align2::CENTER_CENTER, Stop::Code.label(), FontId::proportional(16.0), Stop::Code.color());
    }
    paint_flow_hints(painter, world, transform);
}

/// "feeds →", "makes →", "↑ runs as", "↓ changes": how the districts relate, in the gaps between
/// them.
fn paint_flow_hints(painter: &Painter, world: &WorldLayout, transform: &CanvasTransform) {
    let at = |x: f32, y: f32| transform.world_to_screen(Pos2::new(x, y));
    let code = world.code_cell;
    let row = code.center().y;
    let hints = [
        (at((world.files.right() + code.left()) * 0.5, row), "feeds →"),
        (at((code.right() + world.run.left()) * 0.5, row), "makes →"),
        // Beside the Code label, which has the middle of that gap.
        (at(code.center().x, (world.pipeline.bottom() + code.top()) * 0.5) + vec2(84.0, 0.0), "↑ runs as"),
        (at(code.center().x, (world.desk.bottom() + world.changes.top()) * 0.5), "↓ changes"),
    ];
    let gap_px = studio_ui::FOLDER_TEXT_MIN_PX * 2.0;
    if (world.files.right() - code.left()).abs() * transform.zoom < gap_px {
        return;
    }
    for (pos, text) in hints {
        let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(11.0), TEXT_SECONDARY);
        let r = Rect::from_center_size(pos, galley.size() + vec2(12.0, 6.0));
        painter.rect(r, CornerRadius::from(7.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        painter.galley(r.min + vec2(6.0, 3.0), galley, TEXT_SECONDARY);
    }
}

/// Dims every district but `focus` (none for the world view), fading as the camera moves.
pub fn paint_veils(painter: &Painter, world: &WorldLayout, transform: &CanvasTransform, screen: Rect, focus: Stop) {
    for stop in Stop::DISTRICTS {
        let target = if focus == Stop::World || focus == stop { 0.0 } else { 1.0 };
        let shown = painter.ctx().animate_value_with_time(Id::new(("district_veil", stop)), target, 0.3);
        if shown <= 0.0 {
            continue;
        }
        let r = screen_rect(transform, world.rect(stop)).intersect(screen.expand(2.0));
        if r.is_positive() {
            painter.rect_filled(r, CornerRadius::ZERO, with_alpha(CANVAS_BG, (shown * 150.0) as u8));
        }
    }
}
