use crate::colors::*;
use egui::{epaint::RectShape, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};

pub struct GroupClusterProps<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub category: &'a str,
    pub subtitle: Option<&'a str>,
    pub color_index: usize,
    pub item_count: usize,
    /// Detail level (docs/VISUAL_LANGUAGE.md L5): 0 minimised (a pill), 1 node view, 2 open.
    pub detail: usize,
    pub depth: usize,
    pub zoom: f32,
    /// Paint the frame (background, borders). False when the GPU already drew it.
    pub frame: bool,
    /// Paint the name, totals and count at their zoomed size. False when the name is drawn as a
    /// map label instead (see [`paint_folder_map_label`]).
    pub text: bool,
}

pub struct GroupClusterLayout {
    /// The three detail-level buttons (minimised, node view, open); `Rect::NOTHING` when the
    /// header is too small on screen to show them.
    pub detail_buttons: [Rect; 3],
}

/// Icons of the detail-level buttons: minimised, node view, open.
pub const DETAIL_ICONS: [&str; 3] = [
    egui_phosphor::regular::FOLDER_SIMPLE_MINUS,
    egui_phosphor::regular::LIST_BULLETS,
    egui_phosphor::regular::FOLDER_OPEN,
];

/// Paints the three detail-level buttons right-aligned in `header`, the current one highlighted.
/// Returns their rects, or `Rect::NOTHING` when the header is too small to hold them and a label.
fn paint_detail_control(painter: &Painter, header: Rect, detail: usize, z: f32) -> [Rect; 3] {
    let size = (header.height() * 0.66).clamp(10.0, 24.0);
    let gap = 2.0;
    let width = 3.0 * size + 2.0 * gap;
    let margin = (6.0 * z).clamp(3.0, 8.0);
    if header.height() < 14.0 || header.width() < width + margin + 60.0 {
        return [Rect::NOTHING; 3];
    }
    let mut rects = [Rect::NOTHING; 3];
    for (i, icon) in DETAIL_ICONS.iter().enumerate() {
        let x = header.max.x - margin - width + i as f32 * (size + gap);
        let r = Rect::from_min_size(Pos2::new(x, header.center().y - size * 0.5), Vec2::splat(size));
        let selected = i == detail;
        painter.rect(
            r,
            CornerRadius::from(3.0),
            if selected { CARD_BORDER_SELECTED } else { Color32::from_white_alpha(10) },
            Stroke::NONE,
            egui::StrokeKind::Middle,
        );
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            *icon,
            FontId::new(size * 0.72, FontFamily::Proportional),
            if selected { TEXT_PRIMARY } else { TEXT_DIM },
        );
        rects[i] = r;
    }
    rects
}

/// Width the detail control takes from the right of a header, for placing text left of it.
fn detail_control_width(buttons: &[Rect; 3]) -> f32 {
    if buttons[0] == Rect::NOTHING {
        0.0
    } else {
        buttons[2].max.x - buttons[0].min.x + 6.0
    }
}

/// Fill and border colours of a folder: a faint tint, a little stronger for deeper folders so
/// they stand out from their parent. The border carries the colour; the fill stays quiet so the
/// names and wires on top read clearly.
pub fn cluster_tint(color_index: usize, depth: usize) -> (Color32, Color32) {
    let (base_fill, base_stroke) = CLUSTER_TINTS[color_index % CLUSTER_TINTS.len()];
    let boost = (depth.min(255) as u8).saturating_mul(6);
    let fill = Color32::from_rgba_premultiplied(
        base_fill.r().saturating_add(boost / 2),
        base_fill.g().saturating_add(boost / 2),
        base_fill.b().saturating_add(boost / 2),
        base_fill.a().saturating_add(boost),
    );
    (fill, base_stroke)
}

/// Paints a folder: a pill when minimised, otherwise a container with a header tab. The header
/// carries the folder name, totals and the three detail-level buttons (L5).
pub fn paint_group_cluster(painter: &Painter, props: GroupClusterProps<'_>) -> GroupClusterLayout {
    let z = props.zoom;
    let (fill_tint, border_stroke_color) = cluster_tint(props.color_index, props.depth);
    let rounding = CornerRadius::from(10.0 * z);

    if props.detail == 0 {
        // Minimised: a compact folder pill
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

        let detail_buttons = paint_detail_control(painter, pill_rect, props.detail, z);
        let reserved = detail_control_width(&detail_buttons);
        if !props.text {
            return GroupClusterLayout { detail_buttons };
        }

        // Folder icon + label
        let font_size = (11.5 * z).max(6.0);
        let chevron_rect = Rect::from_min_size(pill_rect.min, Vec2::new(10.0 * z, pill_rect.height()));
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
            return GroupClusterLayout { detail_buttons };
        }

        // Count pill
        let count_text = format!("{} items", props.item_count);
        let count_pos = Pos2::new(pill_rect.max.x - 10.0 * z - reserved, pill_rect.center().y);
        painter.text(
            count_pos,
            egui::Align2::RIGHT_CENTER,
            count_text,
            FontId::new((9.5 * z).max(5.0), FontFamily::Monospace),
            TEXT_DIM,
        );

        return GroupClusterLayout { detail_buttons };
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

    let btn_w = 10.0 * z;
    let font_size = (11.5 * z).max(6.0);
    let control_row = Rect::from_min_size(header_rect.min, Vec2::new(header_width, (32.0 * z).max(18.0)));
    let detail_buttons = paint_detail_control(painter, control_row, props.detail, z);
    let reserved = detail_control_width(&detail_buttons);
    if !props.text {
        return GroupClusterLayout { detail_buttons };
    }

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
    let pill_pos = Pos2::new(header_rect.max.x - 10.0 * z - reserved, header_rect.min.y + 16.0 * z);

    painter.text(
        pill_pos,
        egui::Align2::RIGHT_CENTER,
        pill_text,
        FontId::new(pill_font_size, FontFamily::Monospace),
        TEXT_DIM,
    );

    GroupClusterLayout { detail_buttons }
}

/// Screen size below which a folder's zoomed name is too small to read; the map label takes
/// over.
pub const FOLDER_TEXT_MIN_PX: f32 = 10.0;

/// A folder's name at a constant, readable screen size, like a label on a map: larger for
/// folders higher in the tree, centred on a closed folder and at the top left of an open one,
/// shortened to fit the folder's width. Nothing is drawn when even a short name would not fit.
pub fn paint_folder_map_label(painter: &Painter, rect: Rect, label: &str, depth: usize, closed: bool) {
    let size = match depth {
        0 => 18.0,
        1 => 15.0,
        _ => 13.0,
    };
    let chars = ((rect.width() - 16.0) / (size * 0.55)).floor() as usize;
    if chars < 4 || rect.height() < size * 0.9 {
        return;
    }
    let text = crate::truncate_with_ellipsis(label, chars);
    let font = FontId::new(size, FontFamily::Proportional);
    let (pos, align) = if closed {
        (rect.center(), egui::Align2::CENTER_CENTER)
    } else {
        (rect.min + Vec2::new(8.0, 6.0), egui::Align2::LEFT_TOP)
    };
    // A dark halo keeps the name readable over wires.
    for offset in [Vec2::new(1.0, 1.0), Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(-1.0, 1.0)] {
        painter.text(pos + offset, align, &text, font.clone(), Color32::from_black_alpha(200));
    }
    painter.text(pos, align, text, font, TEXT_PRIMARY);
}
