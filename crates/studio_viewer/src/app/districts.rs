//! Turns what the sources read into what the districts show. Run when a source reports, never
//! per frame.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use studio_canvas::{
    FilesView, IgnoredView, PackageBrick, RunView, SettingRow, SettingsCardView, ToolTile, WorkflowRow,
};
use studio_graph::{human_bytes, Graph};
use studio_sources::{GitDisk, Packages, Settings, TargetKind, Tool, ToolKind, Tools};

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
/// binary), CI workflows, tests by package and the package wall.
pub fn run_view(tools: &Tools, packages: &Packages, graph: &Graph) -> RunView {
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
    }
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
    FilesView {
        tracked: disk.map(|d| d.tracked),
        lfs: disk.map(|d| d.lfs),
        changes: disk.map(|d| format!("{} changes", d.changes.total())),
        ignored,
        sizes: top,
        cards,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use studio_sources::{Package, PackageSource, Target};

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
        let view = run_view(&tools, &packages, &graph);
        let names: Vec<&str> = view.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["api-server", "xtask", "build"], "the alias that runs xtask is not shown twice");
        assert_eq!(view.workflows.len(), 1);
        assert_eq!(
            view.tests,
            [("api".to_string(), 3), ("inner".to_string(), 2)],
            "innermost package, files outside left out"
        );
    }
}
