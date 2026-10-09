use crate::colors::*;
use egui::{epaint::RectShape, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};

pub struct FloatingToolbarLayout {
    pub center_btn: Rect,
    pub inspect_btn: Rect,
    pub code_toggle_btn: Rect,
    pub delete_btn: Rect,
}

/// Paints the CodeSee-style floating action toolbar attached below the selected node card.
pub fn paint_floating_toolbar(
    painter: &Painter,
    node_rect: Rect,
    zoom: f32,
    is_code_expanded: bool,
    hovered_pos: Option<Pos2>,
) -> FloatingToolbarLayout {
    let z = zoom.clamp(0.5, 1.6);
    let bar_height = 30.0 * z;
    let btn_width = 32.0 * z;
    let num_buttons = 4.0;
    let bar_width = (btn_width * num_buttons) + 12.0 * z;

    // Anchor toolbar floating centered below the node card
    let bar_top = node_rect.max.y + 8.0 * z;
    let bar_min_x = node_rect.center().x - (bar_width * 0.5);
    let bar_rect = Rect::from_min_size(Pos2::new(bar_min_x, bar_top), Vec2::new(bar_width, bar_height));

    // Drop shadow
    painter.rect(
        bar_rect.translate(Vec2::new(0.0, 3.0 * z)),
        CornerRadius::from(bar_height * 0.5),
        Color32::from_black_alpha(80),
        Stroke::NONE,
        egui::StrokeKind::Middle,
    );

    // Pill background
    painter.add(RectShape::new(
        bar_rect,
        CornerRadius::from(bar_height * 0.5),
        FLOATING_TOOLBAR_BG,
        Stroke::new((1.2 * z).max(1.0), FLOATING_TOOLBAR_BORDER),
        egui::StrokeKind::Middle,
    ));

    let font_id = FontId::new((13.0 * z).max(8.0), FontFamily::Proportional);
    let start_x = bar_rect.min.x + 6.0 * z;

    let center_btn =
        Rect::from_min_size(Pos2::new(start_x, bar_rect.min.y + 2.0 * z), Vec2::new(btn_width, bar_height - 4.0 * z));
    let inspect_btn = Rect::from_min_size(
        Pos2::new(start_x + btn_width, bar_rect.min.y + 2.0 * z),
        Vec2::new(btn_width, bar_height - 4.0 * z),
    );
    let code_toggle_btn = Rect::from_min_size(
        Pos2::new(start_x + btn_width * 2.0, bar_rect.min.y + 2.0 * z),
        Vec2::new(btn_width, bar_height - 4.0 * z),
    );
    let delete_btn = Rect::from_min_size(
        Pos2::new(start_x + btn_width * 3.0, bar_rect.min.y + 2.0 * z),
        Vec2::new(btn_width, bar_height - 4.0 * z),
    );

    // Helper to paint a button icon with hover effect
    let paint_btn = |rect: Rect, label: &str, text_color: Color32| {
        let is_hov = hovered_pos.map(|p| rect.contains(p)).unwrap_or(false);
        if is_hov {
            painter.rect_filled(rect, CornerRadius::from(4.0 * z), FLOATING_BTN_HOVER);
        }
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font_id.clone(),
            if is_hov { TEXT_HIGHLIGHT } else { text_color },
        );
    };

    paint_btn(center_btn, egui_phosphor::regular::EYE, TEXT_SECONDARY);
    paint_btn(inspect_btn, egui_phosphor::regular::SIDEBAR_SIMPLE, TEXT_SECONDARY);
    paint_btn(
        code_toggle_btn,
        if is_code_expanded { egui_phosphor::regular::ARROWS_IN } else { egui_phosphor::regular::ARROWS_OUT },
        if is_code_expanded { ARCHETYPE_COMPUTE } else { TEXT_SECONDARY },
    );
    paint_btn(delete_btn, egui_phosphor::regular::TRASH_SIMPLE, Color32::from_rgb(248, 113, 113));

    FloatingToolbarLayout { center_btn, inspect_btn, code_toggle_btn, delete_btn }
}
