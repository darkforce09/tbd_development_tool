use egui::{
    epaint::RectShape, Color32, FontFamily, FontId, Painter, Pos2, Rect, CornerRadius, Stroke, Vec2,
};
use crate::colors::*;
use crate::syntax::{detect_language, SyntaxHighlighter};
use crate::widgets::file_card::FileCardMember;

/// Port display metadata for the Ports & Wires tab on expanded canvas cards.
#[derive(Clone, Debug)]
pub struct PortDisplayInfo<'a> {
    pub id: u64,
    pub name: &'a str,
    pub type_name: &'a str,
    pub type_color: Color32,
    pub connected_node_title: Option<String>,
    pub connected_node_id: Option<u64>,
}

pub struct CodeCardProps<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub file_path: Option<&'a str>,
    pub start_line: usize,
    pub code: &'a str,
    pub doc_comment: Option<&'a str>,
    pub is_selected: bool,
    pub zoom: f32,
    pub language_hint: Option<&'a str>,
    pub active_tab: usize,
    pub is_markdown_preview: bool,
    pub members: &'a [FileCardMember<'a>],
    pub expanded_member_id: Option<&'a str>,
    pub collapsed_subnodes: Option<&'a std::collections::BTreeSet<String>>,
    pub inputs: &'a [PortDisplayInfo<'a>],
    pub outputs: &'a [PortDisplayInfo<'a>],
    pub hovered_pos: Option<Pos2>,
    pub scroll_y: f32,
}

pub struct CodeCardLayout {
    pub line_y_offsets: Vec<f32>,
    pub close_button_rect: Rect,
    pub preview_toggle_rect: Option<Rect>,
    pub tab_rects: Vec<(Rect, usize)>,
    pub subnode_clicks: Vec<(Rect, usize)>,
    pub member_fold_clicks: Vec<(Rect, String)>,
    pub jump_clicks: Vec<(Rect, u64)>,
    pub member_row_clicks: Vec<(Rect, String)>,
    pub member_code_clicks: Vec<(Rect, String)>,
}

/// Paints an All-in-One Code Canvas Node Panel on the infinite canvas.
/// Integrates all capabilities previously in the right inspector:
/// - Tab 0: Syntax-highlighted Code or formatted Markdown Document Preview
/// - Tab 1: Rich Documentation / Headings Outline
/// - Tab 2: File Members with interactive inline code snippet drawers
/// - Tab 3: Haystack Ports Schema with Jump buttons to connected nodes
pub fn paint_code_card(painter: &Painter, props: CodeCardProps<'_>) -> CodeCardLayout {
    let z = props.zoom;
    let rounding = CornerRadius::from(8.0 * z);
    let lang = detect_language(props.file_path, props.language_hint);
    let is_markdown = lang == "md" || lang == "markdown" || props.language_hint == Some("MD");

    // 1. Drop shadow & selection glow
    painter.rect(
        props.rect.translate(Vec2::new(0.0, 4.0 * z)),
        rounding,
        Color32::from_black_alpha(90),
        Stroke::NONE, egui::StrokeKind::Middle,
    );

    let border_color = if props.is_selected {
        CARD_BORDER_SELECTED
    } else {
        CARD_BORDER_NORMAL
    };

    painter.add(RectShape::new(
        props.rect,
        rounding,
        CODE_EDITOR_BG,
        Stroke::new((1.2 * z).max(1.0), border_color), egui::StrokeKind::Middle,
    ));

    let card_painter = painter.with_clip_rect(props.rect);

    // 2. Header Bar (32px high)
    let header_height = (32.0 * z).max(18.0);
    let header_rect = Rect::from_min_size(
        props.rect.min,
        Vec2::new(props.rect.width(), header_height),
    );

    card_painter.add(RectShape::new(
        header_rect,
        CornerRadius {
            nw: (8.0 * z).round() as u8,
            ne: (8.0 * z).round() as u8,
            sw: 0,
            se: 0,
        },
        CARD_HEADER_BG,
        Stroke::NONE, egui::StrokeKind::Middle,
    ));

    // Language / Archetype Pill Badge
    let ext_col = file_extension_color(&lang);
    let badge_font_size = (9.0 * z).max(4.5);
    let badge_text = lang.to_uppercase();
    let badge_pos = header_rect.min + Vec2::new(8.0 * z, header_height * 0.5);
    card_painter.text(
        badge_pos,
        egui::Align2::LEFT_CENTER,
        format!("[{}]", badge_text),
        FontId::new(badge_font_size, FontFamily::Monospace),
        ext_col,
    );

    // Close button [X] on far right
    let btn_size = Vec2::splat(18.0 * z);
    let close_button_rect = Rect::from_center_size(
        Pos2::new(header_rect.max.x - 14.0 * z, header_rect.center().y),
        btn_size,
    );

    let is_close_hovered = props.hovered_pos.map(|p| close_button_rect.contains(p)).unwrap_or(false);
    if is_close_hovered {
        card_painter.rect_filled(
            close_button_rect,
            CornerRadius::from(3.0 * z),
            Color32::from_rgba_premultiplied(239, 68, 68, 45),
        );
    }
    card_painter.text(
        close_button_rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::X,
        FontId::new((12.0 * z).max(6.0), FontFamily::Proportional),
        if is_close_hovered { Color32::from_rgb(248, 113, 113) } else { TEXT_DIM },
    );

    // Title & File Path text
    let title_x = badge_pos.x + (badge_text.len() as f32 * 6.5 * z).max(18.0 * z) + 8.0 * z;
    let max_title_w = (close_button_rect.min.x - title_x - 10.0 * z).max(20.0);
    let approx_char_w = (7.0 * z).max(3.0);
    let max_chars = ((max_title_w / approx_char_w) as usize).clamp(6, 42);

    let header_text = if let Some(path) = props.file_path {
        format!("{}  •  {}", props.title, path)
    } else {
        props.title.to_string()
    };
    let display_title = crate::truncate_with_ellipsis(&header_text, max_chars);

    card_painter.text(
        Pos2::new(title_x, header_rect.center().y),
        egui::Align2::LEFT_CENTER,
        &display_title,
        FontId::new((11.5 * z).max(6.0), FontFamily::Proportional),
        TEXT_PRIMARY,
    );

    // 3. Navigation Tabs Bar (28px high) right under the header
    let tab_bar_y = header_rect.max.y;
    let tab_bar_h = (28.0 * z).max(16.0);
    let tab_bar_rect = Rect::from_min_size(
        Pos2::new(props.rect.min.x, tab_bar_y),
        Vec2::new(props.rect.width(), tab_bar_h),
    );

    card_painter.rect_filled(tab_bar_rect, CornerRadius::ZERO, Color32::from_rgb(18, 20, 28));

    // Define tabs depending on whether this is a Markdown document or Code file
    let mut tab_defs: Vec<(usize, String, &'static str)> = Vec::new();
    if is_markdown {
        tab_defs.push((0, "Preview".to_string(), egui_phosphor::regular::EYE));
        tab_defs.push((1, "Source".to_string(), egui_phosphor::regular::PENCIL_SIMPLE));
        tab_defs.push((2, "Outline".to_string(), egui_phosphor::regular::LIST_BULLETS));
    } else {
        tab_defs.push((0, "Code".to_string(), egui_phosphor::regular::CODE));
        let nodes_label = if !props.members.is_empty() {
            format!("Nodes ({})", props.members.len())
        } else {
            "Nodes".to_string()
        };
        tab_defs.push((1, nodes_label, egui_phosphor::regular::CUBE));
        tab_defs.push((2, "Documentation".to_string(), egui_phosphor::regular::BOOK_OPEN));
    }

    let effective_tab = if props.active_tab > 2 { 0 } else { props.active_tab };

    let mut tab_rects = Vec::new();
    let mut curr_tab_x = props.rect.min.x + 8.0 * z;
    let tab_font_size = (10.0 * z).max(5.0);

    for (tab_idx, label, icon) in &tab_defs {
        let text_chars = label.len() + 3;
        let tab_w = (text_chars as f32 * 6.5 * z + 16.0 * z).clamp(48.0 * z, 140.0 * z);
        let tab_rect = Rect::from_min_size(
            Pos2::new(curr_tab_x, tab_bar_y + 3.0 * z),
            Vec2::new(tab_w, tab_bar_h - 6.0 * z),
        );

        let is_active = effective_tab == *tab_idx;
        let is_hov = props.hovered_pos.map(|p| tab_rect.contains(p)).unwrap_or(false);

        if is_active {
            card_painter.rect(
                tab_rect,
                CornerRadius::from(4.0 * z),
                Color32::from_rgba_premultiplied(99, 102, 241, 55),
                Stroke::new((1.0 * z).max(0.5), Color32::from_rgb(129, 140, 248)), egui::StrokeKind::Middle,
            );
        } else if is_hov {
            card_painter.rect_filled(
                tab_rect,
                CornerRadius::from(4.0 * z),
                Color32::from_rgba_premultiplied(148, 163, 184, 30),
            );
        }

        let tab_col = if is_active {
            TEXT_HIGHLIGHT
        } else if is_hov {
            TEXT_PRIMARY
        } else {
            TEXT_SECONDARY
        };

        card_painter.text(
            tab_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{} {}", icon, label),
            FontId::new(tab_font_size, FontFamily::Proportional),
            tab_col,
        );

        tab_rects.push((tab_rect, *tab_idx));
        curr_tab_x += tab_w + 4.0 * z;
    }

    // Divider under navigation tabs
    let tabs_bottom = tab_bar_y + tab_bar_h;
    card_painter.line_segment(
        [
            Pos2::new(props.rect.min.x, tabs_bottom),
            Pos2::new(props.rect.max.x, tabs_bottom),
        ],
        Stroke::new((1.0 * z).max(0.5), PANEL_BORDER),
    );

    // 4. Content Area
    let body_top = tabs_bottom;
    let body_rect = Rect::from_min_max(
        Pos2::new(props.rect.min.x, body_top),
        props.rect.max,
    );
    let body_clip = painter.with_clip_rect(body_rect);

    let mut line_y_offsets = Vec::new();
    let mut jump_clicks = Vec::new();
    let mut member_row_clicks = Vec::new();
    let mut member_code_clicks = Vec::new();
    let mut member_fold_clicks = Vec::new();
    let mut subnode_clicks = Vec::new();

    let render_markdown_preview = is_markdown && effective_tab == 0;
    let render_markdown_outline = is_markdown && effective_tab == 2;
    let render_nodes_browser = !is_markdown && effective_tab == 1;
    let render_documentation = !is_markdown && effective_tab == 2;

    if render_markdown_preview {
        // TAB 0 (Markdown): Formatted Markdown Document Preview
        let content_left = body_rect.min.x + 14.0 * z;
        let content_width = body_rect.width() - 28.0 * z;
        let font_size = (11.0 * z).max(5.5);
        let mut curr_y = body_rect.min.y + 10.0 * z - props.scroll_y;

        let mut in_code_block = false;
        let mut code_block_lang = String::new();

        for line in props.code.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("```") {
                if !in_code_block {
                    in_code_block = true;
                    code_block_lang = trimmed.trim_start_matches("```").trim().to_string();
                } else {
                    in_code_block = false;
                    code_block_lang.clear();
                }
                if curr_y >= body_rect.min.y - 20.0 && curr_y <= body_rect.max.y {
                    body_clip.text(
                        Pos2::new(content_left, curr_y),
                        egui::Align2::LEFT_TOP,
                        trimmed,
                        FontId::new(font_size * 0.9, FontFamily::Monospace),
                        TEXT_DIM,
                    );
                }
                curr_y += font_size * 1.3;
                continue;
            }

            if in_code_block {
                if curr_y >= body_rect.min.y - 20.0 && curr_y <= body_rect.max.y {
                    let hl = SyntaxHighlighter::global();
                    let tokens = hl.highlight_line_tokens(&code_block_lang, line);
                    let mut job = egui::text::LayoutJob::default();
                    for (color, token) in tokens {
                        job.append(
                            token,
                            0.0,
                            egui::TextFormat {
                                font_id: FontId::new(font_size * 0.95, FontFamily::Monospace),
                                color,
                                ..Default::default()
                            },
                        );
                    }
                    let galley = body_clip.layout_job(job);
                    body_clip.galley(Pos2::new(content_left + 8.0 * z, curr_y), galley, Color32::WHITE);
                }
                curr_y += font_size * 1.3;
                continue;
            }

            if trimmed.is_empty() {
                curr_y += font_size * 0.7;
                continue;
            }

            if curr_y < body_rect.min.y - 40.0 {
                curr_y += font_size * 1.3;
                continue;
            }
            if curr_y > body_rect.max.y + 40.0 {
                break;
            }

            if trimmed.starts_with("# ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed.trim_start_matches("# ").trim(),
                    FontId::new(font_size * 1.4, FontFamily::Proportional),
                    VSCODE_KEYWORD,
                );
                curr_y += font_size * 1.8;
            } else if trimmed.starts_with("## ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed.trim_start_matches("## ").trim(),
                    FontId::new(font_size * 1.25, FontFamily::Proportional),
                    TEXT_HIGHLIGHT,
                );
                curr_y += font_size * 1.6;
            } else if trimmed.starts_with("### ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed.trim_start_matches("### ").trim(),
                    FontId::new(font_size * 1.1, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
                curr_y += font_size * 1.4;
            } else if trimmed.starts_with("- [ ] ") || trimmed.starts_with("* [ ] ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    egui_phosphor::regular::SQUARE,
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_DIM,
                );
                body_clip.text(
                    Pos2::new(content_left + 14.0 * z, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed[6..].trim(),
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
                curr_y += font_size * 1.3;
            } else if trimmed.starts_with("- [x] ") || trimmed.starts_with("* [x] ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    egui_phosphor::regular::CHECK_SQUARE,
                    FontId::new(font_size, FontFamily::Proportional),
                    Color32::from_rgb(0x2d, 0xd4, 0xbf),
                );
                body_clip.text(
                    Pos2::new(content_left + 14.0 * z, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed[6..].trim(),
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_DIM,
                );
                curr_y += font_size * 1.3;
            } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    "•",
                    FontId::new(font_size, FontFamily::Monospace),
                    Color32::from_rgb(0x2d, 0xd4, 0xbf),
                );
                body_clip.text(
                    Pos2::new(content_left + 12.0 * z, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed[2..].trim(),
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
                curr_y += font_size * 1.3;
            } else if let Some(quote) = trimmed.strip_prefix("> ") {
                body_clip.rect_filled(
                    Rect::from_min_size(Pos2::new(content_left, curr_y), Vec2::new(3.0 * z, font_size * 1.2)),
                    CornerRadius::from(1.0),
                    ARCHETYPE_FILE,
                );
                body_clip.text(
                    Pos2::new(content_left + 8.0 * z, curr_y),
                    egui::Align2::LEFT_TOP,
                    quote.trim(),
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_SECONDARY,
                );
                curr_y += font_size * 1.3;
            } else {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    crate::truncate_with_ellipsis(trimmed, (content_width / (font_size * 0.55)) as usize),
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
                curr_y += font_size * 1.3;
            }
        }
    } else if render_markdown_outline {
        // TAB 2 (Markdown): Document Headings Outline
        let content_left = body_rect.min.x + 14.0 * z;
        let mut curr_y = body_rect.min.y + 12.0 * z - props.scroll_y;
        let font_size = (11.0 * z).max(5.5);

        body_clip.text(
            Pos2::new(content_left, curr_y),
            egui::Align2::LEFT_TOP,
            "DOCUMENT HEADINGS OUTLINE",
            FontId::new((10.0 * z).max(5.0), FontFamily::Monospace),
            TEXT_DIM,
        );
        curr_y += font_size * 1.8;

        for (i, line) in props.code.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                let level = trimmed.chars().take_while(|c| *c == '#').count();
                let heading_text = trimmed.trim_start_matches('#').trim();
                let indent = level.saturating_sub(1) as f32 * 12.0 * z;
                let row_r = Rect::from_min_size(
                    Pos2::new(content_left + indent, curr_y),
                    Vec2::new(body_rect.width() - 28.0 * z - indent, 18.0 * z),
                );
                let is_hov = props.hovered_pos.map(|p| row_r.contains(p)).unwrap_or(false);
                if is_hov {
                    body_clip.rect_filled(
                        row_r,
                        CornerRadius::from(3.0 * z),
                        Color32::from_rgba_premultiplied(99, 102, 241, 30),
                    );
                }
                body_clip.text(
                    Pos2::new(content_left + indent, curr_y + 1.0 * z),
                    egui::Align2::LEFT_TOP,
                    heading_text,
                    FontId::new(font_size, FontFamily::Proportional),
                    if is_hov { TEXT_HIGHLIGHT } else { TEXT_PRIMARY },
                );
                let target_line = props.start_line + i;
                subnode_clicks.push((row_r, target_line));
                curr_y += font_size * 1.6;
            }
        }
    } else if render_nodes_browser {
        // TAB 1 (Code): Members & Sub-Nodes View (Simplified Node View)
        let content_left = body_rect.min.x + 14.0 * z;
        let mut curr_y = body_rect.min.y + 12.0 * z - props.scroll_y;
        let font_size = (11.0 * z).max(5.5);

        body_clip.text(
            Pos2::new(content_left, curr_y),
            egui::Align2::LEFT_TOP,
            format!("FILE MEMBERS & SUB-NODES  [{}]", props.members.len()),
            FontId::new((10.0 * z).max(5.0), FontFamily::Monospace),
            TEXT_DIM,
        );
        curr_y += font_size * 1.8;

        if props.members.is_empty() {
            body_clip.text(
                Pos2::new(content_left, curr_y),
                egui::Align2::LEFT_TOP,
                "(No members or functions extracted)",
                FontId::new(font_size, FontFamily::Proportional),
                TEXT_DIM,
            );
        } else {
            for member in props.members {
                if curr_y > body_rect.max.y {
                    break;
                }

                let is_code_open = props.expanded_member_id == Some(member.id);
                let row_h = 24.0 * z;
                let row_rect = Rect::from_min_size(
                    Pos2::new(content_left, curr_y),
                    Vec2::new(body_rect.width() - 28.0 * z, row_h),
                );

                let is_row_hov = props.hovered_pos.map(|p| row_rect.contains(p)).unwrap_or(false);
                if is_row_hov {
                    body_clip.rect_filled(
                        row_rect,
                        CornerRadius::from(4.0 * z),
                        with_alpha(member.archetype_color, 25),
                    );
                }

                // Archetype tag pill [FN], [STR], etc.
                let tag_w = 26.0 * z;
                let tag_h = 16.0 * z;
                let tag_rect = Rect::from_center_size(
                    Pos2::new(row_rect.min.x + 16.0 * z, row_rect.center().y),
                    Vec2::new(tag_w, tag_h),
                );
                body_clip.rect(
                    tag_rect,
                    CornerRadius::from(3.0 * z),
                    with_alpha(member.archetype_color, 45),
                    Stroke::new((1.0 * z).max(0.5), with_alpha(member.archetype_color, 180)), egui::StrokeKind::Middle,
                );
                body_clip.text(
                    tag_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    member.archetype_tag,
                    FontId::new((8.5 * z).max(4.5), FontFamily::Monospace),
                    member.archetype_color,
                );

                let mut text_x = tag_rect.max.x + 8.0 * z;
                if !member.visibility.is_empty() {
                    body_clip.text(
                        Pos2::new(text_x, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        member.visibility,
                        FontId::new((9.0 * z).max(4.5), FontFamily::Monospace),
                        Color32::from_rgb(52, 211, 153),
                    );
                    text_x += (member.visibility.len() as f32 * 6.5 * z).max(22.0 * z);
                }

                body_clip.text(
                    Pos2::new(text_x, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    member.name,
                    FontId::new(font_size, FontFamily::Proportional),
                    if is_row_hov { TEXT_HIGHLIGHT } else { TEXT_PRIMARY },
                );

                // Inline code toggle button [ < > ]
                let code_btn_w = 22.0 * z;
                let code_btn_h = 18.0 * z;
                let code_btn_rect = Rect::from_center_size(
                    Pos2::new(row_rect.max.x - 48.0 * z, row_rect.center().y),
                    Vec2::new(code_btn_w, code_btn_h),
                );
                let is_code_hov = props.hovered_pos.map(|p| code_btn_rect.contains(p)).unwrap_or(false);
                if is_code_open {
                    body_clip.rect(
                        code_btn_rect,
                        CornerRadius::from(3.0 * z),
                        Color32::from_rgba_premultiplied(99, 102, 241, 60),
                        Stroke::new((1.0 * z).max(0.5), Color32::from_rgb(129, 140, 248)), egui::StrokeKind::Middle,
                    );
                } else if is_code_hov {
                    body_clip.rect_filled(
                        code_btn_rect,
                        CornerRadius::from(3.0 * z),
                        Color32::from_rgba_premultiplied(148, 163, 184, 30),
                    );
                }
                body_clip.text(
                    code_btn_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    egui_phosphor::regular::CODE,
                    FontId::new((11.0 * z).max(5.5), FontFamily::Proportional),
                    if is_code_open { TEXT_HIGHLIGHT } else { TEXT_DIM },
                );
                member_code_clicks.push((code_btn_rect, member.id.to_string()));

                // Line number label :line_number
                let line_rect = Rect::from_min_max(
                    Pos2::new(row_rect.max.x - 34.0 * z, row_rect.min.y),
                    row_rect.max,
                );
                body_clip.text(
                    Pos2::new(row_rect.max.x - 6.0 * z, row_rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!(":{}", member.line_number),
                    FontId::new((9.5 * z).max(4.5), FontFamily::Monospace),
                    TEXT_DIM,
                );
                subnode_clicks.push((line_rect, member.line_number));

                member_row_clicks.push((row_rect, member.id.to_string()));

                // Inline Code Drawer
                if is_code_open {
                    let code_lines: Vec<&str> = member.source_code.lines().take(40).collect();
                    let line_count = code_lines.len().max(1);
                    let line_h = 14.0 * z;
                    let drawer_h = (line_count as f32 * line_h + 12.0 * z).clamp(36.0 * z, 420.0 * z);

                    let drawer_rect = Rect::from_min_max(
                        Pos2::new(content_left + 4.0 * z, curr_y + row_h + 2.0 * z),
                        Pos2::new(body_rect.max.x - 14.0 * z, curr_y + row_h + 2.0 * z + drawer_h),
                    );

                    body_clip.rect(
                        drawer_rect,
                        CornerRadius::from(4.0 * z),
                        Color32::from_rgb(12, 14, 20),
                        Stroke::new((1.0 * z).max(0.5), Color32::from_rgb(45, 50, 68)), egui::StrokeKind::Middle,
                    );

                    let drawer_clip = body_clip.with_clip_rect(drawer_rect);
                    let mut code_y = drawer_rect.min.y + 6.0 * z;

                    for (idx, line) in code_lines.iter().enumerate() {
                        let l_num = member.line_number + idx;
                        drawer_clip.text(
                            Pos2::new(drawer_rect.min.x + 6.0 * z, code_y),
                            egui::Align2::LEFT_TOP,
                            format!("{:2}", l_num),
                            FontId::new((8.5 * z).max(4.0), FontFamily::Monospace),
                            Color32::from_rgb(90, 96, 120),
                        );

                        let hl = SyntaxHighlighter::global();
                        let tokens = hl.highlight_line_tokens(&lang, line);
                        let font_id = FontId::new((9.0 * z).max(4.5), FontFamily::Monospace);
                        let mut job = egui::text::LayoutJob::default();
                        for (color, token) in tokens {
                            job.append(token, 0.0, egui::TextFormat { font_id: font_id.clone(), color, ..Default::default() });
                        }
                        let galley = drawer_clip.layout_job(job);
                        drawer_clip.galley(Pos2::new(drawer_rect.min.x + 28.0 * z, code_y), galley, Color32::WHITE);

                        code_y += line_h;
                    }

                    curr_y += row_h + drawer_h + 6.0 * z;
                } else {
                    curr_y += row_h + 2.0 * z;
                }
            }
        }
    } else if render_documentation {
        // TAB 2 (Code): Pure Documentation & Annotations (NO members/symbols here)
        let content_left = body_rect.min.x + 14.0 * z;
        let mut curr_y = body_rect.min.y + 12.0 * z - props.scroll_y;
        let font_size = (11.0 * z).max(5.5);

        body_clip.text(
            Pos2::new(content_left, curr_y),
            egui::Align2::LEFT_TOP,
            "DOCUMENTATION & ANNOTATIONS",
            FontId::new((10.0 * z).max(5.0), FontFamily::Monospace),
            TEXT_DIM,
        );
        curr_y += font_size * 1.8;

        if let Some(doc_text) = props.doc_comment {
            for line in doc_text.lines() {
                if curr_y > body_rect.max.y {
                    break;
                }
                let trimmed = line.trim().trim_start_matches("///").trim_start_matches("//!").trim();
                if trimmed.is_empty() {
                    curr_y += font_size * 0.7;
                    continue;
                }
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    trimmed,
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_PRIMARY,
                );
                curr_y += font_size * 1.3;
            }
        } else {
            let doc_lines: Vec<&str> = props.code.lines()
                .filter(|l| l.trim().starts_with("///") || l.trim().starts_with("//!"))
                .collect();
            if doc_lines.is_empty() {
                body_clip.text(
                    Pos2::new(content_left, curr_y),
                    egui::Align2::LEFT_TOP,
                    "No doc comments found for this file.",
                    FontId::new(font_size, FontFamily::Proportional),
                    TEXT_DIM,
                );
            } else {
                for line in doc_lines {
                    if curr_y > body_rect.max.y {
                        break;
                    }
                    let trimmed = line.trim().trim_start_matches("///").trim_start_matches("//!").trim();
                    body_clip.text(
                        Pos2::new(content_left, curr_y),
                        egui::Align2::LEFT_TOP,
                        trimmed,
                        FontId::new(font_size, FontFamily::Proportional),
                        TEXT_PRIMARY,
                    );
                    curr_y += font_size * 1.3;
                }
            }
        }
    } else {
        // TAB 0 (Code): VS Code Syntax-Highlighted Code with Line Gutter
        // Sockets rendered as border dots with horizontal connector lines (clean code lines)
        // Sub-nodes separated with collapsible boundaries (no heavy row highlight)
        let gutter_width = (36.0 * z).max(20.0);
        let gutter_rect = Rect::from_min_max(
            Pos2::new(props.rect.min.x, body_rect.min.y),
            Pos2::new(props.rect.min.x + gutter_width, props.rect.max.y),
        );

        body_clip.rect_filled(gutter_rect, CornerRadius::ZERO, CODE_GUTTER_BG);
        body_clip.line_segment(
            [
                Pos2::new(gutter_rect.max.x, body_rect.min.y),
                Pos2::new(gutter_rect.max.x, props.rect.max.y),
            ],
            Stroke::new((1.0 * z).max(0.5), PANEL_BORDER),
        );

        let line_height = (18.0 * z).max(8.0);
        let font_size = (11.0 * z).max(5.0);
        let hl = SyntaxHighlighter::global();

        let lines: Vec<&str> = props.code.lines().collect();
        let mut skip_until_line: Option<usize> = None;

        for (i, line) in lines.iter().enumerate() {
            let line_num = props.start_line + i;

            if let Some(skip_end) = skip_until_line {
                if line_num <= skip_end {
                    continue;
                } else {
                    skip_until_line = None;
                }
            }

            let line_y = body_rect.min.y + 8.0 * z + (i as f32 * line_height) - props.scroll_y;
            if line_y + line_height < body_rect.min.y {
                continue;
            }
            if line_y > body_rect.max.y {
                break;
            }

            line_y_offsets.push(line_y);

            // Line number in gutter
            let line_num_str = format!("{}", line_num);
            body_clip.text(
                Pos2::new(gutter_rect.max.x - 6.0 * z, line_y + line_height * 0.5),
                egui::Align2::RIGHT_CENTER,
                line_num_str,
                FontId::new(font_size * 0.9, FontFamily::Monospace),
                CODE_GUTTER_TEXT,
            );

            let member_match = props.members.iter().find(|m| m.line_number == line_num);

            // Sub-nodes visual separation: Subtle divider line and fold caret
            if let Some(m) = member_match {
                // Subtle horizontal boundary line above function
                body_clip.line_segment(
                    [
                        Pos2::new(gutter_rect.max.x + 2.0 * z, line_y - 1.0 * z),
                        Pos2::new(body_rect.max.x - 8.0 * z, line_y - 1.0 * z),
                    ],
                    Stroke::new((1.0 * z).max(0.5), with_alpha(m.archetype_color, 80)),
                );

                let is_collapsed = props.collapsed_subnodes
                    .map(|s| s.contains(m.id) || s.contains(m.name))
                    .unwrap_or(false);

                // Fold Caret [▼] or [▶]
                let caret_size = Vec2::splat(12.0 * z);
                let caret_rect = Rect::from_min_size(
                    Pos2::new(gutter_rect.min.x + 2.0 * z, line_y + (line_height - 12.0 * z) * 0.5),
                    caret_size,
                );
                let is_caret_hov = props.hovered_pos.map(|p| caret_rect.contains(p)).unwrap_or(false);
                if is_caret_hov {
                    body_clip.rect_filled(caret_rect, CornerRadius::from(2.0 * z), Color32::from_rgba_premultiplied(99, 102, 241, 40));
                }
                body_clip.text(
                    caret_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if is_collapsed { egui_phosphor::regular::CARET_RIGHT } else { egui_phosphor::regular::CARET_DOWN },
                    FontId::new((9.5 * z).max(4.5), FontFamily::Proportional),
                    if is_caret_hov { TEXT_HIGHLIGHT } else { TEXT_DIM },
                );
                member_fold_clicks.push((caret_rect, m.id.to_string()));

                if is_collapsed {
                    let member_lines_count = m.source_code.lines().count().max(1);
                    if member_lines_count > 1 {
                        skip_until_line = Some(line_num + member_lines_count - 1);
                    }
                }
            }

            // Input Port socket at this line: Dot on edge + connector line into code
            let in_port = if let Some(m) = member_match {
                props.inputs.iter().find(|p| p.name.starts_with(m.name) || p.name == format!("{}:in", m.name) || props.inputs.len() == 1)
            } else if line_num == 1 {
                props.inputs.iter().find(|p| p.name == "in" || (!p.name.contains(':') && props.members.is_empty()) || props.inputs.len() == 1)
            } else {
                None
            };

            if let Some(inp) = in_port {
                let socket_pos = Pos2::new(props.rect.min.x, line_y + line_height * 0.5);
                let socket_r = (3.5 * z).max(2.0);

                // Horizontal connector line extending into code
                body_clip.line_segment(
                    [socket_pos, Pos2::new(gutter_rect.max.x + 4.0 * z, socket_pos.y)],
                    Stroke::new((1.0 * z).max(0.5), with_alpha(inp.type_color, 150)),
                );

                // Circular socket dot at left border
                body_clip.circle_filled(socket_pos, socket_r, inp.type_color);
                body_clip.circle_stroke(
                    socket_pos,
                    socket_r,
                    Stroke::new((1.0 * z).max(0.5), Color32::from_white_alpha(200)),
                );

                let hit_rect = Rect::from_center_size(socket_pos, Vec2::splat(14.0 * z));
                if let Some(target_id) = inp.connected_node_id {
                    jump_clicks.push((hit_rect, target_id));
                }
            }

            // Output Port socket at this line: Dot on edge + connector line into code
            let out_port = if let Some(m) = member_match {
                props.outputs.iter().find(|p| p.name.starts_with(m.name) || p.name == format!("{}:out", m.name) || props.outputs.len() == 1)
            } else if line_num == 1 {
                props.outputs.iter().find(|p| p.name == "out" || (!p.name.contains(':') && props.members.is_empty()) || props.outputs.len() == 1)
            } else {
                None
            };

            if let Some(outp) = out_port {
                let socket_pos = Pos2::new(props.rect.max.x, line_y + line_height * 0.5);
                let socket_r = (3.5 * z).max(2.0);

                // Horizontal connector line extending into code from right edge
                body_clip.line_segment(
                    [Pos2::new(props.rect.max.x - 20.0 * z, socket_pos.y), socket_pos],
                    Stroke::new((1.0 * z).max(0.5), with_alpha(outp.type_color, 150)),
                );

                // Circular socket dot at right border
                body_clip.circle_filled(socket_pos, socket_r, outp.type_color);
                body_clip.circle_stroke(
                    socket_pos,
                    socket_r,
                    Stroke::new((1.0 * z).max(0.5), Color32::from_white_alpha(200)),
                );

                let hit_rect = Rect::from_center_size(socket_pos, Vec2::splat(14.0 * z));
                if let Some(target_id) = outp.connected_node_id {
                    jump_clicks.push((hit_rect, target_id));
                }
            }

            // Clean Code Line with VS Code Syntax Highlighting (NO obscuring text chips!)
            let code_start_x = gutter_rect.max.x + 8.0 * z;
            let tokens = hl.highlight_line_tokens(&lang, line);
            let mut job = egui::text::LayoutJob::default();
            for (color, token) in tokens {
                job.append(
                    token,
                    0.0,
                    egui::TextFormat {
                        font_id: FontId::new(font_size, FontFamily::Monospace),
                        color,
                        ..Default::default()
                    },
                );
            }

            let galley = body_clip.layout_job(job);
            body_clip.galley(Pos2::new(code_start_x, line_y), galley, Color32::WHITE);
        }
    }

    CodeCardLayout {
        line_y_offsets,
        close_button_rect,
        preview_toggle_rect: None,
        tab_rects,
        subnode_clicks,
        member_fold_clicks,
        jump_clicks,
        member_row_clicks,
        member_code_clicks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paint_code_card_tabs_and_jump_clicks() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::background());
            let rect = Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(500.0, 400.0));

            let input_port = PortDisplayInfo {
                id: 10,
                name: "req",
                type_name: "Request",
                type_color: Color32::GREEN,
                connected_node_title: Some("Client".to_string()),
                connected_node_id: Some(1),
            };
            let output_port = PortDisplayInfo {
                id: 20,
                name: "res",
                type_name: "Response",
                type_color: Color32::BLUE,
                connected_node_title: Some("Server".to_string()),
                connected_node_id: Some(2),
            };

            let member = FileCardMember {
                id: "m1",
                name: "handle",
                archetype_tag: "FN",
                archetype_color: Color32::LIGHT_BLUE,
                visibility: "pub",
                signature: "pub fn handle()",
                line_number: 1,
                source_code: "pub fn handle() {}",
            };

            let inputs = [input_port];
            let outputs = [output_port];
            let members = [member];

            // Test tab 0 (Code) produces 3 tabs, 2 jump click targets on edge sockets, and 1 member fold click
            let layout = paint_code_card(
                &painter,
                CodeCardProps {
                    rect,
                    title: "handler.rs",
                    file_path: Some("src/handler.rs"),
                    start_line: 1,
                    code: "fn handle() {}",
                    doc_comment: Some("Handler function"),
                    is_selected: false,
                    zoom: 1.0,
                    language_hint: Some("RS"),
                    active_tab: 0,
                    is_markdown_preview: false,
                    members: &members,
                    expanded_member_id: None,
                    collapsed_subnodes: None,
                    inputs: &inputs,
                    outputs: &outputs,
                    hovered_pos: None,
                    scroll_y: 0.0,
                },
            );

            assert_eq!(layout.tab_rects.len(), 3);
            assert_eq!(layout.jump_clicks.len(), 2);
            assert_eq!(layout.jump_clicks[0].1, 1);
            assert_eq!(layout.jump_clicks[1].1, 2);
            assert_eq!(layout.member_fold_clicks.len(), 1);
            assert_eq!(layout.member_fold_clicks[0].1, "m1");

            // Test tab 1 (Nodes) produces member row clicks and member code clicks
            let layout_nodes = paint_code_card(
                &painter,
                CodeCardProps {
                    rect,
                    title: "handler.rs",
                    file_path: Some("src/handler.rs"),
                    start_line: 1,
                    code: "fn handle() {}",
                    doc_comment: Some("Handler function"),
                    is_selected: false,
                    zoom: 1.0,
                    language_hint: Some("RS"),
                    active_tab: 1,
                    is_markdown_preview: false,
                    members: &members,
                    expanded_member_id: Some("m1"),
                    collapsed_subnodes: None,
                    inputs: &inputs,
                    outputs: &outputs,
                    hovered_pos: None,
                    scroll_y: 0.0,
                },
            );
            assert_eq!(layout_nodes.member_row_clicks.len(), 1);
            assert_eq!(layout_nodes.member_row_clicks[0].1, "m1");
            assert_eq!(layout_nodes.member_code_clicks.len(), 1);
            assert_eq!(layout_nodes.member_code_clicks[0].1, "m1");
            assert_eq!(layout_nodes.subnode_clicks.len(), 1);

            // Test tab 2 (Documentation) is pure documentation with 0 member clicks
            let layout_docs = paint_code_card(
                &painter,
                CodeCardProps {
                    rect,
                    title: "handler.rs",
                    file_path: Some("src/handler.rs"),
                    start_line: 1,
                    code: "fn handle() {}",
                    doc_comment: Some("Handler function"),
                    is_selected: false,
                    zoom: 1.0,
                    language_hint: Some("RS"),
                    active_tab: 2,
                    is_markdown_preview: false,
                    members: &members,
                    expanded_member_id: None,
                    collapsed_subnodes: None,
                    inputs: &inputs,
                    outputs: &outputs,
                    hovered_pos: None,
                    scroll_y: 0.0,
                },
            );
            assert_eq!(layout_docs.member_row_clicks.len(), 0);
            assert_eq!(layout_docs.member_code_clicks.len(), 0);
        });
        // Font atlas uploads are normally consumed by the renderer.
        output.textures_delta.clear();
    }
}

