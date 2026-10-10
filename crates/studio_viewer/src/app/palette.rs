//! The ⌘K palette: a dropdown under the title bar's search field. It finds files, symbols,
//! commands and what the sources read, and goes where each result lives (PLAN D4): Enter opens a
//! result, Shift+Enter shows it on the map. It only finds and navigates; nothing it does changes
//! what the map says.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Id, Order, Rect, Sense, Stroke, StrokeKind};
use studio_canvas::districts::changes::{row_world_rect, session_world_rect};
use studio_canvas::districts::pipeline::{group_world_rect, is_expanded, reveal_rect};
use studio_canvas::shortcuts::{consume, KeyFocus, ShortcutAction};
use studio_canvas::{CameraTarget, CanvasAction, Stop, WorldLayout};
use studio_graph::{FolderDetail, Graph, NodeId};
use studio_parser::builder::common::node_rel_path;
use studio_parser::search_index::{member_index, SOURCES_SLOT};
use studio_parser::{Entry, EntryKind, Query, Scope, Segment};
use studio_sources::{AgentIndex, GitHistory, LinkKind, Pipeline, Settings, StepKind, TicketIndex, Tool, Tools};
use studio_ui::color_tokens::*;

use super::commands::{command_list, run_command, AppCommand, CommandSpec};
use super::view_menu::VIEW_TOGGLES;
use super::StudioApp;

/// The dropdown's width, before it is clamped inside the window.
const WIDTH: f32 = 560.0;
/// Height of one result row.
const ROW_HEIGHT: f32 = 30.0;
/// Most of the window's height the dropdown takes.
const MAX_HEIGHT_SHARE: f32 = 0.6;
/// Longest value shown for a settings fact, in chars.
const FACT_VALUE_CHARS: usize = 60;

/// What a result of the sources-and-commands segment does, by its `Entry::key`.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteTarget {
    Command(AppCommand),
    /// A Run tool: where it is defined, and its top-level tool (whose tile the Run district shows).
    Tool {
        file: PathBuf,
        line: usize,
        top: ToolRef,
    },
    /// A settings fact or an ignore rule.
    Setting {
        file: PathBuf,
        line: usize,
    },
    /// A pipeline route or flow: the step to open, and its group in the Pipeline district.
    Step {
        file: PathBuf,
        line: usize,
        group: Option<usize>,
    },
    Branch(String),
    Worktree(PathBuf),
    /// A ticket's `.toml`.
    Ticket(PathBuf),
    /// An agent session, by id.
    Session(String),
}

/// A top-level tool, as its Run tile is found.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRef {
    pub name: String,
    pub invocation: String,
    pub file: PathBuf,
    pub line: usize,
}

/// One shown result.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub entry: Entry,
    /// Byte offsets of the matched chars: in `entry.name`, or in `entry.detail` when `in_path`.
    pub positions: Vec<u32>,
    pub in_path: bool,
}

/// A group of shown results under its header.
#[derive(Debug, Clone, PartialEq)]
pub struct RowGroup {
    pub title: &'static str,
    /// Every match of the group; the rows are the best of them.
    pub total: usize,
    pub rows: Vec<Row>,
}

/// What the sources-and-commands segment was built from, to build it again only when that changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SourcesStamp {
    /// The sources' data, by address.
    sources: [usize; 6],
    /// The View toggles, which the commands' details show.
    toggles: Vec<bool>,
}

/// The palette: its field, its results and what its entries do.
#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub text: String,
    /// The text the shown results are for; `None` searches again.
    searched: Option<String>,
    pub groups: Vec<RowGroup>,
    /// The highlighted row, counted across groups.
    pub highlight: usize,
    /// Bring the highlighted row into view on the next frame.
    scroll: bool,
    /// Entries per kind, for the counts in the corner; `None` counts again.
    counts: Option<[usize; 12]>,
    /// Where the title bar's field is, for the dropdown to hang from.
    pub field_rect: Option<Rect>,
    /// Where the canvas is, for a view at 100%.
    pub canvas_rect: Option<Rect>,
    /// The sources-and-commands segment, kept for every new index the loader sends.
    pub segment: Arc<Segment>,
    /// What each entry of `segment` does, by its key.
    pub targets: Vec<PaletteTarget>,
    stamp: Option<SourcesStamp>,
    /// A file to show on the map once its folder has loaded (project-relative).
    pub pending_reveal: Option<String>,
    /// File cards by project-relative path, built when the graph changes.
    pub nodes: HashMap<String, NodeId>,
}

impl PaletteState {
    /// Every shown row, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.groups.iter().flat_map(|g| &g.rows)
    }

    fn row_count(&self) -> usize {
        self.groups.iter().map(|g| g.rows.len()).sum()
    }

    /// Moves the highlight by `step`, wrapping around.
    fn step(&mut self, step: isize) {
        let n = self.row_count();
        if n > 0 {
            self.highlight = (self.highlight as isize + step).rem_euclid(n as isize) as usize;
            self.scroll = true;
        }
    }
}

/// The sources the segment is built from; any of them may not have arrived yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceParts<'a> {
    /// The canonical project root, for project-relative paths.
    pub root: Option<&'a Path>,
    pub tools: Option<&'a Tools>,
    pub settings: Option<&'a Settings>,
    pub pipeline: Option<&'a Pipeline>,
    pub history: Option<&'a GitHistory>,
    pub tickets: Option<&'a TicketIndex>,
    pub agents: Option<&'a AgentIndex>,
    /// How far local time is ahead of UTC, for sessions' start times.
    pub offset_secs: i64,
}

/// Entries and their targets, the key being the target's index.
#[derive(Default)]
struct SegmentBuilder {
    entries: Vec<Entry>,
    targets: Vec<PaletteTarget>,
}

impl SegmentBuilder {
    fn push(
        &mut self,
        kind: EntryKind,
        name: String,
        detail: String,
        at: Option<(String, usize)>,
        target: PaletteTarget,
    ) {
        let (path, line) = match at {
            Some((path, line)) => (Some(path), Some(line as u32)),
            None => (None, None),
        };
        self.entries.push(Entry { kind, name, detail, path, line, key: self.targets.len() as u64 });
        self.targets.push(target);
    }
}

/// The sources-and-commands segment (slot 2): commands, tools, settings facts and ignore rules,
/// pipeline routes and flows, local branches, worktrees, tickets and sessions. A session shows
/// only its slug (or the first 8 chars of its id) and its start time.
pub fn sources_segment(commands: &[(AppCommand, CommandSpec)], parts: &SourceParts) -> (Segment, Vec<PaletteTarget>) {
    let mut b = SegmentBuilder::default();
    for (cmd, spec) in commands {
        b.push(EntryKind::Command, spec.title.clone(), spec.detail.clone(), None, PaletteTarget::Command(*cmd));
    }
    let root = parts.root.unwrap_or(Path::new(""));
    if let Some(tools) = parts.tools {
        push_tools(&mut b, root, tools);
    }
    if let Some(settings) = parts.settings {
        push_settings(&mut b, root, settings);
    }
    if let Some(pipeline) = parts.pipeline {
        push_pipeline(&mut b, root, pipeline);
    }
    if let Some(history) = parts.history {
        push_git(&mut b, history);
    }
    if let Some(tickets) = parts.tickets {
        for t in &tickets.tickets {
            let file = tickets.folder.join(Path::new(&t.file).file_name().unwrap_or_default());
            let name = format!("{} {}", t.id, t.title);
            b.push(EntryKind::Ticket, name, t.status.clone(), None, PaletteTarget::Ticket(file));
        }
    }
    if let Some(agents) = parts.agents {
        for s in &agents.sessions {
            let name = s.slug.clone().unwrap_or_else(|| s.id.chars().take(8).collect());
            let started = local_time(s.started, parts.offset_secs);
            b.push(EntryKind::Session, name, started, None, PaletteTarget::Session(s.id.clone()));
        }
    }
    (Segment::new(b.entries), b.targets)
}

fn push_tools(b: &mut SegmentBuilder, root: &Path, tools: &Tools) {
    for top in &tools.tools {
        let top_ref = ToolRef {
            name: top.name.clone(),
            invocation: top.invocation.clone(),
            file: top.file.clone(),
            line: top.line,
        };
        for tool in top.walk() {
            let file = absolute(root, &tool.file);
            let at = (relative(root, &file), tool.line);
            let target = PaletteTarget::Tool { file, line: tool.line, top: top_ref.clone() };
            b.push(EntryKind::Tool, tool.name.clone(), tool_detail(tool), Some(at), target);
        }
    }
}

/// A tool's command line, or its kind when only CI runs it.
fn tool_detail(tool: &Tool) -> String {
    if tool.invocation.is_empty() {
        tool.kind.label().to_string()
    } else {
        tool.invocation.clone()
    }
}

fn push_settings(b: &mut SegmentBuilder, root: &Path, settings: &Settings) {
    for sheet in &settings.sheets {
        let file = absolute(root, &sheet.file);
        let rel = relative(root, &file);
        for fact in sheet.sections.iter().flat_map(|(_, facts)| facts) {
            let name = format!("{} = {}", fact.key, shortened(&fact.value, FACT_VALUE_CHARS));
            let target = PaletteTarget::Setting { file: file.clone(), line: fact.line };
            b.push(EntryKind::Setting, name, format!("{rel}:{}", fact.line), Some((rel.clone(), fact.line)), target);
        }
    }
    if let Some(ignore) = &settings.gitignore {
        let file = absolute(root, &ignore.file);
        let rel = relative(root, &file);
        for rule in ignore.groups.iter().flat_map(|g| &g.rules) {
            let target = PaletteTarget::Setting { file: file.clone(), line: rule.line };
            let detail = format!("{rel}:{}", rule.line);
            b.push(EntryKind::Setting, rule.pattern.clone(), detail, Some((rel.clone(), rule.line)), target);
        }
    }
}

fn push_pipeline(b: &mut SegmentBuilder, root: &Path, pipeline: &Pipeline) {
    // Each step's group: that of the first flow it is in.
    let mut groups: BTreeMap<usize, usize> = BTreeMap::new();
    for flow in &pipeline.flows {
        for &step in flow.layers.iter().flatten() {
            groups.entry(step).or_insert(flow.group);
        }
    }
    let step_target = |open: usize, group: Option<usize>| {
        let step = &pipeline.steps[open];
        let at = (step.path.to_string_lossy().replace('\\', "/"), step.line);
        (PaletteTarget::Step { file: absolute(root, &step.path), line: step.line, group }, at)
    };
    for (i, step) in pipeline.steps.iter().enumerate().filter(|(_, s)| s.kind == StepKind::Route) {
        // A route opens at its handler, when the route table names one.
        let handler = pipeline
            .links
            .iter()
            .find(|l| l.from == i && l.kind == LinkKind::Serves && l.to < pipeline.steps.len())
            .map_or(i, |l| l.to);
        let (target, at) = step_target(handler, groups.get(&i).copied());
        b.push(EntryKind::Route, step.title.clone(), format!("{}:{}", at.0, at.1), Some(at), target);
    }
    for flow in pipeline.flows.iter().filter(|f| f.entry < pipeline.steps.len()) {
        let (target, at) = step_target(flow.entry, Some(flow.group));
        b.push(EntryKind::Flow, flow.name.clone(), format!("{}:{}", at.0, at.1), Some(at), target);
    }
}

fn push_git(b: &mut SegmentBuilder, history: &GitHistory) {
    for branch in &history.branches {
        let detail = branch.subject.clone();
        b.push(EntryKind::Branch, branch.name.clone(), detail, None, PaletteTarget::Branch(branch.name.clone()));
    }
    for w in &history.worktrees {
        let name = w.path.file_name().map_or_else(|| w.path.display().to_string(), |n| n.to_string_lossy().into());
        let branch =
            w.branch.clone().unwrap_or_else(|| format!("detached at {}", w.head.chars().take(7).collect::<String>()));
        let detail = format!("{branch} · {}", w.path.display());
        b.push(EntryKind::Worktree, name, detail, None, PaletteTarget::Worktree(w.path.clone()));
    }
}

/// `path` under `root` when it is relative.
fn absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_relative() && !root.as_os_str().is_empty() {
        root.join(path)
    } else {
        path.to_path_buf()
    }
}

/// `path` relative to `root`, with `/`.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// At most `max` chars of `text`, with "…" when cut.
fn shortened(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// "2026-10-01 10:00": unix ms as local time, `offset_secs` ahead of UTC.
pub fn local_time(ms: i64, offset_secs: i64) -> String {
    let secs = ms.div_euclid(1000) + offset_secs;
    let (days, of_day) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}", of_day / 3600, of_day % 3600 / 60)
}

/// File cards by project-relative path. Built when the graph changes, never per frame.
pub fn node_paths(graph: &Graph) -> HashMap<String, NodeId> {
    let mut nodes = HashMap::with_capacity(graph.nodes.len());
    for node in graph.nodes.values() {
        if let Some(path) = node_rel_path(node) {
            nodes.entry(path).or_insert(node.id);
        }
    }
    nodes
}

/// The nearest folder above `path` whose contents are not loaded yet (cluster id).
fn lazy_ancestor(graph: &Graph, path: &str) -> Option<String> {
    let mut dir = path;
    while let Some((parent, _)) = dir.rsplit_once('/') {
        let id = format!("dir:{parent}");
        if graph.clusters.iter().any(|c| c.id == id && c.lazy.is_some()) {
            return Some(id);
        }
        dir = parent;
    }
    None
}

/// Rows shown per group: Files and Symbols 8, the rest 5; 50 for a `>` or `@` query.
fn caps(scope: Scope) -> impl Fn(EntryKind) -> usize {
    move |kind| match (scope, kind) {
        (Scope::All, EntryKind::File | EntryKind::Symbol) => 8,
        (Scope::All, _) => 5,
        _ => 50,
    }
}

/// A group's header, by its lead kind.
fn group_title(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File | EntryKind::Folder => "Files",
        EntryKind::Symbol => "Symbols",
        EntryKind::Command => "Commands",
        EntryKind::Tool => "Tools",
        EntryKind::Setting => "Settings",
        EntryKind::Route => "Routes",
        EntryKind::Flow => "Flows",
        EntryKind::Branch => "Branches",
        EntryKind::Worktree => "Worktrees",
        EntryKind::Ticket => "Tickets",
        EntryKind::Session => "Sessions",
    }
}

/// "16,761".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl StudioApp {
    /// Opens the palette with an empty field, focused.
    pub(crate) fn open_palette(&mut self) {
        self.sync_palette_sources();
        let p = &mut self.palette;
        p.open = true;
        p.text.clear();
        p.searched = None;
        p.counts = None;
        p.highlight = 0;
    }

    /// Closes the palette and clears it.
    pub(crate) fn close_palette(&mut self) {
        let p = &mut self.palette;
        p.open = false;
        p.text.clear();
        p.searched = None;
        p.groups.clear();
        p.highlight = 0;
    }

    /// Reads the palette's keys while it is open, Shift+Enter before Enter (Enter's chord ignores
    /// Shift). Each is taken away from the field.
    pub(crate) fn palette_keys(&mut self, ctx: &egui::Context, focus: KeyFocus) {
        use ShortcutAction::*;
        for action in [PaletteReveal, PaletteOpen, PaletteUp, PaletteDown, PaletteClose] {
            if self.palette.open && ctx.input_mut(|i| consume(i, focus, action)) {
                self.palette_key(action);
            }
        }
    }

    /// Does what a palette key says.
    pub(crate) fn palette_key(&mut self, action: ShortcutAction) {
        self.refresh_palette();
        match action {
            ShortcutAction::PaletteUp => self.palette.step(-1),
            ShortcutAction::PaletteDown => self.palette.step(1),
            ShortcutAction::PaletteOpen => self.palette_pick(false),
            ShortcutAction::PaletteReveal => self.palette_pick(true),
            _ => self.close_palette(),
        }
    }

    /// Searches again when the text or the index changed since the last search; else nothing.
    pub(crate) fn refresh_palette(&mut self) {
        if self.palette.counts.is_none() {
            self.palette.counts = Some(self.search_index.counts());
        }
        if self.palette.searched.as_deref() == Some(self.palette.text.as_str()) {
            return;
        }
        let query = Query::parse(&self.palette.text);
        self.palette.groups = if !query.text.is_empty() {
            let groups = self.search_index.search(&query, caps(query.scope));
            groups
                .into_iter()
                .map(|g| RowGroup {
                    title: group_title(g.kind),
                    total: g.total,
                    rows: g
                        .hits
                        .iter()
                        .map(|h| Row {
                            entry: self.search_index.entry(h).clone(),
                            positions: h.positions.clone(),
                            in_path: h.in_path,
                        })
                        .collect(),
                })
                .collect()
        } else if query.scope == Scope::Commands {
            self.every_command()
        } else if query.scope == Scope::All {
            self.empty_query_rows()
        } else {
            Vec::new()
        };
        self.palette.searched = Some(self.palette.text.clone());
        self.palette.highlight = 0;
        self.palette.scroll = true;
    }

    /// `>` alone: every command and tool, in the registry's order.
    fn every_command(&self) -> Vec<RowGroup> {
        [EntryKind::Command, EntryKind::Tool]
            .into_iter()
            .map(|kind| {
                let all = self.palette.segment.entries().iter().filter(|e| e.kind == kind);
                let rows = all.clone().take(caps(Scope::Commands)(kind)).map(plain_row).collect();
                RowGroup { title: group_title(kind), total: all.count(), rows }
            })
            .filter(|g| g.total > 0)
            .collect()
    }

    /// The empty field: the files open on the Desk in the order they were opened, then the stops.
    fn empty_query_rows(&self) -> Vec<RowGroup> {
        let desk = &self.canvas_state.desk;
        let mut cards: Vec<_> = desk.cards.iter().filter(|c| c.session.is_none()).collect();
        cards.sort_by_key(|c| c.id);
        let files: Vec<Row> = cards
            .iter()
            .map(|c| {
                let name = c.path.file_name().map_or_else(|| c.rel.clone(), |n| n.to_string_lossy().into_owned());
                plain_row(&Entry {
                    kind: EntryKind::File,
                    name,
                    detail: c.rel.clone(),
                    path: Some(c.rel.clone()),
                    line: None,
                    key: 0,
                })
            })
            .collect();
        let stops: Vec<Row> = self
            .palette
            .segment
            .entries()
            .iter()
            .filter(|e| {
                matches!(self.palette.targets.get(e.key as usize), Some(PaletteTarget::Command(AppCommand::Fly(_))))
            })
            .map(plain_row)
            .collect();
        [("Open on the Desk", files), ("Go to", stops)]
            .into_iter()
            .filter(|(_, rows)| !rows.is_empty())
            .map(|(title, rows)| RowGroup { title, total: rows.len(), rows })
            .collect()
    }

    /// Opens the highlighted result, or shows it on the map, and closes the palette.
    pub(crate) fn palette_pick(&mut self, reveal: bool) {
        self.refresh_palette();
        let Some(row) = self.palette.rows().nth(self.palette.highlight).cloned() else { return };
        self.close_palette();
        self.palette_route(&row.entry, reveal);
    }

    /// Goes where `entry` lives (PLAN D4): opens it on the Desk, or shows it on the map.
    pub(crate) fn palette_route(&mut self, entry: &Entry, reveal: bool) {
        let path = entry.path.clone().unwrap_or_default();
        let line = entry.line.map(|l| l as usize);
        match entry.kind {
            EntryKind::File if reveal => self.reveal_file(&path),
            EntryKind::File => self.open_on_desk(&self.project_file(&path), None),
            EntryKind::Folder => self.canvas_state.fly_to(CameraTarget::Folder(format!("dir:{path}"))),
            EntryKind::Symbol if reveal => self.reveal_symbol(&path, entry.key),
            EntryKind::Symbol => self.open_on_desk(&self.project_file(&path), line),
            _ => {
                if let Some(target) = self.palette.targets.get(entry.key as usize).cloned() {
                    self.route_target(target, reveal);
                }
            }
        }
    }

    fn route_target(&mut self, target: PaletteTarget, reveal: bool) {
        match target {
            PaletteTarget::Command(cmd) => {
                run_command(self, cmd);
                // A View toggle shows its state in its detail.
                self.sync_palette_sources();
            }
            PaletteTarget::Tool { top, .. } if reveal => self.reveal_tool(&top),
            PaletteTarget::Tool { file, line, .. } => self.open_on_desk(&file, Some(line)),
            PaletteTarget::Setting { file, line } if !reveal => self.open_on_desk(&file, Some(line)),
            PaletteTarget::Setting { .. } => self.canvas_state.fly_to(CameraTarget::Stop(Stop::Files)),
            PaletteTarget::Step { group, .. } if reveal => self.reveal_pipeline_group(group),
            PaletteTarget::Step { file, line, .. } => self.open_on_desk(&file, Some(line)),
            PaletteTarget::Branch(name) => {
                let target = self.changes_row_target(|r| r.branch == name);
                self.canvas_state.fly_to(target);
            }
            PaletteTarget::Worktree(path) => {
                let target = self.changes_row_target(|r| r.path == path);
                self.canvas_state.fly_to(target);
            }
            PaletteTarget::Ticket(_) if reveal => self.canvas_state.fly_to(CameraTarget::Stop(Stop::Changes)),
            PaletteTarget::Ticket(file) => self.open_on_desk(&file, None),
            PaletteTarget::Session(id) => {
                let target = self.session_target(&id);
                self.canvas_state.action_request = Some(CanvasAction::LightSession(Some(id)));
                self.canvas_state.fly_to(target);
            }
        }
    }

    /// Opens `file` on the Desk at `line` and flies there.
    fn open_on_desk(&mut self, file: &Path, line: Option<usize>) {
        self.canvas_state.desk.open(file, line);
        self.canvas_state.fly_to(CameraTarget::Stop(Stop::Desk));
    }

    /// The file at project-relative `rel`: its card's path, else under the project root.
    fn project_file(&self, rel: &str) -> PathBuf {
        let card = self.palette.nodes.get(rel).and_then(|id| self.graph.nodes.get(id));
        if let Some(path) = card.and_then(|n| n.file_path.as_ref()) {
            return PathBuf::from(path);
        }
        let root = self.canvas_state.desk.root.as_ref().or(self.current_project_path.as_ref());
        root.map_or_else(|| PathBuf::from(rel), |r| r.join(rel))
    }

    /// Selects a card and flies to it.
    fn show_node(&mut self, id: NodeId) {
        self.canvas_state.selected_nodes.clear();
        self.canvas_state.selected_nodes.insert(id);
        self.canvas_state.fly_to(CameraTarget::Node(id));
    }

    /// Flies to a file's card. A file in a folder not loaded yet loads the folder first and is
    /// shown once its card exists (E5).
    fn reveal_file(&mut self, path: &str) {
        if let Some(&id) = self.palette.nodes.get(path) {
            self.show_node(id);
        } else if let Some(cluster) = lazy_ancestor(&self.graph, path) {
            self.canvas_state.action_request = Some(CanvasAction::ExpandFolder(cluster, FolderDetail::Open));
            self.palette.pending_reveal = Some(path.to_string());
        } else {
            self.canvas_state.status_message = Some(format!("{path} is not on the map"));
        }
    }

    /// Flies to a symbol's card and opens the symbol on it.
    fn reveal_symbol(&mut self, path: &str, key: u64) {
        let Some(&id) = self.palette.nodes.get(path) else { return self.reveal_file(path) };
        self.show_node(id);
        let member = self.graph.nodes.get(&id).and_then(|n| n.member_nodes.get(member_index(key)));
        if let Some(member) = member {
            self.canvas_state.action_request = Some(CanvasAction::InspectMember(id, member.id.clone()));
        }
    }

    /// Flies to a tool's Run tile (a child tool's parent tile), or to the tools.
    fn reveal_tool(&mut self, top: &ToolRef) {
        let tiles = self.canvas_state.districts.run.as_ref().map(|r| &r.tools[..]).unwrap_or_default();
        let tile = tiles
            .iter()
            .position(|t| t.name == top.name && t.file == top.file && t.line == top.line)
            .or_else(|| tiles.iter().position(|t| !top.invocation.is_empty() && t.invocation == top.invocation));
        run_command(self, tile.map_or(AppCommand::AllTools, AppCommand::Tool));
    }

    /// Brings a Pipeline group into view at 100%, opening it first when closed (as the
    /// district's own Reveal does).
    fn reveal_pipeline_group(&mut self, group: Option<usize>) {
        let canvas = &mut self.canvas_state;
        let (Some(view), Some(g)) = (canvas.districts.pipeline.clone(), group) else {
            return canvas.fly_to(CameraTarget::Stop(Stop::Pipeline));
        };
        if let Some(shown) = view.groups.get(g).filter(|shown| !is_expanded(shown, &canvas.pipeline_open)) {
            if !canvas.pipeline_open.insert(shown.key.clone()) {
                canvas.pipeline_open.remove(&shown.key);
            }
            canvas.refresh_world();
        }
        let world = canvas.world;
        let layout = canvas.pipeline_layout(WorldLayout::centre_width_local(world.code));
        let target = match layout.and_then(|l| group_world_rect(&world, &l, g)) {
            Some(r) => CameraTarget::Rect(match self.palette.canvas_rect {
                Some(screen) => reveal_rect(&world, r, screen),
                None => r,
            }),
            None => CameraTarget::Stop(Stop::Pipeline),
        };
        self.canvas_state.fly_to(target);
    }

    /// The first Changes row that `pick` accepts, or the district.
    fn changes_row_target(&self, pick: impl Fn(&studio_canvas::WorktreeRowView) -> bool) -> CameraTarget {
        let Some(view) = &self.canvas_state.districts.changes else { return CameraTarget::Stop(Stop::Changes) };
        match view.rows.iter().position(pick) {
            Some(row) => CameraTarget::Rect(row_world_rect(&self.canvas_state.world, view, row)),
            None => CameraTarget::Stop(Stop::Changes),
        }
    }

    /// A session's chip in the Changes district, or the district.
    fn session_target(&self, id: &str) -> CameraTarget {
        let Some(view) = &self.canvas_state.districts.changes else { return CameraTarget::Stop(Stop::Changes) };
        let found = view
            .rows
            .iter()
            .enumerate()
            .find_map(|(r, row)| row.sessions.iter().position(|s| s.id == id).map(|s| (r, s)));
        match found {
            Some((row, s)) => CameraTarget::Rect(session_world_rect(&self.canvas_state.world, view, row, s)),
            None => CameraTarget::Stop(Stop::Changes),
        }
    }

    /// Builds the sources-and-commands segment again when its inputs changed, and puts it in the
    /// index's slot 2 (the loader's index has an empty one).
    pub(crate) fn sync_palette_sources(&mut self) {
        let sources = self.sources.as_ref();
        let ptr = |p: Option<*const u8>| p.map_or(0, |p| p as usize);
        let stamp = SourcesStamp {
            sources: [
                ptr(sources.and_then(|s| s.tools.as_ref()).map(|a| Arc::as_ptr(a).cast())),
                ptr(sources.and_then(|s| s.settings.as_ref()).map(|a| Arc::as_ptr(a).cast())),
                ptr(sources.and_then(|s| s.pipeline.as_ref()).map(|(a, _)| Arc::as_ptr(a).cast())),
                ptr(sources.and_then(|s| s.history.as_ref()).map(|a| Arc::as_ptr(a).cast())),
                ptr(sources.and_then(|s| s.tickets.as_ref()).map(|a| Arc::as_ptr(a).cast())),
                ptr(sources.and_then(|s| s.agents.as_ref()).map(|a| Arc::as_ptr(a).cast())),
            ],
            toggles: VIEW_TOGGLES.iter().map(|t| (t.get)(&self.canvas_state)).collect(),
        };
        if self.palette.stamp.as_ref() != Some(&stamp) {
            let started = Instant::now();
            let now_secs = super::sources::now_unix_ms() / 1000;
            let parts = SourceParts {
                root: sources.map(|s| s.root()),
                tools: sources.and_then(|s| s.tools.as_deref()),
                settings: sources.and_then(|s| s.settings.as_deref()),
                pipeline: sources.and_then(|s| s.pipeline.as_ref()).map(|(p, _)| &**p),
                history: sources.and_then(|s| s.history.as_deref()),
                tickets: sources.and_then(|s| s.tickets.as_deref()),
                agents: sources.and_then(|s| s.agents.as_deref()),
                offset_secs: super::title_bar::local_offset_secs(now_secs),
            };
            let (segment, targets) = sources_segment(&command_list(&self.canvas_state), &parts);
            crate::timing::event(|| segment_timing(&segment, started));
            self.palette.segment = Arc::new(segment);
            self.palette.targets = targets;
            self.palette.stamp = Some(stamp);
            self.palette.searched = None;
            self.palette.counts = None;
        }
        self.search_index.replace(SOURCES_SLOT, self.palette.segment.clone());
    }

    /// The graph was replaced, and with it the index: rebuilds the path map and puts slot 2 back.
    pub(crate) fn palette_graph_replaced(&mut self) {
        self.palette.nodes = node_paths(&self.graph);
        self.palette.pending_reveal = None;
        self.palette.searched = None;
        self.palette.counts = None;
        self.sync_palette_sources();
    }

    /// A folder finished loading into the graph: rebuilds the path map and shows the file waiting
    /// for it, loading a folder further down first when the file is in one.
    pub(crate) fn palette_folder_loaded(&mut self, cluster_id: &str) {
        self.palette.nodes = node_paths(&self.graph);
        self.palette.searched = None;
        self.palette.counts = None;
        let Some(path) = self.palette.pending_reveal.take() else { return };
        if let Some(&id) = self.palette.nodes.get(&path) {
            self.show_node(id);
        } else if let Some(cluster) = lazy_ancestor(&self.graph, &path).filter(|c| c != cluster_id) {
            self.canvas_state.action_request = Some(CanvasAction::ExpandFolder(cluster, FolderDetail::Open));
            self.palette.pending_reveal = Some(path);
        }
    }

    /// A folder failed to load: the file waiting for it is not shown.
    pub(crate) fn palette_folder_failed(&mut self, cluster_id: &str) {
        let prefix = format!("{}/", cluster_id.strip_prefix("dir:").unwrap_or(cluster_id));
        if self.palette.pending_reveal.as_ref().is_some_and(|p| p.starts_with(&prefix)) {
            self.palette.pending_reveal = None;
        }
    }

    /// The dropdown, while the palette is open: under the title bar's field, inside the window.
    pub(crate) fn render_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        self.refresh_palette();
        let Some(field) = self.palette.field_rect else { return };
        let screen = ctx.content_rect();
        let width = WIDTH.min(screen.width() - 16.0).max(200.0);
        let mut left = field.left();
        if left + width > screen.right() - 8.0 {
            left = field.right() - width;
        }
        let left = left.max(screen.left() + 8.0);
        let max_height = screen.height() * MAX_HEIGHT_SHARE;
        let mut picked = None;
        let area = egui::Area::new(Id::new("palette_dropdown"))
            .order(Order::Foreground)
            .fixed_pos(pos2(left, field.bottom() + 6.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(FLOATING_TOOLBAR_BG)
                    .stroke(Stroke::new(1.0, FLOATING_TOOLBAR_BORDER))
                    .corner_radius(CornerRadius::from(10.0))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        let inner = width - 16.0;
                        ui.set_width(inner);
                        self.dropdown_header(ui);
                        egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
                            picked = self.dropdown_rows(ui, inner);
                        });
                    });
            });
        if let Some(reveal) = picked {
            self.palette_pick(reveal);
            return;
        }
        // A press outside the field and the dropdown closes it.
        let pressed_at = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
        if pressed_at.is_some_and(|p| !area.response.rect.contains(p) && !field.contains(p)) {
            self.close_palette();
        }
    }

    /// The counts in the corner, and what the keys do.
    fn dropdown_header(&self, ui: &mut egui::Ui) {
        let counts = self.palette.counts.unwrap_or_default();
        let files = counts[EntryKind::File as usize];
        let symbols = counts[EntryKind::Symbol as usize];
        let commands = counts[EntryKind::Command as usize] + counts[EntryKind::Tool as usize];
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Enter opens · Shift+Enter shows on the map").size(11.0).color(TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = format!(
                    "{} files · {} symbols · {} commands",
                    thousands(files),
                    thousands(symbols),
                    thousands(commands)
                );
                ui.label(egui::RichText::new(text).size(11.0).color(TEXT_DIM));
            });
        });
        ui.add_space(2.0);
    }

    /// The groups and their rows. Returns `Some(reveal)` when a row was clicked (Shift: reveal).
    fn dropdown_rows(&mut self, ui: &mut egui::Ui, width: f32) -> Option<bool> {
        if self.palette.groups.is_empty() {
            ui.add_space(6.0);
            ui.label(egui::RichText::new("No matches").size(12.5).color(TEXT_DIM));
            ui.add_space(6.0);
            return None;
        }
        let moving = ui.input(|i| i.pointer.is_moving());
        let shift = ui.input(|i| i.modifiers.shift);
        let mut picked = None;
        let mut index = 0;
        let scroll = std::mem::take(&mut self.palette.scroll);
        let groups = std::mem::take(&mut self.palette.groups);
        for group in &groups {
            ui.add_space(4.0);
            let header = format!("{} · {}", group.title.to_uppercase(), thousands(group.total));
            ui.label(egui::RichText::new(header).size(10.0).color(TEXT_DIM));
            for row in &group.rows {
                let highlighted = index == self.palette.highlight;
                let response = paint_row(ui, row, width, highlighted);
                if highlighted && scroll {
                    response.scroll_to_me(None);
                }
                if response.hovered() && moving {
                    self.palette.highlight = index;
                }
                if response.clicked() {
                    self.palette.highlight = index;
                    picked = Some(shift);
                }
                index += 1;
            }
        }
        self.palette.groups = groups;
        picked
    }
}

/// A result with nothing matched.
fn plain_row(entry: &Entry) -> Row {
    Row { entry: entry.clone(), positions: Vec::new(), in_path: false }
}

/// The timing line for slot 2: its size per kind and how long it took.
fn segment_timing(segment: &Segment, started: Instant) -> String {
    let mut per_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for e in segment.entries() {
        *per_kind.entry(e.kind.label()).or_default() += 1;
    }
    let kinds: Vec<String> = per_kind.iter().map(|(k, n)| format!("{k} {n}")).collect();
    format!(
        "palette sources segment: {} entries in {:.2} ms ({})",
        segment.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        kinds.join(", ")
    )
}

/// A kind's icon and chip colour.
fn kind_chip(kind: EntryKind) -> (&'static str, Color32) {
    use egui_phosphor::regular as icon;
    match kind {
        EntryKind::File => (icon::FILE, DISTRICT_CODE),
        EntryKind::Folder => (icon::FOLDER, DISTRICT_FILES),
        EntryKind::Symbol => (icon::BRACKETS_CURLY, ARCHETYPE_COMPUTE),
        EntryKind::Command => (icon::LIGHTNING, DISTRICT_DESK),
        EntryKind::Tool => (icon::WRENCH, DISTRICT_RUN),
        EntryKind::Setting => (icon::GEAR, DISTRICT_FILES),
        EntryKind::Route => (icon::SIGNPOST, DISTRICT_PIPELINE),
        EntryKind::Flow => (icon::FLOW_ARROW, DISTRICT_PIPELINE),
        EntryKind::Branch => (icon::GIT_BRANCH, DISTRICT_CHANGES),
        EntryKind::Worktree => (icon::TREE_STRUCTURE, DISTRICT_CHANGES),
        EntryKind::Ticket => (icon::TICKET, DISTRICT_CHANGES),
        EntryKind::Session => (icon::ROBOT, DISTRICT_CHANGES),
    }
}

/// `text` with the chars at byte offsets `positions` stressed.
fn stressed(text: &str, positions: &[u32], size: f32, plain: Color32, stress: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut start = 0;
    let mut stressed_run = false;
    for (at, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        let is = at < text.len() && positions.binary_search(&(at as u32)).is_ok();
        if at == text.len() || is != stressed_run {
            if at > start {
                let color = if stressed_run { stress } else { plain };
                job.append(&text[start..at], 0.0, TextFormat::simple(FontId::proportional(size), color));
            }
            start = at;
            stressed_run = is;
        }
    }
    job
}

/// One result row: the kind chip, the name with its matched chars bold, the detail dim, and the
/// kind on the right.
fn paint_row(ui: &mut egui::Ui, row: &Row, width: f32, highlighted: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    if highlighted {
        painter.rect(rect, CornerRadius::from(6.0), FLOATING_BTN_HOVER, Stroke::NONE, StrokeKind::Inside);
    }
    let (icon, color) = kind_chip(row.entry.kind);
    let chip = Rect::from_center_size(rect.left_center() + vec2(16.0, 0.0), vec2(20.0, 20.0));
    painter.rect(chip, CornerRadius::from(5.0), color.gamma_multiply(0.18), Stroke::NONE, StrokeKind::Inside);
    painter.text(chip.center(), Align2::CENTER_CENTER, icon, FontId::proportional(12.0), color);

    let kind = painter.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        row.entry.kind.label(),
        FontId::proportional(10.5),
        TEXT_DIM,
    );
    let (name_bold, detail_bold) =
        if row.in_path { (&[][..], &row.positions[..]) } else { (&row.positions[..], &[][..]) };
    let name_pos = pos2(chip.right() + 10.0, rect.center().y);
    let name_end = bold_text(&painter, name_pos, &row.entry.name, name_bold, 13.0, TEXT_PRIMARY);
    let detail_pos = pos2(name_end + 10.0, rect.center().y);
    if detail_pos.x < kind.left() - 24.0 {
        let clip = Rect::from_min_max(pos2(detail_pos.x, rect.top()), pos2(kind.left() - 10.0, rect.bottom()));
        bold_text(&painter.with_clip_rect(clip), detail_pos, &row.entry.detail, detail_bold, 11.5, TEXT_DIM);
    }
    response
}

/// Paints `text` left-centred at `pos` with the chars at `bold` bold (drawn twice, a hair apart,
/// in the brightest text colour). Returns where it ends.
fn bold_text(painter: &egui::Painter, pos: egui::Pos2, text: &str, bold: &[u32], size: f32, color: Color32) -> f32 {
    let galley = painter.layout_job(stressed(text, bold, size, color, TEXT_HIGHLIGHT));
    let top_left = pos - vec2(0.0, galley.size().y / 2.0);
    let end = top_left.x + galley.size().x;
    painter.galley(top_left, galley, color);
    if !bold.is_empty() {
        let again = painter.layout_job(stressed(text, bold, size, Color32::TRANSPARENT, TEXT_HIGHLIGHT));
        painter.galley(top_left + vec2(0.6, 0.0), again, color);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers, Pos2, Vec2};
    use std::time::Duration;
    use studio_canvas::CanvasView;
    use studio_parser::{file_segment, symbol_segment, FileListing, SearchIndex};
    use studio_sources::{Branch, Fact, FactSheet, Flow, Link, Session, Step, Ticket, ToolKind};

    /// A small project on disk: a Rust file with two functions, and a heavy folder (node_modules)
    /// whose contents load on demand.
    struct Project {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    fn project() -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        write("src/lib.rs", "pub fn alpha() {}\n\npub fn objective_hud() -> u32 {\n    1\n}\n");
        write("node_modules/pkg/index.js", "module.exports = 1;\n");
        Project { _dir: dir, root }
    }

    /// An app with `project` loaded, its index as the loader sends it.
    fn app_with(project: &Project) -> StudioApp {
        let (graph, _) = studio_parser::load_rust_project(&project.root).unwrap();
        let mut app = StudioApp::for_test();
        let files = FileListing::from_graph(&graph, &[]);
        app.search_index = SearchIndex::loaded(Arc::new(file_segment(&files)), Arc::new(symbol_segment(&graph)));
        app.graph = graph;
        app.current_project_path = Some(project.root.clone());
        app.canvas_state.desk.root = Some(project.root.clone());
        app.palette_graph_replaced();
        app
    }

    /// Searches `text` in the open palette.
    fn search(app: &mut StudioApp, text: &str) {
        app.palette.open = true;
        app.palette.text = text.to_string();
        app.refresh_palette();
    }

    /// Highlights the first row `pick` accepts.
    fn highlight(app: &mut StudioApp, pick: impl Fn(&Entry) -> bool) {
        let at = app.palette.rows().position(|r| pick(&r.entry)).expect("a matching row");
        app.palette.highlight = at;
    }

    fn file_entry(path: &str) -> Entry {
        Entry {
            kind: EntryKind::File,
            name: String::new(),
            detail: path.into(),
            path: Some(path.into()),
            line: None,
            key: 0,
        }
    }

    /// E5: Shift+Enter on a file in a folder not loaded yet loads the folder, then flies to the
    /// file's card once it exists.
    #[test]
    fn a_file_in_an_unloaded_folder_loads_it_then_flies_to_its_card() {
        let p = project();
        let mut app = app_with(&p);
        let path = "node_modules/pkg/index.js";
        assert!(!app.palette.nodes.contains_key(path), "not loaded yet");
        app.palette_route(&file_entry(path), true);
        assert_eq!(
            app.canvas_state.action_request,
            Some(CanvasAction::ExpandFolder("dir:node_modules".into(), FolderDetail::Open))
        );
        assert_eq!(app.palette.pending_reveal.as_deref(), Some(path));
        assert_eq!(app.canvas_state.camera.pending, None, "no flight before the card exists");

        // The app starts the load (app/mod.rs), and the folder's contents arrive.
        let Some(CanvasAction::ExpandFolder(id, detail)) = app.canvas_state.action_request.take() else {
            unreachable!()
        };
        app.start_folder_load(id, detail);
        let deadline = Instant::now() + Duration::from_secs(30);
        while !app.folder_loads.is_empty() {
            assert!(Instant::now() < deadline, "the folder loads");
            std::thread::sleep(Duration::from_millis(2));
            app.poll_folder_loads();
        }
        let node = app.palette.nodes[path];
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Node(node), true)), "one flight");
        assert!(app.canvas_state.selected_nodes.contains(&node));
        assert_eq!(app.palette.pending_reveal, None);

        // A load that fails drops the file waiting for it.
        app.palette.pending_reveal = Some("vendor/x.js".into());
        app.palette_folder_failed("dir:vendor");
        assert_eq!(app.palette.pending_reveal, None);
    }

    #[test]
    fn enter_opens_a_file_on_the_desk_and_a_symbol_at_its_line() {
        let p = project();
        let mut app = app_with(&p);
        search(&mut app, "lib.rs");
        highlight(&mut app, |e| e.kind == EntryKind::File && e.path.as_deref() == Some("src/lib.rs"));
        app.palette_pick(false);
        assert!(!app.palette.open, "a pick closes the palette");
        let card = &app.canvas_state.desk.cards[0];
        assert_eq!(card.path, p.root.join("src/lib.rs"));
        assert_eq!(card.scroll_to, None);
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Stop(Stop::Desk), true)));

        search(&mut app, "@objective");
        assert!(app.palette.rows().all(|r| r.entry.kind == EntryKind::Symbol), "@ finds symbols only");
        assert_eq!(app.palette.rows().next().unwrap().entry.name, "objective_hud");
        app.palette_pick(false);
        assert_eq!(app.canvas_state.desk.cards.len(), 1, "the same card");
        assert_eq!(app.canvas_state.desk.cards[0].scroll_to, Some(3));
    }

    #[test]
    fn shift_enter_on_a_symbol_reveals_its_card_and_opens_the_symbol() {
        let p = project();
        let mut app = app_with(&p);
        search(&mut app, "objective_hud");
        highlight(&mut app, |e| e.kind == EntryKind::Symbol);
        app.palette_pick(true);
        let node = app.palette.nodes["src/lib.rs"];
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Node(node), true)));
        assert!(app.canvas_state.selected_nodes.contains(&node));
        let member = app.graph.nodes[&node].member_nodes.iter().find(|m| m.name == "objective_hud").unwrap();
        assert_eq!(app.canvas_state.action_request, Some(CanvasAction::InspectMember(node, member.id.clone())));
        assert!(app.canvas_state.desk.cards.is_empty(), "nothing opened on the Desk");
    }

    fn tool(kind: ToolKind, name: &str, file: &Path, children: Vec<Tool>) -> Tool {
        Tool {
            kind,
            name: name.into(),
            invocation: format!("cargo run --bin {name}"),
            description: None,
            file: file.to_path_buf(),
            line: 4,
            children,
        }
    }

    /// Puts a sources-and-commands segment built from `parts` in slot 2.
    fn install(app: &mut StudioApp, parts: &SourceParts) {
        let (segment, targets) = sources_segment(&command_list(&app.canvas_state), parts);
        app.palette.segment = Arc::new(segment);
        app.palette.targets = targets;
        app.search_index.replace(SOURCES_SLOT, app.palette.segment.clone());
        app.palette.searched = None;
    }

    #[test]
    fn a_greater_than_sign_limits_the_search_to_commands_and_tools() {
        let p = project();
        let mut app = app_with(&p);
        let main = p.root.join("src/main.rs");
        let sub = tool(ToolKind::Subcommand, "files", &main, Vec::new());
        let tools = Tools { tools: vec![tool(ToolKind::Binary, "files_report", &main, vec![sub])] };
        install(&mut app, &SourceParts { root: Some(&p.root), tools: Some(&tools), ..Default::default() });

        search(&mut app, "lib");
        assert!(app.palette.rows().any(|r| r.entry.kind == EntryKind::File), "without >, files too");
        search(&mut app, ">lib");
        assert!(app.palette.rows().all(|r| matches!(r.entry.kind, EntryKind::Command | EntryKind::Tool)));

        search(&mut app, "> files");
        let kinds: Vec<EntryKind> = app.palette.groups.iter().flat_map(|g| &g.rows).map(|r| r.entry.kind).collect();
        assert!(kinds.contains(&EntryKind::Tool), "{kinds:?}");
        assert!(kinds.iter().all(|k| matches!(k, EntryKind::Command | EntryKind::Tool)), "{kinds:?}");
        assert_eq!(app.palette.rows().next().unwrap().entry.name, "Go to Files");
        app.palette_pick(false);
        assert_eq!(app.canvas_state.camera.pending, Some((CameraTarget::Stop(Stop::Files), true)));

        search(&mut app, ">shortcuts");
        app.palette_pick(false);
        assert!(app.shortcut_sheet_open, "> shortcuts opens the sheet");

        search(&mut app, ">");
        assert_eq!(app.palette.groups.iter().map(|g| g.title).collect::<Vec<_>>(), ["Commands", "Tools"]);
        assert_eq!(app.palette.groups[1].total, 2, "a tool and its subcommand");
    }

    /// Runs one frame of the app's shortcuts with `events`; returns whether Escape was left over.
    fn shortcut_frame(app: &mut StudioApp, ctx: &egui::Context, events: Vec<egui::Event>) -> bool {
        let mut left = false;
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            app.handle_shortcuts(ui.ctx());
            left = ui.input(|i| i.key_pressed(Key::Escape));
        });
        output.textures_delta.clear();
        left
    }

    fn key(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    #[test]
    fn escape_closes_the_palette_then_the_sheet() {
        let ctx = egui::Context::default();
        let mut app = StudioApp::for_test();
        app.shortcut_sheet_open = true;
        app.open_palette();
        app.palette.text = "files".into();
        let left = shortcut_frame(&mut app, &ctx, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!app.palette.open);
        assert!(app.palette.text.is_empty(), "closing clears it");
        assert!(app.shortcut_sheet_open, "one Escape, one thing closed");
        assert!(!left, "the canvas never saw it");
        shortcut_frame(&mut app, &ctx, vec![key(Key::Escape, Modifiers::NONE)]);
        assert!(!app.shortcut_sheet_open);
    }

    #[test]
    fn the_same_text_gives_the_same_hits() {
        let p = project();
        let mut app = app_with(&p);
        search(&mut app, "lib");
        let first = app.palette.groups.clone();
        assert!(!first.is_empty());
        app.palette.searched = None;
        app.refresh_palette();
        assert_eq!(app.palette.groups, first, "searched again");
        let mut again = app_with(&p);
        search(&mut again, "lib");
        assert_eq!(again.palette.groups, first, "after a rebuild");
    }

    /// Copies the agents fixture (whose first session has a custom title) with its placeholders
    /// filled.
    fn copy_fixture(from: &Path, to: &Path, root: &Path) {
        std::fs::create_dir_all(to).unwrap();
        let mut entries: Vec<_> = std::fs::read_dir(from).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let target = to.join(path.file_name().unwrap());
            if path.is_dir() {
                copy_fixture(&path, &target, root);
            } else {
                let text = std::fs::read_to_string(&path).unwrap();
                let base = root.parent().unwrap();
                let text = text
                    .replace("{ROOT}", root.to_str().unwrap())
                    .replace("{LINK}", base.join("link").to_str().unwrap())
                    .replace("{OTHER}", base.join("other").to_str().unwrap())
                    .replace("{PARENT}", base.to_str().unwrap())
                    .replace("{CLAUDE}", to.to_str().unwrap());
                std::fs::write(target, text).unwrap();
            }
        }
    }

    /// Privacy: a session is its slug (or the first 8 chars of its id) and its start time, never
    /// its custom title or prompt text.
    #[test]
    fn a_session_shows_only_its_slug_or_id_prefix_and_its_start_time() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("repo");
        for sub in ["src", "sub", "wt", "docs"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../studio_sources/tests/fixtures/agents/claude");
        let claude = base.join("claude");
        copy_fixture(&fixture, &claude, &root);
        let fixture_text = std::fs::read_to_string(claude.join("projects/-repo-a/s1.jsonl")).unwrap();
        assert!(fixture_text.contains("\"customTitle\":\"SECRET"), "the fixture has a custom title");

        let mut agents = studio_sources::index_sessions(&claude, &[root.clone(), root.join("wt")], None, None).unwrap();
        assert!(agents.sessions.iter().any(|s| s.slug.is_some()));
        let mut unnamed = agents.sessions[0].clone();
        unnamed.id = "0123456789abcdef".into();
        unnamed.slug = None;
        agents.sessions.push(unnamed);

        let parts = SourceParts { agents: Some(&agents), offset_secs: 3600, ..Default::default() };
        let (segment, targets) = sources_segment(&[], &parts);
        assert_eq!(segment.len(), agents.sessions.len());
        for (entry, session) in segment.entries().iter().zip(&agents.sessions) {
            let id_prefix: String = session.id.chars().take(8).collect();
            assert_eq!(entry.name, session.slug.clone().unwrap_or(id_prefix));
            assert_eq!(entry.detail, local_time(session.started, 3600));
            assert!(!entry.name.contains("SECRET") && !entry.detail.contains("SECRET"));
            assert_eq!(targets[entry.key as usize], PaletteTarget::Session(session.id.clone()));
        }
        assert_eq!(segment.entries().last().unwrap().name, "01234567");
    }

    #[test]
    fn local_time_reads_as_a_date_and_a_clock() {
        assert_eq!(local_time(0, 0), "1970-01-01 00:00");
        assert_eq!(local_time(1_790_848_801_000, 0), "2026-10-01 10:00");
        assert_eq!(local_time(1_790_848_801_000, 7_200), "2026-10-01 12:00");
        assert_eq!(local_time(1_790_848_801_000, -36_000), "2026-10-01 00:00");
        assert_eq!(local_time(951_782_400_000, 0), "2000-02-29 00:00", "a leap day");
    }

    fn session(id: &str) -> Session {
        Session {
            id: id.into(),
            slug: Some("brave-blue-fox".into()),
            root: 0,
            branches: Vec::new(),
            started: 0,
            ended: 0,
            logs: Vec::new(),
            plan: None,
            tasks: Vec::new(),
            touch_count: 0,
        }
    }

    fn step(kind: StepKind, title: &str, path: &str, line: usize) -> Step {
        Step {
            kind,
            title: title.into(),
            detail: String::new(),
            language: "Rust".into(),
            path: path.into(),
            line,
            line_end: None,
        }
    }

    /// The first entry of `kind` named `name`.
    fn entry_named(app: &StudioApp, kind: EntryKind, name: &str) -> Entry {
        app.palette.segment.entries().iter().find(|e| e.kind == kind && e.name == name).unwrap().clone()
    }

    /// D4 for the sources: where Enter and Shift+Enter go.
    #[test]
    fn source_results_go_where_d4_says() {
        let root = PathBuf::from("/p");
        let settings = Settings {
            sheets: vec![FactSheet {
                title: "Cargo".into(),
                file: root.join("Cargo.toml"),
                summary: String::new(),
                sections: vec![(
                    "workspace".into(),
                    vec![Fact { key: "edition".into(), value: "2021".into(), line: 7 }],
                )],
            }],
            ..Default::default()
        };
        let pipeline = Pipeline {
            steps: vec![
                step(StepKind::Route, "GET /api/objectives", "src/routes.rs", 10),
                step(StepKind::Handler, "list_objectives", "src/handlers.rs", 42),
            ],
            links: vec![Link {
                from: 0,
                to: 1,
                kind: LinkKind::Serves,
                provenance: studio_graph::Provenance::proven(studio_graph::Basis::NameMatch),
                conditional: None,
                crossing: None,
                label: String::new(),
                evidence: Vec::new(),
            }],
            flows: vec![Flow {
                name: "objectives".into(),
                entry: 0,
                group: 0,
                layers: vec![vec![0], vec![1]],
                links: vec![0],
                truncated: 0,
                crosses_languages: false,
            }],
            ..Default::default()
        };
        let history = GitHistory {
            branches: vec![Branch {
                name: "v4-s8".into(),
                head: "abc".into(),
                upstream: None,
                ahead: 0,
                behind: 0,
                time: 0,
                subject: "palette".into(),
            }],
            ..Default::default()
        };
        let tickets = TicketIndex {
            tickets: vec![Ticket {
                id: "T-12".into(),
                title: "Objective HUD".into(),
                status: "ready".into(),
                kind: None,
                priority: None,
                order: None,
                parent: None,
                shipped_at: None,
                shipped: studio_sources::Shipped::None,
                spec: None,
                plan: None,
                file: ".ai/tickets/T-12.toml".into(),
            }],
            status_counts: Default::default(),
            files: 1,
            bad_files: Vec::new(),
            folder: root.join(".ai/tickets"),
        };
        let agents = AgentIndex { sessions: vec![session("s1-full-id")], ..Default::default() };
        let tools = Tools { tools: vec![tool(ToolKind::Binary, "report", Path::new("src/main.rs"), Vec::new())] };
        let parts = SourceParts {
            root: Some(&root),
            tools: Some(&tools),
            settings: Some(&settings),
            pipeline: Some(&pipeline),
            history: Some(&history),
            tickets: Some(&tickets),
            agents: Some(&agents),
            offset_secs: 0,
        };
        let mut app = StudioApp::for_test();
        install(&mut app, &parts);
        let desk_at = |app: &StudioApp| {
            let card = app.canvas_state.desk.cards.last().unwrap();
            (card.path.clone(), card.scroll_to)
        };
        let went = |app: &mut StudioApp, kind, name: &str, reveal| {
            app.canvas_state.camera.pending = None;
            let entry = entry_named(app, kind, name);
            app.palette_route(&entry, reveal);
            app.canvas_state.camera.pending.clone().map(|(t, _)| t)
        };
        let stop = |s| Some(CameraTarget::Stop(s));

        // A setting opens at its line, or shows the Files district.
        let fact = entry_named(&app, EntryKind::Setting, "edition = 2021");
        assert_eq!(fact.detail, "Cargo.toml:7");
        assert_eq!(went(&mut app, EntryKind::Setting, "edition = 2021", false), stop(Stop::Desk));
        assert_eq!(desk_at(&app), (root.join("Cargo.toml"), Some(7)));
        assert_eq!(went(&mut app, EntryKind::Setting, "edition = 2021", true), stop(Stop::Files));

        // A route opens at its handler; a flow at its entry.
        went(&mut app, EntryKind::Route, "GET /api/objectives", false);
        assert_eq!(desk_at(&app), (root.join("src/handlers.rs"), Some(42)));
        went(&mut app, EntryKind::Flow, "objectives", false);
        assert_eq!(desk_at(&app), (root.join("src/routes.rs"), Some(10)));
        assert_eq!(went(&mut app, EntryKind::Route, "GET /api/objectives", true), stop(Stop::Pipeline), "no view yet");

        // A tool opens at its definition; with no Run district yet, Shift+Enter flies there.
        went(&mut app, EntryKind::Tool, "report", false);
        assert_eq!(desk_at(&app), (root.join("src/main.rs"), Some(4)));
        assert_eq!(went(&mut app, EntryKind::Tool, "report", true), stop(Stop::Run));

        // Branches, tickets and sessions go to the Changes district.
        assert_eq!(went(&mut app, EntryKind::Branch, "v4-s8", false), stop(Stop::Changes));
        went(&mut app, EntryKind::Ticket, "T-12 Objective HUD", false);
        assert_eq!(desk_at(&app), (root.join(".ai/tickets/T-12.toml"), None));
        assert_eq!(went(&mut app, EntryKind::Ticket, "T-12 Objective HUD", true), stop(Stop::Changes));
        assert_eq!(went(&mut app, EntryKind::Session, "brave-blue-fox", false), stop(Stop::Changes));
        assert_eq!(app.canvas_state.action_request, Some(CanvasAction::LightSession(Some("s1-full-id".into()))));
    }

    /// Runs frames the way the app draws them: keys, the title bar, the canvas, the dropdown.
    fn frames(app: &mut StudioApp, ctx: &egui::Context, time: &mut f64, events: Vec<egui::Event>, idle: usize) {
        for events in std::iter::once(events).chain((0..idle).map(|_| Vec::new())) {
            *time += 1.0 / 60.0;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                time: Some(*time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                app.handle_shortcuts(ui.ctx());
                app.render_title_bar(ui);
                CanvasView::new(&mut app.canvas_state, &mut app.graph).show(ui);
                app.render_palette(ui.ctx());
            });
            output.textures_delta.clear();
        }
    }

    fn command_k() -> Vec<egui::Event> {
        vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), key(Key::K, Modifiers::COMMAND)]
    }

    fn typed(k: Key, text: &str) -> Vec<egui::Event> {
        vec![key(k, Modifiers::NONE), egui::Event::Text(text.into())]
    }

    #[test]
    fn keys_typed_into_the_field_never_move_the_map() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut app = StudioApp::for_test();
        app.canvas_state.use_gpu_wires = false;
        frames(&mut app, &ctx, &mut time, Vec::new(), 2);
        frames(&mut app, &ctx, &mut time, command_k(), 2);
        assert!(app.palette.open, "⌘K opens it");
        let view = |app: &StudioApp| {
            let t = app.canvas_state.transform;
            (t.pan, t.zoom, app.canvas_state.camera.pending.clone())
        };
        let before = view(&app);
        frames(&mut app, &ctx, &mut time, typed(Key::Minus, "-"), 1);
        frames(&mut app, &ctx, &mut time, typed(Key::Num0, "0"), 1);
        frames(&mut app, &ctx, &mut time, vec![key(Key::Home, Modifiers::NONE)], 1);
        frames(&mut app, &ctx, &mut time, vec![key(Key::Equals, Modifiers::NONE)], 30);
        assert_eq!(app.palette.text, "-0");
        assert_eq!(view(&app), before);
        assert!(app.palette.open);

        // `/` opens the field without typing itself into it.
        frames(&mut app, &ctx, &mut time, vec![key(Key::Escape, Modifiers::NONE)], 2);
        assert!(!app.palette.open);
        frames(&mut app, &ctx, &mut time, typed(Key::Slash, "/"), 2);
        assert!(app.palette.open, "/ opens it");
        assert_eq!(app.palette.text, "");
    }

    #[test]
    fn the_wheel_over_the_dropdown_never_zooms_the_map() {
        let ctx = egui::Context::default();
        let mut time = 0.0;
        let mut app = StudioApp::for_test();
        app.canvas_state.use_gpu_wires = false;
        frames(&mut app, &ctx, &mut time, command_k(), 2);
        frames(&mut app, &ctx, &mut time, vec![egui::Event::Text(">".into())], 2);
        assert!(app.palette.row_count() > 10, "a long list");
        let field = app.palette.field_rect.unwrap();
        let over = Pos2::new(field.right() - 100.0, field.bottom() + 120.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        frames(&mut app, &ctx, &mut time, vec![egui::Event::PointerMoved(over)], 3);
        let zoom = app.canvas_state.transform.zoom;
        frames(&mut app, &ctx, &mut time, vec![wheel.clone()], 30);
        assert_eq!(app.canvas_state.transform.zoom, zoom, "the dropdown took the wheel");
        assert!(app.palette.open);

        frames(&mut app, &ctx, &mut time, vec![key(Key::Escape, Modifiers::NONE)], 2);
        frames(&mut app, &ctx, &mut time, vec![egui::Event::PointerMoved(over + Vec2::X)], 2);
        frames(&mut app, &ctx, &mut time, vec![wheel], 30);
        assert_ne!(app.canvas_state.transform.zoom, zoom, "with the dropdown gone, the map zooms");
    }

    #[test]
    fn the_empty_field_lists_the_desk_then_the_stops_and_arrows_wrap() {
        let mut app = StudioApp::for_test();
        app.canvas_state.desk.open(Path::new("/p/src/b.rs"), None);
        app.canvas_state.desk.open(Path::new("/p/src/a.rs"), None);
        app.canvas_state.desk.open(Path::new("/p/src/b.rs"), Some(3));
        app.open_palette();
        app.refresh_palette();
        let titles: Vec<&str> = app.palette.groups.iter().map(|g| g.title).collect();
        assert_eq!(titles, ["Open on the Desk", "Go to"]);
        let names: Vec<&str> = app.palette.groups[0].rows.iter().map(|r| r.entry.name.as_str()).collect();
        assert_eq!(names, ["b.rs", "a.rs"], "in the order they were opened");
        assert_eq!(app.palette.groups[1].rows.len(), 7, "the seven stops");
        assert_eq!(app.palette.groups[1].rows[0].entry.name, "Go to the world view");
        let stops = app.palette.row_count();
        app.palette_key(ShortcutAction::PaletteUp);
        assert_eq!(app.palette.highlight, stops - 1, "up from the top wraps");
        app.palette_key(ShortcutAction::PaletteDown);
        assert_eq!(app.palette.highlight, 0, "down from the bottom wraps");
    }

    #[test]
    fn counts_read_with_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(16_761), "16,761");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(shortened("abcdef", 3), "abc…");
        assert_eq!(shortened("abc", 3), "abc");
    }
}
