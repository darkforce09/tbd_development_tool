//! R1, the Rust path resolver: exact expectations on the `r1_workspace` fixture (`expected.toml` beside it), and an
//! ignored real-repository check (`STUDIO_REAL_REPOS`, colon-separated paths).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use studio_parser::analysis::crate_graph::{CrateGraph, TargetKind};
use studio_parser::analysis::rust_resolver::{ItemRef, Resolution, RustIndex, UnresolvedReason};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r1_workspace")
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

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn item_text(root: &Path, r: &ItemRef) -> String {
    format!("item {:?} {} {}:{}", r.kind, r.name, rel(root, &r.file), r.line)
}

fn resolution_text(root: &Path, r: &Resolution) -> String {
    match r {
        Resolution::Item(i) => item_text(root, i),
        Resolution::External(p) => format!("external {p}"),
        Resolution::Ambiguous(list) => format!(
            "ambiguous {}",
            list.iter().map(|i| format!("{}:{}", rel(root, &i.file), i.line)).collect::<Vec<_>>().join(", ")
        ),
        Resolution::Unresolved(UnresolvedReason::Other(text)) => format!("unresolved Other: {text}"),
        Resolution::Unresolved(reason) => format!("unresolved {reason:?}"),
    }
}

fn s<'a>(t: &'a toml::Value, key: &str) -> &'a str {
    t.get(key).and_then(|v| v.as_str()).unwrap_or_else(|| panic!("expected.toml: missing `{key}` in {t}"))
}

fn strings(t: &toml::Value, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// Compares two sorted lists and fails with the missing and extra lines.
fn assert_same(what: &str, mut expected: Vec<String>, mut actual: Vec<String>) {
    expected.sort();
    actual.sort();
    let missing: Vec<&String> = expected.iter().filter(|e| !actual.contains(e)).collect();
    let extra: Vec<&String> = actual.iter().filter(|a| !expected.contains(a)).collect();
    assert!(
        missing.is_empty() && extra.is_empty() && expected.len() == actual.len(),
        "{what}: missing (recall) {missing:#?}\nextra (precision) {extra:#?}"
    );
}

#[test]
fn r1_fixture_resolves_exactly() {
    let root = fixture_root();
    let expected: toml::Table = read_file(&root.join("expected.toml")).unwrap().parse().unwrap();
    let manifests = package_manifests(&root);
    let graph = CrateGraph::from_manifests(&root, &manifests);

    // The crate graph.
    let crates: Vec<String> = graph
        .crates
        .iter()
        .map(|c| {
            let deps: Vec<&str> = c.deps.iter().map(|(n, _)| n.as_str()).collect();
            let external: Vec<&str> = c.external_deps.iter().map(|(n, _)| n.as_str()).collect();
            format!(
                "{} {} {:?} {} edition={} deps={deps:?} external={external:?} gated={:?}",
                c.package,
                c.name,
                c.kind,
                rel(&root, &c.root_file),
                c.edition,
                c.gated_deps
            )
        })
        .collect();
    let expected_crates: Vec<String> = expected["crate"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{} {} {} {} edition={} deps={:?} external={:?} gated={:?}",
                s(c, "package"),
                s(c, "name"),
                s(c, "kind"),
                s(c, "root"),
                c.get("edition").and_then(|e| e.as_integer()).expect("expected.toml: crate `edition`"),
                strings(c, "deps"),
                strings(c, "external"),
                strings(c, "gated")
            )
        })
        .collect();
    assert_eq!(crates, expected_crates, "crate graph (order included)");
    for c in &graph.crates {
        for (_, dep) in &c.deps {
            assert!(matches!(graph.crates[*dep].kind, TargetKind::Lib | TargetKind::ProcMacro));
        }
    }

    let index = RustIndex::build(&graph, &read_file);
    let files: Vec<&Path> = index.files().collect();
    assert_eq!(index.module_count() as i64, expected["index"]["modules"].as_integer().unwrap(), "modules");
    assert_eq!(files.len() as i64, expected["index"]["files"].as_integer().unwrap(), "files");

    // Every call site, exactly.
    let mut actual_calls = Vec::new();
    for file in &files {
        for (site, res) in index.resolved_calls(file) {
            let caller = site.caller.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            let method = if site.is_method { " (method)" } else { "" };
            actual_calls.push(format!(
                "{}:{} {} `{}`{method} => {}",
                rel(&root, file),
                site.line,
                caller,
                site.path.join("::"),
                resolution_text(&root, &res)
            ));
        }
    }
    let expected_calls: Vec<String> = expected["call"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let method = if c.get("method").and_then(|m| m.as_bool()) == Some(true) { " (method)" } else { "" };
            let path = s(c, "path").trim_start_matches("::");
            format!("{} {} `{path}`{method} => {}", s(c, "at"), s(c, "caller"), s(c, "expect"))
        })
        .collect();
    assert_same("call sites", expected_calls, actual_calls);

    // Paths resolved directly.
    let mut expected_paths = Vec::new();
    let mut actual_paths = Vec::new();
    for p in expected["path"].as_array().unwrap() {
        let file = root.join(s(p, "file"));
        let scope = index.module_of_file(&file).unwrap_or_else(|| panic!("no module for {}", file.display()));
        let text = s(p, "path");
        let segs: Vec<&str> = match text.strip_prefix("::") {
            Some(rest) => std::iter::once("::").chain(rest.split("::")).collect(),
            None => text.split("::").collect(),
        };
        expected_paths.push(format!("{} `{text}` => {}", s(p, "file"), s(p, "expect")));
        actual_paths.push(format!(
            "{} `{text}` => {}",
            s(p, "file"),
            resolution_text(&root, &index.resolve(scope, &segs))
        ));
    }
    assert_same("paths", expected_paths, actual_paths);

    // Functions of a file: free functions, methods, inline modules; test-only code left out.
    let fns: Vec<String> = index
        .functions_in(&root.join("alpha/src/lib.rs"))
        .iter()
        .map(|f| format!("{}:{}-{}", f.name, f.line, f.line_end))
        .collect();
    assert_eq!(fns, vec!["h:16-16", "h:18-18", "new:27-29", "make:31-33", "entry:36-51"]);
}

/// The S7 exit chain on a real workspace (TBD-Reforger): run with `--ignored` and `STUDIO_REAL_REPOS` set.
#[test]
#[ignore]
fn r1_real_repo_resolves_the_exit_chain() {
    let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else {
        eprintln!("STUDIO_REAL_REPOS unset: skipped");
        return;
    };
    let target = "crates/api/api_server_infrastructure/src/routes.rs";
    let Some(root) = repos.split(':').map(PathBuf::from).find(|r| r.join(target).is_file()) else {
        eprintln!("no repository in STUDIO_REAL_REPOS has {target}: skipped");
        return;
    };

    let t0 = Instant::now();
    let manifests = package_manifests(&root);
    let graph = CrateGraph::from_manifests(&root, &manifests);
    let graph_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let t1 = Instant::now();
    let index = RustIndex::build(&graph, &read_file);
    let build_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let files: Vec<PathBuf> = index.files().map(Path::to_path_buf).collect();

    let t2 = Instant::now();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0;
    for file in &files {
        for (site, res) in index.resolved_calls(file) {
            total += 1;
            let key = match res {
                Resolution::Item(_) => "Item".to_string(),
                Resolution::External(_) => "External".to_string(),
                Resolution::Ambiguous(_) => "Ambiguous".to_string(),
                Resolution::Unresolved(UnresolvedReason::Other(text)) => format!("Unresolved(Other: {text})"),
                Resolution::Unresolved(reason) => format!("Unresolved({reason:?})"),
            };
            *counts.entry(key.clone()).or_default() += 1;
            // `R1_SAMPLES=<text>` lists the call sites whose category contains the text.
            if std::env::var("R1_SAMPLES").is_ok_and(|k| key.contains(&k)) {
                println!("SAMPLE {key} {}:{} {}", rel(&root, file), site.line, site.path.join("::"));
            }
        }
    }
    let resolve_ms = t2.elapsed().as_secs_f64() * 1000.0;
    println!(
        "R1 on {}: {} manifests, {} crates, {} modules, {} files; graph {graph_ms:.1} ms, index build {build_ms:.1} ms, \
         resolving {total} call sites {resolve_ms:.1} ms",
        root.display(),
        manifests.len(),
        graph.crates.len(),
        index.module_count(),
        files.len()
    );
    for (key, n) in &counts {
        println!("  {key}: {n}");
    }

    let check = |file: &str, path: &[&str], want_file: &str, want_line: usize| {
        let scope = index.module_of_file(&root.join(file)).unwrap_or_else(|| panic!("{file} is not in the index"));
        match index.resolve(scope, path) {
            Resolution::Item(item) => {
                assert_eq!(
                    (rel(&root, &item.file), item.line),
                    (want_file.to_string(), want_line),
                    "{}",
                    path.join("::")
                )
            }
            other => panic!("{file}: {} resolved to {other:?}", path.join("::")),
        }
    };
    check(
        target,
        &["handlers", "fleet_executor", "finish_fleet_command"],
        "crates/api/api_server_infrastructure/src/handlers/fleet_executor.rs",
        81,
    );
    check("crates/api/api_server/src/bin/api_server.rs", &["router"], "crates/api/api_server/src/router.rs", 73);
    check(
        "crates/api/api_server/src/router.rs",
        &["api_server_infrastructure", "routes"],
        "crates/api/api_server_infrastructure/src/routes.rs",
        23,
    );
    // The same chain as call sites.
    let bin = root.join("crates/api/api_server/src/bin/api_server.rs");
    let router_call = index
        .resolved_calls(&bin)
        .into_iter()
        .find(|(site, _)| site.path == ["router"])
        .expect("the `router(state)` call");
    assert!(
        matches!(&router_call.1, Resolution::Item(i) if i.line == 73 && i.file.ends_with("api_server/src/router.rs")),
        "{router_call:?}"
    );
}
