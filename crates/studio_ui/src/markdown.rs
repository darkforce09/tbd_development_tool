use egui::{
    Color32, FontFamily, FontId, RichText, CornerRadius, Stroke, Ui, Vec2,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::colors::*;
use crate::syntax::SyntaxHighlighter;

/// Configurable Markdown viewer widget for egui.
pub struct MarkdownViewer<'a> {
    markdown: &'a str,
    base_font_size: f32,
}

impl<'a> MarkdownViewer<'a> {
    pub fn new(markdown: &'a str) -> Self {
        Self {
            markdown,
            base_font_size: 11.5,
        }
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.base_font_size = size;
        self
    }

    pub fn show(self, ui: &mut Ui) {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        let parser = Parser::new_ext(self.markdown, options);
        let mut renderer = MarkdownRenderer {
            base_font_size: self.base_font_size,
            current_heading: None,
            in_blockquote: false,
            in_code_block: false,
            code_block_lang: String::new(),
            code_block_text: String::new(),
            in_table: false,
            table_headers: Vec::new(),
            table_rows: Vec::new(),
            current_table_row: Vec::new(),
            in_table_head: false,
            current_cell_text: String::new(),
            list_depth: 0,
            list_index: None,
            current_link_url: None,
            is_strong: false,
            is_emphasis: false,
            is_strikethrough: false,
            paragraph_spans: Vec::new(),
        };

        renderer.render(ui, parser);
    }
}

struct MarkdownSpan {
    text: String,
    is_strong: bool,
    is_emphasis: bool,
    is_strikethrough: bool,
    is_code: bool,
    link_url: Option<String>,
}

struct MarkdownRenderer {
    base_font_size: f32,
    current_heading: Option<HeadingLevel>,
    in_blockquote: bool,
    in_code_block: bool,
    code_block_lang: String,
    code_block_text: String,
    in_table: bool,
    table_headers: Vec<String>,
    table_rows: Vec<Vec<String>>,
    current_table_row: Vec<String>,
    in_table_head: bool,
    current_cell_text: String,
    list_depth: usize,
    list_index: Option<u64>,
    current_link_url: Option<String>,
    is_strong: bool,
    is_emphasis: bool,
    is_strikethrough: bool,
    paragraph_spans: Vec<MarkdownSpan>,
}

impl MarkdownRenderer {
    fn render(&mut self, ui: &mut Ui, parser: Parser) {
        for event in parser {
            match event {
                Event::Start(tag) => self.start_tag(ui, tag),
                Event::End(tag_end) => self.end_tag(ui, tag_end),
                Event::Text(text) => self.handle_text(&text),
                Event::Code(code) => self.handle_code(&code),
                Event::Rule => self.handle_rule(ui),
                Event::TaskListMarker(checked) => self.handle_task_marker(checked),
                Event::SoftBreak | Event::HardBreak => self.handle_break(),
                _ => {}
            }
        }

        // Flush any remaining paragraph spans
        self.flush_paragraph(ui);
    }

    fn start_tag(&mut self, ui: &mut Ui, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_paragraph(ui);
                self.current_heading = Some(level);
            }
            Tag::Paragraph => {
                self.flush_paragraph(ui);
            }
            Tag::BlockQuote(_) => {
                self.flush_paragraph(ui);
                self.in_blockquote = true;
            }
            Tag::CodeBlock(kind) => {
                self.flush_paragraph(ui);
                self.in_code_block = true;
                self.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_block_text.clear();
            }
            Tag::List(first_num) => {
                self.flush_paragraph(ui);
                self.list_depth += 1;
                self.list_index = first_num;
            }
            Tag::Item => {
                self.flush_paragraph(ui);
            }
            Tag::Table(_) => {
                self.flush_paragraph(ui);
                self.in_table = true;
                self.table_headers.clear();
                self.table_rows.clear();
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.current_table_row.clear();
            }
            Tag::TableRow => {
                self.current_table_row.clear();
            }
            Tag::TableCell => {
                self.current_cell_text.clear();
            }
            Tag::Strong => self.is_strong = true,
            Tag::Emphasis => self.is_emphasis = true,
            Tag::Strikethrough => self.is_strikethrough = true,
            Tag::Link { dest_url, .. } => {
                self.current_link_url = Some(dest_url.to_string());
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, ui: &mut Ui, tag_end: TagEnd) {
        match tag_end {
            TagEnd::Heading(_) => {
                self.render_heading(ui);
                self.current_heading = None;
            }
            TagEnd::Paragraph => {
                self.flush_paragraph(ui);
                ui.add_space(4.0);
            }
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph(ui);
                self.in_blockquote = false;
                ui.add_space(6.0);
            }
            TagEnd::CodeBlock => {
                self.render_code_block(ui);
                self.in_code_block = false;
                self.code_block_text.clear();
                self.code_block_lang.clear();
                ui.add_space(6.0);
            }
            TagEnd::List(_) => {
                self.flush_paragraph(ui);
                if self.list_depth > 0 {
                    self.list_depth -= 1;
                }
                self.list_index = None;
                ui.add_space(4.0);
            }
            TagEnd::Item => {
                self.render_list_item(ui);
            }
            TagEnd::Table => {
                self.render_table(ui);
                self.in_table = false;
                ui.add_space(6.0);
            }
            TagEnd::TableHead => {
                self.table_headers = std::mem::take(&mut self.current_table_row);
                self.in_table_head = false;
            }
            TagEnd::TableRow
                if !self.in_table_head => {
                    let row = std::mem::take(&mut self.current_table_row);
                    if !row.is_empty() {
                        self.table_rows.push(row);
                    }
                }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.current_cell_text);
                self.current_table_row.push(cell.trim().to_string());
            }
            TagEnd::Strong => self.is_strong = false,
            TagEnd::Emphasis => self.is_emphasis = false,
            TagEnd::Strikethrough => self.is_strikethrough = false,
            TagEnd::Link => self.current_link_url = None,
            _ => {}
        }
    }

    fn handle_text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_block_text.push_str(text);
        } else if self.in_table {
            self.current_cell_text.push_str(text);
        } else {
            self.paragraph_spans.push(MarkdownSpan {
                text: text.to_string(),
                is_strong: self.is_strong,
                is_emphasis: self.is_emphasis,
                is_strikethrough: self.is_strikethrough,
                is_code: false,
                link_url: self.current_link_url.clone(),
            });
        }
    }

    fn handle_code(&mut self, code: &str) {
        if self.in_table {
            self.current_cell_text.push_str(code);
        } else {
            self.paragraph_spans.push(MarkdownSpan {
                text: code.to_string(),
                is_strong: false,
                is_emphasis: false,
                is_strikethrough: false,
                is_code: true,
                link_url: self.current_link_url.clone(),
            });
        }
    }

    fn handle_rule(&mut self, ui: &mut Ui) {
        self.flush_paragraph(ui);
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
    }

    fn handle_task_marker(&mut self, checked: bool) {
        let icon = if checked {
            egui_phosphor::regular::CHECK_SQUARE
        } else {
            egui_phosphor::regular::SQUARE
        };
        self.paragraph_spans.push(MarkdownSpan {
            text: format!("{} ", icon),
            is_strong: false,
            is_emphasis: false,
            is_strikethrough: false,
            is_code: false,
            link_url: None,
        });
    }

    fn handle_break(&mut self) {
        self.paragraph_spans.push(MarkdownSpan {
            text: "\n".to_string(),
            is_strong: false,
            is_emphasis: false,
            is_strikethrough: false,
            is_code: false,
            link_url: None,
        });
    }

    fn flush_paragraph(&mut self, ui: &mut Ui) {
        if self.paragraph_spans.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.paragraph_spans);
        let font_size = self.base_font_size;

        if self.in_blockquote {
            // Blockquote container styling
            ui.horizontal(|ui| {
                let quote_accent = ARCHETYPE_FILE;
                ui.add_space(8.0);
                // Vertical bar
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::from(1.5), quote_accent);
                ui.add_space(6.0);

                ui.vertical(|ui| {
                    render_spans_flow(ui, &spans, font_size, true);
                });
            });
        } else {
            render_spans_flow(ui, &spans, font_size, false);
        }
    }

    fn render_heading(&mut self, ui: &mut Ui) {
        if self.paragraph_spans.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.paragraph_spans);
        let level = self.current_heading.unwrap_or(HeadingLevel::H1);

        let (size_multiplier, color, show_divider) = match level {
            HeadingLevel::H1 => (1.65, VSCODE_KEYWORD, true),
            HeadingLevel::H2 => (1.40, TEXT_HIGHLIGHT, true),
            HeadingLevel::H3 => (1.20, TEXT_PRIMARY, false),
            HeadingLevel::H4 => (1.10, TEXT_PRIMARY, false),
            HeadingLevel::H5 => (1.00, TEXT_SECONDARY, false),
            HeadingLevel::H6 => (0.90, TEXT_DIM, false),
        };

        ui.add_space(8.0);
        let heading_font_size = self.base_font_size * size_multiplier;

        ui.horizontal_wrapped(|ui| {
            for span in spans {
                let rt = RichText::new(&span.text)
                    .font(FontId::new(heading_font_size, FontFamily::Proportional))
                    .color(color)
                    .strong();
                ui.label(rt);
            }
        });

        if show_divider {
            ui.add_space(2.0);
            ui.separator();
        }
        ui.add_space(4.0);
    }

    fn render_list_item(&mut self, ui: &mut Ui) {
        if self.paragraph_spans.is_empty() {
            return;
        }

        let spans = std::mem::take(&mut self.paragraph_spans);
        let indent = (self.list_depth as f32 * 14.0).max(10.0);
        let font_size = self.base_font_size;

        let bullet_text = if let Some(idx) = self.list_index {
            self.list_index = Some(idx + 1);
            format!("{}. ", idx)
        } else if self.list_depth > 1 {
            "◦ ".to_string()
        } else {
            "• ".to_string()
        };

        ui.horizontal(|ui| {
            ui.add_space(indent);

            ui.label(
                RichText::new(bullet_text)
                    .font(FontId::new(font_size, FontFamily::Monospace))
                    .color(ARCHETYPE_FILE)
                    .strong(),
            );

            render_spans_flow(ui, &spans, font_size, false);
        });
    }

    fn render_code_block(&mut self, ui: &mut Ui) {
        let code = self.code_block_text.trim_end();
        if code.is_empty() {
            return;
        }

        let lang_label = if self.code_block_lang.is_empty() {
            "CODE".to_string()
        } else {
            self.code_block_lang.to_uppercase()
        };

        let hl = SyntaxHighlighter::global();
        let lang = if self.code_block_lang.is_empty() {
            "rs"
        } else {
            &self.code_block_lang
        };

        let font_size = self.base_font_size * 0.95;

        // Container frame
        egui::Frame::NONE
            .fill(Color32::from_rgb(0x0e, 0x11, 0x17))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x28, 0x2e, 0x3d)))
            .corner_radius(CornerRadius::from(6.0))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                // Header badge
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&lang_label)
                            .font(FontId::new(9.0, FontFamily::Monospace))
                            .color(Color32::from_rgb(0x38, 0xbd, 0xf8))
                            .background_color(Color32::from_rgba_premultiplied(56, 189, 248, 30)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let lines_count = code.lines().count();
                        ui.label(
                            RichText::new(format!("{} lines", lines_count))
                                .font(FontId::new(9.5, FontFamily::Monospace))
                                .color(TEXT_DIM),
                        );
                    });
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // Highlighted lines
                for line in code.lines() {
                    let tokens = hl.highlight_line_tokens(lang, line);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (color, token) in tokens {
                            ui.label(
                                RichText::new(token)
                                    .font(FontId::new(font_size, FontFamily::Monospace))
                                    .color(color),
                            );
                        }
                    });
                }
            });
    }

    fn render_table(&mut self, ui: &mut Ui) {
        if self.table_headers.is_empty() && self.table_rows.is_empty() {
            return;
        }

        let headers = std::mem::take(&mut self.table_headers);
        let rows = std::mem::take(&mut self.table_rows);
        let font_size = self.base_font_size;

        egui::Frame::NONE
            .fill(CARD_BG)
            .stroke(Stroke::new(1.0, CARD_BORDER_NORMAL))
            .corner_radius(CornerRadius::from(4.0))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::Grid::new(ui.next_auto_id())
                    .striped(true)
                    .spacing(Vec2::new(12.0, 6.0))
                    .show(ui, |ui| {
                        // Header row
                        if !headers.is_empty() {
                            for header in &headers {
                                ui.label(
                                    RichText::new(header)
                                        .font(FontId::new(font_size, FontFamily::Proportional))
                                        .color(TEXT_HIGHLIGHT)
                                        .strong(),
                                );
                            }
                            ui.end_row();
                        }

                        // Data rows
                        for row in &rows {
                            for cell in row {
                                ui.label(
                                    RichText::new(cell)
                                        .font(FontId::new(font_size, FontFamily::Proportional))
                                        .color(TEXT_PRIMARY),
                                );
                            }
                            ui.end_row();
                        }
                    });
            });
    }
}

fn render_spans_flow(ui: &mut Ui, spans: &[MarkdownSpan], base_font_size: f32, is_quote: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;

        for span in spans {
            if span.text == "\n" {
                ui.end_row();
                continue;
            }

            if span.is_code {
                // Inline code pill badge
                let code_rt = RichText::new(&span.text)
                    .font(FontId::new(base_font_size * 0.92, FontFamily::Monospace))
                    .color(Color32::from_rgb(0xfa, 0xcc, 0x15))
                    .background_color(Color32::from_rgb(0x1f, 0x24, 0x33));
                ui.label(code_rt);
            } else if let Some(ref url) = span.link_url {
                // Clickable link
                let link_rt = RichText::new(&span.text)
                    .font(FontId::new(base_font_size, FontFamily::Proportional))
                    .color(VSCODE_LINK)
                    .underline();
                if ui.link(link_rt).on_hover_text(url).clicked() {
                    // Link navigation hook
                }
            } else {
                let mut text_color = if is_quote {
                    TEXT_SECONDARY
                } else {
                    TEXT_PRIMARY
                };
                if span.is_strikethrough {
                    text_color = TEXT_DIM;
                }

                let mut rt = RichText::new(&span.text)
                    .font(FontId::new(base_font_size, FontFamily::Proportional))
                    .color(text_color);

                if span.is_strong {
                    rt = rt.strong();
                }
                if span.is_emphasis || is_quote {
                    rt = rt.italics();
                }
                if span.is_strikethrough {
                    rt = rt.strikethrough();
                }

                ui.label(rt);
            }
        }
    });
}

/// Helper function to render a Markdown string directly into an egui Ui.
pub fn render_markdown(ui: &mut Ui, markdown: &str) {
    MarkdownViewer::new(markdown).show(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_parser_event_flow() {
        let sample = "# Header 1\n\nThis is a **bold** paragraph with `inline code`.\n\n- Item 1\n- Item 2\n\n```rust\nfn main() {}\n```";
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        let parser = Parser::new_ext(sample, options);
        let events: Vec<_> = parser.collect();
        assert!(!events.is_empty());
        let has_heading = events.iter().any(|e| matches!(e, Event::Start(Tag::Heading { .. })));
        let has_code = events.iter().any(|e| matches!(e, Event::Start(Tag::CodeBlock(..))));
        assert!(has_heading);
        assert!(has_code);
    }

    #[test]
    fn test_markdown_table_and_tasklist_events() {
        let sample = "| Name | Role |\n|---|---|\n| Alice | Dev |\n\n- [x] Done\n- [ ] Pending";
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        options.insert(Options::ENABLE_TASKLISTS);
        let parser = Parser::new_ext(sample, options);
        let events: Vec<_> = parser.collect();
        let has_table = events.iter().any(|e| matches!(e, Event::Start(Tag::Table(_))));
        let has_task = events.iter().any(|e| matches!(e, Event::TaskListMarker(_)));
        assert!(has_table);
        assert!(has_task);
    }
}
