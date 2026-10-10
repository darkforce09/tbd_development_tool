//! The Changes district, south of the map: one row per worktree (branch, changes, commits and the
//! agent sessions working in it), every local branch under them, and the tickets to do next.
//!
//! Laid out in the district's own units like the Run district: a fixed height (the world's
//! `CHANGES_HEIGHT`), so rows, lanes, the strip and the column fit it with "+k more", never
//! overlapping. Session facts come from agents' logs and are Observed; a ticket matched by a
//! branch name is Unresolved and drawn dashed and muted; a ticket's commit git confirmed is Proven.

use std::path::PathBuf;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};
use studio_graph::EvidenceTier;
use studio_ui::color_tokens::*;
use studio_ui::{truncate_with_ellipsis, with_alpha};

use crate::transform::CanvasTransform;
use crate::view::gate::InputGate;
use crate::world::WorldLayout;

/// What the Changes district shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangesView {
    /// Commits reachable from HEAD.
    pub commits_on_head: usize,
    /// Local branches.
    pub local_branches: usize,
    /// Of the last `commits_read` commits, how many carry a `Co-authored-by:` trailer.
    pub co_authored: usize,
    /// How many recent commits were read for `co_authored`.
    pub commits_read: usize,
    /// One row per worktree, the repository's own checkout first, then by path.
    pub rows: Vec<WorktreeRowView>,
    /// Every local branch, by name.
    pub branches: Vec<BranchView>,
    /// The tickets column on the right.
    pub tickets: TicketsColumnView,
    /// Said plainly instead of the rows and the strip ("No git repository here", "Reading git…").
    pub message: Option<String>,
    /// Said in every session lane instead of sessions ("No agent sessions for this project").
    pub sessions_message: Option<String>,
}

/// A worktree, as its row shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorktreeRowView {
    /// The branch checked out, or "detached at <short>".
    pub branch: String,
    /// "you" for the repository's own checkout, else the worktree's path.
    pub label: String,
    pub path: PathBuf,
    /// The repository's own checkout, not a linked worktree.
    pub is_main: bool,
    /// Commits ahead of and behind the branch's upstream, when it has one.
    pub ahead_behind: Option<(usize, usize)>,
    /// Uncommitted changes by kind, as (word, count), only the non-zero ones.
    pub change_counts: Vec<(&'static str, usize)>,
    /// Changed files with their added and removed lines, first few only.
    pub files: Vec<FileChangeView>,
    /// Changed files not listed in `files`.
    pub more_files: usize,
    /// Recent commits of the row's branch, newest first.
    pub commits: Vec<CommitBead>,
    /// Agent sessions whose root is this worktree, newest first.
    pub sessions: Vec<SessionChip>,
    /// Sessions not listed in `sessions`.
    pub more_sessions: usize,
    /// Ticket ids in the branch name that name a ticket (Unresolved: matched by branch name).
    pub tickets: Vec<String>,
}

/// A changed file in a row or a commit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileChangeView {
    /// Path relative to the worktree.
    pub path: String,
    pub added: usize,
    pub removed: usize,
    /// Where a renamed or copied file came from.
    pub old_path: Option<String>,
    /// Git counts no lines for it.
    pub binary: bool,
}

/// A commit on a row's track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommitBead {
    /// Full commit id.
    pub id: String,
    pub short: String,
    /// Unix time.
    pub time: i64,
    /// Has a `Co-authored-by:` trailer.
    pub co_authored: bool,
    /// The files it changed, once loaded (`None` until then).
    pub files: Option<Vec<FileChangeView>>,
    /// How long ago, in words ("3 h ago").
    pub age: String,
}

/// An agent session in a row's lane: slug, start and counts only (never titles or prompts).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionChip {
    pub id: String,
    pub slug: String,
    /// Unix milliseconds.
    pub started_ms: i64,
    /// Files it touched.
    pub touches: usize,
    /// It wrote a plan.
    pub has_plan: bool,
    /// When it started, in words ("3 h ago").
    pub started: String,
}

/// A local branch in the strip under the rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BranchView {
    pub name: String,
    pub ahead_behind: Option<(usize, usize)>,
    /// Unix time of its last commit.
    pub time: i64,
    /// Sessions seen on it.
    pub sessions: usize,
    /// How long ago its last commit was, in words.
    pub age: String,
}

/// The tickets column.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketsColumnView {
    /// Tickets by status, as (status, count).
    pub counts: Vec<(String, usize)>,
    /// The tickets to do next.
    pub next: Vec<TicketView>,
    /// Said plainly instead of the column ("No tickets here (.ai/tickets)").
    pub message: Option<String>,
    /// Tickets linked to a worktree row, each with its [`TicketLinkView`].
    pub linked: Vec<TicketView>,
    /// Ready or queued tickets not listed in `next`.
    pub more_next: usize,
}

/// A ticket, as the column lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TicketView {
    pub id: String,
    pub status: String,
    pub priority: Option<String>,
    pub title: String,
    /// The ticket's `.toml`.
    pub file: PathBuf,
    /// What git says about its `shipped_at` commit; `None` when it records none.
    pub shipped: Option<ShippedView>,
    /// How it is linked to a row, in [`TicketsColumnView::linked`].
    pub link: Option<TicketLinkView>,
}

/// A ticket's `shipped_at` commit, as git confirmed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShippedView {
    /// Git has the commit (short id): Proven, basis commit id.
    Commit(String),
    /// Git does not know the commit.
    NotInRepo,
    /// A commit is recorded but there was no git to ask.
    NotChecked,
}

impl ShippedView {
    /// In words.
    pub fn label(&self) -> String {
        match self {
            ShippedView::Commit(short) => format!("shipped in {short}"),
            ShippedView::NotInRepo => "commit not in this repo".to_string(),
            ShippedView::NotChecked => "not checked".to_string(),
        }
    }

    /// The link's tier: only a commit git confirmed is a link.
    pub fn tier(&self) -> Option<EvidenceTier> {
        matches!(self, ShippedView::Commit(_)).then_some(EvidenceTier::Proven)
    }
}

/// How a ticket is linked to a worktree row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketLinkView {
    /// Index into [`ChangesView::rows`].
    pub row: usize,
    pub tier: EvidenceTier,
    /// What it was worked out from, in words ("branch name").
    pub basis: &'static str,
}

/// What is picked in the district.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangesSelection {
    /// The worktree row picked.
    pub row: Option<usize>,
    /// The commit bead picked, as (row, bead).
    pub commit: Option<(usize, usize)>,
    /// The session lit on the map, copied from `CanvasState::session_lit` when painting.
    pub lit_session: Option<String>,
}

/// What a click in the district asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangesAction {
    /// Pick this worktree row.
    SelectRow(usize),
    /// Pick this commit bead, as (row, bead).
    SelectCommit(usize, usize),
    /// Read the files this commit (full id) changed.
    LoadCommitFiles(String),
    /// Light what this session touched on the map, or nothing.
    LightSession(Option<String>),
    /// Open this session's plan on the Desk.
    OpenSessionPlan(String),
    /// Open this file on the Desk.
    OpenFile(PathBuf),
    /// Fly the camera to this row.
    FlyToRow(usize),
}

/// Where each row, the branch strip and the tickets column sit, in the district's units.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangesLayout {
    /// The rows shown, in order (the first `rows.len()` of the view's).
    pub rows: Vec<Rect>,
    pub strip: Rect,
    pub tickets: Rect,
    /// Where the rows go (and the district's message, when it has one).
    pub rows_area: Rect,
    /// The "+k more worktrees" line, when not every row fits.
    pub more_rows: Option<Rect>,
    /// Branch chips per strip line.
    pub strip_columns: usize,
}

/// The district's height in local units (the world's `CHANGES_HEIGHT`, fixed).
pub const HEIGHT: f32 = 1000.0;

const MARGIN: f32 = 36.0;
const LABEL_Y: f32 = 92.0;
const TOP: f32 = 120.0;
const TICKETS_W: f32 = 620.0;
const COLUMN_GAP: f32 = 40.0;
const SECTION_GAP: f32 = 28.0;
const ROW_GAP: f32 = 12.0;
const MIN_ROW: f32 = 100.0;
const MAX_ROW: f32 = 200.0;
const MORE_H: f32 = 26.0;
const STRIP_HEADER: f32 = 30.0;
const CHIP: [f32; 2] = [220.0, 42.0];
const CHIP_GAP: f32 = 6.0;
const STRIP_LINES: usize = 5;
const PAD: f32 = 14.0;
const LEFT_W: f32 = 330.0;
const LANE_W: f32 = 500.0;
const PART_GAP: f32 = 24.0;
const SESSION_TOP: f32 = 28.0;
const SESSION_H: f32 = 26.0;
const SESSION_STEP: f32 = 30.0;
const BEAD: f32 = 14.0;
const BEAD_MIN_STEP: f32 = 18.0;
const BEAD_MAX_STEP: f32 = 64.0;
const FILES_TOP: f32 = 50.0;
const FILE_H: f32 = 24.0;
const FILE_STEP: f32 = 30.0;
const FILE_GAP: f32 = 6.0;
const MORE_CHIP_W: f32 = 96.0;
const TICKET_CARD: f32 = 58.0;
const LINKED_CARD: f32 = 76.0;
const CARD_GAP: f32 = 8.0;
const HEADING: f32 = 26.0;
const MAX_LINKED: usize = 3;

/// The accent of the repository's own checkout.
const MAIN_COLOR: Color32 = DISTRICT_CODE;
const ADDED: Color32 = Color32::from_rgb(0x30, 0xd1, 0x58);
const REMOVED: Color32 = Color32::from_rgb(0xff, 0x6b, 0x63);

/// Lays the district out in its own units: rows top left, the branch strip under them (as tall
/// as its branches need, at most five lines), the tickets column on the right.
pub fn layout(view: &ChangesView, width: f32) -> ChangesLayout {
    let bottom = HEIGHT - MARGIN;
    let tickets = Rect::from_min_max(pos2((width - MARGIN - TICKETS_W).max(MARGIN), TOP), pos2(width - MARGIN, bottom));
    let (left, right) = (MARGIN, (tickets.left() - COLUMN_GAP).max(MARGIN + CHIP[0]));
    let strip_columns = strip_columns(right - left);
    let lines = view.branches.len().div_ceil(strip_columns).clamp(1, STRIP_LINES);
    let strip_h = STRIP_HEADER + lines as f32 * (CHIP[1] + CHIP_GAP) - CHIP_GAP;
    let strip = Rect::from_min_max(pos2(left, bottom - strip_h), pos2(right, bottom));
    let rows_area = Rect::from_min_max(pos2(left, TOP), pos2(right, strip.top() - SECTION_GAP));

    let fits = |h: f32| ((h + ROW_GAP) / (MIN_ROW + ROW_GAP)).floor().max(0.0) as usize;
    let n = view.rows.len();
    let (shown, room) = if n <= fits(rows_area.height()) {
        (n, rows_area.height())
    } else {
        let room = rows_area.height() - MORE_H - ROW_GAP;
        (fits(room).min(n), room)
    };
    let row_h = if shown == 0 { 0.0 } else { ((room - (shown - 1) as f32 * ROW_GAP) / shown as f32).min(MAX_ROW) };
    let rows: Vec<Rect> = (0..shown)
        .map(|i| Rect::from_min_size(pos2(left, TOP + i as f32 * (row_h + ROW_GAP)), vec2(right - left, row_h)))
        .collect();
    let more_rows = (shown < n)
        .then(|| Rect::from_min_max(pos2(left, rows_area.bottom() - MORE_H), pos2(right, rows_area.bottom())));
    ChangesLayout { rows, strip, tickets, rows_area, more_rows, strip_columns }
}

fn strip_columns(width: f32) -> usize {
    ((width + CHIP_GAP) / (CHIP[0] + CHIP_GAP)).floor().max(1.0) as usize
}

/// A branch chip's place in the strip.
fn chip_rect(strip: Rect, columns: usize, i: usize) -> Rect {
    let (col, line) = (i % columns, i / columns);
    Rect::from_min_size(
        strip.min + vec2(col as f32 * (CHIP[0] + CHIP_GAP), STRIP_HEADER + line as f32 * (CHIP[1] + CHIP_GAP)),
        vec2(CHIP[0], CHIP[1]),
    )
}

/// How many branch chips the strip holds.
fn strip_capacity(strip: Rect, columns: usize) -> usize {
    let lines = ((strip.height() - STRIP_HEADER + CHIP_GAP) / (CHIP[1] + CHIP_GAP)).floor().max(0.0) as usize;
    lines * columns
}

/// A row's three parts: who and what (left), commits and files (middle), sessions (right).
#[derive(Debug, Clone, Copy, PartialEq)]
struct RowParts {
    left: Rect,
    middle: Rect,
    lane: Rect,
}

fn row_parts(row: Rect) -> RowParts {
    let inner = row.shrink(PAD);
    let left = Rect::from_min_size(inner.min, vec2(LEFT_W.min(inner.width()), inner.height()));
    let lane_left = (inner.right() - LANE_W).max(left.right());
    let lane = Rect::from_min_max(pos2(lane_left, inner.top()), inner.max);
    let middle = Rect::from_min_max(
        pos2(left.right() + PART_GAP, inner.top()),
        pos2((lane.left() - PART_GAP).max(left.right() + PART_GAP), inner.bottom()),
    );
    RowParts { left, middle, lane }
}

/// How many session chips a lane holds.
fn session_slots(lane: Rect) -> usize {
    ((lane.height() - SESSION_TOP + SESSION_STEP - SESSION_H) / SESSION_STEP).floor().max(0.0) as usize
}

fn session_slot(lane: Rect, i: usize) -> Rect {
    Rect::from_min_size(lane.min + vec2(0.0, SESSION_TOP + i as f32 * SESSION_STEP), vec2(lane.width(), SESSION_H))
}

/// What a list of `total` items shows in `slots`: the first `n`, and whether the last slot says
/// "+k more".
fn fit(total: usize, slots: usize) -> (usize, bool) {
    if total <= slots {
        (total, false)
    } else {
        (slots.saturating_sub(1), slots > 0)
    }
}

/// The beads' centres on a row's track, newest (index 0) on the right; only as many as fit.
fn bead_centers(middle: Rect, n: usize) -> Vec<Pos2> {
    let y = middle.top() + BEAD * 0.5 + 2.0;
    let span = (middle.width() - BEAD).max(0.0);
    let k = n.min((span / BEAD_MIN_STEP).floor() as usize + 1);
    let step = if k > 1 { (span / (k - 1) as f32).min(BEAD_MAX_STEP) } else { 0.0 };
    let right = middle.right() - BEAD * 0.5;
    (0..k).map(|i| pos2(right - i as f32 * step, y)).collect()
}

/// File chips of the given widths, wrapped in the middle part under the track and caption, and the
/// "+k more" chip when not all fit. Returns the chips placed (a prefix of `widths`).
fn file_chips(middle: Rect, widths: &[f32], more_beyond: usize) -> (Vec<Rect>, Option<Rect>) {
    let lines = ((middle.height() - FILES_TOP + FILE_STEP - FILE_H) / FILE_STEP).floor().max(0.0) as usize;
    let place = |count: usize, extra: Option<f32>| -> Option<Vec<Rect>> {
        let mut out = Vec::new();
        let (mut x, mut line) = (middle.left(), 0);
        for w in widths[..count].iter().copied().chain(extra) {
            let w = w.min(middle.width());
            if x + w > middle.right() + 0.01 {
                x = middle.left();
                line += 1;
            }
            if line >= lines {
                return None;
            }
            out.push(Rect::from_min_size(pos2(x, middle.top() + FILES_TOP + line as f32 * FILE_STEP), vec2(w, FILE_H)));
            x += w + FILE_GAP;
        }
        Some(out)
    };
    if more_beyond == 0 {
        if let Some(all) = place(widths.len(), None) {
            return (all, None);
        }
    }
    for count in (0..=widths.len()).rev() {
        if let Some(mut chips) = place(count, Some(MORE_CHIP_W)) {
            let more = chips.pop();
            return (chips, more);
        }
    }
    (Vec::new(), None)
}

/// A file chip's width for its label.
fn file_chip_width(f: &FileChangeView) -> f32 {
    let (name, counts) = file_label(f);
    24.0 + name.chars().count() as f32 * 7.3 + 10.0 + counts.chars().count() as f32 * 6.6
}

/// A file chip's name and counts ("+12 −3", "binary").
fn file_label(f: &FileChangeView) -> (String, String) {
    let name = match &f.old_path {
        Some(old) => format!("{old} → {}", f.path),
        None => f.path.clone(),
    };
    let counts = if f.binary { "binary".to_string() } else { counts_label(f.added, f.removed) };
    (truncate_start(&name, 44), counts)
}

fn counts_label(added: usize, removed: usize) -> String {
    match (added, removed) {
        (0, 0) => String::new(),
        (a, 0) => format!("+{a}"),
        (0, r) => format!("−{r}"),
        (a, r) => format!("+{a} −{r}"),
    }
}

/// `text` cut from the start to at most `max` characters, "…" first.
fn truncate_start(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(n + 1 - max).collect();
    format!("…{tail}")
}

/// `n` with thousands separated: 4,286.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The ticket cards in the column: the heading and cards of tickets linked to rows (at most
/// three), then the heading and cards of the next ones, and the "+k more" line.
#[derive(Debug, Clone, PartialEq)]
struct TicketCards {
    counts: Rect,
    linked_heading: Option<Rect>,
    linked: Vec<Rect>,
    next_heading: Rect,
    next: Vec<Rect>,
    more: Option<Rect>,
}

fn ticket_cards(column: Rect, linked: usize, next: usize, more_next: usize) -> TicketCards {
    let inner = column.shrink(18.0);
    let counts = Rect::from_min_size(inner.min, vec2(inner.width(), 40.0));
    let mut y = counts.bottom() + 12.0;
    let line = |y: f32, h: f32| Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), h));
    let mut linked_rects = Vec::new();
    let linked_heading = (linked > 0).then(|| {
        let r = line(y, HEADING);
        y += HEADING;
        for _ in 0..linked.min(MAX_LINKED) {
            if y + LINKED_CARD > inner.bottom() {
                break;
            }
            linked_rects.push(line(y, LINKED_CARD));
            y += LINKED_CARD + CARD_GAP;
        }
        y += 6.0;
        r
    });
    let next_heading = line(y.min(inner.bottom() - HEADING), HEADING);
    y = next_heading.bottom();
    let slots = ((inner.bottom() - y + CARD_GAP) / (TICKET_CARD + CARD_GAP)).floor().max(0.0) as usize;
    let (cards_shown, more_line) = if more_next == 0 && next <= slots {
        (next, false)
    } else {
        // Room for the "+k more" line under the cards.
        let room = ((inner.bottom() - y - MORE_H + CARD_GAP) / (TICKET_CARD + CARD_GAP)).floor().max(0.0) as usize;
        (next.min(room), true)
    };
    let next_rects: Vec<Rect> =
        (0..cards_shown).map(|i| line(y + i as f32 * (TICKET_CARD + CARD_GAP), TICKET_CARD)).collect();
    let more = more_line.then(|| {
        let top = next_rects.last().map_or(y, |r| r.bottom() + CARD_GAP);
        line(top, MORE_H)
    });
    TicketCards { counts, linked_heading, linked: linked_rects, next_heading, next: next_rects, more }
}

fn to_world(world: &WorldLayout, local: Rect) -> Rect {
    Rect::from_min_size(world.changes.min + local.min.to_vec2() * world.scale, local.size() * world.scale)
}

/// A worktree row's place in the world, for the camera: the row, else the "+k more" line, else
/// the rows' area.
pub fn row_world_rect(world: &WorldLayout, view: &ChangesView, row: usize) -> Rect {
    let layout = layout(view, world.changes.width() / world.scale);
    let local = layout.rows.get(row).copied().or(layout.more_rows).unwrap_or(layout.rows_area);
    to_world(world, local)
}

/// A session chip's place in the world, for the camera: the chip, else its row's lane, else the
/// row as [`row_world_rect`] finds it.
pub fn session_world_rect(world: &WorldLayout, view: &ChangesView, row: usize, session: usize) -> Rect {
    let layout = layout(view, world.changes.width() / world.scale);
    let (Some(r), Some(v)) = (layout.rows.get(row), view.rows.get(row)) else {
        return row_world_rect(world, view, row);
    };
    let lane = row_parts(*r).lane;
    let (shown, _) = fit(v.sessions.len() + v.more_sessions, session_slots(lane));
    let local = if session < shown.min(v.sessions.len()) { session_slot(lane, session) } else { lane };
    to_world(world, local)
}

/// Draws in the district's units on the screen.
struct Pen<'a> {
    painter: Painter,
    origin: Pos2,
    local: f32,
    text_ok: bool,
    screen: Rect,
    gate: &'a InputGate,
    click: Option<Pos2>,
}

impl Pen<'_> {
    fn at(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.origin + r.min.to_vec2() * self.local, self.origin + r.max.to_vec2() * self.local)
    }
    fn pt(&self, p: Pos2) -> Pos2 {
        self.origin + p.to_vec2() * self.local
    }
    fn font(&self, pt: f32) -> FontId {
        FontId::proportional((pt * self.local).min(140.0))
    }
    fn mono(&self, pt: f32) -> FontId {
        FontId::monospace((pt * self.local).min(140.0))
    }
    fn radius(&self, r: f32) -> CornerRadius {
        CornerRadius::from((r * self.local).clamp(1.0, 24.0))
    }
    fn clicked(&self, r: Rect) -> bool {
        self.click.is_some_and(|p| r.contains(p))
    }
    fn text(&self, at: Pos2, align: Align2, text: impl ToString, font: FontId, color: Color32) -> Rect {
        self.painter.text(at, align, text, font, color)
    }
    /// A dashed outline: an Unresolved link.
    fn dashed(&self, r: Rect, color: Color32) {
        let points = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        let dash = (6.0 * self.local).max(2.0);
        self.painter.extend(Shape::dashed_line(&points, Stroke::new(1.2, color), dash, dash * 0.7));
    }
    /// Text in a small box by the pointer, at a constant size.
    fn tip(&self, at: Pos2, text: &str) {
        let galley = self.painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), TEXT_PRIMARY);
        let r = Rect::from_min_size(at + vec2(14.0, 14.0), galley.size() + vec2(14.0, 8.0));
        self.painter.rect(r, CornerRadius::from(6.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        self.painter.galley(r.min + vec2(7.0, 4.0), galley, TEXT_PRIMARY);
    }
}

/// Paints the district and returns what a click on it asks for.
pub fn paint_changes(
    painter: &Painter,
    world: &WorldLayout,
    transform: &CanvasTransform,
    screen: Rect,
    view: &ChangesView,
    selection: &ChangesSelection,
    gate: &InputGate,
) -> Option<ChangesAction> {
    let local = transform.zoom * world.scale;
    let origin = transform.world_to_screen(world.changes.min);
    let district = Rect::from_min_max(origin, transform.world_to_screen(world.changes.max));
    if !district.intersects(screen) || local < 0.08 {
        return None;
    }
    let pen = Pen {
        painter: painter.with_clip_rect(district.intersect(screen)),
        origin,
        local,
        text_ok: local >= crate::world::paint::FAR_LOCAL_ZOOM,
        screen,
        gate,
        click: gate.click(),
    };
    let layout = layout(view, world.changes.width() / world.scale);
    let mut tip: Option<(Pos2, String)> = None;
    let mut action = None;

    if pen.text_ok {
        paint_header(&pen, view, world.changes.width() / world.scale);
    }
    match &view.message {
        Some(message) => {
            if pen.text_ok {
                let area = pen.at(layout.rows_area.union(layout.strip));
                pen.text(area.center(), Align2::CENTER_CENTER, message, pen.font(17.0), TEXT_DIM);
            }
        }
        None => {
            for (i, (row, rect)) in view.rows.iter().zip(&layout.rows).enumerate() {
                if let Some(a) = paint_row(&pen, view, i, row, *rect, selection, &mut tip) {
                    action = Some(a);
                }
            }
            if let (Some(more), true) = (layout.more_rows, pen.text_ok) {
                let hidden = view.rows.len() - layout.rows.len();
                let r = pen.at(more);
                let text =
                    if hidden == 1 { "+1 more worktree".to_string() } else { format!("+{hidden} more worktrees") };
                pen.text(r.left_center(), Align2::LEFT_CENTER, text, pen.font(13.0), TEXT_DIM);
            }
            if let Some(a) = paint_strip(&pen, view, &layout, &mut tip) {
                action = Some(a);
            }
        }
    }
    if let Some(a) = paint_tickets(&pen, view, layout.tickets, &mut tip) {
        action = Some(a);
    }
    if let (Some((at, text)), true) = (tip, pen.text_ok) {
        pen.tip(at, &text);
    }
    action
}

/// The section labels, and the pills top right: commits, branches, co-authored.
fn paint_header(pen: &Pen, view: &ChangesView, width: f32) {
    let label = |x: f32, text: &str| {
        pen.text(pen.pt(pos2(x, LABEL_Y)), Align2::LEFT_TOP, text, pen.font(13.0), TEXT_DIM);
    };
    label(MARGIN, "WORKTREES · SIDE BY SIDE");
    label(width - MARGIN - TICKETS_W, "NEXT · TICKETS");
    if view.message.is_some() && view.commits_read == 0 {
        return;
    }
    let pills = [
        (format!("{} commits", thousands(view.commits_on_head)), TEXT_SECONDARY, BADGE_BG),
        (format!("{} branches", thousands(view.local_branches)), TEXT_SECONDARY, BADGE_BG),
        (
            format!("{} of the last {} co-authored", thousands(view.co_authored), thousands(view.commits_read)),
            DISTRICT_CHANGES,
            with_alpha(DISTRICT_CHANGES, 40),
        ),
    ];
    let mut right = width - MARGIN;
    for (text, color, fill) in pills.iter().rev() {
        let w = 28.0 + text.chars().count() as f32 * 7.4;
        let r = Rect::from_min_max(pos2(right - w, 36.0), pos2(right, 66.0));
        pen.painter.rect(pen.at(r), pen.radius(15.0), *fill, Stroke::NONE, StrokeKind::Inside);
        pen.text(pen.at(r).center(), Align2::CENTER_CENTER, text, pen.font(13.0), *color);
        right -= w + 8.0;
    }
}

/// One worktree row: branch and changes, the commit track with the picked commit's files (or the
/// uncommitted ones), and its session lane.
fn paint_row(
    pen: &Pen,
    view: &ChangesView,
    i: usize,
    row: &WorktreeRowView,
    rect: Rect,
    selection: &ChangesSelection,
    tip: &mut Option<(Pos2, String)>,
) -> Option<ChangesAction> {
    let r = pen.at(rect);
    if !r.intersects(pen.screen) {
        return None;
    }
    let color = if row.is_main { MAIN_COLOR } else { DISTRICT_CHANGES };
    let selected = selection.row == Some(i);
    let fill = if pen.gate.hovers(r) { CARD_BG_HOVER } else { CARD_BG };
    let stroke = if selected { Stroke::new(2.0, color) } else { Stroke::new(1.5, with_alpha(color, 110)) };
    pen.painter.rect(r, pen.radius(18.0), fill, stroke, StrokeKind::Inside);
    let parts = row_parts(rect);
    let mut hit = None;

    // Left: branch, who, how far from upstream, changes, tickets the branch names.
    let left = parts.left;
    pen.painter.circle_filled(pen.pt(left.min + vec2(5.0, 10.0)), 5.0 * pen.local, color);
    if pen.text_ok {
        let line = |dy: f32| pos2(left.left(), left.top() + dy);
        let fits = |dy: f32, h: f32| dy + h <= left.height();
        pen.text(
            pen.pt(line(0.0) + vec2(18.0, 0.0)),
            Align2::LEFT_TOP,
            truncate_with_ellipsis(&row.branch, 34),
            pen.mono(15.0),
            TEXT_PRIMARY,
        );
        if fits(24.0, 16.0) {
            let (text, c) = if row.is_main {
                (row.label.clone(), with_alpha(MAIN_COLOR, 230))
            } else {
                (truncate_start(&row.label, 44), TEXT_SECONDARY)
            };
            pen.text(pen.pt(line(24.0)), Align2::LEFT_TOP, text, pen.font(13.0), c);
        }
        if fits(46.0, 16.0) {
            let mut words: Vec<String> = Vec::new();
            if let Some((ahead, behind)) = row.ahead_behind {
                words.push(format!("{ahead} ahead · {behind} behind"));
            }
            if row.change_counts.is_empty() {
                words.push("no changes".to_string());
            }
            words.extend(row.change_counts.iter().map(|(word, n)| format!("{n} {word}")));
            pen.text(
                pen.pt(line(46.0)),
                Align2::LEFT_TOP,
                truncate_with_ellipsis(&words.join(" · "), 46),
                pen.font(12.5),
                TEXT_SECONDARY,
            );
        }
        // Tickets named by the branch: Unresolved, dashed and muted.
        if fits(68.0, 20.0) {
            let mut x = left.left();
            for id in &row.tickets {
                let w = 16.0 + id.chars().count() as f32 * 7.0;
                if x + w > left.right() {
                    break;
                }
                let chip = Rect::from_min_size(pos2(x, left.top() + 68.0), vec2(w, 20.0));
                let s = pen.at(chip);
                pen.dashed(s, with_alpha(TEXT_DIM, 200));
                pen.text(s.center(), Align2::CENTER_CENTER, id, pen.mono(11.0), TEXT_DIM);
                if pen.gate.hovers(s) {
                    *tip = pen.gate.pointer.map(|p| (p, format!("{id} · matched by branch name · unresolved")));
                }
                if pen.clicked(s) {
                    let file = view.tickets.linked.iter().find(|t| t.id == *id).map(|t| t.file.clone());
                    hit = file.map(ChangesAction::OpenFile);
                }
                x += w + 6.0;
            }
        }
    }

    // Middle: the commit track, newest on the right; co-authored beads ringed in the agents' colour.
    let middle = parts.middle;
    let centers = bead_centers(middle, row.commits.len());
    if let Some(c) = centers.first() {
        pen.painter.line_segment(
            [pen.pt(pos2(middle.left(), c.y)), pen.pt(pos2(middle.right(), c.y))],
            Stroke::new(2.0 * pen.local.min(1.0), with_alpha(TEXT_PRIMARY, 26)),
        );
    }
    let picked = selection.commit.filter(|(on, _)| *on == i).map(|(_, b)| b);
    for (b, (bead, c)) in row.commits.iter().zip(&centers).enumerate() {
        let at = pen.pt(*c);
        let is_picked = picked == Some(b);
        let radius = (if is_picked { BEAD * 0.65 } else { BEAD * 0.5 }) * pen.local;
        let ring = if is_picked {
            TEXT_HIGHLIGHT
        } else if bead.co_authored {
            DISTRICT_CHANGES
        } else {
            with_alpha(color, 120)
        };
        pen.painter.circle(at, radius, with_alpha(color, 200), Stroke::new(2.5 * pen.local.min(1.0), ring));
        let target = Rect::from_center_size(at, vec2(BEAD_MIN_STEP, BEAD + 8.0) * pen.local);
        if pen.gate.hovers(target) {
            let co = if bead.co_authored { " · co-authored" } else { "" };
            *tip = pen.gate.pointer.map(|p| (p, format!("{} · {}{co}", bead.short, bead.age)));
        }
        if pen.clicked(target) {
            hit = Some(if is_picked { ChangesAction::SelectRow(i) } else { ChangesAction::SelectCommit(i, b) });
        }
    }
    let commit = picked.and_then(|b| row.commits.get(b));
    if pen.text_ok {
        let caption = match commit {
            Some(c) => {
                let co = if c.co_authored { " · co-authored" } else { "" };
                match &c.files {
                    Some(files) => format!("Commit {} · {}{co} · {} files", c.short, c.age, files.len()),
                    None => format!("Commit {} · {}{co} · Reading…", c.short, c.age),
                }
            }
            None if row.files.is_empty() && row.more_files == 0 => {
                "No uncommitted changes to tracked files".to_string()
            }
            None => format!("Uncommitted · {} changed files", row.files.len() + row.more_files),
        };
        pen.text(pen.pt(middle.min + vec2(0.0, 26.0)), Align2::LEFT_TOP, caption, pen.font(12.0), TEXT_DIM);
    }
    let (files, more_beyond): (&[FileChangeView], usize) = match commit {
        Some(c) => (c.files.as_deref().unwrap_or_default(), 0),
        None => (&row.files, row.more_files),
    };
    let widths: Vec<f32> = files.iter().map(file_chip_width).collect();
    let (chips, more) = file_chips(middle, &widths, more_beyond);
    for (f, chip) in files.iter().zip(&chips) {
        let s = pen.at(*chip);
        let hovered = pen.gate.hovers(s);
        pen.painter.rect_filled(s, pen.radius(8.0), with_alpha(TEXT_PRIMARY, if hovered { 22 } else { 12 }));
        if pen.text_ok {
            let (name, counts) = file_label(f);
            pen.text(
                s.left_center() + vec2(10.0 * pen.local, 0.0),
                Align2::LEFT_CENTER,
                name,
                pen.mono(12.0),
                TEXT_PRIMARY,
            );
            let right = s.right_center() - vec2(10.0 * pen.local, 0.0);
            if f.binary {
                pen.text(right, Align2::RIGHT_CENTER, counts, pen.font(11.5), TEXT_DIM);
            } else {
                let removed = (f.removed > 0).then(|| format!("−{}", f.removed));
                let mut x = right;
                if let Some(text) = removed {
                    x = pen.text(x, Align2::RIGHT_CENTER, text, pen.font(11.5), REMOVED).left_center()
                        - vec2(5.0 * pen.local, 0.0);
                }
                if f.added > 0 {
                    pen.text(x, Align2::RIGHT_CENTER, format!("+{}", f.added), pen.font(11.5), ADDED);
                }
            }
        }
        if pen.clicked(s) {
            hit = Some(ChangesAction::OpenFile(row.path.join(&f.path)));
        }
    }
    if let (Some(m), true) = (more, pen.text_ok) {
        let hidden = files.len() - chips.len() + more_beyond;
        let s = pen.at(m);
        pen.text(s.left_center(), Align2::LEFT_CENTER, format!("+{hidden} more"), pen.font(12.0), TEXT_DIM);
    }

    // Right: the sessions that worked here, newest first. Observed: seen in the agents' logs.
    let lane = parts.lane;
    if pen.text_ok {
        pen.text(
            pen.pt(lane.min),
            Align2::LEFT_TOP,
            format!("AGENT SESSIONS · {}", EvidenceTier::Observed.label()),
            pen.font(11.5),
            with_alpha(DISTRICT_CHANGES, 200),
        );
    }
    let empty = view.sessions_message.as_deref().or(row.sessions.is_empty().then_some("No sessions in this worktree"));
    match empty {
        Some(text) => {
            if pen.text_ok {
                pen.text(pen.pt(lane.min + vec2(0.0, SESSION_TOP)), Align2::LEFT_TOP, text, pen.font(12.5), TEXT_DIM);
            }
        }
        None => {
            let total = row.sessions.len() + row.more_sessions;
            let (shown, more) = fit(total, session_slots(lane));
            for (s_i, s) in row.sessions.iter().take(shown).enumerate() {
                if let Some(a) = paint_session(pen, s, session_slot(lane, s_i), selection, tip) {
                    hit = Some(a);
                }
            }
            if more && pen.text_ok {
                let slot = pen.at(session_slot(lane, shown));
                let text = format!("+{} more sessions", total - shown.min(row.sessions.len()));
                pen.text(slot.left_center(), Align2::LEFT_CENTER, text, pen.font(12.0), TEXT_DIM);
            }
        }
    }

    if hit.is_none() && pen.clicked(r) {
        hit = Some(ChangesAction::SelectRow(i));
    }
    hit
}

/// A session chip: slug · start · touches, and its plan mark.
fn paint_session(
    pen: &Pen,
    s: &SessionChip,
    slot: Rect,
    selection: &ChangesSelection,
    tip: &mut Option<(Pos2, String)>,
) -> Option<ChangesAction> {
    let r = pen.at(slot);
    let lit = selection.lit_session.as_deref() == Some(s.id.as_str());
    let hovered = pen.gate.hovers(r);
    let fill = with_alpha(
        DISTRICT_CHANGES,
        if lit {
            90
        } else if hovered {
            44
        } else {
            24
        },
    );
    let stroke = if lit { Stroke::new(1.5, DISTRICT_CHANGES) } else { Stroke::NONE };
    pen.painter.rect(r, pen.radius(8.0), fill, stroke, StrokeKind::Inside);
    let plan = s.has_plan.then(|| Rect::from_min_max(pos2(r.right() - 64.0 * pen.local, r.top()), r.max));
    if pen.text_ok {
        let touches = if s.touches == 1 { "1 touch".to_string() } else { format!("{} touches", s.touches) };
        let text = format!("{} · {} · {touches}", truncate_with_ellipsis(&s.slug, 34), s.started);
        pen.text(
            r.left_center() + vec2(10.0 * pen.local, 0.0),
            Align2::LEFT_CENTER,
            text,
            pen.font(12.0),
            TEXT_PRIMARY,
        );
        if let Some(p) = plan {
            let c = if pen.gate.hovers(p) { TEXT_HIGHLIGHT } else { DISTRICT_CHANGES };
            let mark = format!("{} plan", egui_phosphor::regular::NOTEBOOK);
            pen.text(p.right_center() - vec2(10.0 * pen.local, 0.0), Align2::RIGHT_CENTER, mark, pen.font(12.0), c);
        }
        if hovered {
            let what = if lit { "click to stop lighting it" } else { "click to light what it touched" };
            *tip = pen.gate.pointer.map(|p| (p, format!("{} · observed · {what}", s.slug)));
        }
    }
    if plan.is_some_and(|p| pen.clicked(p)) {
        return Some(ChangesAction::OpenSessionPlan(s.id.clone()));
    }
    pen.clicked(r).then(|| ChangesAction::LightSession(if lit { None } else { Some(s.id.clone()) }))
}

/// Every local branch as a chip: name, distance from upstream, last commit, sessions seen on it.
fn paint_strip(
    pen: &Pen,
    view: &ChangesView,
    layout: &ChangesLayout,
    tip: &mut Option<(Pos2, String)>,
) -> Option<ChangesAction> {
    let strip = layout.strip;
    if !pen.at(strip).intersects(pen.screen) {
        return None;
    }
    if pen.text_ok {
        let title = format!("BRANCHES · {}", thousands(view.branches.len()));
        pen.text(pen.pt(strip.min), Align2::LEFT_TOP, title, pen.font(13.0), TEXT_DIM);
        if view.branches.is_empty() {
            pen.text(
                pen.pt(strip.min + vec2(0.0, STRIP_HEADER)),
                Align2::LEFT_TOP,
                "No local branches",
                pen.font(12.5),
                TEXT_DIM,
            );
        }
    }
    let (shown, more) = fit(view.branches.len(), strip_capacity(strip, layout.strip_columns));
    let mut action = None;
    for (i, b) in view.branches.iter().take(shown).enumerate() {
        let s = pen.at(chip_rect(strip, layout.strip_columns, i));
        if !s.intersects(pen.screen) {
            continue;
        }
        let row = view.rows.iter().position(|r| r.branch == b.name);
        let hovered = pen.gate.hovers(s);
        let border = match row.map(|r| view.rows[r].is_main) {
            Some(true) => with_alpha(MAIN_COLOR, 140),
            Some(false) => with_alpha(DISTRICT_CHANGES, 140),
            None => CARD_BORDER_NORMAL,
        };
        let fill = if hovered { CARD_BG_HOVER } else { CARD_BG };
        pen.painter.rect(s, pen.radius(8.0), fill, Stroke::new(1.0, border), StrokeKind::Inside);
        if pen.text_ok {
            let x = s.left() + 10.0 * pen.local;
            pen.text(
                pos2(x, s.top() + 6.0 * pen.local),
                Align2::LEFT_TOP,
                truncate_with_ellipsis(&b.name, 27),
                pen.mono(12.0),
                TEXT_PRIMARY,
            );
            let mut facts = Vec::new();
            if let Some((ahead, behind)) = b.ahead_behind {
                facts.push(format!("↑{ahead} ↓{behind}"));
            }
            if !b.age.is_empty() {
                facts.push(b.age.clone());
            }
            let at = pen.text(
                pos2(x, s.bottom() - 6.0 * pen.local),
                Align2::LEFT_BOTTOM,
                facts.join(" · "),
                pen.font(11.0),
                TEXT_DIM,
            );
            if b.sessions > 0 {
                let sep = if facts.is_empty() { "" } else { " · " };
                let n = if b.sessions == 1 { "1 session".to_string() } else { format!("{} sessions", b.sessions) };
                pen.text(at.right_bottom(), Align2::LEFT_BOTTOM, format!("{sep}{n}"), pen.font(11.0), DISTRICT_CHANGES);
            }
            if hovered {
                *tip = pen.gate.pointer.map(|p| (p, b.name.clone()));
            }
        }
        if let (Some(r), true) = (row, pen.clicked(s)) {
            action = Some(ChangesAction::SelectRow(r));
        }
    }
    if more && pen.text_ok {
        let s = pen.at(chip_rect(strip, layout.strip_columns, shown));
        let text = format!("+{} more", view.branches.len() - shown);
        pen.text(s.left_center() + vec2(10.0 * pen.local, 0.0), Align2::LEFT_CENTER, text, pen.font(12.5), TEXT_DIM);
    }
    action
}

/// The tickets column: counts by status, tickets the rows' branches name, then what is next.
fn paint_tickets(
    pen: &Pen,
    view: &ChangesView,
    column: Rect,
    tip: &mut Option<(Pos2, String)>,
) -> Option<ChangesAction> {
    let c = pen.at(column);
    if !c.intersects(pen.screen) {
        return None;
    }
    pen.painter.rect(c, pen.radius(18.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
    let col = &view.tickets;
    if let Some(message) = &col.message {
        if pen.text_ok {
            pen.text(c.center(), Align2::CENTER_CENTER, message, pen.font(14.0), TEXT_DIM);
        }
        return None;
    }
    let cards = ticket_cards(column, col.linked.len(), col.next.len(), col.more_next);
    let mut action = None;
    if pen.text_ok {
        // Counts, wrapped by words into at most two lines.
        let words: Vec<String> = col.counts.iter().map(|(s, n)| format!("{} {s}", thousands(*n))).collect();
        let mut lines: Vec<String> = vec![String::new()];
        for w in words {
            let current = lines.last_mut().expect("one line");
            if current.is_empty() {
                current.push_str(&w);
            } else if current.chars().count() + w.chars().count() + 3 <= 80 {
                current.push_str(" · ");
                current.push_str(&w);
            } else {
                lines.push(w);
            }
        }
        for (k, line) in lines.iter().take(2).enumerate() {
            let at = pen.pt(cards.counts.min + vec2(0.0, k as f32 * 19.0));
            pen.text(at, Align2::LEFT_TOP, truncate_with_ellipsis(line, 84), pen.font(12.5), TEXT_SECONDARY);
        }
        if let Some(h) = cards.linked_heading {
            pen.text(pen.pt(h.min), Align2::LEFT_TOP, "ON A WORKTREE'S BRANCH", pen.font(11.5), TEXT_DIM);
        }
        pen.text(pen.pt(cards.next_heading.min), Align2::LEFT_TOP, "READY AND QUEUED", pen.font(11.5), TEXT_DIM);
        if col.next.is_empty() {
            pen.text(
                pen.pt(cards.next_heading.min + vec2(0.0, HEADING)),
                Align2::LEFT_TOP,
                "Nothing ready or queued",
                pen.font(12.5),
                TEXT_DIM,
            );
        }
    }
    let lists = col.linked.iter().zip(&cards.linked).chain(col.next.iter().zip(&cards.next));
    for (t, rect) in lists {
        let s = pen.at(*rect);
        if !s.intersects(pen.screen) {
            continue;
        }
        let hovered = pen.gate.hovers(s);
        pen.painter.rect_filled(s, pen.radius(12.0), if hovered { CARD_BG_HOVER } else { CARD_BG });
        if let Some(link) = &t.link {
            match link.tier {
                EvidenceTier::Unresolved => pen.dashed(s.shrink(1.0), with_alpha(TEXT_DIM, 200)),
                _ => {
                    let stroke = Stroke::new(1.0, with_alpha(DISTRICT_CHANGES, 150));
                    pen.painter.rect_stroke(s, pen.radius(12.0), stroke, StrokeKind::Inside);
                }
            }
        }
        if pen.text_ok {
            let x = s.left() + 12.0 * pen.local;
            let line = |dy: f32| pos2(x, s.top() + dy * pen.local);
            let id_color = if t.link.is_some() { TEXT_SECONDARY } else { DISTRICT_PIPELINE };
            let id = pen.text(line(9.0), Align2::LEFT_TOP, &t.id, pen.mono(12.0), id_color);
            let mut state = t.status.clone();
            if let Some(p) = &t.priority {
                state.push_str(&format!(" · priority {p}"));
            }
            pen.text(id.right_top() + vec2(8.0 * pen.local, 0.0), Align2::LEFT_TOP, state, pen.font(12.0), TEXT_DIM);
            pen.text(line(29.0), Align2::LEFT_TOP, truncate_with_ellipsis(&t.title, 62), pen.font(14.0), TEXT_PRIMARY);
            if let Some(link) = &t.link {
                let row = view.rows.get(link.row).map_or("", |r| r.branch.as_str());
                let mut note = format!("{} · matched by {} {}", link.tier.label(), link.basis, row);
                if let Some(shipped) = &t.shipped {
                    note.push_str(" · ");
                    note.push_str(&shipped.label());
                    if let Some(tier) = shipped.tier() {
                        note.push_str(&format!(" ({})", tier.label()));
                    }
                }
                pen.text(line(52.0), Align2::LEFT_TOP, truncate_with_ellipsis(&note, 78), pen.font(11.5), TEXT_DIM);
                if hovered {
                    *tip = pen.gate.pointer.map(|p| (p, format!("matched by {} · {}", link.basis, link.tier.label())));
                }
            }
        }
        if pen.clicked(s) {
            action = Some(ChangesAction::OpenFile(t.file.clone()));
        }
    }
    if let (Some(m), true) = (cards.more, pen.text_ok) {
        let hidden = col.next.len() - cards.next.len() + col.more_next;
        if hidden > 0 {
            let s = pen.at(m);
            let text = format!("+{} more ready or queued", thousands(hidden));
            pen.text(s.left_center(), Align2::LEFT_CENTER, text, pen.font(12.5), TEXT_DIM);
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(i: usize, sessions: usize, files: usize) -> WorktreeRowView {
        WorktreeRowView {
            branch: format!("b{i}"),
            label: if i == 0 { "you".into() } else { format!("/w/{i}") },
            path: PathBuf::from(format!("/w/{i}")),
            is_main: i == 0,
            files: (0..files)
                .map(|f| FileChangeView {
                    path: format!("src/file_{f}.rs"),
                    added: f,
                    removed: 1,
                    ..Default::default()
                })
                .collect(),
            commits: (0..30).map(|c| CommitBead { id: format!("{c:040}"), ..Default::default() }).collect(),
            sessions: (0..sessions).map(|s| SessionChip { id: format!("s{s}"), ..Default::default() }).collect(),
            ..Default::default()
        }
    }

    fn view(rows: usize, branches: usize) -> ChangesView {
        ChangesView {
            rows: (0..rows).map(|i| row(i, i * 3, i * 7)).collect(),
            branches: (0..branches).map(|i| BranchView { name: format!("br{i}"), ..Default::default() }).collect(),
            tickets: TicketsColumnView {
                next: (0..12).map(|i| TicketView { id: format!("T-{i}"), ..Default::default() }).collect(),
                linked: (0..2).map(|i| TicketView { id: format!("T-9{i}"), ..Default::default() }).collect(),
                more_next: 70,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn district(width: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(width, HEIGHT))
    }

    fn apart(rects: &[Rect]) {
        for (a, ra) in rects.iter().enumerate() {
            for rb in &rects[a + 1..] {
                assert!(!ra.intersects(rb.shrink(0.01)), "{ra:?} overlaps {rb:?}");
            }
        }
    }

    #[test]
    fn rows_strip_and_column_stay_apart_and_inside_for_any_count() {
        for rows in [1, 3, 12] {
            for branches in [0, 40, 200] {
                let v = view(rows, branches);
                let l = layout(&v, 2800.0);
                let d = district(2800.0);
                let mut all = l.rows.clone();
                all.extend(l.more_rows);
                all.push(l.strip);
                all.push(l.tickets);
                apart(&all);
                assert!(all.iter().all(|r| d.contains_rect(*r)), "{rows} rows, {branches} branches");
                assert!(l.rows.iter().chain(&l.more_rows).all(|r| l.rows_area.contains_rect(*r)));
                assert!(l.rows.iter().all(|r| r.height() >= MIN_ROW - 0.01), "rows keep their least height");
                assert_eq!(l.more_rows.is_some(), l.rows.len() < rows, "a hidden row is counted");
                assert!(!l.rows.is_empty());

                // The branch chips the strip holds stay in it and apart.
                let (shown, more) = fit(branches, strip_capacity(l.strip, l.strip_columns));
                let chips: Vec<Rect> =
                    (0..shown + usize::from(more)).map(|i| chip_rect(l.strip, l.strip_columns, i)).collect();
                assert!(chips.iter().all(|c| l.strip.contains_rect(*c)));
                apart(&chips);

                // Inside each row: three parts apart, sessions in the lane, beads and file chips in
                // the middle.
                for (r, rv) in l.rows.iter().zip(&v.rows) {
                    let p = row_parts(*r);
                    apart(&[p.left, p.middle, p.lane]);
                    assert!([p.left, p.middle, p.lane].iter().all(|x| r.contains_rect(*x)));
                    let (n, more) = fit(rv.sessions.len(), session_slots(p.lane));
                    let slots: Vec<Rect> = (0..n + usize::from(more)).map(|i| session_slot(p.lane, i)).collect();
                    assert!(slots.iter().all(|s| p.lane.contains_rect(*s)));
                    apart(&slots);
                    assert!(bead_centers(p.middle, 30).iter().all(|c| p.middle.contains(*c)));
                    let widths: Vec<f32> = rv.files.iter().map(file_chip_width).collect();
                    let (mut files, more) = file_chips(p.middle, &widths, 3);
                    files.extend(more);
                    assert!(files.iter().all(|f| p.middle.contains_rect(*f)));
                    apart(&files);
                }
            }
        }
    }

    #[test]
    fn three_worktrees_and_forty_branches_show_whole() {
        let v = view(3, 40);
        let l = layout(&v, 2800.0);
        assert_eq!(l.rows.len(), 3);
        assert!(l.more_rows.is_none());
        assert!(strip_capacity(l.strip, l.strip_columns) >= 40, "TBD's 40 branches all fit");
        let twelve = layout(&view(12, 0), 2800.0);
        assert!(twelve.rows.len() < 12 && twelve.more_rows.is_some());
    }

    #[test]
    fn ticket_cards_fit_the_column() {
        let l = layout(&view(1, 0), 2800.0);
        for (linked, next, more) in [(0, 0, 0), (2, 12, 70), (9, 3, 0), (0, 40, 0)] {
            let cards = ticket_cards(l.tickets, linked, next, more);
            let mut all = vec![cards.counts, cards.next_heading];
            all.extend(cards.linked_heading);
            all.extend(cards.linked.iter().copied());
            all.extend(cards.next.iter().copied());
            all.extend(cards.more);
            apart(&all);
            assert!(all.iter().all(|r| l.tickets.contains_rect(*r)), "{linked} {next} {more}");
            assert!(cards.linked.len() <= MAX_LINKED);
            assert_eq!(cards.more.is_some(), more > 0 || cards.next.len() < next);
        }
    }

    #[test]
    fn row_and_session_rects_lie_in_the_district() {
        let world = WorldLayout::default();
        assert!((world.changes.height() / world.scale - HEIGHT).abs() < 0.01, "HEIGHT is the world's");
        for rows in [0, 1, 3, 12] {
            let v = view(rows, 40);
            for i in 0..rows + 1 {
                assert!(world.changes.contains_rect(row_world_rect(&world, &v, i)));
                for s in 0..10 {
                    let session = session_world_rect(&world, &v, i, s);
                    assert!(world.changes.contains_rect(session));
                }
            }
        }
        let big = WorldLayout::compute(Rect::from_min_size(Pos2::ZERO, vec2(9000.0, 7000.0)));
        let v = view(3, 0);
        let r = row_world_rect(&big, &v, 2);
        assert!(big.changes.contains_rect(r));
        assert!(r.contains_rect(session_world_rect(&big, &v, 2, 0)), "a session sits in its row");
    }

    #[test]
    fn labels_say_counts_renames_and_binaries() {
        let f = |old: Option<&str>, added, removed, binary| FileChangeView {
            path: "b.rs".into(),
            old_path: old.map(str::to_string),
            added,
            removed,
            binary,
        };
        assert_eq!(file_label(&f(None, 3, 1, false)), ("b.rs".into(), "+3 −1".into()));
        assert_eq!(file_label(&f(Some("a.rs"), 0, 0, false)), ("a.rs → b.rs".into(), String::new()));
        assert_eq!(file_label(&f(None, 0, 0, true)).1, "binary");
        assert_eq!(thousands(4286), "4,286");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(truncate_start("abcdef", 4), "…def");
        assert_eq!(ShippedView::Commit("abc1234".into()).tier(), Some(EvidenceTier::Proven));
        assert_eq!(ShippedView::NotInRepo.tier(), None);
    }

    /// Paints the district at 100% with a click at `at` (district units) and returns the action.
    fn click_at(
        view: &ChangesView,
        selection: &ChangesSelection,
        at: Option<Pos2>,
        zoom: f32,
    ) -> Option<ChangesAction> {
        let world = WorldLayout::default();
        let transform = CanvasTransform::new(-world.changes.min.to_vec2() * zoom, zoom);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(2800.0, HEIGHT) * zoom);
        let gate = InputGate {
            pointer: at.map(|p| (p.to_vec2() * zoom).to_pos2()),
            clicked: at.is_some(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut action = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            action = paint_changes(&painter, &world, &transform, screen, view, selection, &gate);
        });
        output.textures_delta.clear();
        action
    }

    #[test]
    fn clicks_ask_for_what_they_land_on() {
        let mut v = view(3, 200);
        v.rows[1].sessions[1].has_plan = true;
        v.rows[1].tickets = vec!["T-90".into()];
        let l = layout(&v, 2800.0);
        let parts = row_parts(l.rows[1]);
        let none = ChangesSelection::default();
        let session = session_slot(parts.lane, 0).center();
        assert_eq!(click_at(&v, &none, Some(session), 1.0), Some(ChangesAction::LightSession(Some("s0".into()))));
        let lit = ChangesSelection { lit_session: Some("s0".into()), ..Default::default() };
        assert_eq!(click_at(&v, &lit, Some(session), 1.0), Some(ChangesAction::LightSession(None)), "again: off");
        let plan = session_slot(parts.lane, 1).right_center() - vec2(20.0, 0.0);
        assert_eq!(click_at(&v, &none, Some(plan), 1.0), Some(ChangesAction::OpenSessionPlan("s1".into())));
        let bead = bead_centers(parts.middle, v.rows[1].commits.len())[0];
        assert_eq!(click_at(&v, &none, Some(bead), 1.0), Some(ChangesAction::SelectCommit(1, 0)));
        let picked = ChangesSelection { row: Some(1), commit: Some((1, 0)), ..Default::default() };
        assert_eq!(click_at(&v, &picked, Some(bead), 1.0), Some(ChangesAction::SelectRow(1)), "again: unpick");
        let empty = parts.left.right_bottom() - vec2(4.0, 4.0);
        assert_eq!(click_at(&v, &none, Some(empty), 1.0), Some(ChangesAction::SelectRow(1)));
        let file = file_chips(parts.middle, &v.rows[1].files.iter().map(file_chip_width).collect::<Vec<_>>(), 0).0[0];
        assert_eq!(
            click_at(&v, &none, Some(file.center()), 1.0),
            Some(ChangesAction::OpenFile(PathBuf::from("/w/1/src/file_0.rs")))
        );
        let ticket = ticket_cards(l.tickets, v.tickets.linked.len(), v.tickets.next.len(), v.tickets.more_next);
        assert_eq!(
            click_at(&v, &none, Some(ticket.next[0].center()), 1.0),
            Some(ChangesAction::OpenFile(PathBuf::new()))
        );
        // Far away nothing reads but nothing breaks either; messages and empty columns paint too.
        assert_eq!(click_at(&v, &none, None, 0.2), None);
        let message = ChangesView {
            message: Some("No git repository here".into()),
            tickets: TicketsColumnView { message: Some("No tickets here (.ai/tickets)".into()), ..Default::default() },
            ..Default::default()
        };
        assert_eq!(click_at(&message, &none, Some(pos2(400.0, 400.0)), 1.0), None);
    }
}
