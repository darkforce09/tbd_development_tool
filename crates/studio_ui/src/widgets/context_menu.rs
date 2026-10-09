use crate::colors::*;
use egui::{Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    ToggleExpand,
    ViewDocs,
    Center,
    ToggleWires,
    CopyPath,
    Delete,
}

pub struct NodeContextMenuProps<'a> {
    pub position: Pos2,
    pub title: &'a str,
    pub is_code_expanded: bool,
    pub show_member_wires: bool,
    pub has_docs: bool,
    pub hovered_pos: Option<Pos2>,
}

pub struct ContextMenuLayout {
    pub rect: Rect,
    pub item_clicks: Vec<(Rect, ContextMenuItem)>,
}

/// Paints a sleek dark-themed context menu when right-clicking a node.
pub fn paint_node_context_menu(painter: &Painter, props: NodeContextMenuProps<'_>) -> ContextMenuLayout {
    let width = 196.0;
    let row_h = 27.0;
    let header_h = 24.0;
    let divider_h = 7.0;

    let items = [
        (
            ContextMenuItem::ToggleExpand,
            if props.is_code_expanded { "Collapse to Card" } else { "Expand to Code" },
            if props.is_code_expanded {
                egui_phosphor::regular::ARROWS_IN_SIMPLE
            } else {
                egui_phosphor::regular::ARROWS_OUT_SIMPLE
            },
            false,
        ),
        (ContextMenuItem::ViewDocs, "View Documentation", egui_phosphor::regular::NOTE, false),
        (ContextMenuItem::Center, "Center in View", egui_phosphor::regular::CROSSHAIR, false),
        (
            ContextMenuItem::ToggleWires,
            if props.show_member_wires { "Hide Member Wires" } else { "Show Member Wires" },
            egui_phosphor::regular::PLUGS,
            false,
        ),
        (ContextMenuItem::CopyPath, "Copy File Path", egui_phosphor::regular::COPY, false),
        (
            ContextMenuItem::Delete,
            "Delete Node",
            egui_phosphor::regular::TRASH,
            true, // is_danger
        ),
    ];

    let total_h = header_h + (items.len() as f32 * row_h) + divider_h + 8.0;
    let menu_rect = Rect::from_min_size(props.position, Vec2::new(width, total_h));

    // Drop shadow
    painter.rect(
        menu_rect.translate(Vec2::new(0.0, 5.0)),
        CornerRadius::from(7.0),
        Color32::from_black_alpha(130),
        Stroke::NONE,
        egui::StrokeKind::Middle,
    );

    // Menu panel container
    painter.rect(
        menu_rect,
        CornerRadius::from(7.0),
        Color32::from_rgb(20, 24, 34),
        Stroke::new(1.0, Color32::from_rgb(46, 54, 72)),
        egui::StrokeKind::Middle,
    );

    // Header bar with node title
    let header_rect = Rect::from_min_size(menu_rect.min, Vec2::new(width, header_h));
    painter.rect_filled(header_rect, CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 }, Color32::from_rgb(15, 18, 26));
    painter.text(
        header_rect.min + Vec2::new(10.0, header_h * 0.5),
        egui::Align2::LEFT_CENTER,
        crate::truncate_with_ellipsis(props.title, 22),
        FontId::new(10.5, FontFamily::Proportional),
        TEXT_DIM,
    );

    let mut curr_y = header_rect.max.y + 4.0;
    let mut item_clicks = Vec::new();

    for (item, label, icon, is_danger) in items {
        if item == ContextMenuItem::Delete {
            // Divider line before delete
            let div_y = curr_y + 2.0;
            painter.line_segment(
                [Pos2::new(menu_rect.min.x + 8.0, div_y), Pos2::new(menu_rect.max.x - 8.0, div_y)],
                Stroke::new(1.0, Color32::from_rgb(38, 44, 60)),
            );
            curr_y += divider_h;
        }

        let row_rect = Rect::from_min_size(Pos2::new(menu_rect.min.x + 4.0, curr_y), Vec2::new(width - 8.0, row_h));

        let is_hov = props.hovered_pos.map(|p| row_rect.contains(p)).unwrap_or(false);

        if is_hov {
            let fill = if is_danger {
                Color32::from_rgba_premultiplied(239, 68, 68, 40)
            } else {
                Color32::from_rgba_premultiplied(99, 102, 241, 40)
            };
            painter.rect_filled(row_rect, CornerRadius::from(4.0), fill);
        }

        let text_color = if is_danger {
            if is_hov {
                Color32::from_rgb(248, 113, 113)
            } else {
                Color32::from_rgb(220, 80, 80)
            }
        } else if is_hov {
            TEXT_HIGHLIGHT
        } else {
            TEXT_PRIMARY
        };

        // Icon
        painter.text(
            row_rect.min + Vec2::new(10.0, row_h * 0.5),
            egui::Align2::LEFT_CENTER,
            icon,
            FontId::new(12.5, FontFamily::Proportional),
            text_color,
        );

        // Label
        painter.text(
            row_rect.min + Vec2::new(30.0, row_h * 0.5),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::new(11.5, FontFamily::Proportional),
            text_color,
        );

        item_clicks.push((row_rect, item));
        curr_y += row_h;
    }

    ContextMenuLayout { rect: menu_rect, item_clicks }
}
