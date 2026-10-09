use egui::{
    epaint::RectShape, Color32, FontFamily, FontId, Painter, Pos2, Rect, CornerRadius, Stroke, Vec2,
};
use crate::colors::*;
use crate::syntax::SyntaxHighlighter;

/// UI representation of a member item inside a File node (function, struct, enum, trait, method).
#[derive(Clone, Copy)]
pub struct FileCardMember<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub archetype_tag: &'a str, // "FN", "STR", "ENM", "TRT", "MOD"
    pub archetype_color: Color32,
    pub visibility: &'a str,    // "pub" or ""
    pub signature: &'a str,
    pub line_number: usize,
    pub source_code: &'a str,
}

pub struct FileCardProps<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub extension: &'a str,
    pub item_count: usize,
    pub line_count: Option<usize>,
    pub is_selected: bool,
    pub is_hovered: bool,
    pub is_dropdown_expanded: bool,
    pub is_code_expanded: bool,
    pub show_member_wires: bool,
    pub members: &'a [FileCardMember<'a>],
    pub expanded_member_id: Option<&'a str>,
    pub has_docs: bool,
    pub step_number: Option<usize>,
    pub zoom: f32,
    pub hovered_pos: Option<Pos2>,
}

pub struct FileCardLayout {
    pub expand_button_rect: Rect,
    pub dropdown_button_rect: Rect,
    pub wires_toggle_button_rect: Rect,
    pub code_expand_button_rect: Rect,
    pub member_row_clicks: Vec<(Rect, String)>,
    pub member_code_clicks: Vec<(Rect, String)>,
}

/// Paints a CodeSee + CodeCanvas hybrid File Node Card on the canvas.
/// Defaults to a compact $220x42px card with extension pill and step badges.
/// Expands vertically like a dropdown accordion to reveal all of the file's member nodes
/// (functions, structs, enums, traits) with individual archetype tags, line numbers,
/// and inline code snippet drawers.
pub fn paint_file_card(painter: &Painter, props: FileCardProps<'_>) -> FileCardLayout {
    let z = props.zoom;
    let rounding = CornerRadius::from(6.0 * z);

    // 1. Drop shadow & selection glow
    let shadow_offset = Vec2::new(0.0, 3.0 * z);
    if props.is_selected {
        // Vibrant selection glow (CodeSee style)
        painter.rect(
            props.rect.translate(shadow_offset),
            rounding,
            Color32::from_rgba_premultiplied(99, 102, 241, 60),
            Stroke::NONE, egui::StrokeKind::Middle,
        );
    } else {
        painter.rect(
            props.rect.translate(shadow_offset),
            rounding,
            CARD_SHADOW,
            Stroke::NONE, egui::StrokeKind::Middle,
        );
    }

    // 2. Card Background & Border
    let border_stroke = if props.is_selected {
        Stroke::new((2.0 * z).max(1.5), CARD_BORDER_SELECTED)
    } else if props.is_hovered {
        Stroke::new((1.5 * z).max(1.0), CARD_BORDER_HOVER)
    } else {
        Stroke::new((1.0 * z).max(0.75), CARD_BORDER_NORMAL)
    };

    let bg_color = if props.is_hovered {
        CARD_BG_HOVER
    } else {
        CARD_BG
    };

    painter.add(RectShape::new(props.rect, rounding, bg_color, border_stroke, egui::StrokeKind::Middle));

    // 3. Header Area (38px high)
    let header_h = 38.0 * z;
    let header_rect = Rect::from_min_size(props.rect.min, Vec2::new(props.rect.width(), header_h));
    let header_painter = painter.with_clip_rect(props.rect);

    // Extension Pill Badge on the left (e.g., [JS], [RS], [TS])
    let ext_color = file_extension_color(props.extension);
    let badge_h = (22.0 * z).max(14.0);
    let badge_w = (32.0 * z).max(20.0);
    let badge_x = props.rect.min.x + 8.0 * z;
    let badge_y = header_rect.center().y - (badge_h * 0.5);

    let badge_rect = Rect::from_min_size(Pos2::new(badge_x, badge_y), Vec2::new(badge_w, badge_h));

    // Pill background tinted with extension color
    header_painter.rect(
        badge_rect,
        CornerRadius::from(4.0 * z),
        with_alpha(ext_color, 40),
        Stroke::new((1.0 * z).max(0.5), with_alpha(ext_color, 180)), egui::StrokeKind::Middle,
    );

    // Extension text uppercase (e.g., "RS", "JS")
    let ext_display = props.extension.trim_start_matches('.').to_uppercase();
    header_painter.text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        ext_display,
        FontId::new((9.5 * z).max(5.0), FontFamily::Monospace),
        ext_color,
    );

    // Full Code Expand / Collapse button on the far right
    let btn_size = Vec2::splat(18.0 * z);
    let code_expand_btn_rect = Rect::from_center_size(
        Pos2::new(props.rect.max.x - 14.0 * z, header_rect.center().y),
        btn_size,
    );

    let expand_icon = if props.is_code_expanded {
        egui_phosphor::regular::ARROWS_IN_SIMPLE
    } else {
        egui_phosphor::regular::ARROWS_OUT_SIMPLE
    };
    let is_expand_hovered = props.hovered_pos.map(|p| code_expand_btn_rect.contains(p)).unwrap_or(false);
    if is_expand_hovered || props.is_code_expanded {
        let fill = if props.is_code_expanded {
            Color32::from_rgba_premultiplied(168, 85, 247, 45)
        } else {
            Color32::from_rgba_premultiplied(99, 102, 241, 40)
        };
        header_painter.rect_filled(code_expand_btn_rect, CornerRadius::from(3.0 * z), fill);
    }
    let expand_color = if props.is_code_expanded {
        Color32::from_rgb(168, 85, 247)
    } else if is_expand_hovered {
        TEXT_HIGHLIGHT
    } else {
        TEXT_SECONDARY
    };
    header_painter.text(
        code_expand_btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        expand_icon,
        FontId::new((12.5 * z).max(6.5), FontFamily::Proportional),
        expand_color,
    );

    // Dropdown Accordion toggle button (left of full expand button)
    let dropdown_btn_rect = Rect::from_center_size(
        Pos2::new(code_expand_btn_rect.min.x - 11.0 * z, header_rect.center().y),
        btn_size,
    );
    let is_dropdown_hovered = props.hovered_pos.map(|p| dropdown_btn_rect.contains(p)).unwrap_or(false);
    if is_dropdown_hovered || props.is_dropdown_expanded {
        let fill = if props.is_dropdown_expanded {
            Color32::from_rgba_premultiplied(99, 102, 241, 45)
        } else {
            Color32::from_rgba_premultiplied(148, 163, 184, 25)
        };
        header_painter.rect_filled(dropdown_btn_rect, CornerRadius::from(3.0 * z), fill);
    }
    let dropdown_color = if props.is_dropdown_expanded {
        Color32::from_rgb(129, 140, 248)
    } else if is_dropdown_hovered {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let dropdown_icon = if props.is_dropdown_expanded {
        egui_phosphor::regular::CARET_UP
    } else {
        egui_phosphor::regular::CARET_DOWN
    };
    header_painter.text(
        dropdown_btn_rect.center(),
        egui::Align2::CENTER_CENTER,
        dropdown_icon,
        FontId::new((12.0 * z).max(6.0), FontFamily::Proportional),
        dropdown_color,
    );

    // File Title
    let title_x = badge_rect.max.x + 8.0 * z;
    let title_pos = Pos2::new(title_x, header_rect.center().y);
    let title_font_size = (12.0 * z).max(6.0);

    let right_actions_width = 48.0 * z;
    let max_title_width = (props.rect.width() - (title_x - props.rect.min.x) - right_actions_width).max(20.0);
    let approx_char_w = (7.0 * z).max(3.0);
    let max_chars = ((max_title_width / approx_char_w) as usize).clamp(4, 28);

    let display_title = crate::truncate_with_ellipsis(props.title, max_chars);

    header_painter.text(
        title_pos,
        egui::Align2::LEFT_CENTER,
        &display_title,
        FontId::new(title_font_size, FontFamily::Proportional),
        if props.is_selected { TEXT_HIGHLIGHT } else { TEXT_PRIMARY },
    );

    // Item count pill e.g. "(3)"
    let count_val = if props.item_count > 0 { props.item_count } else { props.members.len() };
    if count_val > 0 {
        let title_w = display_title.len() as f32 * approx_char_w;
        let count_pos = Pos2::new(title_x + title_w + 6.0 * z, header_rect.center().y);
        if count_pos.x < dropdown_btn_rect.min.x - 24.0 * z {
            header_painter.text(
                count_pos,
                egui::Align2::LEFT_CENTER,
                format!("({})", count_val),
                FontId::new((9.5 * z).max(5.0), FontFamily::Monospace),
                TEXT_DIM,
            );
        }
    }

    // Step Number Badge (CodeSee style shield badge at top-right corner)
    if let Some(step) = props.step_number {
        let badge_size = Vec2::new(18.0 * z, 18.0 * z);
        let badge_center = Pos2::new(props.rect.max.x - 4.0 * z, props.rect.min.y);
        let step_rect = Rect::from_center_size(badge_center, badge_size);

        painter.circle_filled(step_rect.center(), 9.0 * z, STEP_BADGE_BG);
        painter.circle_stroke(
            step_rect.center(),
            9.0 * z,
            Stroke::new((1.5 * z).max(1.0), Color32::from_rgb(0xff, 0xff, 0xff)),
        );

        painter.text(
            step_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{}", step),
            FontId::new((9.5 * z).max(5.0), FontFamily::Monospace),
            STEP_BADGE_TEXT,
        );
    }

    let mut member_row_clicks = Vec::new();
    let mut member_code_clicks = Vec::new();

    // 4. Dropdown Body (Member items list & inline code drawers)
    if props.is_dropdown_expanded {
        let div_y = header_rect.max.y;
        header_painter.line_segment(
            [
                Pos2::new(props.rect.min.x + 8.0 * z, div_y),
                Pos2::new(props.rect.max.x - 8.0 * z, div_y),
            ],
            Stroke::new((1.0 * z).max(0.5), Color32::from_rgb(38, 42, 58)),
        );

        let mut current_y = div_y + 4.0 * z;

        if props.members.is_empty() {
            header_painter.text(
                Pos2::new(props.rect.center().x, current_y + 12.0 * z),
                egui::Align2::CENTER_CENTER,
                "(No items extracted)",
                FontId::new((10.0 * z).max(5.0), FontFamily::Proportional),
                TEXT_DIM,
            );
        } else {
            for member in props.members {
                let row_h = 24.0 * z;
                let row_rect = Rect::from_min_max(
                    Pos2::new(props.rect.min.x + 6.0 * z, current_y),
                    Pos2::new(props.rect.max.x - 6.0 * z, current_y + row_h),
                );

                let is_row_hovered = props.hovered_pos.map(|p| row_rect.contains(p)).unwrap_or(false);
                if is_row_hovered {
                    header_painter.rect_filled(
                        row_rect,
                        CornerRadius::from(4.0 * z),
                        with_alpha(member.archetype_color, 28),
                    );
                }

                member_row_clicks.push((row_rect, member.id.to_string()));

                // Archetype Tag pill [FN], [STR], [ENM], [TRT]
                let tag_w = 26.0 * z;
                let tag_h = 16.0 * z;
                let tag_rect = Rect::from_center_size(
                    Pos2::new(row_rect.min.x + 16.0 * z, row_rect.center().y),
                    Vec2::new(tag_w, tag_h),
                );

                header_painter.rect(
                    tag_rect,
                    CornerRadius::from(3.0 * z),
                    with_alpha(member.archetype_color, 45),
                    Stroke::new((1.0 * z).max(0.5), with_alpha(member.archetype_color, 180)), egui::StrokeKind::Middle,
                );

                header_painter.text(
                    tag_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    member.archetype_tag,
                    FontId::new((8.5 * z).max(4.5), FontFamily::Monospace),
                    member.archetype_color,
                );

                let mut text_x = tag_rect.max.x + 6.0 * z;

                // Visibility pill e.g. "pub"
                if !member.visibility.is_empty() {
                    header_painter.text(
                        Pos2::new(text_x, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        member.visibility,
                        FontId::new((9.0 * z).max(4.5), FontFamily::Monospace),
                        Color32::from_rgb(52, 211, 153),
                    );
                    text_x += 24.0 * z;
                }

                // Member Name
                let name_max_w = (row_rect.max.x - text_x - 52.0 * z).max(10.0);
                let name_max_chars = ((name_max_w / (6.5 * z).max(2.5)) as usize).clamp(3, 20);
                let display_name = crate::truncate_with_ellipsis(member.name, name_max_chars);

                header_painter.text(
                    Pos2::new(text_x, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    display_name,
                    FontId::new((10.5 * z).max(5.5), FontFamily::Monospace),
                    if is_row_hovered { TEXT_HIGHLIGHT } else { TEXT_PRIMARY },
                );

                // Line Number e.g. ":35"
                let line_str = format!(":{}", member.line_number);
                header_painter.text(
                    Pos2::new(row_rect.max.x - 22.0 * z, row_rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    line_str,
                    FontId::new((9.5 * z).max(4.5), FontFamily::Monospace),
                    TEXT_DIM,
                );

                // Code Drawer toggle button on the far right
                let code_btn_rect = Rect::from_center_size(
                    Pos2::new(row_rect.max.x - 10.0 * z, row_rect.center().y),
                    Vec2::splat(16.0 * z),
                );

                let is_code_open = props.expanded_member_id == Some(member.id);
                let code_btn_color = if is_code_open {
                    Color32::from_rgb(168, 85, 247)
                } else if is_row_hovered {
                    TEXT_PRIMARY
                } else {
                    TEXT_DIM
                };

                header_painter.text(
                    code_btn_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if is_code_open { egui_phosphor::regular::CARET_UP } else { egui_phosphor::regular::CODE },
                    FontId::new((10.0 * z).max(5.0), FontFamily::Proportional),
                    code_btn_color,
                );

                member_code_clicks.push((code_btn_rect, member.id.to_string()));

                // Inline Code Drawer (rendered directly under this member row)
                if is_code_open {
                    let code_lines: Vec<&str> = member.source_code.lines().take(40).collect();
                    let line_count = code_lines.len().max(1);
                    let line_h = 14.0 * z;
                    let drawer_h = (line_count as f32 * line_h + 12.0 * z).clamp(36.0 * z, 560.0 * z);

                    let drawer_rect = Rect::from_min_max(
                        Pos2::new(props.rect.min.x + 8.0 * z, current_y + row_h + 2.0 * z),
                        Pos2::new(props.rect.max.x - 8.0 * z, current_y + row_h + 2.0 * z + drawer_h),
                    );

                    // Background container
                    header_painter.rect(
                        drawer_rect,
                        CornerRadius::from(4.0 * z),
                        Color32::from_rgb(12, 14, 20),
                        Stroke::new((1.0 * z).max(0.5), Color32::from_rgb(45, 50, 68)), egui::StrokeKind::Middle,
                    );

                    let drawer_clip = header_painter.with_clip_rect(drawer_rect);
                    let mut code_y = drawer_rect.min.y + 6.0 * z;

                    for (idx, line) in code_lines.iter().enumerate() {
                        let l_num = member.line_number + idx;
                        // Line number gutter
                        drawer_clip.text(
                            Pos2::new(drawer_rect.min.x + 6.0 * z, code_y),
                            egui::Align2::LEFT_TOP,
                            format!("{:2}", l_num),
                            FontId::new((8.5 * z).max(4.0), FontFamily::Monospace),
                            Color32::from_rgb(90, 96, 120),
                        );

                        // Code line text with VS Code syntax highlighting (full line width)
                        let code_snippet = line;
                        let hl = SyntaxHighlighter::global();
                        let tokens = hl.highlight_line_tokens(props.extension, code_snippet);

                        let font_id = FontId::new((9.0 * z).max(4.5), FontFamily::Monospace);
                        let mut job = egui::text::LayoutJob::default();
                        for (color, token) in tokens {
                            job.append(
                                token,
                                0.0,
                                egui::TextFormat {
                                    font_id: font_id.clone(),
                                    color,
                                    ..Default::default()
                                },
                            );
                        }

                        let galley = drawer_clip.layout_job(job);
                        drawer_clip.galley(Pos2::new(drawer_rect.min.x + 28.0 * z, code_y), galley, Color32::WHITE);

                        code_y += line_h;
                    }

                    current_y += row_h + drawer_h + 6.0 * z;
                } else {
                    current_y += row_h + 2.0 * z;
                }
            }
        }
    }

    FileCardLayout {
        expand_button_rect: code_expand_btn_rect,
        dropdown_button_rect: dropdown_btn_rect,
        wires_toggle_button_rect: Rect::NOTHING,
        code_expand_button_rect: code_expand_btn_rect,
        member_row_clicks,
        member_code_clicks,
    }
}
