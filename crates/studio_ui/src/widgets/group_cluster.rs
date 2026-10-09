use crate::colors::*;
use egui::{epaint::RectShape, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};

pub struct GroupClusterProps<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub category: &'a str,
    pub subtitle: Option<&'a str>,
    pub color_index: usize,
    pub item_count: usize,
    pub is_collapsed: bool,
    pub depth: usize,
    pub zoom: f32,
    /// Paint the frame (background, borders). False when the GPU already drew it.
    pub frame: bool,
}

pub struct GroupClusterLayout {
    pub collapse_button_rect: Rect,
}

/// Fill and border colours of a folder: its tint, a little stronger for deeper folders so they
/// stand out from their parent.
pub fn cluster_tint(color_index: usize, depth: usize) -> (Color32, Color32) {
    let (base_fill, base_stroke) = CLUSTER_TINTS[color_index % CLUSTER_TINTS.len()];
    let boost = (depth.min(255) as u8).saturating_mul(15);
    let fill = Color32::from_rgba_premultiplied(
        base_fill.r().saturating_add(boost / 2),
        base_fill.g().saturating_add(boost / 2),
        base_fill.b().saturating_add(boost / 2),
        base_fill.a().saturating_add(boost),
    );
    (fill, base_stroke)
}

/// Paints a CodeSee-style cluster/folder container behind grouped nodes and subfolders.
/// Supports collapsible states, nested hierarchy depth, subtitles, and child count badges.
pub fn paint_group_cluster(painter: &Painter, props: GroupClusterProps<'_>) -> GroupClusterLayout {
    let z = props.zoom;
    let (fill_tint, border_stroke_color) = cluster_tint(props.color_index, props.depth);
    let rounding = CornerRadius::from(10.0 * z);

    if props.is_collapsed {
        // Collapsed Mode: Render sleek compact folder pill
        let pill_rect = props.rect;
        if props.frame {
            painter.rect(
                pill_rect.translate(Vec2::new(0.0, 2.0 * z)),
                CornerRadius::from(8.0 * z),
                Color32::from_black_alpha(70),
                Stroke::NONE,
                egui::StrokeKind::Middle,
            );

            painter.add(RectShape::new(
                pill_rect,
                CornerRadius::from(8.0 * z),
                CLUSTER_HEADER_BG,
                Stroke::new((1.2 * z).max(1.0), border_stroke_color),
                egui::StrokeKind::Middle,
            ));
        }

        // Chevron ▸ + Folder Icon + Label
        let font_size = (11.5 * z).max(6.0);
        let chevron_rect = Rect::from_min_size(pill_rect.min, Vec2::new(28.0 * z, pill_rect.height()));
        painter.text(
            chevron_rect.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::regular::CARET_RIGHT,
            FontId::new(font_size * 1.1, FontFamily::Proportional),
            TEXT_PRIMARY,
        );

        let label_text = format!("{}  {}", egui_phosphor::regular::FOLDER, props.label);
        let subtitle = props.subtitle.filter(|s| !s.is_empty());
        let label_y =
            if subtitle.is_some() { pill_rect.top() + pill_rect.height() * 0.34 } else { pill_rect.center().y };
        let label_pos = Pos2::new(chevron_rect.max.x + 4.0 * z, label_y);
        painter.text(
            label_pos,
            egui::Align2::LEFT_CENTER,
            label_text,
            FontId::new(font_size, FontFamily::Proportional),
            TEXT_PRIMARY,
        );

        // Folder totals (e.g. "48,213 files · 312 MB · gitignored") on a second line
        if let Some(subtitle) = subtitle {
            painter.text(
                Pos2::new(label_pos.x, pill_rect.top() + pill_rect.height() * 0.72),
                egui::Align2::LEFT_CENTER,
                crate::truncate_with_ellipsis(subtitle, 40),
                FontId::new((9.0 * z).max(5.0), FontFamily::Monospace),
                TEXT_DIM,
            );
            return GroupClusterLayout { collapse_button_rect: pill_rect };
        }

        // Count pill
        let count_text = format!("{} items", props.item_count);
        let count_pos = Pos2::new(pill_rect.max.x - 10.0 * z, pill_rect.center().y);
        painter.text(
            count_pos,
            egui::Align2::RIGHT_CENTER,
            count_text,
            FontId::new((9.5 * z).max(5.0), FontFamily::Monospace),
            TEXT_DIM,
        );

        return GroupClusterLayout { collapse_button_rect: pill_rect };
    }

    // Expanded Mode: Full container with header tab
    // 1. Shaded background container
    if props.frame {
        painter.add(RectShape::new(
            props.rect,
            rounding,
            fill_tint,
            Stroke::new((1.5 * z).clamp(1.0, 3.0), border_stroke_color),
            egui::StrokeKind::Middle,
        ));
    }

    // 2. Top Folder Header Tab
    let has_subtitle = props.subtitle.map(|s| !s.is_empty()).unwrap_or(false);
    let header_height = if has_subtitle { (42.0 * z).max(24.0) } else { (32.0 * z).max(18.0) };

    let header_width = (props.rect.width() * 0.75).clamp(200.0 * z, 420.0 * z).min(props.rect.width());

    let header_rect = Rect::from_min_size(props.rect.min, Vec2::new(header_width, header_height));

    let tab_rounding =
        CornerRadius { nw: (10.0 * z).round() as u8, ne: (6.0 * z).round() as u8, sw: 0, se: (8.0 * z).round() as u8 };

    if props.frame {
        painter.add(RectShape::new(
            header_rect,
            tab_rounding,
            CLUSTER_HEADER_BG,
            Stroke::new((1.0 * z).max(0.75), border_stroke_color),
            egui::StrokeKind::Middle,
        ));
    }

    // Chevron ▾ Button
    let btn_w = 22.0 * z;
    let collapse_button_rect = Rect::from_min_size(header_rect.min, Vec2::new(btn_w, header_height));

    let font_size = (11.5 * z).max(6.0);
    painter.text(
        collapse_button_rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::CARET_DOWN,
        FontId::new(font_size * 1.1, FontFamily::Proportional),
        TEXT_PRIMARY,
    );

    // Folder Icon + Label
    let label_pos = if has_subtitle {
        Pos2::new(header_rect.min.x + btn_w + 4.0 * z, header_rect.min.y + 12.0 * z)
    } else {
        Pos2::new(header_rect.min.x + btn_w + 4.0 * z, header_rect.center().y)
    };

    let label_display = format!("{}  {}", egui_phosphor::regular::FOLDER, props.label);
    painter.text(
        label_pos,
        egui::Align2::LEFT_CENTER,
        label_display,
        FontId::new(font_size, FontFamily::Proportional),
        TEXT_PRIMARY,
    );

    // Subtitle (CodeSee style description below folder tab)
    if let Some(subtitle) = props.subtitle {
        if !subtitle.is_empty() {
            let sub_pos = Pos2::new(header_rect.min.x + btn_w + 4.0 * z, header_rect.min.y + 28.0 * z);
            let sub_chars = ((header_width * 0.8) / (6.0 * z).max(2.0)) as usize;
            painter.text(
                sub_pos,
                egui::Align2::LEFT_CENTER,
                crate::truncate_with_ellipsis(subtitle, sub_chars.max(10)),
                FontId::new((9.0 * z).max(5.0), FontFamily::Proportional),
                TEXT_DIM,
            );
        }
    }

    // Pill badge for category & count
    let pill_text = format!("{} • {}", props.category, props.item_count);
    let pill_font_size = (9.5 * z).max(5.0);
    let pill_pos = Pos2::new(header_rect.max.x - 10.0 * z, header_rect.min.y + 16.0 * z);

    painter.text(
        pill_pos,
        egui::Align2::RIGHT_CENTER,
        pill_text,
        FontId::new(pill_font_size, FontFamily::Monospace),
        TEXT_DIM,
    );

    GroupClusterLayout { collapse_button_rect }
}
