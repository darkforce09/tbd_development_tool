//! The axum route-table evaluator: exact expectations on the `axum` fixture (`expected.toml` beside it), and an
//! ignored real-repository check (`STUDIO_REAL_REPOS`, colon-separated paths).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use studio_parser::analysis::axum::{evaluate_routes, RouteEntry, RouteTable};
use studio_parser::analysis::crate_graph::CrateGraph;
use studio_parser::analysis::route_template::RouteTemplate;
use studio_parser::analysis::rust_resolver::{Resolution, RustIndex, UnresolvedReason};
use studio_parser::analysis::tags::{owner_spans, scan_tags, TagOwner};
use studio_parser::extractor::{extract_source, SourceLang};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/axum")
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Every `Cargo.toml` with a `[package]` table under `root`, skipping `target*` and hidden folders.
fn package_manifests(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            e.depth() == 0 || !(e.file_type().is_dir() && (name.starts_with("target") || name.starts_with('.')))
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == "Cargo.toml")
        .map(|e| e.into_path())
        .filter(|p| read_file(p).and_then(|t| t.parse::<toml::Table>().ok()).is_some_and(|t| t.contains_key("package")))
        .collect();
    out.sort();
    out
}

fn evaluate(root: &Path) -> (RouteTable, RustIndex) {
    let graph = CrateGraph::from_manifests(root, &package_manifests(root));
    let index = RustIndex::build(&graph, &read_file);
    let table = evaluate_routes(&index, &graph, &read_file);
    (table, index)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn handler_text(root: &Path, r: &Resolution) -> String {
    match r {
        Resolution::Item(i) => format!("{}:{}", rel(root, &i.file), i.line),
        Resolution::External(p) => format!("external {p}"),
        Resolution::Ambiguous(list) => format!("ambiguous {}", list.len()),
        Resolution::Unresolved(UnresolvedReason::Other(text)) => format!("unresolved Other: {text}"),
        Resolution::Unresolved(reason) => format!("unresolved {reason:?}"),
    }
}

fn entry_line(root: &Path, e: &RouteEntry) -> String {
    format!(
        "{} {} => {} `{}` if [{}] at {}:{} root {}:{}",
        e.method,
        e.template,
        handler_text(root, &e.handler),
        e.handler_text,
        e.conditional.as_deref().unwrap_or(""),
        rel(root, &e.defined_at.0),
        e.defined_at.1,
        rel(root, &e.root.0),
        e.root.1
    )
}

fn s<'a>(t: &'a toml::Value, key: &str) -> &'a str {
    t.get(key).and_then(|v| v.as_str()).unwrap_or_else(|| panic!("expected.toml: missing `{key}` in {t}"))
}

/// Compares two ordered lists and fails with the missing and extra lines.
fn assert_same(what: &str, expected: &[String], actual: &[String]) {
    let missing: Vec<&String> = expected.iter().filter(|e| !actual.contains(e)).collect();
    let extra: Vec<&String> = actual.iter().filter(|a| !expected.contains(a)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}: missing (recall) {missing:#?}\nextra (precision) {extra:#?}"
    );
    assert_eq!(expected, actual, "{what}: order");
}

#[test]
fn fixture_route_table_matches_expected() {
    let root = fixture_root();
    let expected: toml::Value =
        std::fs::read_to_string(root.join("expected.toml")).unwrap().parse().expect("expected.toml parses");
    let (table, _) = evaluate(&root);

    let actual: Vec<String> = table.entries.iter().map(|e| entry_line(&root, e)).collect();
    let wanted: Vec<String> = expected["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            format!(
                "{} {} => {} `{}` if [{}] at {} root {}",
                s(e, "method"),
                s(e, "template"),
                s(e, "handler"),
                s(e, "handler_text"),
                e.get("conditional").and_then(|c| c.as_str()).unwrap_or(""),
                s(e, "at"),
                s(e, "root")
            )
        })
        .collect();
    assert_same("entries", &wanted, &actual);

    let skipped: Vec<String> =
        table.skipped.iter().map(|(file, line, reason)| format!("{}:{line} {reason}", rel(&root, file))).collect();
    let wanted_skipped: Vec<String> =
        expected["skipped"].as_array().unwrap().iter().map(|k| format!("{} {}", s(k, "at"), s(k, "reason"))).collect();
    assert_same("skipped", &wanted_skipped, &skipped);
    assert_eq!(table.services as i64, expected["services"].as_integer().unwrap(), "services");
    assert_eq!(table.entries.len(), wanted.len(), "entry count");

    // D9: nothing from `tests/` or `#[cfg(test)]` code.
    assert!(table.entries.iter().all(|e| !e.template.contains("test-only")));
    assert!(table.entries.iter().all(|e| !rel(&root, &e.defined_at.0).contains("tests/")));
    // Deterministic: a second run gives the same table.
    assert_eq!(evaluate(&root).0, table);
}

/// The route table of TBD-Reforger. Run with
/// `STUDIO_REAL_REPOS=/path/to/TBD-Reforger cargo test -p studio_parser --test axum -- --ignored --nocapture`.
#[test]
#[ignore]
fn axum_real_repo_route_table() {
    let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else {
        eprintln!("STUDIO_REAL_REPOS unset: skipped");
        return;
    };
    let target = "crates/api/api_server_infrastructure/src/routes.rs";
    let Some(root) = repos.split(':').map(PathBuf::from).find(|r| r.join(target).is_file()) else {
        eprintln!("no repository in STUDIO_REAL_REPOS has {target}: skipped");
        return;
    };
    let started = Instant::now();
    let manifests = package_manifests(&root);
    let graph = CrateGraph::from_manifests(&root, &manifests);
    let index = RustIndex::build(&graph, &read_file);
    let indexed = started.elapsed();
    let started = Instant::now();
    let table = evaluate_routes(&index, &graph, &read_file);
    let evaluated = started.elapsed();

    for e in &table.entries {
        println!("{}", entry_line(&root, e));
    }
    let resolved = table.entries.iter().filter(|e| matches!(e.handler, Resolution::Item(_))).count();
    let mut unresolved: BTreeMap<String, usize> = BTreeMap::new();
    for e in table.entries.iter().filter(|e| !matches!(e.handler, Resolution::Item(_))) {
        *unresolved.entry(handler_text(&root, &e.handler)).or_default() += 1;
    }
    let mut roots: BTreeMap<String, usize> = BTreeMap::new();
    for e in &table.entries {
        *roots.entry(format!("{}:{}", rel(&root, &e.root.0), e.root.1)).or_default() += 1;
    }
    println!("skipped {}:", table.skipped.len());
    for (file, line, reason) in &table.skipped {
        println!("  {}:{line} {reason}", rel(&root, file));
    }
    println!(
        "entries {} (handlers resolved {resolved}, unresolved {unresolved:?}), services {}, skipped {}, roots {roots:?}",
        table.entries.len(),
        table.services,
        table.skipped.len()
    );
    let distinct_routes: std::collections::BTreeSet<&(PathBuf, usize)> =
        table.entries.iter().map(|e| &e.defined_at).collect();
    println!("distinct `.route(` lines {}", distinct_routes.len());
    println!("index {indexed:?}, evaluation {evaluated:?}");

    let find = |method: &str, template: &str| {
        table
            .entries
            .iter()
            .find(|e| e.method == method && e.template == template)
            .unwrap_or_else(|| panic!("no entry {method} {template}"))
    };
    let handlers = "crates/api/api_server_infrastructure/src/handlers/fleet_executor.rs";
    for (last, line) in [("result", 81), ("executing", 66)] {
        let e = find("POST", &format!("/api/v1/fleet-executor/commands/{{commandId}}/{last}"));
        match &e.handler {
            Resolution::Item(item) => assert_eq!((rel(&root, &item.file).as_str(), item.line), (handlers, line)),
            other => panic!("handler of {last}: {other:?}"),
        }
    }
    let dev_login =
        table.entries.iter().find(|e| e.template.ends_with("/auth/dev-login")).expect("an /auth/dev-login entry");
    assert_eq!(dev_login.conditional.as_deref(), Some("dev"));
    assert!(table.entries.iter().all(|e| !rel(&root, &e.defined_at.0).split('/').any(|part| part == "tests")));

    // Cross-check: Rust `@route` tags on handler functions against the route entries whose handler is that function.
    let mut by_handler: BTreeMap<(PathBuf, usize), Vec<&RouteEntry>> = BTreeMap::new();
    for e in &table.entries {
        if let Resolution::Item(item) = &e.handler {
            by_handler.entry((item.file.clone(), item.line)).or_default().push(e);
        }
    }
    let (mut agree, mut disagree, mut not_handler) = (0, 0, 0);
    let mut disagreements = Vec::new();
    for file in index.files() {
        let Some(text) = read_file(file) else { continue };
        if !text.contains("@route") {
            continue;
        }
        let rel_path = file.strip_prefix(&root).unwrap_or(file);
        let owners = owner_spans(&extract_source(file, rel_path, &text));
        for tag in scan_tags(file, &text, SourceLang::Rust, &owners).routes {
            let where_ = format!(
                "{}:{} @route {} {}",
                rel(&root, file),
                tag.line,
                tag.methods.join("|"),
                tag.templates.join("|")
            );
            let TagOwner::Item(owner) = &tag.owner else {
                not_handler += 1;
                println!("  tag not on a route handler (file-level) {where_}");
                continue;
            };
            let routes: Vec<&RouteEntry> = by_handler
                .iter()
                .filter(|((f, line), _)| f == file && (owner.line..=owner.line_end).contains(line))
                .flat_map(|(_, list)| list.iter().copied())
                .collect();
            if routes.is_empty() {
                not_handler += 1;
                println!("  tag not on a route handler (owner `{}`) {where_}", owner.name);
                continue;
            }
            let every = tag.methods.iter().all(|m| {
                tag.templates.iter().all(|t| {
                    let t = RouteTemplate::parse(t);
                    routes.iter().any(|e| e.method == *m && RouteTemplate::parse(&e.template).unifies(&t))
                })
            });
            if every {
                agree += 1;
            } else {
                disagree += 1;
                let shown: Vec<String> = routes.iter().map(|e| format!("{} {}", e.method, e.template)).collect();
                disagreements.push(format!("{where_} vs route {}", shown.join(", ")));
            }
        }
    }
    println!("handler tags agree {agree}, disagree {disagree}, tags not on a route handler {not_handler}");
    for d in &disagreements {
        println!("  disagree {d}");
    }
}
