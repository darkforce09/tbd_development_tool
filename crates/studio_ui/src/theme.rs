use egui::{epaint::Shadow, Color32, Context, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Visuals};

use crate::colors::*;

/// Configure egui context to match the Studio design system.
pub fn apply_theme(ctx: &Context) {
    let mut visuals = Visuals::dark();

    // Base background colors
    visuals.panel_fill = CANVAS_BG;
    visuals.window_fill = CARD_BG;
    visuals.extreme_bg_color = CANVAS_BG;

    // Window / card framing
    visuals.window_corner_radius = CornerRadius::from(8.0);
    visuals.window_stroke = Stroke::new(1.0, CARD_BORDER_NORMAL);
    visuals.window_shadow = Shadow { offset: [0, 4], blur: 8, spread: 0, color: CARD_SHADOW };

    // Widget styling
    visuals.widgets.noninteractive.bg_fill = CARD_BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_BORDER_NORMAL);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::from(6.0);

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x22, 0x22, 0x2a);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, CARD_BORDER_NORMAL);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::from(6.0);

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2c, 0x2c, 0x36);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, CARD_BORDER_HOVER);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_HIGHLIGHT);
    visuals.widgets.hovered.corner_radius = CornerRadius::from(6.0);

    visuals.widgets.active.bg_fill = Color32::from_rgb(0x38, 0x38, 0x48);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, CARD_BORDER_SELECTED);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_HIGHLIGHT);
    visuals.widgets.active.corner_radius = CornerRadius::from(6.0);

    visuals.widgets.open.bg_fill = CARD_BG;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, CARD_BORDER_SELECTED);
    visuals.widgets.open.corner_radius = CornerRadius::from(6.0);

    visuals.selection.bg_fill = Color32::from_rgb(0x43, 0x38, 0xca);
    visuals.selection.stroke = Stroke::new(1.0, CARD_BORDER_SELECTED);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);

    // Font styles
    style.text_styles = [
        (TextStyle::Heading, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(11.0, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
    ]
    .into();

    ctx.set_global_style(style);

    // Register Phosphor icon font as fallback for egui Proportional fonts
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}
