//! Turns what the sources read into what the districts show. Run when a source reports, never
//! per frame.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use studio_canvas::districts::files::ClassView;
use studio_canvas::{
    BranchView, ChangesView, CommitBead, FileChangeView, FilesView, IgnoredView, PackageBrick, PipelineGroup,
    PipelineLane, PipelineLink, PipelineStep, PipelineView, RunView, SessionChip, SettingRow, SettingsCardView,
    ShippedView, StepSlot, Stop, TicketLinkView, TicketView, ToolTile, WorkflowRow, WorktreeRowView,
};
use studio_graph::{human_bytes, EvidenceTier, Graph};
use studio_sources::tickets::TICKETS_FOLDER;
use studio_sources::{
    next_tickets, ticket_ids_in, AgentIndex, Commit, FileChange, GitDisk, GitHistory, Packages, Pipeline, Session,
    Settings, Shipped, SourceKind, SourceState, TargetKind, Ticket, TicketIndex, Tool, ToolKind, Tools, Worktree,
};

/// The order tools are shown and pinned in: by kind, then by name.
fn kind_rank(kind: ToolKind) -> usize {
    match kind {
        ToolKind::Binary => 0,
        ToolKind::CargoAlias => 1,
        ToolKind::NpmScript => 2,
        ToolKind::MakeTarget => 3,
        ToolKind::JustRecipe => 4,
        ToolKind::Example => 5,
        ToolKind::Subcommand => 6,
        ToolKind::Workflow | ToolKind::WorkflowJob => 7,
    }
}

/// The icon for a kind of tool.
pub fn tool_icon(kind: ToolKind) -> &'static str {
    use egui_phosphor::regular as icon;
    match kind {
        ToolKind::Binary => icon::TERMINAL_WINDOW,
        ToolKind::Example => icon::CODE,
        ToolKind::CargoAlias => icon::LINK,
        ToolKind::Subcommand => icon::TERMINAL,
        ToolKind::NpmScript => icon::PACKAGE,
        ToolKind::MakeTarget => icon::WRENCH,
        ToolKind::JustRecipe => icon::LIGHTNING,
        ToolKind::Workflow | ToolKind::WorkflowJob => icon::GIT_BRANCH,
    }
}

fn tile(tool: &Tool) -> ToolTile {
    ToolTile {
        name: tool.name.clone(),
        kind: tool.kind.label(),
        icon: tool_icon(tool.kind),
        invocation: tool.invocation.clone(),
        description: tool.description.clone(),
        file: tool.file.clone(),
        line: tool.line,
        children: tool.children.iter().map(tile).collect(),
    }
}

/// The Run district's view: the tools (an alias that only runs a binary is shown once, as the
/// binary), CI workflows, tests by package and the package wall. Without packages (a project
/// with no Cargo), the wall says why, in the packages source's words.
pub fn run_view(
    tools: &Tools,
    packages: Option<&Packages>,
    graph: &Graph,
    status: &BTreeMap<SourceKind, SourceState>,
) -> RunView {
    let packages_note =
        packages.is_none().then(|| source_note(status, SourceKind::Packages, "Packages", "Reading packages…"));
    let no_packages = Packages::default();
    let packages = packages.unwrap_or(&no_packages);
    let mut top: Vec<&Tool> = tools.tools.iter().filter(|t| !matches!(t.kind, ToolKind::Workflow)).collect();
    let binaries: Vec<String> =
        top.iter().filter(|t| t.kind == ToolKind::Binary).map(|t| t.invocation.clone()).collect();
    top.retain(|t| t.kind != ToolKind::CargoAlias || !binaries.contains(&t.invocation));
    top.sort_by(|a, b| kind_rank(a.kind).cmp(&kind_rank(b.kind)).then(a.name.cmp(&b.name)));

    let workflows = tools
        .tools
        .iter()
        .filter(|t| t.kind == ToolKind::Workflow)
        .map(|w| WorkflowRow {
            name: w.name.clone(),
            triggers: w.description.clone(),
            jobs: w.children.iter().map(|j| j.name.clone()).collect(),
            file: w.file.clone(),
        })
        .collect();

    RunView {
        tools: top.into_iter().map(tile).collect(),
        workflows,
        tests: tests_by_package(packages, graph),
        packages: packages
            .packages
            .iter()
            .map(|p| PackageBrick {
                name: p.name.clone(),
                binaries: p.targets.iter().filter(|t| t.kind == TargetKind::Bin).count(),
            })
            .collect(),
        packages_note,
    }
}

/// Each district's note while it has no view: its source's own words when the project has
/// nothing for it or it failed. A district whose source is still reading has none.
pub fn district_notes(status: &BTreeMap<SourceKind, SourceState>) -> BTreeMap<Stop, String> {
    let sources = [
        (Stop::Run, SourceKind::Tools, "The project's tools"),
        (Stop::Pipeline, SourceKind::Pipeline, "The pipeline"),
        (Stop::Files, SourceKind::Disk, "The disk"),
        (Stop::Changes, SourceKind::Git, "Git"),
    ];
    sources
        .into_iter()
        .filter(|(_, kind, _)| matches!(status.get(kind), Some(SourceState::Unavailable(_) | SourceState::Failed(_))))
        .map(|(stop, kind, what)| (stop, source_note(status, kind, what, "")))
        .collect()
}

/// Tests per package (files belong to the innermost package holding them), most first.
fn tests_by_package(packages: &Packages, graph: &Graph) -> Vec<(String, u64)> {
    let mut roots: Vec<(&Path, &str)> = packages.packages.iter().map(|p| (p.root.as_path(), p.name.as_str())).collect();
    roots.sort_by_key(|(root, _)| std::cmp::Reverse(root.components().count()));
    let mut totals: std::collections::BTreeMap<&str, u64> = Default::default();
    for node in graph.nodes.values().filter(|n| n.test_count > 0) {
        let Some(path) = node.file_path.as_deref().map(Path::new) else { continue };
        if let Some((_, name)) = roots.iter().find(|(root, _)| path.starts_with(root)) {
            *totals.entry(name).or_default() += node.test_count as u64;
        }
    }
    let mut out: Vec<(String, u64)> = totals.into_iter().map(|(n, t)| (n.to_string(), t)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// The Files district's view: git's facts, sizes, and each settings file as a card. Every part
/// shows as soon as its source has reported.
pub fn files_view(disk: Option<&GitDisk>, sizes: &BTreeMap<PathBuf, u64>, settings: Option<&Settings>) -> FilesView {
    let ignored: Vec<IgnoredView> = disk
        .map(|d| {
            d.ignored
                .iter()
                .map(|e| IgnoredView {
                    path: e.path.clone(),
                    rule: e.rule.as_ref().map(|r| (r.source.clone(), r.line, r.pattern.clone())),
                })
                .collect()
        })
        .unwrap_or_default();
    let size_of = |rel: &str| sizes.get(Path::new(rel.trim_end_matches('/'))).copied();
    let mut top: Vec<(String, u64)> = sizes
        .iter()
        .filter(|(p, _)| p.components().count() == 1)
        .map(|(p, b)| (p.to_string_lossy().into_owned(), *b))
        .collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut cards = Vec::new();
    if let Some(ignore) = settings.and_then(|s| s.gitignore.as_ref()) {
        let sections = ignore
            .groups
            .iter()
            .map(|g| {
                let rows = g
                    .rules
                    .iter()
                    .map(|rule| {
                        let matched: Vec<&IgnoredView> = ignored
                            .iter()
                            .filter(|e| {
                                e.rule
                                    .as_ref()
                                    .is_some_and(|(file, line, _)| *line == rule.line && file == &ignore.file)
                            })
                            .collect();
                        let bytes: u64 = matched.iter().filter_map(|e| size_of(&e.path)).sum();
                        let value = match (matched.len(), bytes) {
                            (0, _) => String::new(),
                            (n, 0) => format!("{n} here"),
                            (n, b) => format!("{n} here · {}", human_bytes(b)),
                        };
                        SettingRow { key: rule.pattern.clone(), value, line: rule.line, file: None }
                    })
                    .collect();
                (g.title.clone(), rows)
            })
            .collect();
        cards.push(SettingsCardView {
            title: "Git ignore rules".into(),
            file: ignore.file.clone(),
            summary: format!("{} patterns in {} groups", ignore.patterns(), ignore.groups.len()),
            sections,
        });
    }
    for sheet in settings.map(|s| s.sheets.as_slice()).unwrap_or_default() {
        cards.push(SettingsCardView {
            title: sheet.title.clone(),
            file: sheet.file.clone(),
            summary: sheet.summary.clone(),
            sections: sheet
                .sections
                .iter()
                .map(|(t, facts)| {
                    (
                        t.clone(),
                        facts
                            .iter()
                            .map(|f| SettingRow {
                                key: f.key.clone(),
                                value: f.value.clone(),
                                line: f.line,
                                file: None,
                            })
                            .collect(),
                    )
                })
                .collect(),
        });
    }
    if let Some(schemas) = settings.map(|s| &s.schemas).filter(|s| !s.is_empty()) {
        let rows = schemas
            .iter()
            .map(|s| {
                let name = s.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut shape = vec![s.kind.clone()];
                if s.properties > 0 {
                    shape.push(format!("{} fields", s.properties));
                }
                if s.required > 0 {
                    shape.push(format!("{} required", s.required));
                }
                if s.refs > 0 {
                    shape.push(format!("{} refs", s.refs));
                }
                SettingRow {
                    key: name.trim_end_matches(".schema.json").to_string(),
                    value: shape.join(" · "),
                    line: 1,
                    file: Some(s.file.clone()),
                }
            })
            .collect();
        cards.push(SettingsCardView {
            title: "Data shapes".into(),
            file: schemas[0].file.clone(),
            summary: format!("{} JSON Schemas", schemas.len()),
            sections: vec![(String::new(), rows)],
        });
    }
    let view = FilesView {
        tracked: disk.map(|d| d.tracked),
        lfs: disk.map(|d| d.lfs),
        changes: disk.map(|d| format!("{} changes", d.changes.total())),
        ignored,
        sizes: top,
        cards,
        classes: disk.map(class_views).unwrap_or_default(),
        git_note: None,
        lfs_paths: disk.map(|d| d.lfs_paths.clone()).unwrap_or_default(),
        generated: disk.map(|d| d.generated_paths.clone()).unwrap_or_default(),
        vendored: disk.map(|d| d.vendored_paths.clone()).unwrap_or_default(),
        lookups: Default::default(),
    };
    view.build_lookups();
    view
}

/// What the Files district says instead of git's counts when the disk source has none: its own
/// words ("No git repository here"), or that git could not be read.
pub fn files_git_note(status: &BTreeMap<SourceKind, SourceState>) -> Option<String> {
    matches!(status.get(&SourceKind::Disk), Some(SourceState::Unavailable(_) | SourceState::Failed(_)))
        .then(|| source_note(status, SourceKind::Disk, "Git", ""))
}

/// Tracked files by class, as the disk job classified them (with git's Linguist attributes and
/// full project-relative paths), in the table's order.
fn class_views(disk: &GitDisk) -> Vec<ClassView> {
    disk.classes
        .iter()
        .map(|c| ClassView {
            label: c.class.label().to_string(),
            files: c.files,
            bytes: c.bytes,
            rules: c.rules.clone(),
        })
        .collect()
}

/// Changed files a row lists (the district shows as many as fit).
const ROW_FILES: usize = 40;
/// Commits a row's track holds.
const ROW_BEADS: usize = 40;
/// Sessions a row's lane lists.
const LANE_SESSIONS: usize = 24;
/// Tickets the column lists as next.
const NEXT_TICKETS: usize = 12;

/// What a source that has not reported says: its own words when the project has nothing for it,
/// else that it failed, else that it is still reading.
fn source_note(status: &BTreeMap<SourceKind, SourceState>, kind: SourceKind, what: &str, reading: &str) -> String {
    match status.get(&kind) {
        Some(SourceState::Unavailable(m)) => m.clone(),
        Some(SourceState::Failed(e)) => format!("{what} could not be read: {e}"),
        _ => reading.to_string(),
    }
}

/// The row that holds a folder's sessions when it has no git.
pub const NO_GIT_ROW: &str = "project folder · no git";

/// A lane's session chips, the first `LANE_SESSIONS` of `sessions` (slug and start only).
fn session_chips(sessions: &[&Session], now_unix_ms: i64) -> Vec<SessionChip> {
    sessions
        .iter()
        .take(LANE_SESSIONS)
        .map(|s| SessionChip {
            id: s.id.clone(),
            slug: s.slug.clone().unwrap_or_else(|| s.id.chars().take(8).collect()),
            started_ms: s.started,
            touches: s.touch_count,
            has_plan: s.plan.is_some(),
            started: ago(s.started, now_unix_ms),
        })
        .collect()
}

/// How long ago `then_ms` was at `now_ms`, in words; past a week, the (UTC) date.
fn ago(then_ms: i64, now_ms: i64) -> String {
    if then_ms <= 0 {
        return String::new();
    }
    let minutes = (now_ms - then_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        1440..=2879 => "yesterday".to_string(),
        2880..=10079 => format!("{} days ago", minutes / 1440),
        _ => {
            // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
            let z = then_ms.div_euclid(86_400_000) + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let day = doy - (153 * mp + 2) / 5 + 1;
            let month = if mp < 10 { mp + 3 } else { mp - 9 };
            let year = yoe + era * 400 + i64::from(month <= 2);
            format!("{year:04}-{month:02}-{day:02}")
        }
    }
}

fn file_change_view(f: &FileChange) -> FileChangeView {
    FileChangeView {
        path: f.path.clone(),
        added: f.added.unwrap_or(0),
        removed: f.deleted.unwrap_or(0),
        old_path: f.old_path.clone(),
        binary: f.added.is_none() || f.deleted.is_none(),
    }
}

fn ticket_view(t: &Ticket, folder: &Path, short_len: usize) -> TicketView {
    let shipped = match (&t.shipped, &t.shipped_at) {
        (Shipped::Commit(id), _) => Some(ShippedView::Commit(id.chars().take(short_len).collect())),
        (Shipped::NotInRepo, _) => Some(ShippedView::NotInRepo),
        (Shipped::None, Some(_)) => Some(ShippedView::NotChecked),
        (Shipped::None, None) => None,
    };
    let name = Path::new(&t.file).file_name().map_or_else(|| t.file.clone().into(), |n| n.to_os_string());
    TicketView {
        id: t.id.clone(),
        status: t.status.clone(),
        priority: t.priority.map(|p| p.to_string()),
        title: t.title.clone(),
        file: folder.join(name),
        shipped,
        link: None,
    }
}

/// The Changes district: one row per worktree, the branch strip and the tickets column, from git,
/// the agent session index and the ticket registry. Each part that has not reported says why.
pub fn changes_view(
    history: Option<&GitHistory>,
    agents: Option<&AgentIndex>,
    tickets: Option<&TicketIndex>,
    commit_files: &BTreeMap<String, Arc<Vec<FileChange>>>,
    status: &BTreeMap<SourceKind, SourceState>,
    now_unix_ms: i64,
) -> ChangesView {
    let mut view = ChangesView {
        sessions_message: match agents {
            None => Some(source_note(status, SourceKind::Agents, "Agent sessions", "Reading agent sessions…")),
            Some(a) if a.sessions.is_empty() => Some(studio_sources::agents::NO_SESSIONS.to_string()),
            Some(_) => None,
        },
        ..Default::default()
    };
    let short_len = history.and_then(|h| h.commits.first()).map_or(7, |c| c.short.len().max(7));

    // The tickets column, before the rows link to it.
    view.tickets.message = match tickets {
        None => Some(source_note(status, SourceKind::Tickets, "Tickets", "Reading tickets…")),
        Some(index) if index.tickets.is_empty() && index.bad_files.is_empty() => {
            Some(format!("No tickets here ({TICKETS_FOLDER})"))
        }
        Some(index) if index.tickets.is_empty() => {
            Some(format!("{} ticket files could not be read", index.bad_files.len()))
        }
        Some(_) => None,
    };
    if let Some(index) = tickets.filter(|_| view.tickets.message.is_none()) {
        let rank = |s: &str| match s {
            "ready" => 0,
            "queued" => 1,
            _ => 2,
        };
        let mut counts: Vec<(String, usize)> = index.status_counts.iter().map(|(s, n)| (s.clone(), *n)).collect();
        counts.sort_by(|a, b| rank(&a.0).cmp(&rank(&b.0)).then(a.0.cmp(&b.0)));
        view.tickets.counts = counts;
        view.tickets.next =
            next_tickets(index, NEXT_TICKETS).into_iter().map(|t| ticket_view(t, &index.folder, short_len)).collect();
        let waiting: usize = ["ready", "queued"].iter().filter_map(|s| index.status_counts.get(*s)).sum();
        view.tickets.more_next = waiting.saturating_sub(view.tickets.next.len());
    }

    let sessions_at = |path: &Path| -> Vec<&Session> {
        let Some(a) = agents else { return Vec::new() };
        let mut found: Vec<&Session> =
            a.sessions.iter().filter(|s| a.roots.get(s.root).is_some_and(|r| r == path)).collect();
        found.sort_by(|x, y| y.started.cmp(&x.started).then(x.id.cmp(&y.id)));
        found
    };

    let Some(history) = history else {
        // A folder without git says so through the disk source too, which may report first.
        let message = match (status.get(&SourceKind::Git), status.get(&SourceKind::Disk)) {
            (Some(SourceState::Unavailable(_) | SourceState::Failed(_)), _) | (_, None) => {
                source_note(status, SourceKind::Git, "Git", "Reading git…")
            }
            (_, Some(SourceState::Unavailable(m))) => m.clone(),
            _ => source_note(status, SourceKind::Git, "Git", "Reading git…"),
        };
        // Sessions still worked in the folder: one row holds them, the message is its note.
        // The project root is the agents index's first root.
        let root = agents.and_then(|a| a.roots.first());
        let sessions = root.map(|r| sessions_at(r)).unwrap_or_default();
        if let (Some(root), false) = (root, sessions.is_empty()) {
            let chips = session_chips(&sessions, now_unix_ms);
            view.rows.push(WorktreeRowView {
                branch: NO_GIT_ROW.to_string(),
                label: root.display().to_string(),
                path: root.clone(),
                more_sessions: sessions.len() - chips.len(),
                sessions: chips,
                note: Some(message.clone()),
                ..Default::default()
            });
        }
        view.message = Some(message);
        return view;
    };
    view.commits_on_head = history.commit_count;
    view.local_branches = history.branches.len();
    view.co_authored = history.co_authored_commits;
    view.commits_read = history.commits.len();

    let bead = |c: &Commit| CommitBead {
        id: c.id.clone(),
        short: c.short.clone(),
        time: c.time,
        co_authored: !c.co_authors.is_empty(),
        files: commit_files.get(&c.id).map(|files| files.iter().map(file_change_view).collect()),
        age: ago(c.time * 1000, now_unix_ms),
    };
    let head = history.commits.first().map(|c| c.id.as_str());

    let mut worktrees: Vec<&Worktree> = history.worktrees.iter().collect();
    worktrees.sort_by(|a, b| b.is_main.cmp(&a.is_main).then(a.path.cmp(&b.path)));
    for (i, w) in worktrees.into_iter().enumerate() {
        let short: String = w.head.chars().take(short_len).collect();
        let branch = history.branches.iter().find(|b| Some(&b.name) == w.branch.as_ref());
        // HEAD's history is this worktree's when it has HEAD checked out; else only its own head
        // commit is known.
        let commits = if head == Some(w.head.as_str()) {
            history.commits.iter().take(ROW_BEADS).map(bead).collect()
        } else if let Some(c) = history.commits.iter().find(|c| c.id == w.head) {
            vec![bead(c)]
        } else if !w.head.is_empty() {
            let time = branch.map_or(0, |b| b.time);
            vec![CommitBead {
                id: w.head.clone(),
                short: short.clone(),
                time,
                co_authored: false,
                files: commit_files.get(&w.head).map(|files| files.iter().map(file_change_view).collect()),
                age: ago(time * 1000, now_unix_ms),
            }]
        } else {
            Vec::new()
        };
        let c = &w.changes;
        let change_counts: Vec<(&'static str, usize)> = [
            ("modified", c.modified),
            ("added", c.added),
            ("deleted", c.deleted),
            ("renamed", c.renamed),
            ("untracked", c.untracked),
            ("conflicted", c.conflicted),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .collect();
        let sessions = sessions_at(&w.path);
        let chips = session_chips(&sessions, now_unix_ms);

        // Tickets the branch names: matched by name only, so Unresolved.
        let mut row_tickets = Vec::new();
        if let (Some(index), Some(name)) = (tickets, &w.branch) {
            for id in ticket_ids_in(name) {
                if let Some(t) = index.tickets.iter().find(|t| t.id == id) {
                    let mut linked = ticket_view(t, &index.folder, short_len);
                    linked.link = Some(TicketLinkView { row: i, tier: EvidenceTier::Unresolved, basis: "branch name" });
                    view.tickets.linked.push(linked);
                    row_tickets.push(id);
                }
            }
        }

        view.rows.push(WorktreeRowView {
            branch: w.branch.clone().unwrap_or_else(|| format!("detached at {short}")),
            label: if w.is_main { "you".to_string() } else { w.path.display().to_string() },
            path: w.path.clone(),
            is_main: w.is_main,
            ahead_behind: branch.filter(|b| b.upstream.is_some()).map(|b| (b.ahead, b.behind)),
            change_counts,
            files: w.files.iter().take(ROW_FILES).map(file_change_view).collect(),
            more_files: w.files.len().saturating_sub(ROW_FILES),
            commits,
            more_sessions: sessions.len() - chips.len(),
            sessions: chips,
            tickets: row_tickets,
            note: None,
        });
    }

    let mut branches: Vec<BranchView> = history
        .branches
        .iter()
        .map(|b| BranchView {
            name: b.name.clone(),
            ahead_behind: b.upstream.as_ref().map(|_| (b.ahead, b.behind)),
            time: b.time,
            sessions: agents.map_or(0, |a| a.sessions.iter().filter(|s| s.branches.contains(&b.name)).count()),
            age: ago(b.time * 1000, now_unix_ms),
        })
        .collect();
    branches.sort_by(|a, b| a.name.cmp(&b.name));
    view.branches = branches;
    view
}

/// The Pipeline district's view: the groups in the source's order, each flow as a lane of its
/// steps by layer and the links between them. A group's key is its kind and name, so what the
/// user opened stays open when the pipeline is read again.
pub fn pipeline_view(p: &Pipeline) -> PipelineView {
    let step = |i: usize| {
        p.steps.get(i).map(|s| PipelineStep {
            title: s.title.clone(),
            detail: s.detail.clone(),
            language: s.language.clone(),
            path: s.path.clone(),
            line: s.line,
        })
    };
    let lane = |flow: &studio_sources::Flow| {
        let mut slots = BTreeMap::new();
        let mut layers = Vec::with_capacity(flow.layers.len());
        for (l, layer) in flow.layers.iter().enumerate() {
            let mut steps = Vec::with_capacity(layer.len());
            for &i in layer {
                let Some(s) = step(i) else { continue };
                slots.entry(i).or_insert(StepSlot { layer: l, index: steps.len() });
                steps.push(s);
            }
            layers.push(steps);
        }
        let links = flow
            .links
            .iter()
            .filter_map(|&i| p.links.get(i))
            .filter_map(|link| {
                Some(PipelineLink {
                    from: *slots.get(&link.from)?,
                    to: *slots.get(&link.to)?,
                    tier: link.provenance.tier,
                    candidates: link.provenance.candidates,
                    conditional: link.conditional.clone(),
                    crossing: link.crossing.is_some(),
                    label: link.label.clone(),
                })
            })
            .collect();
        PipelineLane {
            name: flow.name.clone(),
            crossing: flow.crosses_languages,
            layers,
            links,
            truncated: flow.truncated,
        }
    };
    let groups = p
        .groups
        .iter()
        .map(|g| {
            let lanes: Vec<PipelineLane> = g.flows.iter().filter_map(|&i| p.flows.get(i)).map(lane).collect();
            PipelineGroup {
                key: format!("{:?}:{}", g.kind, g.name),
                title: g.name.clone(),
                count: lanes.len(),
                expanded: g.expanded_by_default,
                lanes,
            }
        })
        .collect();
    PipelineView { groups }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use studio_sources::{Package, PackageSource, Target};

    /// The Files view carries git's per-file marks and has its lookups built when made.
    #[test]
    fn the_files_view_carries_marks_sizes_and_rule_counts() {
        use studio_sources::{IgnoreFile, IgnoreGroup, IgnoreRule, IgnoreRuleLine, IgnoredEntry};
        let gitignore = PathBuf::from("/p/.gitignore");
        let set = |paths: &[&str]| paths.iter().map(|p| p.to_string()).collect::<std::collections::BTreeSet<_>>();
        let disk = GitDisk {
            tracked: 3,
            lfs: 1,
            lfs_paths: set(&["art/logo.png"]),
            generated_paths: set(&["gen/api.rs"]),
            vendored_paths: set(&["vendor/lib.js"]),
            ignored: vec![IgnoredEntry {
                path: "target/".into(),
                rule: Some(IgnoreRule { source: gitignore.clone(), line: 2, pattern: "/target/".into() }),
            }],
            ..Default::default()
        };
        let sizes = BTreeMap::from([(PathBuf::from("target"), 4096), (PathBuf::from("src"), 100)]);
        let rule = IgnoreRuleLine { pattern: "/target/".into(), line: 2, note: None };
        let settings = Settings {
            gitignore: Some(IgnoreFile {
                file: gitignore.clone(),
                groups: vec![IgnoreGroup { title: "build".into(), line: 1, rules: vec![rule] }],
            }),
            ..Default::default()
        };
        let view = files_view(Some(&disk), &sizes, Some(&settings));
        assert_eq!((view.lfs_paths.clone(), view.generated.clone()), (disk.lfs_paths, disk.generated_paths));
        assert_eq!(view.vendored, disk.vendored_paths);
        assert_eq!(view.sizes, [("target".to_string(), 4096), ("src".to_string(), 100)]);
        assert!(view.is_ignored("target/debug/app") && !view.is_ignored("src"));
        assert_eq!(view.cards[0].sections[0].1[0].value, "1 here · 4.0 KB");
        let world = studio_canvas::WorldLayout::default();
        assert!(studio_canvas::districts::files::card_world_rect(&world, &view, &gitignore).is_some());
        assert!(view.classes.is_empty(), "no classes before git has listed the files");
    }

    /// In a folder without git the disk source says so at once and still measures sizes; the
    /// Files header shows its words instead of reading forever.
    #[test]
    fn a_folder_without_git_says_so_in_the_files_header_and_keeps_its_sizes() {
        use studio_sources::{disk_job, SourceEvent, SourceHub};
        let dir = tempfile::tempdir().unwrap();
        // A broken .git file keeps git from finding any repository around the folder.
        std::fs::write(dir.path().join(".git"), b"not a gitdir\n").unwrap();
        std::fs::write(dir.path().join("notes.txt"), vec![b'x'; 5000]).unwrap();
        let hub = SourceHub::new(dir.path(), || {});
        hub.spawn(SourceKind::Disk, disk_job());
        let (mut status, mut sizes) = (BTreeMap::new(), BTreeMap::new());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut ended = false;
        while !ended && std::time::Instant::now() < deadline {
            for event in hub.drain() {
                match event {
                    SourceEvent::FolderSize(rel, bytes) => {
                        sizes.insert(rel, bytes);
                    }
                    SourceEvent::Status { source, state } => {
                        // Sizes come after the first Unavailable, so the job ends on the second.
                        ended = !matches!(state, SourceState::Running) && sizes.contains_key(Path::new("notes.txt"));
                        status.insert(source, state);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(status.get(&SourceKind::Disk), Some(&SourceState::Unavailable("No git repository here".into())));
        let mut view = files_view(None, &sizes, None);
        view.git_note = files_git_note(&status);
        assert_eq!(studio_canvas::districts::files::tree_summary(&view), "No git repository here");
        assert!(view.sizes.iter().any(|(name, bytes)| name == "notes.txt" && *bytes > 0), "sizes still shown");
        let git = BTreeMap::from([(SourceKind::Disk, SourceState::Ready { elapsed: Default::default() })]);
        assert_eq!(files_git_note(&git), None, "a git repository keeps git's counts");
    }

    /// Every class git's files fall in is shown, in the table's order, with its rules.
    #[test]
    fn the_files_view_totals_tracked_files_by_class() {
        use studio_parser::classify::FileClass;
        use studio_sources::disk::ClassTotal;
        let total = |class, files, bytes, rules: &[(&str, usize)]| ClassTotal {
            class,
            files,
            bytes,
            rules: rules.iter().map(|(w, n)| (w.to_string(), *n)).collect(),
        };
        let disk = GitDisk {
            tracked: 7,
            classes: vec![
                total(FileClass::Code, 4, 4000, &[("extension", 4)]),
                total(FileClass::Tests, 2, 300, &[("tool convention: Cargo", 1), ("file name", 1)]),
                total(FileClass::BuildOutput, 1, 5, &[("gitattributes: linguist-generated", 1)]),
            ],
            ..Default::default()
        };
        let view = files_view(Some(&disk), &BTreeMap::new(), None);
        let rows: Vec<(&str, usize, u64)> = view.classes.iter().map(|c| (c.label.as_str(), c.files, c.bytes)).collect();
        assert_eq!(rows, [("code", 4, 4000), ("tests", 2, 300), ("build output", 1, 5)]);
        assert_eq!(view.classes[1].why(), "tool convention: Cargo 1 · file name 1");
        assert_eq!(view.classes[2].why(), "gitattributes: linguist-generated 1");
        let world = studio_canvas::WorldLayout::default();
        let parts = studio_canvas::districts::files::layout(world.files.size() / world.scale, &view, 0);
        assert_eq!(parts.class_rows.len(), 3);
        assert!(parts.classes.top() > parts.map.bottom() && parts.classes.contains_rect(parts.class_rows[2]));
    }

    fn tool(kind: ToolKind, name: &str, invocation: &str) -> Tool {
        Tool {
            kind,
            name: name.into(),
            invocation: invocation.into(),
            description: None,
            file: PathBuf::from("/p/x"),
            line: 1,
            children: Vec::new(),
        }
    }

    #[test]
    fn tools_are_shown_once_by_kind_then_name_and_tests_by_package() {
        let tools = Tools {
            tools: vec![
                tool(ToolKind::CargoAlias, "xtask", "cargo xtask"),
                tool(ToolKind::MakeTarget, "build", "make build"),
                tool(ToolKind::Binary, "xtask", "cargo xtask"),
                tool(ToolKind::Binary, "api-server", "cargo run -p api --"),
                tool(ToolKind::Workflow, "CI", ""),
            ],
        };
        let package = |name: &str, root: &str| Package {
            name: name.into(),
            version: "0.1.0".into(),
            manifest: PathBuf::from(root).join("Cargo.toml"),
            root: PathBuf::from(root),
            targets: vec![Target {
                kind: TargetKind::Bin,
                name: name.into(),
                src: PathBuf::from(root).join("src/main.rs"),
            }],
            depends_on: Vec::new(),
            source: PackageSource::Cargo,
            edition: String::new(),
            proc_macro: false,
            dependencies: Vec::new(),
        };
        let packages = Packages {
            packages: vec![package("api", "/p/api"), package("inner", "/p/api/inner")],
            fallbacks: Vec::new(),
        };
        let mut graph = Graph::new();
        for (path, tests) in [("/p/api/src/a.rs", 3), ("/p/api/inner/src/b.rs", 2), ("/p/other.rs", 9)] {
            let id = graph.add_node("f", studio_graph::NodeArchetype::File, "", None, vec![], vec![], [0.0, 0.0]);
            let node = graph.nodes.get_mut(&id).unwrap();
            node.file_path = Some(path.into());
            node.test_count = tests;
        }
        let view = run_view(&tools, Some(&packages), &graph, &BTreeMap::new());
        assert_eq!(view.packages_note, None);
        let names: Vec<&str> = view.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["api-server", "xtask", "build"], "the alias that runs xtask is not shown twice");
        assert_eq!(view.workflows.len(), 1);
        assert_eq!(
            view.tests,
            [("api".to_string(), 3), ("inner".to_string(), 2)],
            "innermost package, files outside left out"
        );
    }

    #[test]
    fn the_pipeline_view_keeps_the_order_and_places_each_step_by_layer() {
        use studio_graph::{Basis, EvidenceTier, Provenance};
        use studio_sources::{Flow, FlowGroup, FlowGroupKind, Link, LinkKind, Step, StepKind};
        let step = |kind, title: &str, language: &str, path: &str, line| Step {
            kind,
            title: title.into(),
            detail: format!("{path}:{line}"),
            language: language.into(),
            path: PathBuf::from(path),
            line,
            line_end: None,
        };
        let link = |from, to, kind, provenance, crossing: Option<(&str, &str)>, label: &str| Link {
            from,
            to,
            kind,
            provenance,
            conditional: None,
            crossing: crossing.map(|(a, b)| (a.to_string(), b.to_string())),
            label: label.into(),
            evidence: Vec::new(),
        };
        let pipeline = Pipeline {
            steps: vec![
                step(StepKind::Sender, "SendReport", "Enforce", "mod/Exec.c", 116),
                step(StepKind::Route, "POST /api/v1/x/{id}/result", "Rust", "api/routes.rs", 76),
                step(StepKind::Handler, "finish", "Rust", "api/handlers.rs", 81),
                step(StepKind::Main, "main", "Rust", "tools/src/main.rs", 3),
                step(StepKind::Function, "helper", "Rust", "api/other.rs", 9),
            ],
            links: vec![
                link(
                    0,
                    1,
                    LinkKind::Requests,
                    Provenance::proven(Basis::RouteTable),
                    Some(("Enforce", "Rust")),
                    "x.schema.json#/a",
                ),
                link(1, 2, LinkKind::Serves, Provenance::possible(Basis::RouteTable, 2), None, ""),
                // Into a step the flow does not hold: left out.
                link(2, 4, LinkKind::Calls, Provenance::default(), None, ""),
            ],
            flows: vec![
                Flow {
                    name: "POST /api/v1/x/{id}/result".into(),
                    entry: 1,
                    group: 0,
                    layers: vec![vec![0, 1], vec![2]],
                    links: vec![0, 1, 2],
                    truncated: 0,
                    crosses_languages: true,
                },
                Flow {
                    name: "tools".into(),
                    entry: 3,
                    group: 1,
                    layers: vec![vec![3]],
                    links: vec![],
                    truncated: 4,
                    crosses_languages: false,
                },
            ],
            groups: vec![
                FlowGroup {
                    kind: FlowGroupKind::CrossLanguage,
                    name: "Cross-language".into(),
                    flows: vec![0],
                    expanded_by_default: true,
                },
                FlowGroup {
                    kind: FlowGroupKind::Mains,
                    name: "Programs".into(),
                    flows: vec![1],
                    expanded_by_default: false,
                },
            ],
            stats: Default::default(),
        };
        let view = pipeline_view(&pipeline);
        let summary: Vec<(&str, &str, usize, bool)> =
            view.groups.iter().map(|g| (g.key.as_str(), g.title.as_str(), g.count, g.expanded)).collect();
        assert_eq!(
            summary,
            [("CrossLanguage:Cross-language", "Cross-language", 1, true), ("Mains:Programs", "Programs", 1, false)]
        );
        let lane = &view.groups[0].lanes[0];
        assert!(lane.crossing);
        let titles: Vec<Vec<&str>> = lane.layers.iter().map(|l| l.iter().map(|s| s.title.as_str()).collect()).collect();
        assert_eq!(titles, [vec!["SendReport", "POST /api/v1/x/{id}/result"], vec!["finish"]]);
        assert_eq!((lane.layers[0][0].language.as_str(), lane.layers[0][0].line), ("Enforce", 116));
        assert_eq!(lane.layers[1][0].path, PathBuf::from("api/handlers.rs"));
        assert_eq!(lane.links.len(), 2, "the link to a step outside the flow is left out");
        let sender_to_route = &lane.links[0];
        assert_eq!(
            (sender_to_route.from, sender_to_route.to),
            (StepSlot { layer: 0, index: 0 }, StepSlot { layer: 0, index: 1 })
        );
        assert_eq!(sender_to_route.tier, EvidenceTier::Proven);
        assert!(sender_to_route.crossing);
        assert_eq!(sender_to_route.label, "x.schema.json#/a");
        let route_to_handler = &lane.links[1];
        assert_eq!(route_to_handler.to, StepSlot { layer: 1, index: 0 });
        assert_eq!((route_to_handler.tier, route_to_handler.candidates), (EvidenceTier::PossibleSet, 2));
        assert!(!route_to_handler.crossing);
        assert_eq!(view.groups[1].lanes[0].truncated, 4);
        assert_eq!(pipeline_view(&pipeline), view, "the same every time");
        assert!(pipeline_view(&Pipeline::default()).is_empty());
    }

    #[test]
    fn a_project_without_cargo_shows_its_other_tools_and_says_why_it_has_no_packages() {
        let tools = Tools {
            tools: vec![tool(ToolKind::NpmScript, "dev", "npm run dev"), tool(ToolKind::MakeTarget, "all", "make all")],
        };
        let mut status = BTreeMap::new();
        status.insert(SourceKind::Packages, SourceState::Unavailable("No Cargo packages here".into()));
        let view = run_view(&tools, None, &Graph::new(), &status);
        let names: Vec<&str> = view.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["dev", "all"]);
        assert!(view.packages.is_empty() && view.tests.is_empty());
        assert_eq!(view.packages_note.as_deref(), Some("No Cargo packages here"));

        status.insert(SourceKind::Packages, SourceState::Failed("cargo metadata exited 101".into()));
        let view = run_view(&tools, None, &Graph::new(), &status);
        assert_eq!(view.packages_note.as_deref(), Some("Packages could not be read: cargo metadata exited 101"));
    }

    #[test]
    fn a_district_whose_source_has_nothing_says_why_instead_of_reading() {
        let mut status = BTreeMap::new();
        assert!(district_notes(&status).is_empty(), "nothing reported: still reading");
        status.insert(SourceKind::Pipeline, SourceState::Running);
        status.insert(SourceKind::Tools, SourceState::Ready { elapsed: Default::default() });
        assert!(district_notes(&status).is_empty(), "running or ready: no note");

        status.insert(SourceKind::Pipeline, SourceState::Failed("routes.rs: bad table".into()));
        status.insert(SourceKind::Tools, SourceState::Unavailable("No tools here".into()));
        let notes = district_notes(&status);
        assert_eq!(
            notes.get(&Stop::Pipeline).map(String::as_str),
            Some("The pipeline could not be read: routes.rs: bad table")
        );
        assert_eq!(notes.get(&Stop::Run).map(String::as_str), Some("No tools here"));
        assert_eq!(notes.len(), 2);
    }
}

#[cfg(test)]
mod changes_tests {
    use super::*;
    use studio_sources::{Branch, PlanRef, StatusCounts};

    /// 2025-10-09T08:53:20Z.
    const NOW: i64 = 1_760_000_000_000;

    fn commit(n: usize, co: bool) -> Commit {
        Commit {
            id: format!("{n:040x}"),
            short: format!("{n:07x}"),
            author: "a".into(),
            time: NOW / 1000 - n as i64 * 3600,
            subject: String::new(),
            parents: 1,
            co_authors: if co { vec!["Claude <c@x>".into()] } else { Vec::new() },
        }
    }

    fn worktree(path: &str, head: &str, branch: Option<&str>, is_main: bool) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: head.into(),
            branch: branch.map(str::to_string),
            is_main,
            changes: StatusCounts { modified: 2, untracked: 1, ..Default::default() },
            files: vec![
                FileChange { path: "a.rs".into(), old_path: None, added: Some(3), deleted: Some(1) },
                FileChange { path: "b.rs".into(), old_path: Some("old.rs".into()), added: Some(0), deleted: Some(0) },
                FileChange { path: "logo.png".into(), old_path: None, added: None, deleted: None },
            ],
        }
    }

    fn branch(name: &str, head: &str, upstream: bool) -> Branch {
        Branch {
            name: name.into(),
            head: head.into(),
            upstream: upstream.then(|| format!("origin/{name}")),
            ahead: 2,
            behind: 1,
            time: NOW / 1000 - 7200,
            subject: String::new(),
        }
    }

    /// HEAD is commit 1 on `main`; `slice/T-940.11` (worktree wt-b) is at commit 9, not in HEAD's
    /// history; wt-a is detached at HEAD.
    fn history() -> GitHistory {
        let commits: Vec<Commit> = (1..=5).map(|n| commit(n, n % 2 == 1)).collect();
        let head = commits[0].id.clone();
        let side = commit(9, false).id;
        GitHistory {
            branches: vec![
                branch("zeta", &side, false),
                branch("main", &head, true),
                branch("slice/T-940.11", &side, false),
            ],
            // Linked worktrees first and out of order: rows put the main checkout first, then by path.
            worktrees: vec![
                worktree("/r/wt-b", &side, Some("slice/T-940.11"), false),
                worktree("/r/wt-a", &head, None, false),
                worktree("/r/main", &head, Some("main"), true),
            ],
            commit_count: 4286,
            co_authors: Default::default(),
            co_authored_commits: 3,
            commits,
        }
    }

    fn session(id: &str, root: usize, started: i64, plan: bool, branches: &[&str]) -> Session {
        Session {
            id: id.into(),
            slug: Some(format!("slug-{id}")),
            root,
            branches: branches.iter().map(|b| b.to_string()).collect(),
            started,
            ended: started + 1,
            logs: Vec::new(),
            plan: plan.then_some(PlanRef { path: None, log: 0, offset: 0, len: 0, time: started }),
            tasks: Vec::new(),
            touch_count: 4,
        }
    }

    fn agents() -> AgentIndex {
        AgentIndex {
            roots: vec![PathBuf::from("/r/main"), PathBuf::from("/r/wt-b"), PathBuf::from("/r/wt-a")],
            sessions: vec![
                session("old", 0, NOW - 3 * 86_400_000, false, &["main"]),
                session("new", 0, NOW - 120_000, true, &["main"]),
                session("side", 1, NOW - 7_200_000, false, &["slice/T-940.11", "main"]),
            ],
            ..Default::default()
        }
    }

    fn ticket(id: &str, status: &str, priority: Option<i64>, shipped_at: Option<&str>, shipped: Shipped) -> Ticket {
        Ticket {
            id: id.into(),
            title: format!("title {id}"),
            status: status.into(),
            kind: None,
            priority,
            order: None,
            parent: None,
            shipped_at: shipped_at.map(str::to_string),
            shipped,
            spec: None,
            plan: None,
            file: format!(".ai/tickets/{id}.toml"),
        }
    }

    fn tickets(shipped: Shipped, shipped_at: Option<&str>) -> TicketIndex {
        let tickets = vec![
            ticket("T-1", "queued", Some(0), None, Shipped::None),
            ticket("T-2", "ready", Some(1), None, Shipped::None),
            ticket("T-3", "idea", None, None, Shipped::None),
            ticket("T-940.11", "shipped", None, shipped_at, shipped),
        ];
        let mut status_counts = BTreeMap::new();
        for t in &tickets {
            *status_counts.entry(t.status.clone()).or_insert(0) += 1;
        }
        TicketIndex {
            files: tickets.len(),
            tickets,
            status_counts,
            bad_files: Vec::new(),
            folder: "/r/main/.ai/tickets".into(),
        }
    }

    fn ready() -> BTreeMap<SourceKind, SourceState> {
        BTreeMap::new()
    }

    #[test]
    fn rows_are_worktrees_main_first_with_their_commits_files_and_sessions() {
        let h = history();
        let a = agents();
        let mut loaded = BTreeMap::new();
        let head = h.commits[0].id.clone();
        loaded.insert(
            head.clone(),
            Arc::new(vec![FileChange { path: "x.rs".into(), old_path: None, added: Some(1), deleted: None }]),
        );
        let view = changes_view(Some(&h), Some(&a), None, &loaded, &ready(), NOW);

        assert_eq!((view.commits_on_head, view.local_branches, view.co_authored, view.commits_read), (4286, 3, 3, 5));
        assert!(view.message.is_none() && view.sessions_message.is_none());
        let paths: Vec<&str> = view.rows.iter().map(|r| r.path.to_str().unwrap()).collect();
        assert_eq!(paths, ["/r/main", "/r/wt-a", "/r/wt-b"]);

        let main = &view.rows[0];
        assert_eq!((main.branch.as_str(), main.label.as_str(), main.is_main), ("main", "you", true));
        assert_eq!(main.ahead_behind, Some((2, 1)));
        assert_eq!(main.change_counts, [("modified", 2), ("untracked", 1)]);
        assert_eq!(main.commits.len(), 5, "HEAD's history on the row that has HEAD checked out");
        assert!(main.commits[0].co_authored && !main.commits[1].co_authored);
        assert_eq!(main.commits[0].age, "1 h ago");
        assert_eq!(main.commits[0].files.as_ref().map(Vec::len), Some(1), "loaded files are shown");
        assert!(main.commits[0].files.as_ref().unwrap()[0].binary, "no count is binary");
        assert!(main.commits[1].files.is_none(), "not loaded yet");
        assert_eq!(main.files[1].old_path.as_deref(), Some("old.rs"));
        assert!(main.files[2].binary && !main.files[0].binary);

        // Sessions by root, newest first; slug and start only.
        let ids: Vec<&str> = main.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(main.sessions[0].slug, "slug-new");
        assert_eq!(main.sessions[0].started, "2 min ago");
        assert!(main.sessions[0].has_plan && !main.sessions[1].has_plan);
        assert_eq!(main.sessions[1].started, "3 days ago");

        let detached = &view.rows[1];
        assert_eq!(detached.branch, format!("detached at {}", &head[..7]));
        assert_eq!(detached.label, "/r/wt-a");
        assert_eq!(detached.commits.len(), 5, "a worktree at HEAD shares HEAD's history");
        assert!(detached.sessions.is_empty(), "root 2 has no sessions");

        let side = &view.rows[2];
        assert_eq!(side.ahead_behind, None, "no upstream");
        assert_eq!(side.commits.len(), 1, "only the branch's own head is known");
        assert_eq!(side.commits[0].short, "0000000", "the first seven of its id");
        assert_eq!(side.commits[0].age, "2 h ago", "from the branch's time");
        assert_eq!(side.sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["side"]);

        let names: Vec<&str> = view.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["main", "slice/T-940.11", "zeta"], "by name");
        let sessions: Vec<usize> = view.branches.iter().map(|b| b.sessions).collect();
        assert_eq!(sessions, [3, 1, 0]);
        assert_eq!(view.branches[0].ahead_behind, Some((2, 1)));
        assert_eq!(view.branches[2].ahead_behind, None);
    }

    #[test]
    fn tickets_link_by_branch_name_unresolved_and_say_what_git_knows_of_their_commit() {
        let h = history();
        let sha = "0000000000000000000000000000000000000009";
        let cases = [
            (Shipped::Commit(sha.into()), Some(sha), Some(ShippedView::Commit("0000000".into()))),
            (Shipped::NotInRepo, Some("abcdef1"), Some(ShippedView::NotInRepo)),
            (Shipped::None, Some("abcdef1"), Some(ShippedView::NotChecked)),
            (Shipped::None, None, None),
        ];
        for (shipped, at, expected) in cases {
            let t = tickets(shipped, at);
            let view = changes_view(Some(&h), None, Some(&t), &BTreeMap::new(), &ready(), NOW);
            assert!(view.tickets.message.is_none());
            assert_eq!(view.tickets.counts[0], ("ready".to_string(), 1), "ready, then queued, then by name");
            assert_eq!(view.tickets.counts[1].0, "queued");
            let next: Vec<&str> = view.tickets.next.iter().map(|t| t.id.as_str()).collect();
            assert_eq!(next, ["T-2", "T-1"]);
            assert_eq!(view.tickets.more_next, 0);
            assert_eq!(view.tickets.next[0].priority.as_deref(), Some("1"));
            assert_eq!(view.tickets.next[0].file, PathBuf::from("/r/main/.ai/tickets/T-2.toml"));

            assert_eq!(view.tickets.linked.len(), 1);
            let linked = &view.tickets.linked[0];
            assert_eq!(linked.id, "T-940.11");
            let link = linked.link.as_ref().unwrap();
            assert_eq!((link.row, link.tier, link.basis), (2, EvidenceTier::Unresolved, "branch name"));
            assert_eq!(linked.shipped, expected);
            assert_eq!(view.rows[2].tickets, ["T-940.11"]);
            assert!(view.rows[0].tickets.is_empty());
        }
        assert_eq!(ShippedView::Commit("0000000".into()).tier(), Some(EvidenceTier::Proven));
    }

    #[test]
    fn a_folder_without_git_keeps_its_sessions_in_one_row() {
        let mut status = BTreeMap::new();
        status.insert(SourceKind::Git, SourceState::Unavailable("No git repository here".into()));
        let agents = AgentIndex {
            roots: vec![PathBuf::from("/p")],
            sessions: vec![session("a", 0, NOW - 60_000, false, &[]), session("b", 0, NOW - 120_000, true, &[])],
            ..Default::default()
        };
        let view = changes_view(None, Some(&agents), None, &BTreeMap::new(), &status, NOW);
        assert_eq!(view.message.as_deref(), Some("No git repository here"));
        assert_eq!(view.rows.len(), 1);
        let row = &view.rows[0];
        assert_eq!((row.branch.as_str(), row.path.as_path()), (NO_GIT_ROW, Path::new("/p")));
        assert_eq!(row.note.as_deref(), Some("No git repository here"), "the message is the row's note");
        let ids: Vec<&str> = row.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "newest first");
        assert_eq!(row.more_sessions, 0);
        assert!(row.commits.is_empty() && row.files.is_empty() && !row.is_main);

        // No sessions: no row, the message alone.
        let none = AgentIndex { roots: vec![PathBuf::from("/p")], ..Default::default() };
        assert!(changes_view(None, Some(&none), None, &BTreeMap::new(), &status, NOW).rows.is_empty());
    }

    #[test]
    fn each_missing_source_says_so_plainly() {
        let mut status = BTreeMap::new();
        let view = changes_view(None, None, None, &BTreeMap::new(), &status, NOW);
        assert_eq!(view.message.as_deref(), Some("Reading git…"));
        assert_eq!(view.sessions_message.as_deref(), Some("Reading agent sessions…"));
        assert_eq!(view.tickets.message.as_deref(), Some("Reading tickets…"));

        status.insert(SourceKind::Git, SourceState::Unavailable("No git repository here".into()));
        status.insert(SourceKind::Agents, SourceState::Unavailable(studio_sources::agents::NO_SESSIONS.into()));
        status.insert(SourceKind::Tickets, SourceState::Unavailable("No tickets here (.ai/tickets)".into()));
        let view = changes_view(None, None, None, &BTreeMap::new(), &status, NOW);
        assert_eq!(view.message.as_deref(), Some("No git repository here"));
        assert_eq!(view.sessions_message.as_deref(), Some("No agent sessions for this project"));
        assert_eq!(view.tickets.message.as_deref(), Some("No tickets here (.ai/tickets)"));
        assert!(view.rows.is_empty());

        // The disk source may say there is no git before the history job does.
        let mut disk_first = BTreeMap::new();
        disk_first.insert(SourceKind::Git, SourceState::Running);
        disk_first.insert(SourceKind::Disk, SourceState::Unavailable("No git repository here".into()));
        let view = changes_view(None, None, None, &BTreeMap::new(), &disk_first, NOW);
        assert_eq!(view.message.as_deref(), Some("No git repository here"));

        // Read, but empty.
        let empty_agents = AgentIndex::default();
        let no_tickets = TicketIndex {
            tickets: Vec::new(),
            status_counts: BTreeMap::new(),
            files: 0,
            bad_files: Vec::new(),
            folder: "/r/.ai/tickets".into(),
        };
        let view =
            changes_view(Some(&history()), Some(&empty_agents), Some(&no_tickets), &BTreeMap::new(), &ready(), NOW);
        assert!(view.message.is_none());
        assert_eq!(view.sessions_message.as_deref(), Some("No agent sessions for this project"));
        assert_eq!(view.tickets.message.as_deref(), Some("No tickets here (.ai/tickets)"));
        assert!(view.rows.iter().all(|r| r.sessions.is_empty()));
    }

    #[test]
    fn ages_read_as_words_then_dates() {
        assert_eq!(ago(NOW - 10_000, NOW), "just now");
        assert_eq!(ago(NOW - 5 * 60_000, NOW), "5 min ago");
        assert_eq!(ago(NOW - 26 * 3_600_000, NOW), "yesterday");
        assert_eq!(ago(NOW - 30 * 86_400_000, NOW), "2025-09-09");
        assert_eq!(ago(0, NOW), "");
    }
}
