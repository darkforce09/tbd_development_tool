//! `discover_tools` on `tests/fixtures/tools` (packages read from its manifests), checked against its
//! `expected.toml`: every tool, nested ones included, with its kind, name, file:line, invocation and description.

use std::path::{Path, PathBuf};

use studio_sources::packages::read_manifests;
use studio_sources::{discover_tools, Tool, ToolKind};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tools")
}

fn expected() -> toml::Table {
    std::fs::read_to_string(fixture().join("expected.toml")).unwrap().parse().unwrap()
}

/// Every file under `root` except `expected.toml`, sorted, as the folder scan lists them.
fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.file_name() != "expected.toml")
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

/// One line per tool, depth first, children indented by two spaces per level.
fn describe(root: &Path, tool: &Tool, depth: usize, out: &mut Vec<String>) {
    out.push(format!(
        "{}{} {} ({}:{}) $ {} # {}",
        "  ".repeat(depth),
        tool.kind.label(),
        tool.name,
        tool.file.strip_prefix(root).unwrap().display(),
        tool.line,
        tool.invocation,
        tool.description.as_deref().unwrap_or("-")
    ));
    for child in &tool.children {
        describe(root, child, depth + 1, out);
    }
}

#[test]
fn every_tool_of_the_fixture_is_found_exactly() {
    let root = fixture();
    let files = project_files(&root);
    let manifests: Vec<PathBuf> = files.iter().filter(|f| f.ends_with("Cargo.toml")).cloned().collect();
    let tools = discover_tools(&root, &read_manifests(&manifests), &files);

    let mut lines = Vec::new();
    for tool in &tools.tools {
        describe(&root, tool, 0, &mut lines);
    }
    let expected = expected();
    let wanted: Vec<String> =
        expected["tools"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    for (got, want) in lines.iter().zip(&wanted) {
        assert_eq!(got, want);
    }
    assert_eq!(lines, wanted);

    let counts = expected["counts"].as_table().unwrap();
    let kinds = [
        ToolKind::Binary,
        ToolKind::Example,
        ToolKind::CargoAlias,
        ToolKind::Subcommand,
        ToolKind::NpmScript,
        ToolKind::MakeTarget,
        ToolKind::JustRecipe,
        ToolKind::Workflow,
        ToolKind::WorkflowJob,
    ];
    for kind in kinds {
        assert_eq!(tools.count(kind) as i64, counts[kind.label()].as_integer().unwrap(), "{kind:?}");
    }
    assert_eq!(counts.len(), kinds.len());
    assert_eq!(tools.all().count() as i64, expected["total"].as_integer().unwrap());
}
