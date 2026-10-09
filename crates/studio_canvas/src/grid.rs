use egui::{
    epaint::{CircleShape, RectShape},
    Color32, Painter, Pos2, Rect, CornerRadius, Stroke,
};
use studio_ui::{color_tokens::*, with_alpha};

use crate::transform::CanvasTransform;

/// Paints an infinite dot-grid canvas background scaled and panned in world coordinates.
pub fn paint_infinite_grid(
    painter: &Painter,
    clip_rect: Rect,
    transform: &CanvasTransform,
) {
    // 1. Fill base canvas background
    painter.add(RectShape::new(
        clip_rect,
        CornerRadius::ZERO,
        CANVAS_BG,
        Stroke::NONE, egui::StrokeKind::Middle,
    ));

    // If zoomed out very far (galaxy overview), canvas background is sufficient
    if transform.zoom < 0.04 {
        return;
    }

    // Calculate grid spacing adaptively so screen dots never exceed ~3,000
    let base_spacing = 32.0;
    let mut step = base_spacing;
    while step * transform.zoom < 24.0 {
        step *= 4.0;
    }
    let alpha_scale = ((transform.zoom - 0.04) / 0.31).clamp(0.2, 1.0);

    // World-space visible bounds
    let world_min = transform.screen_to_world(clip_rect.min);
    let world_max = transform.screen_to_world(clip_rect.max);

    let start_i = (world_min.x / step).floor() as i64 - 1;
    let end_i = (world_max.x / step).ceil() as i64 + 1;
    let start_j = (world_min.y / step).floor() as i64 - 1;
    let end_j = (world_max.y / step).ceil() as i64 + 1;

    let minor_color = with_alpha(CANVAS_GRID_DOT_MINOR, (160.0 * alpha_scale) as u8);
    let major_color = with_alpha(CANVAS_GRID_DOT_MAJOR, (220.0 * alpha_scale) as u8);

    let dot_radius = (1.2 * transform.zoom.sqrt()).clamp(1.0, 2.0);

    for i in start_i..=end_i {
        let is_major_x = i % 4 == 0;
        let wx = i as f32 * step;

        for j in start_j..=end_j {
            let is_major = is_major_x && (j % 4 == 0);
            let wy = j as f32 * step;

            let screen_pos = transform.world_to_screen(Pos2::new(wx, wy));
            if !clip_rect.contains(screen_pos) {
                continue;
            }

            let (fill, r) = if is_major {
                (major_color, dot_radius * 1.3)
            } else {
                (minor_color, dot_radius)
            };

            painter.add(CircleShape {
                center: screen_pos,
                radius: r,
                fill,
                stroke: Stroke::NONE,
            });
        }
    }

    // Subtle origin indicator at (0, 0)
    let origin_screen = transform.world_to_screen(Pos2::ZERO);
    if clip_rect.contains(origin_screen) {
        painter.line_segment(
            [
                origin_screen + egui::vec2(-8.0, 0.0),
                origin_screen + egui::vec2(8.0, 0.0),
            ],
            Stroke::new(1.0, with_alpha(Color32::from_rgb(0x63, 0x66, 0xf1), 80)),
        );
        painter.line_segment(
            [
                origin_screen + egui::vec2(0.0, -8.0),
                origin_screen + egui::vec2(0.0, 8.0),
            ],
            Stroke::new(1.0, with_alpha(Color32::from_rgb(0x63, 0x66, 0xf1), 80)),
        );
    }
}
