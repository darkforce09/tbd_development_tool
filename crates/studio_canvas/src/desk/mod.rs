//! The Desk, below the Code map: the files you open, side by side, as real text you can select
//! and copy, with a navigator that reaches any file of a part in a few clicks.
//!
//! The canvas owns what is on the Desk; the host reads the files it asks for (off the UI thread)
//! and hands their contents back with [`DeskState::fill`].

pub mod link;
pub mod navigator;
pub mod plan;
pub mod session;
pub mod view;

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use studio_ui::{CodeDocument, GutterMark, GutterMarkKind, STALE_TIP};

pub use link::{parse_link, LinkTarget};
pub use navigator::{Navigator, NavigatorColumn, NavigatorRow};
pub use plan::{EditMatch, PlanBody, PlanDoc, PlanEdit, PlanStep};
pub use session::SessionLights;

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
    /// An agent session's plan, its steps and its edits (built by the host from the session log).
    Plan(Box<PlanDoc>),
}

/// How a range of lines is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LitKind {
    /// Lit: a link's lines, or an edit found exactly once (Observed).
    Lit,
    /// One of several places an edit could be: marked, not lit (Possible set).
    Candidate,
    /// Where an edit was, which the file no longer holds: an amber tick.
    Stale,
}

/// Lines lit on a card, and what lit them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LitRange {
    /// Lines (1-based).
    pub range: RangeInclusive<usize>,
    pub kind: LitKind,
    /// What lit them, e.g. "session brave-blue-fox · Edit", or [`LINK_SOURCE`].
    pub source: String,
}

/// The source of lines lit by a clicked link.
pub const LINK_SOURCE: &str = "link";

impl LitRange {
    pub fn new(range: RangeInclusive<usize>, kind: LitKind, source: impl Into<String>) -> Self {
        Self { range, kind, source: source.into() }
    }
}

/// What a card's code view shows for its lights: the banded ranges and the gutter marks. Only
/// [`LitKind::Lit`] ranges are banded; lines lit by a link get no mark (they are not an edit).
pub fn code_lights<'a>(
    lights: impl IntoIterator<Item = &'a LitRange>,
) -> (Vec<RangeInclusive<usize>>, Vec<GutterMark>) {
    let mut lit = Vec::new();
    let mut marks = Vec::new();
    for l in lights {
        let kind = match l.kind {
            LitKind::Lit => {
                lit.push(l.range.clone());
                if l.source == LINK_SOURCE {
                    continue;
                }
                GutterMarkKind::Edit
            }
            LitKind::Candidate => GutterMarkKind::Candidate,
            LitKind::Stale => GutterMarkKind::Stale,
        };
        let tip = match l.kind {
            LitKind::Stale => format!("{STALE_TIP} ({})", l.source),
            _ => l.source.clone(),
        };
        marks.push(GutterMark { lines: l.range.clone(), kind, tip });
    }
    (lit, marks)
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
    /// Lines lit by what opened the card (a link, a plan's edit).
    pub lit: Vec<LitRange>,
    /// Lines lit by the agent session chosen in the Changes district.
    pub session_lit: Vec<LitRange>,
    /// For Markdown: show the source instead of the rendered document.
    pub show_source: bool,
    /// For a plan card: the session (id) whose plan it shows. Its `path` is then empty.
    pub session: Option<String>,
    /// For a plan card: its name (the session's slug).
    pub name: Option<String>,
}

impl DeskCard {
    pub fn title(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        self.path.file_name().map_or_else(|| self.rel.clone(), |n| n.to_string_lossy().into_owned())
    }

    /// Every light on the card: what opened it, then the chosen session's.
    pub fn lights(&self) -> impl Iterator<Item = &LitRange> {
        self.lit.iter().chain(&self.session_lit)
    }

    /// The card's text, once read (code or Markdown).
    pub fn document(&self) -> Option<&CodeDocument> {
        match &self.content {
            CardContent::Code(doc) | CardContent::Markdown(doc) => Some(doc),
            _ => None,
        }
    }
}

/// What the Desk wants from the host or the map this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum DeskRequest {
    /// Read this file for that card.
    Read { card: u64, path: PathBuf },
    /// Build the plan card of this agent session (id).
    Plan { card: u64, session: String },
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
    /// The chosen agent session's files, for lighting them on the map.
    pub session: SessionLights,
    next_id: u64,
}

impl DeskState {
    /// Opens `path` on the Desk at `line`, or brings its card forward if it is open already.
    /// Returns the card's id.
    pub fn open(&mut self, path: &Path, line: Option<usize>) -> u64 {
        if let Some(card) = self.cards.iter_mut().find(|c| c.session.is_none() && c.path == path) {
            card.scroll_to = line.or(card.scroll_to);
            self.reveal = Some(card.id);
            return card.id;
        }
        let rel = self
            .root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let id = self.push_card(path.to_path_buf(), rel, line, None, None);
        self.requests.push(DeskRequest::Read { card: id, path: path.to_path_buf() });
        id
    }

    /// Opens `path` with `lights` on it (replacing the lights that opened it before), scrolled
    /// to the first; an open card is brought forward. A Markdown file is shown as source, where
    /// its lines can be seen. Returns the card's id.
    pub fn open_lit(&mut self, path: &Path, lights: Vec<LitRange>) -> u64 {
        let first = lights.iter().map(|l| *l.range.start()).min();
        let id = self.open(path, first);
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id) {
            if first.is_some() {
                card.scroll_to = first;
                card.show_source = true;
            }
            card.lit = lights;
        }
        id
    }

    /// Opens the plan card of an agent session (id), named `name`, or brings it forward.
    /// Returns the card's id.
    pub fn open_plan(&mut self, session: &str, name: &str) -> u64 {
        if let Some(card) = self.cards.iter().find(|c| c.session.as_deref() == Some(session)) {
            self.reveal = Some(card.id);
            return card.id;
        }
        let id = self.push_card(PathBuf::new(), "session plan".to_string(), None, Some(session), Some(name));
        self.requests.push(DeskRequest::Plan { card: id, session: session.to_string() });
        id
    }

    /// A new card, the one opened longest ago put away when the Desk is full.
    fn push_card(
        &mut self,
        path: PathBuf,
        rel: String,
        line: Option<usize>,
        session: Option<&str>,
        name: Option<&str>,
    ) -> u64 {
        if self.cards.len() >= MAX_CARDS {
            self.cards.remove(0);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.cards.push(DeskCard {
            id,
            path,
            rel,
            content: CardContent::Loading,
            scroll_to: line,
            lit: Vec::new(),
            session_lit: Vec::new(),
            show_source: false,
            session: session.map(str::to_string),
            name: name.map(str::to_string),
        });
        self.reveal = Some(id);
        id
    }

    /// Puts the chosen session's lights on a card, if it is still open.
    pub fn set_session_lights(&mut self, id: u64, lights: Vec<LitRange>) {
        if let Some(card) = self.cards.iter_mut().find(|c| c.id == id) {
            card.session_lit = lights;
        }
    }

    /// Takes every session light off every card.
    pub fn clear_session_lights(&mut self) {
        for card in &mut self.cards {
            card.session_lit.clear();
        }
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

    fn light(range: RangeInclusive<usize>, kind: LitKind) -> LitRange {
        LitRange::new(range, kind, "session s · Edit")
    }

    #[test]
    fn opening_lit_updates_an_open_card_and_scrolls_to_the_first_light() {
        let mut desk = DeskState::default();
        let a =
            desk.open_lit(Path::new("/p/a.rs"), vec![light(30..=32, LitKind::Lit), light(4..=4, LitKind::Candidate)]);
        assert_eq!(desk.cards[0].scroll_to, Some(4));
        assert_eq!(desk.requests.len(), 1);
        desk.cards[0].scroll_to = None;
        desk.reveal = None;

        // Already open: its lights are replaced, nothing is read again.
        let again = desk.open_lit(Path::new("/p/a.rs"), vec![light(10..=12, LitKind::Lit)]);
        assert_eq!(again, a);
        assert_eq!(desk.cards.len(), 1);
        assert_eq!(desk.requests.len(), 1);
        assert_eq!(desk.cards[0].lit, [light(10..=12, LitKind::Lit)]);
        assert_eq!((desk.cards[0].scroll_to, desk.reveal), (Some(10), Some(a)));

        // A plain open keeps the lights; Markdown opened lit shows its source.
        desk.open(Path::new("/p/a.rs"), Some(1));
        assert_eq!(desk.cards[0].lit.len(), 1);
        desk.open_lit(Path::new("/p/README.md"), vec![light(5..=5, LitKind::Lit)]);
        assert!(desk.cards[1].show_source);
        desk.open_lit(Path::new("/p/b.md"), Vec::new());
        assert!(!desk.cards[2].show_source && desk.cards[2].scroll_to.is_none());
    }

    #[test]
    fn plan_cards_open_once_and_count_toward_the_limit() {
        let mut desk = DeskState::default();
        let plan = desk.open_plan("s1", "brave-blue-fox");
        assert_eq!(desk.requests, [DeskRequest::Plan { card: plan, session: "s1".into() }]);
        assert_eq!(desk.cards[0].title(), "brave-blue-fox");
        assert_eq!(desk.open_plan("s1", "brave-blue-fox"), plan, "brought forward, not built again");
        assert_eq!(desk.requests.len(), 1);
        // A file with an empty path is not the plan card.
        assert_ne!(desk.open(Path::new(""), None), plan);

        for i in 0..MAX_CARDS - 2 {
            desk.open(&PathBuf::from(format!("/f{i}.rs")), None);
        }
        assert_eq!(desk.cards.len(), MAX_CARDS);
        desk.open_plan("s2", "calm-green-owl");
        assert_eq!(desk.cards.len(), MAX_CARDS);
        assert!(desk.cards.iter().all(|c| c.id != plan), "the plan opened first was put away first");
        assert_eq!(desk.cards.last().unwrap().session.as_deref(), Some("s2"));
    }

    #[test]
    fn session_lights_come_and_go_apart_from_the_cards_own() {
        let mut desk = DeskState::default();
        let a = desk.open_lit(Path::new("/p/a.rs"), vec![LitRange::new(1..=2, LitKind::Lit, LINK_SOURCE)]);
        desk.set_session_lights(a, vec![light(5..=5, LitKind::Lit), light(9..=9, LitKind::Stale)]);
        assert_eq!(desk.cards[0].lights().count(), 3);
        let (lit, marks) = code_lights(desk.cards[0].lights());
        assert_eq!(lit, [1..=2, 5..=5], "a link's lines are lit, not marked as an edit");
        let kinds: Vec<_> = marks.iter().map(|m| (m.lines.clone(), m.kind)).collect();
        assert_eq!(kinds, [(5..=5, GutterMarkKind::Edit), (9..=9, GutterMarkKind::Stale)]);
        assert!(marks[1].tip.starts_with(STALE_TIP));
        desk.clear_session_lights();
        assert_eq!(desk.cards[0].lights().count(), 1);
        desk.set_session_lights(999, vec![light(1..=1, LitKind::Lit)]);
        assert_eq!(desk.cards[0].session_lit.len(), 0, "a closed card gets nothing");
    }

    #[test]
    fn candidates_are_marked_not_lit() {
        let (lit, marks) = code_lights(&[light(3..=3, LitKind::Candidate), light(8..=8, LitKind::Candidate)]);
        assert!(lit.is_empty());
        assert!(marks.iter().all(|m| m.kind == GutterMarkKind::Candidate));
    }

    #[test]
    fn markdown_is_rendered_and_everything_else_is_code() {
        assert!(matches!(content_for_text(Path::new("README.md"), "# Hi".into()), CardContent::Markdown(_)));
        assert!(matches!(content_for_text(Path::new("Makefile"), "all:".into()), CardContent::Code(_)));
    }
}
