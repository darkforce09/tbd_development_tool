//! The Run district, east of the map: the tools the project defines, its CI workflows, its tests
//! by part, the packages it builds, and the console.

use std::path::PathBuf;

use egui::{pos2, vec2, Align2, CornerRadius, FontId, Painter, Rect, Stroke, StrokeKind};
use studio_ui::color_tokens::*;
use studio_ui::{truncate_with_ellipsis, with_alpha};

use crate::transform::CanvasTransform;
use crate::view::gate::InputGate;
use crate::world::WorldLayout;

/// A tool, as the district shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolTile {
    pub name: String,
    pub kind: &'static str,
    pub icon: &'static str,
    /// The exact command line; empty for what only CI runs.
    pub invocation: String,
    pub description: Option<String>,
    pub file: PathBuf,
    pub line: usize,
    pub children: Vec<ToolTile>,
}

impl ToolTile {
    /// Everything under it, depth first.
    pub fn count_nested(&self) -> usize {
        self.children.iter().map(|c| 1 + c.count_nested()).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRow {
    pub name: String,
    pub triggers: Option<String>,
    pub jobs: Vec<String>,
    pub file: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageBrick {
    pub name: String,
    pub binaries: usize,
}

/// What the Run district shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunView {
    pub tools: Vec<ToolTile>,
    pub workflows: Vec<WorkflowRow>,
    /// Tests by part, most first.
    pub tests: Vec<(String, u64)>,
    pub packages: Vec<PackageBrick>,
    /// Why there are no packages, when their source said why ("No Cargo packages here").
    pub packages_note: Option<String>,
}

/// What the tools grid says when the project has no tools.
pub const NO_TOOLS: &str = "No tools found: no Cargo binaries or aliases, npm scripts, make, just or workflows";
/// What the package wall says when it is empty and no source said why.
pub const NO_PACKAGES: &str = "No packages found";

/// The sections of the district, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSection {
    Tools,
    Ci,
    Tests,
    Builds,
    Console,
}

/// What a click in the district asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum RunAction {
    /// Select the tool tile at this index (or none).
    Select(Option<usize>),
    /// Open this file at this line on the Desk.
    Open(PathBuf, usize),
}

const MARGIN: f32 = 36.0;
const TOP: f32 = 112.0;
const TILE: [f32; 2] = [360.0, 120.0];
const GAP: f32 = 24.0;
const COLUMNS: usize = 3;
const HEADER: f32 = 44.0;
const ROW: f32 = 30.0;
const BRICK: [f32; 2] = [92.0, 30.0];
const MAX_TESTS: usize = 10;
const DETAIL_ROWS: usize = 22;

/// Where each section and tile sits, in the district's units.
#[derive(Debug, Clone, PartialEq)]
pub struct RunLayout {
    pub sections: Vec<(RunSection, Rect)>,
    pub tiles: Vec<Rect>,
    pub detail: Rect,
}

/// Lays the district out: the tools grid with a detail panel beside it, then CI, tests, the
/// package wall and the console, each as tall as what it holds.
pub fn layout(view: &RunView, width: f32) -> RunLayout {
    let inner = width - MARGIN * 2.0;
    let mut y = TOP;
    let mut sections = Vec::new();
    let mut tiles = Vec::new();

    let rows = view.tools.len().div_ceil(COLUMNS).max(1);
    let tools_h = HEADER + rows as f32 * (TILE[1] + GAP);
    for (i, _) in view.tools.iter().enumerate() {
        let (col, row) = (i % COLUMNS, i / COLUMNS);
        let min = pos2(MARGIN + col as f32 * (TILE[0] + GAP), y + HEADER + row as f32 * (TILE[1] + GAP));
        tiles.push(Rect::from_min_size(min, vec2(TILE[0], TILE[1])));
    }
    let grid_w = COLUMNS as f32 * (TILE[0] + GAP);
    let detail = Rect::from_min_max(
        pos2(MARGIN + grid_w, y + HEADER),
        pos2(width - MARGIN, y + tools_h.max(HEADER + DETAIL_ROWS as f32 * ROW + 60.0) - GAP),
    );
    let tools_h = tools_h.max(detail.bottom() - y + GAP);
    sections.push((RunSection::Tools, Rect::from_min_size(pos2(MARGIN, y), vec2(inner, tools_h))));
    y += tools_h + GAP;

    let ci_h = HEADER + view.workflows.len().max(1) as f32 * (ROW + 8.0);
    sections.push((RunSection::Ci, Rect::from_min_size(pos2(MARGIN, y), vec2(inner, ci_h))));
    y += ci_h + GAP;

    let tests_h = HEADER + view.tests.len().clamp(1, MAX_TESTS) as f32 * ROW;
    sections.push((RunSection::Tests, Rect::from_min_size(pos2(MARGIN, y), vec2(inner, tests_h))));
    y += tests_h + GAP;

    let per_row = ((inner + 8.0) / (BRICK[0] + 8.0)).floor().max(1.0) as usize;
    let brick_rows = view.packages.len().div_ceil(per_row).max(1);
    let builds_h = HEADER + brick_rows as f32 * (BRICK[1] + 8.0);
    sections.push((RunSection::Builds, Rect::from_min_size(pos2(MARGIN, y), vec2(inner, builds_h))));
    y += builds_h + GAP;

    sections.push((RunSection::Console, Rect::from_min_size(pos2(MARGIN, y), vec2(inner, 220.0))));
    RunLayout { sections, tiles, detail }
}

/// A section's place in the world, for the camera.
pub fn section_world_rect(world: &WorldLayout, view: &RunView, section: RunSection) -> Rect {
    let layout = layout(view, world.run.width() / world.scale);
    let local = layout.sections.iter().find(|(s, _)| *s == section).map_or(Rect::NOTHING, |(_, r)| *r);
    Rect::from_min_size(world.run.min + local.min.to_vec2() * world.scale, local.size() * world.scale)
}

/// The tile of the tool named `name`, in the world.
pub fn tile_world_rect(world: &WorldLayout, view: &RunView, index: usize) -> Option<Rect> {
    let layout = layout(view, world.run.width() / world.scale);
    let local = *layout.tiles.get(index)?;
    Some(Rect::from_min_size(world.run.min + local.min.to_vec2() * world.scale, local.size() * world.scale))
}

/// Paints the district and returns what a click on it asks for.
pub fn paint_run(
    painter: &Painter,
    world: &WorldLayout,
    transform: &CanvasTransform,
    screen: Rect,
    view: &RunView,
    selected: Option<usize>,
    gate: &InputGate,
) -> Option<RunAction> {
    let local = transform.zoom * world.scale;
    let origin = transform.world_to_screen(world.run.min);
    let district = Rect::from_min_max(origin, transform.world_to_screen(world.run.max));
    if !district.intersects(screen) || local < 0.08 {
        return None;
    }
    let painter = painter.with_clip_rect(district.intersect(screen));
    let at = |r: Rect| Rect::from_min_max(origin + r.min.to_vec2() * local, origin + r.max.to_vec2() * local);
    let font = |pt: f32| FontId::proportional((pt * local).min(140.0));
    let mono = |pt: f32| FontId::monospace((pt * local).min(140.0));
    let text_ok = local >= crate::world::paint::FAR_LOCAL_ZOOM;
    let layout = layout(view, world.run.width() / world.scale);
    let radius = |r: f32| CornerRadius::from((r * local).clamp(1.0, 24.0));
    let accent = DISTRICT_RUN;
    let mut action = None;
    let click = gate.click();

    for (section, rect) in &layout.sections {
        let r = at(*rect);
        if !r.intersects(screen) {
            continue;
        }
        if text_ok {
            let (title, count) = match section {
                RunSection::Tools => (
                    "Tools",
                    format!(
                        "{} tools · {} commands",
                        view.tools.len(),
                        view.tools.iter().map(|t| 1 + t.count_nested()).sum::<usize>()
                    ),
                ),
                RunSection::Ci => ("CI", format!("{} workflows", view.workflows.len())),
                RunSection::Tests => ("Tests", format!("{} in all", view.tests.iter().map(|t| t.1).sum::<u64>())),
                RunSection::Builds => ("Builds", format!("{} packages", view.packages.len())),
                RunSection::Console => ("Console", String::new()),
            };
            painter.text(r.min, Align2::LEFT_TOP, title, font(20.0), TEXT_PRIMARY);
            painter.text(r.min + vec2(120.0 * local, 5.0 * local), Align2::LEFT_TOP, count, font(13.0), TEXT_DIM);
        }
    }

    let section = |s: RunSection| layout.sections.iter().find(|(k, _)| *k == s).map(|(_, r)| at(*r));
    // Tools: one tile each, the selected one's commands beside the grid.
    if let Some(r) = section(RunSection::Tools).filter(|r| r.intersects(screen) && text_ok && view.tools.is_empty()) {
        painter.text(r.min + vec2(0.0, HEADER * local), Align2::LEFT_TOP, NO_TOOLS, font(13.0), TEXT_DIM);
    }
    for (i, (tool, rect)) in view.tools.iter().zip(&layout.tiles).enumerate() {
        let r = at(*rect);
        if !r.intersects(screen) {
            continue;
        }
        let is_selected = selected == Some(i);
        let hovered = gate.hovers(r);
        let fill = if hovered { CARD_BG_HOVER } else { CARD_BG };
        let stroke = if is_selected { Stroke::new(2.0, accent) } else { Stroke::new(1.0, CARD_BORDER_NORMAL) };
        painter.rect(r, radius(14.0), fill, stroke, StrokeKind::Inside);
        if text_ok {
            let pad = vec2(18.0, 16.0) * local;
            painter.text(r.min + pad, Align2::LEFT_TOP, tool.icon, font(24.0), accent);
            let x = r.min.x + 58.0 * local;
            painter.text(
                pos2(x, r.min.y + 14.0 * local),
                Align2::LEFT_TOP,
                truncate_with_ellipsis(&tool.name, 26),
                font(16.0),
                TEXT_PRIMARY,
            );
            painter.text(pos2(x, r.min.y + 36.0 * local), Align2::LEFT_TOP, tool.kind, font(11.0), TEXT_DIM);
            if !tool.invocation.is_empty() {
                painter.text(
                    pos2(r.min.x + 18.0 * local, r.min.y + 64.0 * local),
                    Align2::LEFT_TOP,
                    truncate_with_ellipsis(&tool.invocation, 40),
                    mono(11.5),
                    TEXT_SECONDARY,
                );
            }
            if let Some(d) = &tool.description {
                painter.text(
                    pos2(r.min.x + 18.0 * local, r.min.y + 88.0 * local),
                    Align2::LEFT_TOP,
                    truncate_with_ellipsis(d, 44),
                    font(12.0),
                    TEXT_DIM,
                );
            }
            let nested = tool.count_nested();
            if nested > 0 {
                let label = if nested == 1 { "1 command".to_string() } else { format!("{nested} commands") };
                painter.text(
                    r.right_top() + vec2(-14.0, 14.0) * local,
                    Align2::RIGHT_TOP,
                    label,
                    font(11.0),
                    with_alpha(accent, 220),
                );
            }
        }
        if click.is_some_and(|p| r.contains(p)) {
            action = Some(RunAction::Select(if is_selected { None } else { Some(i) }));
        }
    }

    // The selected tool: its commands, each with the line that runs it.
    let detail = at(layout.detail);
    if detail.intersects(screen) {
        painter.rect(detail, radius(14.0), PANEL_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            let pad = 18.0 * local;
            match selected.and_then(|i| view.tools.get(i)) {
                None => {
                    painter.text(
                        detail.min + vec2(pad, pad),
                        Align2::LEFT_TOP,
                        "Pick a tool to see its commands.",
                        font(13.0),
                        TEXT_DIM,
                    );
                }
                Some(tool) => {
                    painter.text(detail.min + vec2(pad, pad), Align2::LEFT_TOP, &tool.name, font(16.0), TEXT_PRIMARY);
                    let open =
                        Rect::from_min_size(detail.min + vec2(pad, pad + 28.0 * local), vec2(220.0, 20.0) * local);
                    let open_color = if gate.hovers(open) { TEXT_HIGHLIGHT } else { accent };
                    painter.text(
                        open.min,
                        Align2::LEFT_TOP,
                        format!("{} Open where it is defined", egui_phosphor::regular::ARROW_SQUARE_OUT),
                        font(12.0),
                        open_color,
                    );
                    if click.is_some_and(|p| open.contains(p)) {
                        action = Some(RunAction::Open(tool.file.clone(), tool.line));
                    }
                    let mut y = open.bottom() + 14.0 * local;
                    for child in tool.children.iter().take(DETAIL_ROWS) {
                        let row = Rect::from_min_size(
                            pos2(detail.min.x + pad, y),
                            vec2(detail.width() - pad * 2.0, ROW * local),
                        );
                        if gate.hovers(row) {
                            painter.rect_filled(row, radius(6.0), FLOATING_BTN_HOVER);
                        }
                        painter.text(
                            row.left_center() + vec2(6.0 * local, 0.0),
                            Align2::LEFT_CENTER,
                            truncate_with_ellipsis(&child.name, 18),
                            mono(12.0),
                            TEXT_PRIMARY,
                        );
                        if let Some(d) = &child.description {
                            painter.text(
                                row.left_center() + vec2(150.0 * local, 0.0),
                                Align2::LEFT_CENTER,
                                truncate_with_ellipsis(d, 22),
                                font(11.0),
                                TEXT_DIM,
                            );
                        }
                        if click.is_some_and(|p| row.contains(p)) {
                            action = Some(RunAction::Open(child.file.clone(), child.line));
                        }
                        y += ROW * local;
                    }
                    if tool.children.len() > DETAIL_ROWS {
                        painter.text(
                            pos2(detail.min.x + pad, y + 4.0 * local),
                            Align2::LEFT_TOP,
                            format!("and {} more", tool.children.len() - DETAIL_ROWS),
                            font(11.0),
                            TEXT_DIM,
                        );
                    }
                }
            }
        }
    }

    // CI: each workflow with its jobs.
    if let Some(r) = section(RunSection::Ci).filter(|r| r.intersects(screen) && text_ok) {
        if view.workflows.is_empty() {
            painter.text(
                r.min + vec2(0.0, HEADER * local),
                Align2::LEFT_TOP,
                "No CI workflows in .github/workflows",
                font(13.0),
                TEXT_DIM,
            );
        }
        for (i, w) in view.workflows.iter().enumerate() {
            let y = r.min.y + (HEADER + i as f32 * (ROW + 8.0)) * local;
            let line = Rect::from_min_size(pos2(r.min.x, y), vec2(r.width(), ROW * local));
            painter.text(
                line.left_center(),
                Align2::LEFT_CENTER,
                format!("{} {}", egui_phosphor::regular::GIT_BRANCH, w.name),
                font(14.0),
                TEXT_PRIMARY,
            );
            let jobs = w.jobs.join(" · ");
            let info = match &w.triggers {
                Some(t) => format!("{t} · {} jobs: {jobs}", w.jobs.len()),
                None => format!("{} jobs: {jobs}", w.jobs.len()),
            };
            painter.text(
                line.left_center() + vec2(260.0 * local, 0.0),
                Align2::LEFT_CENTER,
                truncate_with_ellipsis(&info, 120),
                font(12.0),
                TEXT_DIM,
            );
            if click.is_some_and(|p| line.contains(p)) {
                action = Some(RunAction::Open(w.file.clone(), 1));
            }
        }
    }

    // Tests: a bar per part.
    if let Some(r) = section(RunSection::Tests).filter(|r| r.intersects(screen)) {
        let most = view.tests.first().map_or(1, |t| t.1.max(1)) as f32;
        for (i, (part, tests)) in view.tests.iter().take(MAX_TESTS).enumerate() {
            let y = r.min.y + (HEADER + i as f32 * ROW) * local;
            let bar_x = r.min.x + 300.0 * local;
            let full = r.right() - bar_x - 90.0 * local;
            let bar =
                Rect::from_min_size(pos2(bar_x, y + 8.0 * local), vec2(full * (*tests as f32 / most), 14.0 * local));
            painter.rect_filled(bar, radius(4.0), with_alpha(accent, 150));
            if text_ok {
                painter.text(
                    pos2(r.min.x, y + 15.0 * local),
                    Align2::LEFT_CENTER,
                    truncate_with_ellipsis(part, 34),
                    font(12.5),
                    TEXT_SECONDARY,
                );
                painter.text(
                    bar.right_center() + vec2(8.0 * local, 0.0),
                    Align2::LEFT_CENTER,
                    tests.to_string(),
                    mono(11.5),
                    TEXT_DIM,
                );
            }
        }
        if view.tests.is_empty() && text_ok {
            painter.text(r.min + vec2(0.0, HEADER * local), Align2::LEFT_TOP, "No tests found", font(13.0), TEXT_DIM);
        }
    }

    // Builds: a brick per package, the ones that build programs in the district's colour.
    if let Some(r) = section(RunSection::Builds).filter(|r| r.intersects(screen)) {
        if view.packages.is_empty() && text_ok {
            let note = view.packages_note.as_deref().unwrap_or(NO_PACKAGES);
            painter.text(r.min + vec2(0.0, HEADER * local), Align2::LEFT_TOP, note, font(13.0), TEXT_DIM);
        }
        let per_row = ((r.width() / local + 8.0) / (BRICK[0] + 8.0)).floor().max(1.0) as usize;
        for (i, p) in view.packages.iter().enumerate() {
            let (col, row) = (i % per_row, i / per_row);
            let min = r.min + vec2(col as f32 * (BRICK[0] + 8.0), HEADER + row as f32 * (BRICK[1] + 8.0)) * local;
            let brick = Rect::from_min_size(min, vec2(BRICK[0], BRICK[1]) * local);
            let color = if p.binaries > 0 { with_alpha(accent, 90) } else { with_alpha(CARD_BORDER_NORMAL, 255) };
            painter.rect(brick, radius(5.0), color, Stroke::NONE, StrokeKind::Inside);
            if local * 9.5 >= 6.0 {
                painter.text(
                    brick.center(),
                    Align2::CENTER_CENTER,
                    truncate_with_ellipsis(&p.name, 12),
                    font(9.5),
                    TEXT_SECONDARY,
                );
            }
        }
    }

    // Console: nothing runs from Studio yet.
    if let Some(r) = section(RunSection::Console).filter(|r| r.intersects(screen)) {
        let body = Rect::from_min_max(r.min + vec2(0.0, HEADER * local), r.max);
        painter.rect(body, radius(14.0), CODE_EDITOR_BG, Stroke::new(1.0, PANEL_BORDER), StrokeKind::Inside);
        if text_ok {
            painter.text(
                body.min + vec2(18.0, 18.0) * local,
                Align2::LEFT_TOP,
                "Nothing has run yet.",
                mono(12.5),
                TEXT_DIM,
            );
        }
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    fn tool(name: &str, children: usize) -> ToolTile {
        ToolTile {
            name: name.into(),
            kind: "binary",
            icon: "",
            invocation: format!("cargo run -p {name} --"),
            description: None,
            file: PathBuf::from(format!("/p/{name}.rs")),
            line: 1,
            children: (0..children).map(|i| tool(&format!("{name}-{i}"), 0)).collect(),
        }
    }

    fn view() -> RunView {
        RunView {
            tools: (0..7).map(|i| tool(&format!("t{i}"), i)).collect(),
            workflows: vec![WorkflowRow {
                name: "ci".into(),
                triggers: None,
                jobs: vec!["check".into()],
                file: "/p/ci.yml".into(),
            }],
            tests: vec![("api".into(), 499), ("web".into(), 12)],
            packages: (0..30).map(|i| PackageBrick { name: format!("p{i}"), binaries: i % 3 }).collect(),
            packages_note: None,
        }
    }

    #[test]
    fn sections_stack_without_overlapping_and_hold_their_tiles() {
        let layout = layout(&view(), 1600.0);
        let order: Vec<RunSection> = layout.sections.iter().map(|(s, _)| *s).collect();
        assert_eq!(
            order,
            [RunSection::Tools, RunSection::Ci, RunSection::Tests, RunSection::Builds, RunSection::Console]
        );
        for pair in layout.sections.windows(2) {
            assert!(pair[0].1.bottom() <= pair[1].1.top(), "{:?} runs into {:?}", pair[0].0, pair[1].0);
        }
        let tools = layout.sections[0].1;
        assert_eq!(layout.tiles.len(), 7);
        assert!(layout.tiles.iter().all(|t| tools.contains_rect(*t)));
        assert!(tools.contains_rect(layout.detail), "the detail panel sits beside the tiles");
        assert!(layout.tiles.iter().all(|t| !t.intersects(layout.detail)));
    }

    #[test]
    fn the_dock_can_find_each_section_and_tile_in_the_world() {
        let world = WorldLayout::compute(Rect::from_min_size(Pos2::ZERO, vec2(5600.0, 3600.0)), &Default::default());
        let v = view();
        let tools = section_world_rect(&world, &v, RunSection::Tools);
        assert!(world.run.contains_rect(tools));
        let tile = tile_world_rect(&world, &v, 4).unwrap();
        assert!(tools.contains_rect(tile));
        assert!(tile_world_rect(&world, &v, 99).is_none());
    }
}
