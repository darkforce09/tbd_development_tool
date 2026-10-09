use egui::{
    epaint::RectShape, Color32, FontFamily, FontId, Painter, Pos2, Rect, CornerRadius, Stroke, Vec2,
};
use studio_graph::{Graph, NodeId};
use studio_ui::{color_tokens::*, paint_node_context_menu, with_alpha, NodeContextMenuProps};

use super::types::{CanvasState, RenderEvents, ZoomAction};

#[allow(clippy::too_many_arguments)]
pub fn render_hud(
    painter: &Painter,
    state: &CanvasState,
    graph: &Graph,
    rect: Rect,
    _single_selected_rect: Option<(NodeId, Rect)>,
    pointer_pos: Pos2,
    pointer_clicked: bool,
    interactive_rects: &mut Vec<Rect>,
    events: &mut RenderEvents,
) {
    // Layer E-2: Right-Click Node Context Menu
    if let Some(ref menu) = state.context_menu {
        if let Some(node) = graph.nodes.get(&menu.node_id) {
            let menu_layout = paint_node_context_menu(
                painter,
                NodeContextMenuProps {
                    position: menu.screen_pos,
                    title: &node.title,
                    is_code_expanded: node.is_code_expanded,
                    show_member_wires: node.show_member_wires,
                    has_docs: node.doc_comment.is_some(),
                    hovered_pos: Some(pointer_pos),
                },
            );

            interactive_rects.push(menu_layout.rect);
            interactive_rects.extend(menu_layout.item_clicks.iter().map(|(r, _)| *r));

            if pointer_clicked {
                for (item_r, action) in menu_layout.item_clicks {
                    if item_r.contains(pointer_pos) {
                        events.context_menu_action = Some((menu.node_id, action));
                        break;
                    }
                }
            }
        }
    }

    // Layer F: Interactive Bottom-Right Zoom HUD & Slider Bar (CodeSee style)
    let hud_size = Vec2::new(260.0, 34.0);
    let hud_rect = Rect::from_min_size(
        Pos2::new(rect.max.x - hud_size.x - 16.0, rect.max.y - hud_size.y - 16.0),
        hud_size,
    );

    painter.rect(
        hud_rect.translate(Vec2::new(0.0, 2.0)),
        CornerRadius::from(6.0),
        Color32::from_black_alpha(80),
        Stroke::NONE, egui::StrokeKind::Middle,
    );
    painter.add(RectShape::new(
        hud_rect,
        CornerRadius::from(6.0),
        with_alpha(PANEL_BG, 235),
        Stroke::new(1.0, PANEL_BORDER), egui::StrokeKind::Middle,
    ));

    // Interactive buttons inside zoom HUD
    let btn_zoom_out = Rect::from_min_size(hud_rect.min + Vec2::new(6.0, 5.0), Vec2::new(24.0, 24.0));
    let btn_zoom_in = Rect::from_min_size(hud_rect.min + Vec2::new(98.0, 5.0), Vec2::new(24.0, 24.0));
    let btn_fit = Rect::from_min_size(hud_rect.min + Vec2::new(132.0, 5.0), Vec2::new(56.0, 24.0));
    let btn_reset = Rect::from_min_size(hud_rect.min + Vec2::new(194.0, 5.0), Vec2::new(60.0, 24.0));

    let paint_hud_btn = |btn_r: Rect, text: &str| {
        let is_hov = btn_r.contains(pointer_pos);
        if is_hov {
            painter.rect_filled(btn_r, CornerRadius::from(4.0), FLOATING_BTN_HOVER);
        }
        painter.text(
            btn_r.center(),
            egui::Align2::CENTER_CENTER,
            text,
            FontId::new(11.5, FontFamily::Proportional),
            if is_hov { TEXT_HIGHLIGHT } else { TEXT_SECONDARY },
        );
    };

    paint_hud_btn(btn_zoom_out, egui_phosphor::regular::MINUS);
    // Zoom % text
    let zoom_pct_rect = Rect::from_min_size(hud_rect.min + Vec2::new(32.0, 5.0), Vec2::new(64.0, 24.0));
    let zoom_pct = state.transform.zoom * 100.0;
    let zoom_str = if zoom_pct < 1.0 {
        format!("{:.2}%", zoom_pct)
    } else if zoom_pct < 10.0 {
        format!("{:.1}%", zoom_pct)
    } else {
        format!("{:.0}%", zoom_pct)
    };
    painter.text(
        zoom_pct_rect.center(),
        egui::Align2::CENTER_CENTER,
        zoom_str,
        FontId::new(11.0, FontFamily::Monospace),
        TEXT_PRIMARY,
    );
    paint_hud_btn(btn_zoom_in, egui_phosphor::regular::PLUS);
    let fit_label = format!("{} Fit", egui_phosphor::regular::CORNERS_OUT);
    paint_hud_btn(btn_fit, &fit_label);
    let reset_label = format!("{} Reset", egui_phosphor::regular::ARROWS_COUNTER_CLOCKWISE);
    paint_hud_btn(btn_reset, &reset_label);

    if pointer_clicked {
        if btn_zoom_out.contains(pointer_pos) {
            events.zoom_action = Some(ZoomAction::ZoomOut);
        } else if btn_zoom_in.contains(pointer_pos) {
            events.zoom_action = Some(ZoomAction::ZoomIn);
        } else if btn_fit.contains(pointer_pos) {
            events.zoom_action = Some(ZoomAction::Fit);
        } else if btn_reset.contains(pointer_pos) {
            events.zoom_action = Some(ZoomAction::Reset);
        }
    }
}
