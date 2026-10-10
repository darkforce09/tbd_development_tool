//! Read-only code, the way the Desk shows it: line numbers, a gutter column kept for change
//! marks, highlighted text that can be selected and copied, and only the rows on screen laid out.
//!
//! A document is highlighted once per content, on a worker thread, in one pass that keeps the
//! highlighter's state from line to line, so a comment or string spanning lines is coloured all
//! the way through. Until the colours arrive the text shows plain.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex, OnceLock};

use egui::text::LayoutJob;
use egui::{vec2, Align2, Color32, CornerRadius, FontId, Id, Label, Rect, Sense, TextFormat, TextWrapMode, Ui, Vec2};
use syntect::easy::HighlightLines;

use crate::colors::*;
use crate::syntax::SyntaxHighlighter;

/// A file's text with its lines found once.
#[derive(Debug, Clone)]
pub struct CodeDocument {
    pub text: Arc<str>,
    /// Language or extension, as [`SyntaxHighlighter::find_syntax`] takes it.
    pub lang: String,
    /// Hash of the text and language: the highlight cache key.
    pub hash: u64,
    /// Byte range of each line, without its line break.
    lines: Vec<(usize, usize)>,
}

impl CodeDocument {
    pub fn new(text: impl Into<Arc<str>>, lang: impl Into<String>) -> Self {
        let text = text.into();
        let lang = lang.into();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        lang.hash(&mut hasher);
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                let end = if i > start && text.as_bytes()[i - 1] == b'\r' { i - 1 } else { i };
                lines.push((start, end));
                start = i + 1;
            }
        }
        if start < text.len() || lines.is_empty() {
            lines.push((start, text.len()));
        }
        Self { text, lang, hash: hasher.finish(), lines }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Line `i` (0-based), without its line break.
    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", |&(a, b)| &self.text[a..b])
    }
}

/// Colours of each line: (start byte, end byte, colour) runs within the line.
pub type Highlights = Vec<Vec<(u32, u32, Color32)>>;

/// Highlights a whole document in one pass, carrying the highlighter's state across lines.
pub fn highlight_document(doc: &CodeDocument) -> Highlights {
    let highlighter = SyntaxHighlighter::global();
    let syntax = highlighter.find_syntax(&doc.lang);
    let mut lines = HighlightLines::new(syntax, &highlighter.theme);
    let mut out = Vec::with_capacity(doc.line_count());
    let mut buffer = String::new();
    for i in 0..doc.line_count() {
        let line = doc.line(i);
        buffer.clear();
        buffer.push_str(line);
        buffer.push('\n');
        let mut runs = Vec::new();
        if let Ok(ranges) = lines.highlight_line(&buffer, &highlighter.syntax_set) {
            let mut at = 0usize;
            for (style, token) in ranges {
                let end = (at + token.len()).min(line.len());
                if end > at {
                    let fg = style.foreground;
                    runs.push((at as u32, end as u32, Color32::from_rgb(fg.r, fg.g, fg.b)));
                }
                at += token.len();
            }
        }
        out.push(runs);
    }
    out
}

/// Highlights already worked out (or being worked out), by document hash.
struct HighlightCache {
    done: HashMap<u64, Arc<Highlights>>,
    running: std::collections::HashSet<u64>,
    /// Hashes in the order they finished, to forget the oldest first.
    order: std::collections::VecDeque<u64>,
}

/// Most documents whose colours are kept.
const CACHE_DOCUMENTS: usize = 48;

fn highlight_cache() -> &'static Mutex<HighlightCache> {
    static CACHE: OnceLock<Mutex<HighlightCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(HighlightCache { done: HashMap::new(), running: Default::default(), order: Default::default() })
    })
}

/// The document's colours if they are ready; otherwise starts working them out on a worker
/// thread and calls `ready` when done (to ask for a repaint).
pub fn highlights_for(doc: &CodeDocument, ready: impl FnOnce() + Send + 'static) -> Option<Arc<Highlights>> {
    let mut cache = highlight_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = cache.done.get(&doc.hash) {
        return Some(h.clone());
    }
    if cache.running.insert(doc.hash) {
        let doc = doc.clone();
        rayon::spawn(move || {
            let highlights = Arc::new(highlight_document(&doc));
            let mut cache = highlight_cache().lock().unwrap_or_else(|e| e.into_inner());
            cache.running.remove(&doc.hash);
            cache.done.insert(doc.hash, highlights);
            cache.order.push_back(doc.hash);
            while cache.order.len() > CACHE_DOCUMENTS {
                if let Some(old) = cache.order.pop_front() {
                    cache.done.remove(&old);
                }
            }
            drop(cache);
            ready();
        });
    }
    None
}

/// One line as text runs for egui.
fn line_job(line: &str, runs: Option<&[(u32, u32, Color32)]>, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let format = |color| TextFormat { font_id: font.clone(), color, ..Default::default() };
    let mut at = 0usize;
    for &(start, end, color) in runs.unwrap_or_default() {
        let (start, end) = (start as usize, (end as usize).min(line.len()));
        if start > at {
            job.append(&line[at..start], 0.0, format(TEXT_PRIMARY));
        }
        if end > start && line.is_char_boundary(start) && line.is_char_boundary(end) {
            job.append(&line[start..end], 0.0, format(color));
            at = end;
        }
    }
    if at < line.len() {
        job.append(&line[at..], 0.0, format(TEXT_PRIMARY));
    }
    job
}

/// What the code view did this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CodeViewResponse {
    /// Lines on screen (1-based).
    pub visible: Option<RangeInclusive<usize>>,
}

/// A read-only view of a document.
pub struct CodeView<'a> {
    doc: &'a CodeDocument,
    id: Id,
    /// Number of the first line (1-based), for documents that are a slice of a file.
    first_line: usize,
    /// Lines (1-based) shown lit, e.g. what a plan step changed.
    lit: Option<RangeInclusive<usize>>,
    /// Scroll so this line (1-based) is near the top, once.
    scroll_to: Option<usize>,
    font_size: f32,
}

impl<'a> CodeView<'a> {
    pub fn new(doc: &'a CodeDocument, id: Id) -> Self {
        Self { doc, id, first_line: 1, lit: None, scroll_to: None, font_size: 12.5 }
    }

    pub fn lit(mut self, lines: Option<RangeInclusive<usize>>) -> Self {
        self.lit = lines;
        self
    }

    pub fn scroll_to(mut self, line: Option<usize>) -> Self {
        self.scroll_to = line;
        self
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    pub fn show(self, ui: &mut Ui) -> CodeViewResponse {
        let font = FontId::monospace(self.font_size);
        let row_height = (self.font_size * 1.45).round();
        let ctx = ui.ctx().clone();
        let highlights = highlights_for(self.doc, move || ctx.request_repaint());
        let count = self.doc.line_count();
        let last = self.first_line + count.saturating_sub(1);
        let gutter = (last.to_string().len() as f32 * self.font_size * 0.62 + 18.0).ceil();

        let mut area = egui::ScrollArea::both().id_salt(self.id).auto_shrink([false, false]);
        if let Some(line) = self.scroll_to {
            let row = line.saturating_sub(self.first_line).saturating_sub(3);
            area = area.vertical_scroll_offset(row as f32 * row_height);
        }
        let mut visible = None;
        area.show_rows(ui, row_height, count, |ui, rows| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            visible = Some(self.first_line + rows.start..=self.first_line + rows.end.saturating_sub(1));
            for i in rows {
                let number = self.first_line + i;
                let lit = self.lit.as_ref().is_some_and(|r| r.contains(&number));
                ui.horizontal(|ui| {
                    ui.set_height(row_height);
                    let (row, _) = ui.allocate_exact_size(vec2(gutter, row_height), Sense::hover());
                    if lit {
                        let width = (ui.clip_rect().right() - row.left()).max(gutter);
                        let band = Rect::from_min_size(row.min, vec2(width, row_height));
                        ui.painter().rect_filled(band, CornerRadius::ZERO, CODE_LINE_HIGHLIGHT);
                    }
                    // The first 4 points of the gutter are kept for change marks.
                    ui.painter().text(
                        row.right_center() - vec2(10.0, 0.0),
                        Align2::RIGHT_CENTER,
                        number.to_string(),
                        font.clone(),
                        if lit { TEXT_SECONDARY } else { CODE_GUTTER_TEXT },
                    );
                    let runs = highlights.as_ref().and_then(|h| h.get(i)).map(Vec::as_slice);
                    let job = line_job(self.doc.line(i), runs, &font);
                    ui.add(Label::new(job).selectable(true).wrap_mode(TextWrapMode::Extend));
                });
            }
        });
        CodeViewResponse { visible }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color_of(doc: &CodeDocument, h: &Highlights, line: usize, needle: &str) -> Color32 {
        let at = doc.line(line).find(needle).unwrap() as u32;
        h[line].iter().find(|r| r.0 <= at && at < r.1).map(|r| r.2).unwrap()
    }

    #[test]
    fn lines_split_on_any_line_break() {
        let doc = CodeDocument::new("a\r\nbb\n\nc", "rs");
        assert_eq!(doc.line_count(), 4);
        assert_eq!([doc.line(0), doc.line(1), doc.line(2), doc.line(3)], ["a", "bb", "", "c"]);
        assert_eq!(CodeDocument::new("", "rs").line_count(), 1);
        assert_eq!(CodeDocument::new("x\n", "rs").line_count(), 1);
    }

    #[test]
    fn a_comment_spanning_lines_is_coloured_all_the_way_through() {
        let doc = CodeDocument::new("fn a() {}\n/* first\n   middle\n   last */\nfn b() {}\n", "rs");
        let h = highlight_document(&doc);
        let comment = color_of(&doc, &h, 1, "first");
        assert_eq!(color_of(&doc, &h, 2, "middle"), comment, "the state carries over");
        assert_eq!(color_of(&doc, &h, 3, "last"), comment);
        assert_ne!(color_of(&doc, &h, 4, "fn"), comment, "and ends with the comment");
    }

    #[test]
    fn runs_cover_each_line_in_order() {
        let doc = CodeDocument::new("let s = \"é ✓\"; // ünïcode\n", "rs");
        let h = highlight_document(&doc);
        let line = doc.line(0);
        let mut at = 0;
        for &(start, end, _) in &h[0] {
            assert!(start as usize >= at && end as usize <= line.len());
            assert!(line.is_char_boundary(start as usize) && line.is_char_boundary(end as usize));
            at = end as usize;
        }
        let job = line_job(line, Some(&h[0]), &FontId::monospace(12.0));
        assert_eq!(job.text, line, "nothing lost or doubled");
    }
}
