//! The Files district, west of the map: the disk exactly as it is (every folder and file, with
//! what git ignores, what lives in LFS and how big things are), where the space goes, and the
//! project's settings as facts, each opening the line it comes from.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use egui::{pos2, vec2, Align2, CornerRadius, FontId, Painter, Rect, Stroke, StrokeKind};
use studio_graph::{human_bytes, Graph, NodeId};
use studio_ui::color_tokens::*;
use studio_ui::{truncate_with_ellipsis, with_alpha};

use crate::transform::CanvasTransform;
use crate::view::gate::InputGate;
use crate::world::WorldLayout;

/// A fact of a settings file.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingRow {
    pub key: String,
    pub value: String,
    pub line: usize,
    /// The file it comes from, when not the card's own.
    pub file: Option<PathBuf>,
}

/// A settings file as the district shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsCardView {
    pub title: String,
    pub file: PathBuf,
    pub summary: String,
    /// Headings and their rows; an empty heading shows no title.
    pub sections: Vec<(String, Vec<SettingRow>)>,
}

/// Something git ignores and the rule that does it.
#[derive(Debug, Clone, PartialEq)]
pub struct IgnoredView {
    /// Relative to the root; folders end in `/`.
    pub path: String,
    pub rule: Option<(PathBuf, usize, String)>,
}

/// Tracked files of one class, as the "by class" panel shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassView {
    /// The class's name ("code", "build output").
    pub label: String,
    pub files: usize,
    /// Their lengths in the working tree, summed.
    pub bytes: u64,
    /// The rules that decided them ("tool convention: Cargo") and their files, most first.
    pub rules: Vec<(String, usize)>,
}

impl ClassView {
    /// "extension 4,700 · tool convention: Cargo 131".
    pub fn why(&self) -> String {
        self.rules.iter().map(|(why, n)| format!("{why} {}", group(*n))).collect::<Vec<_>>().join(" · ")
    }
}

/// `n` with thousands separated by commas.
fn group(n: usize) -> String {
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

/// What the Files district shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilesView {
    pub tracked: Option<usize>,
    pub lfs: Option<usize>,
    pub changes: Option<String>,
    pub ignored: Vec<IgnoredView>,
    /// Top-level entries and what they take on disk, largest first.
    pub sizes: Vec<(String, u64)>,
    pub cards: Vec<SettingsCardView>,
    /// Tracked files by class, in the classification table's order; empty classes left out.
    pub classes: Vec<ClassView>,
    /// Why git's facts will not come ("No git repository here"); sizes still do.
    pub git_note: Option<String>,
    /// Tracked files in Git LFS (relative, `/` separated).
    pub lfs_paths: BTreeSet<String>,
    /// Tracked files `.gitattributes` marks `linguist-generated`.
    pub generated: BTreeSet<String>,
    /// Tracked files `.gitattributes` marks `linguist-vendored`.
    pub vendored: BTreeSet<String>,
    /// Lookups over `ignored` and `sizes`, built once: when the view is made, else on first use.
    pub lookups: FilesLookups,
}

/// [`FilesView`]'s lookups, built once from its lists (which never change after). Equal to any
/// other, so views compare by what they show.
#[derive(Clone, Default)]
pub struct FilesLookups(OnceLock<Lookups>);

#[derive(Clone, Default)]
struct Lookups {
    /// Ignored paths without their trailing `/`.
    ignored: BTreeSet<String>,
    sizes: BTreeMap<String, u64>,
}

impl PartialEq for FilesLookups {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for FilesLookups {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FilesLookups")
    }
}

/// What a tree row is marked with, besides its name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMarks<'a> {
    pub ignored: bool,
    /// Listed but not loaded, and why ("build cache").
    pub heavy: Option<&'a str>,
    pub lfs: bool,
    pub generated: bool,
    pub vendored: bool,
    /// Its name starts with a dot.
    pub hidden: bool,
    pub size: Option<u64>,
}

impl FilesView {
    fn lookups(&self) -> &Lookups {
        self.lookups.0.get_or_init(|| Lookups {
            ignored: self.ignored.iter().map(|e| e.path.trim_end_matches('/').to_string()).collect(),
            sizes: self.sizes.iter().cloned().collect(),
        })
    }

    /// Builds the lookups now, so no frame has to.
    pub fn build_lookups(&self) {
        self.lookups();
    }

    /// Whether git ignores this path (relative, `/` separated): it, or a folder holding it.
    pub fn is_ignored(&self, rel: &str) -> bool {
        let ignored = &self.lookups().ignored;
        ignored.contains(rel) || rel.match_indices('/').any(|(i, _)| ignored.contains(&rel[..i]))
    }

    fn size_of(&self, rel: &str) -> Option<u64> {
        self.lookups().sizes.get(rel).copied()
    }

    /// The marks of a tree row.
    pub fn marks<'a>(&self, row: &'a TreeRow) -> RowMarks<'a> {
        let file = row.file.is_some();
        RowMarks {
            ignored: self.is_ignored(&row.rel),
            heavy: row.heavy.as_deref(),
            lfs: file && self.lfs_paths.contains(&row.rel),
            generated: file && self.generated.contains(&row.rel),
            vendored: file && self.vendored.contains(&row.rel),
            hidden: row.label.starts_with('.'),
            size: self.size_of(&row.rel),
        }
    }
}

impl RowMarks<'_> {
    /// The marks as the row shows them, and how many characters of it are the marks that take
    /// room from the name (all but `ignored` and the size, which the name always leaves room for).
    fn text(&self) -> (String, usize) {
        let mut text = String::new();
        if self.ignored {
            join(&mut text, "ignored");
        }
        let before = text.len();
        if let Some(reason) = self.heavy {
            join(&mut text, "heavy");
            if !reason.is_empty() {
                text.push_str(": ");
                text.push_str(reason);
            }
        }
        let flags =
            [(self.lfs, "LFS"), (self.generated, "generated"), (self.vendored, "vendored"), (self.hidden, "hidden")];
        for (_, mark) in flags.iter().filter(|(on, _)| *on) {
            join(&mut text, mark);
        }
        let added = text[before..].chars().count();
        if let Some(bytes) = self.size {
            join(&mut text, &human_bytes(bytes));
        }
        (text, added)
    }
}

/// Adds `mark` to `text`, after a ` · ` when it already holds one.
fn join(text: &mut String, mark: &str) {
    if !text.is_empty() {
        text.push_str(" · ");
    }
    text.push_str(mark);
}

/// A row of the disk tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    /// Relative path, `/` separated.
    pub rel: String,
    pub folder: Option<String>,
    pub file: Option<NodeId>,
    pub open: bool,
    /// Listed but not loaded (version control, build output, dependencies): why.
    pub heavy: Option<String>,
}

/// The disk tree in Finder order (folders first, by name), with `open` folders showing what
/// they hold.
pub fn tree_rows(graph: &Graph, open: &BTreeSet<String>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    let Some(root) = graph.clusters.iter().find(|c| c.parent_id.is_none()) else { return rows };
    let index: std::collections::HashMap<&str, &studio_graph::GroupCluster> =
        graph.clusters.iter().map(|c| (c.id.as_str(), c)).collect();
    fn walk(
        graph: &Graph,
        index: &std::collections::HashMap<&str, &studio_graph::GroupCluster>,
        folder: &studio_graph::GroupCluster,
        prefix: &str,
        depth: usize,
        open: &BTreeSet<String>,
        rows: &mut Vec<TreeRow>,
    ) {
        let mut folders: Vec<&studio_graph::GroupCluster> =
            folder.child_cluster_ids.iter().filter_map(|id| index.get(id.as_str()).copied()).collect();
        folders.sort_by_key(|c| c.label.to_lowercase());
        for child in folders {
            let rel = if prefix.is_empty() { child.label.clone() } else { format!("{prefix}/{}", child.label) };
            let is_open = open.contains(&child.id);
            rows.push(TreeRow {
                depth,
                label: child.label.clone(),
                rel: rel.clone(),
                folder: Some(child.id.clone()),
                file: None,
                open: is_open,
                heavy: child.lazy.as_ref().map(|l| l.reason.clone()),
            });
            if is_open {
                walk(graph, index, child, &rel, depth + 1, open, rows);
            }
        }
        let mut files: Vec<&studio_graph::Node> = folder.node_ids.iter().filter_map(|id| graph.nodes.get(id)).collect();
        files.sort_by_key(|n| n.title.to_lowercase());
        for node in files {
            let rel = if prefix.is_empty() { node.title.clone() } else { format!("{prefix}/{}", node.title) };
            rows.push(TreeRow {
                depth,
                label: node.title.clone(),
                rel,
                folder: None,
                file: Some(node.id),
                open: false,
                heavy: None,
            });
        }
    }
    walk(graph, &index, root, "", 0, open, &mut rows);
    rows
}

/// What a click in the district asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum FilesAction {
    /// Open or close this folder in the tree.
    Toggle(String),
    /// Open this file, at this line, on the Desk.
    Open(PathBuf, Option<usize>),
    /// Open a file card's file on the Desk.
    OpenNode(NodeId),
}

const MARGIN: f32 = 36.0;
const TOP: f32 = 112.0;
const TREE_WIDTH: f32 = 700.0;
const ROW: f32 = 22.0;
const COLUMN: f32 = 340.0;
const GAP: f32 = 24.0;
const MAP_HEIGHT: f32 = 460.0;
const CARD_WIDTH: f32 = 396.0;
const CARD_ROWS: usize = 12;
/// About how wide a character of a row's marks is.
const MARK_CHAR: f32 = 5.5;

fn card_height(card: &SettingsCardView) -> f32 {
    let rows: usize = card.sections.iter().map(|(t, r)| r.len().min(CARD_ROWS) + usize::from(!t.is_empty())).sum();
    70.0 + rows.min(CARD_ROWS + 4) as f32 * ROW + 16.0
}

/// Where the district's parts go, in its local units. Used by painting and by the camera.
#[derive(Debug, Clone, PartialEq)]
pub struct FilesLayout {
    /// The disk tree's panel.
    pub tree: Rect,
    /// The tree rows shown, in order (the first `rows.len()` rows).
    pub rows: Vec<Rect>,
    /// The "where the space goes" panel.
    pub map: Rect,
    /// The "by class" panel (zero height when there are no classes yet) and its rows, one per
    /// class in the view's order.
    pub classes: Rect,
    pub class_rows: Vec<Rect>,
    /// One per settings card, in the view's order.
    pub cards: Vec<CardLayout>,
}

/// A settings card's place and its lines.
#[derive(Debug, Clone, PartialEq)]
pub struct CardLayout {
    pub rect: Rect,
    /// The title and summary; a click opens the file.
    pub head: Rect,
    pub lines: Vec<(Rect, CardLine)>,
}

/// A line of a settings card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardLine {
    /// Section `.0`'s heading.
    Heading(usize),
    /// Fact `.1` of section `.0`.
    Fact(usize, usize),
    /// "and n more" under section `.0`.
    More(usize),
}

/// Lays the district out for a size (local units), a view and a number of tree rows.
pub fn layout(size: egui::Vec2, view: &FilesView, rows: usize) -> FilesLayout {
    let tree = Rect::from_min_size(pos2(MARGIN, TOP), vec2(TREE_WIDTH, size.y - TOP - MARGIN));
    let (per_column, columns) = tree_columns(size);
    let rows = (0..rows.min(per_column * columns))
        .map(|i| {
            let (col, line) = (i / per_column, i % per_column);
            let min = tree.min + vec2(14.0 + col as f32 * COLUMN, 64.0 + line as f32 * ROW);
            Rect::from_min_size(min, vec2(COLUMN - 10.0, ROW))
        })
        .collect();
    let right = MARGIN + TREE_WIDTH + GAP;
    let map = Rect::from_min_size(pos2(right, TOP), vec2(size.x - right - MARGIN, MAP_HEIGHT));
    let (classes, class_rows) =
        classes_layout(view, Rect::from_min_size(map.left_bottom() + vec2(0.0, GAP), map.size()));
    let below = if classes.height() > 0.0 { classes.bottom() } else { map.bottom() };
    let mut y = [below + GAP; 2];
    let cards = view
        .cards
        .iter()
        .map(|card| {
            let column = if y[0] <= y[1] { 0 } else { 1 };
            let h = card_height(card);
            let min = pos2(right + column as f32 * (CARD_WIDTH + GAP), y[column]);
            y[column] += h + GAP;
            card_layout(card, Rect::from_min_size(min, vec2(CARD_WIDTH, h)))
        })
        .collect();
    FilesLayout { tree, rows, map, classes, class_rows, cards }
}

/// The "by class" panel at the top of `area` (its width), with a row per class and a line for
/// the hovered row's rules; zero height without classes.
fn classes_layout(view: &FilesView, area: Rect) -> (Rect, Vec<Rect>) {
    if view.classes.is_empty() {
        return (Rect::from_min_size(area.min, vec2(area.width(), 0.0)), Vec::new());
    }
    let height = 52.0 + (view.classes.len() + 1) as f32 * ROW + 14.0;
    let rect = Rect::from_min_size(area.min, vec2(area.width(), height));
    let rows = (0..view.classes.len())
        .map(|i| Rect::from_min_size(rect.min + vec2(10.0, 48.0 + i as f32 * ROW), vec2(rect.width() - 20.0, ROW)))
        .collect();
    (rect, rows)
}

/// Tree rows per column, and columns.
fn tree_columns(size: egui::Vec2) -> (usize, usize) {
    let per_column = (((size.y - TOP - MARGIN) - 72.0) / ROW).floor().max(1.0) as usize;
    let columns = ((TREE_WIDTH - 24.0) / COLUMN).floor().max(1.0) as usize;
    (per_column, columns)
}

/// A card's lines: each section's heading, its first [`CARD_ROWS`] facts and an "and n more",
/// stopping after [`CARD_ROWS`] + 2 facts in all.
fn card_layout(card: &SettingsCardView, rect: Rect) -> CardLayout {
    const PAD: f32 = 16.0;
    let line = |y: f32| Rect::from_min_size(pos2(rect.min.x + PAD * 0.5, y), vec2(rect.width() - PAD, ROW));
    let mut lines = Vec::new();
    let mut y = rect.min.y + 70.0;
    let mut shown = 0;
    'sections: for (s, (heading, facts)) in card.sections.iter().enumerate() {
        if !heading.is_empty() {
            lines.push((line(y), CardLine::Heading(s)));
            y += ROW;
        }
        for f in 0..facts.len().min(CARD_ROWS) {
            if shown >= CARD_ROWS + 2 {
                break 'sections;
            }
            lines.push((line(y), CardLine::Fact(s, f)));
            y += ROW;
            shown += 1;
        }
        if facts.len() > CARD_ROWS {
            lines.push((line(y), CardLine::More(s)));
            y += ROW;
        }
    }
    CardLayout { rect, head: Rect::from_min_size(rect.min, vec2(rect.width(), 60.0)), lines }
}

/// A local rectangle of the district in the world.
fn to_world(world: &WorldLayout, local: Rect) -> Rect {
    Rect::from_min_size(world.files.min + local.min.to_vec2() * world.scale, local.size() * world.scale)
}

/// The settings card of `file` in the world, for the camera: the card whose file it is, else
/// the card with a fact from it (a schema on "Data shapes").
pub fn card_world_rect(world: &WorldLayout, view: &FilesView, file: &Path) -> Option<Rect> {
    let card = view.cards.iter().position(|c| c.file == file).or_else(|| {
        view.cards.iter().position(|c| c.sections.iter().flat_map(|(_, f)| f).any(|f| f.file.as_deref() == Some(file)))
    })?;
    let local = layout(world.files.size() / world.scale, view, 0).cards[card].rect;
    Some(to_world(world, local))
}

/// The tree row of `rel` in the world, for the camera, when it is shown.
pub fn row_world_rect(world: &WorldLayout, view: &FilesView, rows: &[TreeRow], rel: &str) -> Option<Rect> {
    let i = rows.iter().position(|r| r.rel == rel)?;
    let local = *layout(world.files.size() / world.scale, view, rows.len()).rows.get(i)?;
    Some(to_world(world, local))
}

/// Squarified treemap of `sizes` in `area`: one rectangle per entry, in order.
pub fn treemap(sizes: &[u64], area: Rect) -> Vec<Rect> {
    let total: f64 = sizes.iter().map(|&s| s as f64).sum();
    if total <= 0.0 || sizes.is_empty() {
        return vec![Rect::NOTHING; sizes.len()];
    }
    let scale = (area.width() * area.height()) as f64 / total;
    let areas: Vec<f64> = sizes.iter().map(|&s| s as f64 * scale).collect();
    let mut out = vec![Rect::NOTHING; sizes.len()];
    let mut free = area;
    let mut start = 0;
    while start < areas.len() {
        let short = free.width().min(free.height()) as f64;
        let worst = |row: &[f64]| {
            let sum: f64 = row.iter().sum();
            let (max, min) = row.iter().fold((0.0f64, f64::MAX), |(a, b), &x| (a.max(x), b.min(x)));
            ((short * short * max) / (sum * sum)).max((sum * sum) / (short * short * min.max(1e-9)))
        };
        let mut end = start + 1;
        while end < areas.len() && worst(&areas[start..=end]) <= worst(&areas[start..end]) {
            end += 1;
        }
        let row = &areas[start..end];
        let sum: f64 = row.iter().sum();
        let thickness = (sum / short.max(1e-9)) as f32;
        let along_width = free.width() >= free.height();
        let mut at = 0.0f32;
        for (i, &a) in row.iter().enumerate() {
            let length = (a / sum.max(1e-9)) as f32 * if along_width { free.height() } else { free.width() };
            out[start + i] = if along_width {
                Rect::from_min_size(pos2(free.min.x, free.min.y + at), vec2(thickness, length))
            } else {
                Rect::from_min_size(pos2(free.min.x + at, free.min.y), vec2(length, thickness))
            };
            at += length;
        }
        free = if along_width {
            Rect::from_min_max(pos2(free.min.x + thickness, free.min.y), free.max)
        } else {
            Rect::from_min_max(pos2(free.min.x, free.min.y + thickness), free.max)
        };
        start = end;
    }
    out
}

/// Paints the district and returns what a click on it asks for.
#[allow(clippy::too_many_arguments)]
pub fn paint_files(
    painter: &Painter,
    world: &WorldLayout,
    transform: &CanvasTransform,
    screen: Rect,
    view: &FilesView,
    rows: &[TreeRow],
    gate: &InputGate,
) -> Option<FilesAction> {
    let local = transform.zoom * world.scale;
    let origin = transform.world_to_screen(world.files.min);
    let district = Rect::from_min_max(origin, transform.world_to_screen(world.files.max));
    if !district.intersects(screen) || local < 0.08 {
        return None;
    }
    let painter = painter.with_clip_rect(district.intersect(screen));
    let at = |r: Rect| Rect::from_min_max(origin + r.min.to_vec2() * local, origin + r.max.to_vec2() * local);
    let font = |pt: f32| FontId::proportional((pt * local).min(140.0));
    let text_ok = local >= crate::world::paint::FAR_LOCAL_ZOOM;
    let radius = |r: f32| CornerRadius::from((r * local).clamp(1.0, 24.0));
    let size = world.files.size() / world.scale;
    let parts = layout(size, view, rows.len());
    let paint = Paint { painter: &painter, local, font: &font, radius: &radius, gate, click: gate.click() };
    let mut action = None;

    // The disk tree, flowed into columns; only rows on screen are painted.
    let tree = at(parts.tree);
    if tree.intersects(screen) {
        painter.rect(tree, radius(14.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            let summary = tree_summary(view);
            painter.text(tree.min + vec2(18.0, 14.0) * local, Align2::LEFT_TOP, "On disk", font(17.0), TEXT_PRIMARY);
            painter.text(tree.min + vec2(18.0, 38.0) * local, Align2::LEFT_TOP, summary, font(11.5), TEXT_DIM);
            for (row, r) in rows.iter().zip(&parts.rows) {
                let r = at(*r);
                if r.intersects(screen) {
                    action = paint_row(&paint, view, row, r).or(action);
                }
            }
            if rows.len() > parts.rows.len() {
                painter.text(
                    tree.left_bottom() + vec2(18.0, -14.0) * local,
                    Align2::LEFT_BOTTOM,
                    format!("and {} more — close a folder to see them", rows.len() - parts.rows.len()),
                    font(11.0),
                    TEXT_DIM,
                );
            }
        }
    }

    // Where the space goes: a treemap of the top-level entries.
    let map = at(parts.map);
    if map.intersects(screen) {
        painter.rect(map, radius(14.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            let total: u64 = view.sizes.iter().map(|s| s.1).sum();
            let title = if view.sizes.is_empty() {
                "Measuring where the space goes…".to_string()
            } else {
                format!("Where the space goes · {}", human_bytes(total))
            };
            painter.text(map.min + vec2(18.0, 14.0) * local, Align2::LEFT_TOP, title, font(15.0), TEXT_PRIMARY);
        }
        let inner = Rect::from_min_max(map.min + vec2(14.0, 44.0) * local, map.max - vec2(14.0, 14.0) * local);
        let shown: Vec<&(String, u64)> = view.sizes.iter().filter(|s| s.1 > 0).take(40).collect();
        let tiles = treemap(&shown.iter().map(|s| s.1).collect::<Vec<_>>(), inner);
        for ((name, bytes), tile) in shown.iter().zip(tiles) {
            let tile = tile.shrink(1.5 * local);
            if !tile.is_positive() {
                continue;
            }
            let ignored = view.is_ignored(name);
            let fill = if ignored { with_alpha(TEXT_DIM, 60) } else { with_alpha(DISTRICT_FILES, 70) };
            painter.rect(tile, radius(6.0), fill, Stroke::NONE, StrokeKind::Inside);
            if text_ok && tile.width() > 60.0 && tile.height() > 28.0 {
                painter.text(
                    tile.min + vec2(8.0, 6.0) * local,
                    Align2::LEFT_TOP,
                    truncate_with_ellipsis(name, (tile.width() / (7.0 * local)) as usize),
                    font(12.0),
                    TEXT_PRIMARY,
                );
                painter.text(
                    tile.min + vec2(8.0, 24.0) * local,
                    Align2::LEFT_TOP,
                    human_bytes(*bytes),
                    font(10.5),
                    TEXT_SECONDARY,
                );
            }
            if let Some(id) = paint
                .click
                .filter(|p| tile.contains(*p))
                .and_then(|_| rows.iter().find(|r| &r.rel == name)?.folder.clone())
            {
                action = Some(FilesAction::Toggle(id));
            }
        }
    }

    let classes = at(parts.classes);
    if classes.is_positive() && classes.intersects(screen) {
        painter.rect(classes, radius(14.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            paint_classes(&paint, &at, view, &parts);
        }
    }

    // Settings, as facts: two columns of cards.
    for (card, place) in view.cards.iter().zip(&parts.cards) {
        let r = at(place.rect);
        if !r.intersects(screen) {
            continue;
        }
        painter.rect(r, radius(14.0), CARD_BG, Stroke::new(1.0, CARD_BORDER_NORMAL), StrokeKind::Inside);
        if text_ok {
            action = paint_card(&paint, &at, card, place).or(action);
        }
    }
    action
}

/// What every part of the district paints with.
struct Paint<'a> {
    painter: &'a Painter,
    local: f32,
    font: &'a dyn Fn(f32) -> FontId,
    radius: &'a dyn Fn(f32) -> CornerRadius,
    gate: &'a InputGate,
    click: Option<egui::Pos2>,
}

/// The tree panel's summary line: git's counts, else why they will not come, else that they are
/// being read.
pub fn tree_summary(view: &FilesView) -> String {
    match (view.tracked, view.lfs, &view.git_note) {
        (Some(t), Some(l), _) => format!("{t} tracked · {l} in Git LFS · {} ignored", view.ignored.len()),
        (_, _, Some(note)) => note.clone(),
        _ => "Reading what git tracks…".to_string(),
    }
}

/// Paints the "by class" panel's text: a row per class (name, files, bytes, its main rule) and,
/// under them, every rule of the hovered row.
fn paint_classes(p: &Paint, at: &dyn Fn(Rect) -> Rect, view: &FilesView, parts: &FilesLayout) {
    let (local, font) = (p.local, p.font);
    let panel = at(parts.classes);
    let files: usize = view.classes.iter().map(|c| c.files).sum();
    let title = format!("Tracked files by class · {}", group(files));
    p.painter.text(panel.min + vec2(18.0, 14.0) * local, Align2::LEFT_TOP, title, font(15.0), TEXT_PRIMARY);
    let mut hovered = None;
    for (class, r) in view.classes.iter().zip(&parts.class_rows) {
        let r = at(*r);
        if p.gate.hovers(r) {
            p.painter.rect_filled(r, (p.radius)(5.0), FLOATING_BTN_HOVER);
            hovered = Some(class);
        }
        let left = r.left_center() + vec2(8.0 * local, 0.0);
        p.painter.text(left, Align2::LEFT_CENTER, &class.label, font(12.0), TEXT_PRIMARY);
        let counts = format!("{} · {}", group(class.files), human_bytes(class.bytes));
        p.painter.text(left + vec2(110.0 * local, 0.0), Align2::LEFT_CENTER, counts, font(11.5), TEXT_SECONDARY);
        let main = class.rules.first().map(|(why, _)| why.as_str()).unwrap_or_default();
        let more = if class.rules.len() > 1 { format!("{main} +{}", class.rules.len() - 1) } else { main.to_string() };
        let room = ((r.width() / local - 290.0) / 6.0).max(6.0) as usize;
        p.painter.text(
            r.right_center() - vec2(8.0 * local, 0.0),
            Align2::RIGHT_CENTER,
            truncate_with_ellipsis(&more, room),
            font(10.5),
            TEXT_DIM,
        );
    }
    let detail = match hovered {
        Some(class) => format!("{}: {}", class.label, class.why()),
        None => "Bytes are file lengths. Hover a class for the rules that decided it.".to_string(),
    };
    let room = ((panel.width() / local - 36.0) / 6.0).max(6.0) as usize;
    p.painter.text(
        panel.left_bottom() + vec2(18.0, -14.0) * local,
        Align2::LEFT_BOTTOM,
        truncate_with_ellipsis(&detail, room),
        font(10.5),
        TEXT_DIM,
    );
}

/// Paints a tree row at `r` (on screen) and returns what a click on it asks for.
fn paint_row(p: &Paint, view: &FilesView, row: &TreeRow, r: Rect) -> Option<FilesAction> {
    let (local, font) = (p.local, p.font);
    let marks = view.marks(row);
    if p.gate.hovers(r) {
        p.painter.rect_filled(r, (p.radius)(5.0), FLOATING_BTN_HOVER);
    }
    let indent = 6.0 + row.depth as f32 * 14.0;
    let icon = match (&row.folder, row.open) {
        (Some(_), true) => egui_phosphor::regular::FOLDER_OPEN,
        (Some(_), false) => egui_phosphor::regular::FOLDER,
        (None, _) => egui_phosphor::regular::FILE,
    };
    let color = if marks.ignored || marks.hidden {
        TEXT_DIM
    } else if row.folder.is_some() {
        TEXT_PRIMARY
    } else {
        TEXT_SECONDARY
    };
    let (text, added) = marks.text();
    // The name leaves room for "ignored · size" always, and for any other mark as well.
    let max_chars = ((COLUMN - indent - 110.0 - added as f32 * MARK_CHAR) / 7.0).max(6.0) as usize;
    p.painter.text(
        r.left_center() + vec2(indent * local, 0.0),
        Align2::LEFT_CENTER,
        format!("{icon} {}", truncate_with_ellipsis(&row.label, max_chars)),
        font(12.0),
        color,
    );
    if !text.is_empty() {
        p.painter.text(
            r.right_center() - vec2(8.0 * local, 0.0),
            Align2::RIGHT_CENTER,
            text,
            font(10.5),
            if marks.ignored { with_alpha(DISTRICT_FILES, 200) } else { TEXT_DIM },
        );
    }
    if !p.click.is_some_and(|c| r.contains(c)) {
        return None;
    }
    match (&row.folder, row.file) {
        (Some(id), _) => Some(FilesAction::Toggle(id.clone())),
        (None, Some(node)) => Some(FilesAction::OpenNode(node)),
        _ => None,
    }
}

/// Paints a settings card's text and lines and returns what a click on it asks for.
fn paint_card(
    p: &Paint,
    at: &dyn Fn(Rect) -> Rect,
    card: &SettingsCardView,
    place: &CardLayout,
) -> Option<FilesAction> {
    let (local, font) = (p.local, p.font);
    let mono = |pt: f32| FontId::monospace((pt * local).min(140.0));
    let r = at(place.rect);
    let pad = 16.0 * local;
    let mut action = None;
    p.painter.text(r.min + vec2(pad, pad), Align2::LEFT_TOP, &card.title, font(15.0), TEXT_PRIMARY);
    let file = card.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    p.painter.text(
        r.min + vec2(pad, pad + 22.0 * local),
        Align2::LEFT_TOP,
        format!("{file} · {}", card.summary),
        font(11.0),
        TEXT_DIM,
    );
    for (line, what) in &place.lines {
        let row = at(*line);
        let text_at = row.left_center() + vec2(pad * 0.5, 0.0);
        match *what {
            CardLine::Heading(s) => {
                let heading = &card.sections[s].0;
                p.painter.text(text_at, Align2::LEFT_CENTER, heading, font(11.5), with_alpha(DISTRICT_FILES, 230));
            }
            CardLine::Fact(s, f) => {
                let fact = &card.sections[s].1[f];
                if p.gate.hovers(row) {
                    p.painter.rect_filled(row, (p.radius)(5.0), FLOATING_BTN_HOVER);
                }
                let key = truncate_with_ellipsis(&fact.key, 26);
                p.painter.text(text_at, Align2::LEFT_CENTER, key, mono(11.0), TEXT_PRIMARY);
                p.painter.text(
                    row.right_center() - vec2(pad * 0.5, 0.0),
                    Align2::RIGHT_CENTER,
                    truncate_with_ellipsis(&fact.value, 26),
                    font(11.0),
                    TEXT_SECONDARY,
                );
                if p.click.is_some_and(|c| row.contains(c)) {
                    let file = fact.file.clone().unwrap_or_else(|| card.file.clone());
                    action = Some(FilesAction::Open(file, Some(fact.line)));
                }
            }
            CardLine::More(s) => {
                let more = format!("and {} more", card.sections[s].1.len() - CARD_ROWS);
                p.painter.text(text_at, Align2::LEFT_CENTER, more, font(10.5), TEXT_DIM);
            }
        }
    }
    if p.click.is_some_and(|c| at(place.head).contains(c)) {
        action = Some(FilesAction::Open(card.file.clone(), None));
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_treemap_fills_the_area_in_proportion() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let tiles = treemap(&[600, 300, 100], area);
        let total: f32 = tiles.iter().map(|t| t.area()).sum();
        assert!((total - area.area()).abs() < 1.0);
        assert!((tiles[0].area() / area.area() - 0.6).abs() < 0.01);
        for (i, a) in tiles.iter().enumerate() {
            assert!(area.expand(0.01).contains_rect(*a));
            for b in &tiles[i + 1..] {
                assert!(a.intersect(*b).area() < 0.01, "{a:?} overlaps {b:?}");
            }
        }
        assert!(treemap(&[], area).is_empty());
    }

    #[test]
    fn the_tree_lists_folders_first_and_opens_only_open_folders() {
        use studio_graph::{GroupCluster, NodeArchetype};
        let mut g = Graph::new();
        let folder = |id: &str, label: &str, parent: Option<&str>, kids: &[&str]| {
            let mut c = GroupCluster::new(id, label, "Folder", 0);
            c.parent_id = parent.map(str::to_string);
            c.child_cluster_ids = kids.iter().map(|k| k.to_string()).collect();
            c
        };
        g.clusters = vec![folder("dir:", "root", None, &["dir:src"]), folder("dir:src", "src", Some("dir:"), &[])];
        for (name, at) in [("b.rs", 1), ("a.rs", 1), ("Cargo.toml", 0)] {
            let id = g.add_node(name, NodeArchetype::File, "", None, vec![], vec![], [0.0, 0.0]);
            g.clusters[at].node_ids.push(id);
        }
        let closed = tree_rows(&g, &BTreeSet::new());
        assert_eq!(closed.iter().map(|r| r.rel.as_str()).collect::<Vec<_>>(), ["src", "Cargo.toml"]);
        let open = tree_rows(&g, &BTreeSet::from(["dir:src".to_string()]));
        assert_eq!(
            open.iter().map(|r| r.rel.as_str()).collect::<Vec<_>>(),
            ["src", "src/a.rs", "src/b.rs", "Cargo.toml"]
        );
        assert_eq!(open[1].depth, 1);
    }

    #[test]
    fn ignored_paths_cover_what_is_inside_them() {
        let view =
            FilesView { ignored: vec![IgnoredView { path: "target/".into(), rule: None }], ..Default::default() };
        assert!(view.is_ignored("target") && view.is_ignored("target/debug/app"));
        assert!(!view.is_ignored("target-x") && !view.is_ignored("src"));
    }

    fn row(rel: &str, folder: bool, heavy: Option<&str>) -> TreeRow {
        TreeRow {
            depth: rel.matches('/').count(),
            label: rel.rsplit('/').next().unwrap().to_string(),
            rel: rel.to_string(),
            folder: folder.then(|| format!("dir:{rel}")),
            file: (!folder).then_some(NodeId(1)),
            open: false,
            heavy: heavy.map(str::to_string),
        }
    }

    #[test]
    fn rows_are_marked_by_what_git_and_the_disk_say() {
        let view = FilesView {
            ignored: vec![
                IgnoredView { path: "target/".into(), rule: None },
                IgnoredView { path: "web/dist/".into(), rule: None },
            ],
            sizes: vec![("target".into(), 2048), ("src".into(), 10)],
            lfs_paths: BTreeSet::from(["art/logo.png".to_string()]),
            generated: BTreeSet::from(["gen/api.rs".to_string()]),
            vendored: BTreeSet::from(["vendor/lib.js".to_string()]),
            ..Default::default()
        };
        view.build_lookups();
        let text = |r: &TreeRow| view.marks(r).text();
        assert_eq!(
            text(&row("target", true, Some("build cache"))),
            ("ignored · heavy: build cache · 2.0 KB".into(), 21)
        );
        assert_eq!(text(&row("src", true, None)), ("10 B".into(), 0), "only the size");
        assert_eq!(text(&row("art/logo.png", false, None)), ("LFS".into(), 3));
        assert_eq!(text(&row("gen/api.rs", false, None)).0, "generated");
        assert_eq!(text(&row("vendor/lib.js", false, None)).0, "vendored");
        assert_eq!(text(&row(".env", false, None)).0, "hidden");
        assert_eq!(text(&row("web/dist/app.js", false, None)).0, "ignored", "inside an ignored folder");
        assert_eq!(text(&row("web", true, None)).0, "", "a folder holding an ignored one is not");
        assert!(view.marks(&row("gen", true, None)) == RowMarks::default(), "file marks are for files");
    }

    #[test]
    fn the_class_panel_sits_under_the_map_and_pushes_the_cards_down() {
        let class = |label: &str, files: usize| ClassView {
            label: label.into(),
            files,
            bytes: 0,
            rules: vec![("extension".into(), files), ("tool convention: Cargo".into(), 1)],
        };
        let card = SettingsCardView {
            title: "Cargo.toml".into(),
            file: PathBuf::from("/p/Cargo.toml"),
            summary: String::new(),
            sections: vec![],
        };
        let size = vec2(1600.0, 2400.0);
        let bare = FilesView { cards: vec![card.clone()], ..Default::default() };
        let without = layout(size, &bare, 0);
        assert_eq!((without.classes.height(), without.class_rows.len()), (0.0, 0));
        assert_eq!(without.cards[0].rect.top(), without.map.bottom() + GAP);

        let view = FilesView { cards: vec![card], classes: vec![class("code", 4_831), class("tests", 204)], ..bare };
        let with = layout(size, &view, 0);
        assert_eq!(with.class_rows.len(), 2);
        assert!(with.classes.top() > with.map.bottom());
        assert!(with.class_rows.iter().all(|r| with.classes.contains_rect(*r)));
        assert_eq!(with.cards[0].rect.top(), with.classes.bottom() + GAP);
        assert_eq!(view.classes[0].why(), "extension 4,831 · tool convention: Cargo 1");
    }

    #[test]
    fn cards_and_rows_have_their_place_in_the_world() {
        let fact = |key: &str, file: Option<&str>| SettingRow {
            key: key.into(),
            value: String::new(),
            line: 1,
            file: file.map(PathBuf::from),
        };
        let card = |file: &str, facts: Vec<SettingRow>| SettingsCardView {
            title: file.into(),
            file: PathBuf::from(file),
            summary: String::new(),
            sections: vec![(String::new(), facts)],
        };
        let view = FilesView {
            cards: vec![
                card("/p/Cargo.toml", vec![fact("edition", None)]),
                card("/p/.gitignore", (0..20).map(|i| fact(&format!("rule{i}"), None)).collect()),
                card(
                    "/p/a.schema.json",
                    vec![fact("a", Some("/p/a.schema.json")), fact("b", Some("/p/b.schema.json"))],
                ),
            ],
            ..Default::default()
        };
        let world = WorldLayout::default();
        let parts = layout(world.files.size() / world.scale, &view, 3);
        assert_eq!(parts.rows.len(), 3);
        assert_eq!(parts.cards.len(), 3);
        // Cards go into the shorter column: the second beside the first, the third under the first.
        assert_eq!(parts.cards[1].rect.top(), parts.cards[0].rect.top());
        assert!(parts.cards[2].rect.left() == parts.cards[0].rect.left());
        assert!(parts.cards[2].rect.top() > parts.cards[0].rect.bottom());
        // A long card shows CARD_ROWS facts and "and n more", inside the card.
        let long = &parts.cards[1];
        assert_eq!(long.lines.iter().filter(|(_, l)| matches!(l, CardLine::Fact(..))).count(), CARD_ROWS);
        assert_eq!(long.lines.last().unwrap().1, CardLine::More(0));
        assert!(long
            .lines
            .iter()
            .filter(|(_, l)| matches!(l, CardLine::Fact(..)))
            .all(|(r, _)| long.rect.contains_rect(*r)));

        let at = |local: Rect| to_world(&world, local);
        assert_eq!(card_world_rect(&world, &view, Path::new("/p/.gitignore")), Some(at(parts.cards[1].rect)));
        assert_eq!(card_world_rect(&world, &view, Path::new("/p/b.schema.json")), Some(at(parts.cards[2].rect)));
        assert_eq!(card_world_rect(&world, &view, Path::new("/p/none.toml")), None);
        let rows = [row("src", true, None), row("Cargo.toml", false, None)];
        assert_eq!(row_world_rect(&world, &view, &rows, "Cargo.toml"), Some(at(parts.rows[1])));
        assert_eq!(row_world_rect(&world, &view, &rows, "missing"), None);
        assert!(world.files.contains_rect(at(parts.cards[2].rect)));
    }
}
