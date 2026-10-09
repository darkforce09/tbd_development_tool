use egui::{
    epaint::RectShape,
    Color32, FontFamily, FontId, Painter, Pos2, Rect, CornerRadius, Stroke, Vec2,
};

use crate::colors::*;

/// Truncates a string to at most `max_chars`, appending "..." if truncated.
pub fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else if max_chars <= 3 {
        s.chars().take(max_chars).collect()
    } else {
        let mut truncated: String = s.chars().take(max_chars - 3).collect();
        truncated.push_str("...");
        truncated
    }
}

/// Visual parameters for rendering a node card container.
pub struct CardFrameProps<'a> {
    pub rect: Rect,
    pub zoom: f32,
    pub title: &'a str,
    pub archetype_label: &'a str,
    pub archetype_color: Color32,
    pub description: &'a str,
    pub badge: Option<&'a str>,
    pub is_selected: bool,
    pub is_hovered: bool,
}

/// Paints the card container, header accent bar, title, badge, and description,
/// scaling all typography and dimensions proportionally with `props.zoom`.
pub fn paint_card_frame(painter: &Painter, props: CardFrameProps<'_>) {
    let z = props.zoom;
    let rounding = CornerRadius::from(8.0 * z);

    // Drop shadow
    painter.rect(
        props.rect.translate(Vec2::new(0.0, 3.0 * z)),
        rounding,
        Color32::from_black_alpha(70),
        Stroke::NONE, egui::StrokeKind::Middle,
    );

    // Selection glow
    if props.is_selected {
        painter.add(RectShape::new(
            props.rect.expand(2.5 * z),
            CornerRadius::from(9.5 * z),
            Color32::TRANSPARENT,
            Stroke::new(1.8 * z, with_alpha(CARD_BORDER_SELECTED, 180)), egui::StrokeKind::Middle,
        ));
    }

    // Card background fill and border
    let bg_fill = if props.is_hovered {
        CARD_BG_HOVER
    } else {
        CARD_BG
    };

    let border_stroke = if props.is_selected {
        Stroke::new((1.5 * z).max(1.0), CARD_BORDER_SELECTED)
    } else if props.is_hovered {
        Stroke::new((1.2 * z).max(1.0), CARD_BORDER_HOVER)
    } else {
        Stroke::new((1.0 * z).max(0.75), CARD_BORDER_NORMAL)
    };

    painter.add(RectShape::new(props.rect, rounding, bg_fill, border_stroke, egui::StrokeKind::Middle));

    // Scissored painter to ensure nothing ever bleeds outside the card boundaries
    let card_painter = painter.with_clip_rect(props.rect);

    // Top accent indicator bar along top curved edge
    let header_height = 34.0 * z;
    let header_rect = Rect::from_min_size(
        props.rect.min,
        Vec2::new(props.rect.width(), header_height),
    );

    // Header background
    let header_rounding = CornerRadius {
        nw: (8.0 * z).round() as u8,
        ne: (8.0 * z).round() as u8,
        sw: 0,
        se: 0,
    };
    card_painter.add(RectShape::new(
        header_rect,
        header_rounding,
        CARD_HEADER_BG,
        Stroke::NONE, egui::StrokeKind::Middle,
    ));

    // Archetype accent line at very top
    let accent_rect = Rect::from_min_size(props.rect.min, Vec2::new(props.rect.width(), 3.0 * z));
    card_painter.add(RectShape::new(
        accent_rect,
        CornerRadius {
            nw: (8.0 * z).round() as u8,
            ne: (8.0 * z).round() as u8,
            sw: 0,
            se: 0,
        },
        props.archetype_color,
        Stroke::NONE, egui::StrokeKind::Middle,
    ));

    // Archetype mini pill badge (top-left)
    let badge_rect = Rect::from_min_size(
        props.rect.min + Vec2::new(10.0 * z, 9.0 * z),
        Vec2::new(54.0 * z, 16.0 * z),
    );
    card_painter.add(RectShape::new(
        badge_rect,
        CornerRadius::from(3.0 * z),
        with_alpha(props.archetype_color, 40),
        Stroke::new((1.0 * z).max(0.75), with_alpha(props.archetype_color, 120)), egui::StrokeKind::Middle,
    ));
    card_painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        props.archetype_label,
        FontId::new((9.0 * z).max(3.0), FontFamily::Monospace),
        props.archetype_color,
    );

    // Optional status badge (top right)
    let right_badge_width = if let Some(badge_text) = props.badge {
        let text_chars = badge_text.chars().count() as f32;
        let badge_w = (text_chars * 6.5 + 14.0) * z;
        let badge_tag_pos = Pos2::new(props.rect.max.x - 10.0 * z, props.rect.min.y + 17.0 * z);
        card_painter.text(
            badge_tag_pos,
            egui::Align2::RIGHT_CENTER,
            badge_text,
            FontId::new((9.5 * z).max(3.0), FontFamily::Monospace),
            TEXT_DIM,
        );
        badge_w + 12.0 * z
    } else {
        12.0 * z
    };

    // Node Title (strictly clipped between the pill badge and right badge)
    let title_left = props.rect.min.x + 70.0 * z;
    let title_max_right = props.rect.max.x - right_badge_width;
    let available_title_width = (title_max_right - title_left).max(10.0);
    let max_title_chars = ((available_title_width / (7.2 * z)) as usize).max(4);

    let title_clip = Rect::from_min_max(
        Pos2::new(title_left, props.rect.min.y),
        Pos2::new(title_max_right, props.rect.min.y + header_height),
    );
    let title_painter = painter.with_clip_rect(title_clip);
    title_painter.text(
        Pos2::new(title_left, props.rect.min.y + 17.0 * z),
        egui::Align2::LEFT_CENTER,
        truncate_with_ellipsis(props.title, max_title_chars),
        FontId::new((12.0 * z).max(3.5), FontFamily::Proportional),
        TEXT_PRIMARY,
    );

    // Divider line between header and body
    let divider_y = props.rect.min.y + header_height;
    card_painter.line_segment(
        [
            Pos2::new(props.rect.min.x, divider_y),
            Pos2::new(props.rect.max.x, divider_y),
        ],
        Stroke::new((1.0 * z).max(0.75), CARD_BORDER_NORMAL),
    );

    // Description Banner: dedicated row below the header
    if !props.description.is_empty() {
        let desc_height = 24.0 * z;
        let desc_rect = Rect::from_min_size(
            Pos2::new(props.rect.min.x, divider_y),
            Vec2::new(props.rect.width(), desc_height),
        );

        // Shaded background for description area
        card_painter.rect_filled(
            desc_rect,
            CornerRadius::ZERO,
            Color32::from_black_alpha(35),
        );

        // Divider line below description banner separating it from ports
        let desc_bottom_y = divider_y + desc_height;
        card_painter.line_segment(
            [
                Pos2::new(props.rect.min.x, desc_bottom_y),
                Pos2::new(props.rect.max.x, desc_bottom_y),
            ],
            Stroke::new((1.0 * z).max(0.75), with_alpha(CARD_BORDER_NORMAL, 100)),
        );

        // Clipped description text with ellipsis
        let desc_text_clip = desc_rect.shrink2(Vec2::new(10.0 * z, 0.0));
        let desc_painter = painter.with_clip_rect(desc_text_clip);
        let max_desc_chars = ((desc_text_clip.width() / (6.0 * z)) as usize).max(4);
        desc_painter.text(
            Pos2::new(desc_text_clip.min.x, desc_rect.center().y),
            egui::Align2::LEFT_CENTER,
            truncate_with_ellipsis(props.description, max_desc_chars),
            FontId::new((10.0 * z).max(3.0), FontFamily::Proportional),
            TEXT_SECONDARY,
        );
    }
}
