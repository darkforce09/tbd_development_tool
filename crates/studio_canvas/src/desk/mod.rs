//! The Desk, below the Code map: the files you open, side by side, as real text you can select
//! and copy, with a navigator that reaches any file of a part in a few clicks.
//!
//! The canvas owns what is on the Desk; the host reads the files it asks for (off the UI thread)
//! and hands their contents back with [`DeskState::fill`].

pub mod navigator;
pub mod view;

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use studio_ui::CodeDocument;

pub use navigator::{Navigator, NavigatorColumn, NavigatorRow};

/// Most cards on the Desk; opening one more puts away the one opened longest ago.
pub const MAX_CARDS: usize = 12;

/// What a card shows.
#[derive(Debug, Clone)]
pub enum CardContent {
    /// Being read.
    Loading,
    Code(CodeDocument),
    /// Markdown: shown rendered, or as source.
    Markdown(CodeDocument),
    /// Not text; its size in bytes.
    Binary(u64),
    /// Too large to show; its size in bytes.
    TooLarge(u64),
    Unreadable(String),
}

/// One open file.
#[derive(Debug, Clone)]
pub struct DeskCard {
    pub id: u64,
    pub path: PathBuf,
    /// Path inside the project, for the card's header.
    pub rel: String,
    pub content: CardContent,
    /// Line (1-based) to bring into view, once.
    pub scroll_to: Option<usize>,
    /// Lines (1-based) shown lit.
    pub lit: Option<RangeInclusive<usize>>,
    /// For Markdown: show the source instead of the rendered document.
    pub show_source: bool,
}

impl DeskCard {
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(|| self.rel.clone(), |n| n.to_string_lossy().into_owned())
    }
}

/// What the Desk wants from the host or the map this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DeskRequest {
    /// Read this file for that card.
    Read { card: u64, path: PathBuf },
}

/// Everything on the Desk.
#[derive(Debug, Clone, Default)]
pub struct DeskState {
    pub cards: Vec<DeskCard>,
    pub navigator: Navigator,
    /// The project's folder, for paths shown relative to it.
    pub root: Option<PathBuf>,
    /// For the host: files to read.
    pub requests: Vec<DeskRequest>,
    /// The card to bring into view on the Desk, once.
    pub reveal: Option<u64>,
    next_id: u64,
}

impl DeskState {
    /// Opens `path` on the Desk at `line`, or brings its card forward if it is open already.
    /// Returns the card's id.
    pub fn open(&mut self, path: &Path, line: Option<usize>) -> u64 {
        if let Some(card) = self.cards.iter_mut().find(|c| c.path == path) {
            card.scroll_to = line.or(card.scroll_to);
            self.reveal = Some(card.id);
            return card.id;
        }
        if self.cards.len() >= MAX_CARDS {
            self.cards.remove(0);
        }
        self.next_id += 1;
        let id = self.next_id;
        let rel = self
            .root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        self.cards.push(DeskCard {
            id,
            path: path.to_path_buf(),
            rel,
            content: CardContent::Loading,
            scroll_to: line,
            lit: None,
            show_source: false,
        });
        self.requests.push(DeskRequest::Read { card: id, path: path.to_path_buf() });
        self.reveal = Some(id);
        id
    }

    pub fn close(&mut self, id: u64) {
        self.cards.retain(|c| c.id != id);
    }

    /// Puts what was read into its card, if the card is still open.
    pub fn fill(&mut self, id: u64, content: CardContent) {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id) {
            card.content = content;
        }
    }

    /// Empties the Desk, for a newly opened project.
    pub fn clear(&mut self, root: Option<PathBuf>) {
        *self = DeskState { root, next_id: self.next_id, ..Default::default() };
    }
}

/// How a file's text is shown, from its name.
pub fn content_for_text(path: &Path, text: Arc<str>) -> CardContent {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let lang =
        if ext.is_empty() { path.file_name().and_then(|n| n.to_str()).unwrap_or("txt").to_string() } else { ext };
    let doc = CodeDocument::new(text, lang.clone());
    if matches!(lang.as_str(), "md" | "markdown") {
        CardContent::Markdown(doc)
    } else {
        CardContent::Code(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_reads_once_and_reopening_brings_the_card_forward() {
        let mut desk = DeskState { root: Some(PathBuf::from("/p")), ..Default::default() };
        let a = desk.open(Path::new("/p/src/a.rs"), None);
        assert_eq!(desk.cards[0].rel, "src/a.rs");
        assert_eq!(desk.requests, [DeskRequest::Read { card: a, path: PathBuf::from("/p/src/a.rs") }]);
        desk.requests.clear();

        assert_eq!(desk.open(Path::new("/p/src/a.rs"), Some(40)), a);
        assert!(desk.requests.is_empty(), "already open: nothing to read");
        assert_eq!(desk.cards[0].scroll_to, Some(40));
        assert_eq!(desk.reveal, Some(a));

        desk.fill(a, content_for_text(Path::new("/p/src/a.rs"), "fn a() {}".into()));
        assert!(matches!(desk.cards[0].content, CardContent::Code(_)));
        desk.close(a);
        assert!(desk.cards.is_empty());
        desk.fill(a, CardContent::Loading);
        assert!(desk.cards.is_empty(), "a closed card stays closed");
    }

    #[test]
    fn the_desk_keeps_the_last_twelve_in_the_order_opened() {
        let mut desk = DeskState::default();
        for i in 0..MAX_CARDS + 2 {
            desk.open(&PathBuf::from(format!("/f{i}.rs")), None);
        }
        assert_eq!(desk.cards.len(), MAX_CARDS);
        assert_eq!(desk.cards[0].path, PathBuf::from("/f2.rs"));
        assert_eq!(desk.cards.last().unwrap().path, PathBuf::from(format!("/f{}.rs", MAX_CARDS + 1)));
    }

    #[test]
    fn markdown_is_rendered_and_everything_else_is_code() {
        assert!(matches!(content_for_text(Path::new("README.md"), "# Hi".into()), CardContent::Markdown(_)));
        assert!(matches!(content_for_text(Path::new("Makefile"), "all:".into()), CardContent::Code(_)));
    }
}
