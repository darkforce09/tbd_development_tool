use egui::{Frame, Id, Order, RichText};
use studio_graph::Graph;
use studio_ui::color_tokens::*;
use studio_ui::truncate_with_ellipsis;

use super::types::{CanvasState, ContextMenuAction, RenderEvents};

/// A card's context menu: a small window over everything, the Desk included. A press anywhere
/// else closes it.
pub fn render_context_menu(ctx: &egui::Context, state: &mut CanvasState, graph: &Graph, events: &mut RenderEvents) {
    let Some(menu) = state.context_menu.clone() else { return };
    let Some(node) = graph.nodes.get(&menu.node_id) else {
        state.context_menu = None;
        return;
    };
    let items = [
        (ContextMenuAction::OpenOnDesk, egui_phosphor::regular::BROWSER, "Open on the Desk"),
        (
            ContextMenuAction::ToggleExpand,
            egui_phosphor::regular::ARROWS_OUT_SIMPLE,
            if node.is_code_expanded { "Collapse to card" } else { "Expand on the map" },
        ),
        (ContextMenuAction::ViewDocs, egui_phosphor::regular::NOTE, "View documentation"),
        (ContextMenuAction::Center, egui_phosphor::regular::CROSSHAIR, "Center in view"),
        (
            ContextMenuAction::ToggleWires,
            egui_phosphor::regular::PLUGS,
            if node.show_member_wires { "Hide member wires" } else { "Show member wires" },
        ),
        (ContextMenuAction::CopyPath, egui_phosphor::regular::COPY, "Copy file path"),
    ];
    let mut picked = None;
    let area = egui::Area::new(Id::new("canvas_context_menu"))
        .order(Order::Foreground)
        .fixed_pos(menu.screen_pos)
        .show(ctx, |ui| {
            Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(210.0);
                ui.label(RichText::new(truncate_with_ellipsis(&node.title, 30)).color(TEXT_DIM).small());
                ui.separator();
                for (action, icon, label) in items {
                    if ui.add(egui::Button::new(format!("{icon}  {label}")).frame(false)).clicked() {
                        picked = Some(action);
                    }
                }
            });
        });
    if let Some(action) = picked {
        events.context_menu_action = Some((menu.node_id, action));
    } else if ctx
        .input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)))
    {
        state.context_menu = None;
    }
}
