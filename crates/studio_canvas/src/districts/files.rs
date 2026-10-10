//! The Files district, west of the map: the disk exactly as it is (every folder and file, with
//! what git ignores, what lives in LFS and how big things are), where the space goes, and the
//! project's settings as facts, each opening the line it comes from.

use std::collections::BTreeSet;
use std::path::PathBuf;

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
}

impl FilesView {
    /// Whether git ignores this path (relative, `/` separated).
    pub fn is_ignored(&self, rel: &str) -> bool {
        self.ignored.iter().any(|e| {
            let p = e.path.trim_end_matches('/');
            rel == p || rel.starts_with(&format!("{p}/"))
        })
    }

    fn size_of(&self, rel: &str) -> Option<u64> {
        self.sizes.iter().find(|(name, _)| name == rel).map(|(_, b)| *b)
    }
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
    /// Listed but not loaded (version control, build output, dependencies).
    pub heavy: bool,
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
                heavy: child.lazy.is_some(),
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
                heavy: false,
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

fn card_height(card: &SettingsCardView) -> f32 {
    let rows: usize = card.sections.iter().map(|(t, r)| r.len().min(CARD_ROWS) + usize::from(!t.is_empty())).sum();
    70.0 + rows.min(CARD_ROWS + 4) as f32 * ROW + 16.0
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
    let mono = |pt: f32| FontId::monospace((pt * local).min(140.0));
    let text_ok = local >= crate::world::paint::FAR_LOCAL_ZOOM;
    let radius = |r: f32| CornerRadius::from((r * local).clamp(1.0, 24.0));
    let size = world.files.size() / world.scale;
    let accent = DISTRICT_FILES;
    let click = gate.click();
    let mut action = None;

    // The disk tree, flowed into columns; only rows on screen are painted.
    let tree = at(Rect::from_min_size(pos2(MARGIN, TOP), vec2(TREE_WIDTH, size.y - TOP - MARGIN)));
    if tree.intersects(screen) {
        painter.rect(tree, radius(14.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            let summary = match (view.tracked, view.lfs) {
                (Some(t), Some(l)) => format!("{t} tracked · {l} in Git LFS · {} ignored", view.ignored.len()),
                _ => "Reading what git tracks…".to_string(),
            };
            painter.text(tree.min + vec2(18.0, 14.0) * local, Align2::LEFT_TOP, "On disk", font(17.0), TEXT_PRIMARY);
            painter.text(tree.min + vec2(18.0, 38.0) * local, Align2::LEFT_TOP, summary, font(11.5), TEXT_DIM);
            let per_column = (((size.y - TOP - MARGIN) - 72.0) / ROW).floor().max(1.0) as usize;
            let columns = ((TREE_WIDTH - 24.0) / COLUMN).floor().max(1.0) as usize;
            for (i, row) in rows.iter().take(per_column * columns).enumerate() {
                let (col, line) = (i / per_column, i % per_column);
                let min = tree.min + vec2(14.0 + col as f32 * COLUMN, 64.0 + line as f32 * ROW) * local;
                let r = Rect::from_min_size(min, vec2(COLUMN - 10.0, ROW) * local);
                if !r.intersects(screen) {
                    continue;
                }
                let ignored = view.is_ignored(&row.rel);
                let hidden = row.label.starts_with('.');
                if gate.hovers(r) {
                    painter.rect_filled(r, radius(5.0), FLOATING_BTN_HOVER);
                }
                let indent = 6.0 + row.depth as f32 * 14.0;
                let icon = match (&row.folder, row.open) {
                    (Some(_), true) => egui_phosphor::regular::FOLDER_OPEN,
                    (Some(_), false) => egui_phosphor::regular::FOLDER,
                    (None, _) => egui_phosphor::regular::FILE,
                };
                let color = if ignored || hidden {
                    TEXT_DIM
                } else if row.folder.is_some() {
                    TEXT_PRIMARY
                } else {
                    TEXT_SECONDARY
                };
                let max_chars = ((COLUMN - indent - 110.0) / 7.0).max(6.0) as usize;
                painter.text(
                    r.left_center() + vec2(indent * local, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{icon} {}", truncate_with_ellipsis(&row.label, max_chars)),
                    font(12.0),
                    color,
                );
                let mut marks: Vec<String> = Vec::new();
                if ignored {
                    marks.push("ignored".into());
                }
                if let Some(bytes) = view.size_of(&row.rel) {
                    marks.push(human_bytes(bytes));
                }
                if !marks.is_empty() {
                    painter.text(
                        r.right_center() - vec2(8.0 * local, 0.0),
                        Align2::RIGHT_CENTER,
                        marks.join(" · "),
                        font(10.5),
                        if ignored { with_alpha(accent, 200) } else { TEXT_DIM },
                    );
                }
                if click.is_some_and(|p| r.contains(p)) {
                    action = Some(match (&row.folder, row.file) {
                        (Some(id), _) => FilesAction::Toggle(id.clone()),
                        (None, Some(node)) => FilesAction::OpenNode(node),
                        _ => continue,
                    });
                }
            }
            if rows.len() > per_column * columns {
                painter.text(
                    tree.left_bottom() + vec2(18.0, -14.0) * local,
                    Align2::LEFT_BOTTOM,
                    format!("and {} more — close a folder to see them", rows.len() - per_column * columns),
                    font(11.0),
                    TEXT_DIM,
                );
            }
        }
    }

    // Where the space goes: a treemap of the top-level entries.
    let right = MARGIN + TREE_WIDTH + GAP;
    let map = at(Rect::from_min_size(pos2(right, TOP), vec2(size.x - right - MARGIN, MAP_HEIGHT)));
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
            let fill = if ignored { with_alpha(TEXT_DIM, 60) } else { with_alpha(accent, 70) };
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
            if let Some(id) =
                click.filter(|p| tile.contains(*p)).and_then(|_| rows.iter().find(|r| &r.rel == name)?.folder.clone())
            {
                action = Some(FilesAction::Toggle(id));
            }
        }
    }

    // Settings, as facts: two columns of cards.
    let mut y = [TOP + MAP_HEIGHT + GAP; 2];
    for card in &view.cards {
        let column = if y[0] <= y[1] { 0 } else { 1 };
        let h = card_height(card);
        let min = pos2(right + column as f32 * (CARD_WIDTH + GAP), y[column]);
        y[column] += h + GAP;
        let r = at(Rect::from_min_size(min, vec2(CARD_WIDTH, h)));
        if !r.intersects(screen) {
            continue;
        }
        painter.rect(r, radius(14.0), CARD_BG, Stroke::new(1.0, CARD_BORDER_NORMAL), StrokeKind::Inside);
        if !text_ok {
            continue;
        }
        let pad = 16.0 * local;
        painter.text(r.min + vec2(pad, pad), Align2::LEFT_TOP, &card.title, font(15.0), TEXT_PRIMARY);
        let file = card.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        painter.text(
            r.min + vec2(pad, pad + 22.0 * local),
            Align2::LEFT_TOP,
            format!("{file} · {}", card.summary),
            font(11.0),
            TEXT_DIM,
        );
        let mut ry = r.min.y + 70.0 * local;
        let mut shown = 0;
        'sections: for (heading, facts) in &card.sections {
            if !heading.is_empty() {
                painter.text(
                    pos2(r.min.x + pad, ry + ROW * 0.5 * local),
                    Align2::LEFT_CENTER,
                    heading,
                    font(11.5),
                    with_alpha(accent, 230),
                );
                ry += ROW * local;
            }
            for fact in facts.iter().take(CARD_ROWS) {
                if shown >= CARD_ROWS + 2 {
                    break 'sections;
                }
                let row = Rect::from_min_size(pos2(r.min.x + pad * 0.5, ry), vec2(r.width() - pad, ROW * local));
                if gate.hovers(row) {
                    painter.rect_filled(row, radius(5.0), FLOATING_BTN_HOVER);
                }
                painter.text(
                    row.left_center() + vec2(pad * 0.5, 0.0),
                    Align2::LEFT_CENTER,
                    truncate_with_ellipsis(&fact.key, 26),
                    mono(11.0),
                    TEXT_PRIMARY,
                );
                painter.text(
                    row.right_center() - vec2(pad * 0.5, 0.0),
                    Align2::RIGHT_CENTER,
                    truncate_with_ellipsis(&fact.value, 26),
                    font(11.0),
                    TEXT_SECONDARY,
                );
                if click.is_some_and(|p| row.contains(p)) {
                    action = Some(FilesAction::Open(
                        fact.file.clone().unwrap_or_else(|| card.file.clone()),
                        Some(fact.line),
                    ));
                }
                ry += ROW * local;
                shown += 1;
            }
            if facts.len() > CARD_ROWS {
                painter.text(
                    pos2(r.min.x + pad, ry + ROW * 0.5 * local),
                    Align2::LEFT_CENTER,
                    format!("and {} more", facts.len() - CARD_ROWS),
                    font(10.5),
                    TEXT_DIM,
                );
                ry += ROW * local;
            }
        }
        let head = Rect::from_min_size(r.min, vec2(r.width(), 60.0 * local));
        if click.is_some_and(|p| head.contains(p)) {
            action = Some(FilesAction::Open(card.file.clone(), None));
        }
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
}
