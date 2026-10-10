//! `@route` / `@contract` tags and contract resolution on the `fixtures/tags` project, checked against its
//! `expected.toml`, plus an ignored real-repo check.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use studio_parser::analysis::contracts::{resolve_contract, ContractFiles, ContractResolution};
use studio_parser::analysis::tags::{owner_spans, scan_tags, FileTags, TagOwner};
use studio_parser::extractor::{detect_language, extract_source, SourceLang};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tags")
}

/// Every file under `root` (skipping `.git` and `target*` folders), project-relative, sorted.
fn project_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && e.depth() > 0 && (name == ".git" || name.starts_with("target")))
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect();
    files.sort();
    files
}

/// Tags of every code file, keyed by relative path.
fn scan_project(root: &Path, files: &[PathBuf]) -> BTreeMap<PathBuf, FileTags> {
    files
        .par_iter()
        .filter_map(|rel| {
            let abs = root.join(rel);
            let text = std::fs::read_to_string(&abs).ok()?;
            let lang = detect_language(&abs, &text);
            if matches!(lang, SourceLang::Markdown | SourceLang::Other) {
                return None;
            }
            let owners = owner_spans(&extract_source(&abs, rel, &text));
            let tags = scan_tags(rel, &text, lang, &owners);
            Some((rel.clone(), tags))
        })
        .collect()
}

fn owner_name(owner: &TagOwner) -> String {
    match owner {
        TagOwner::Item(span) => span.name.clone(),
        TagOwner::File => "file".to_string(),
    }
}

fn str_list(value: &toml::Value) -> Vec<String> {
    value.as_array().expect("list").iter().map(|v| v.as_str().expect("string").to_string()).collect()
}

#[test]
fn fixture_tags_and_contracts_match_expected() {
    let root = fixture_root();
    let expected: toml::Table = std::fs::read_to_string(root.join("expected.toml")).unwrap().parse().unwrap();
    let files: Vec<PathBuf> = project_files(&root).into_iter().filter(|f| f != Path::new("expected.toml")).collect();
    let tags = scan_project(&root, &files);
    let contract_files = ContractFiles::new(files.iter().cloned());
    let read = |p: &Path| std::fs::read_to_string(root.join(p)).ok();

    // Routes: (file, line, methods, templates, owner), both ways.
    let mut got_routes: Vec<String> = tags
        .values()
        .flat_map(|t| &t.routes)
        .map(|r| format!("{}:{} {:?} {:?} {}", r.file.display(), r.line, r.methods, r.templates, owner_name(&r.owner)))
        .collect();
    got_routes.sort();
    let mut want_routes: Vec<String> = expected["route"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            format!(
                "{}:{} {:?} {:?} {}",
                r["file"].as_str().unwrap(),
                r["line"].as_integer().unwrap(),
                str_list(&r["methods"]),
                str_list(&r["templates"]),
                r["owner"].as_str().unwrap()
            )
        })
        .collect();
    want_routes.sort();
    assert_eq!(got_routes, want_routes, "route tags (precision and recall)");

    // Contracts with their resolution.
    let mut got_contracts: Vec<String> = tags
        .values()
        .flat_map(|t| &t.contracts)
        .map(|c| {
            let resolution = match resolve_contract(c, &contract_files, &read) {
                ContractResolution::Proven { file, line, .. } => format!("proven {}:{line}", file.display()),
                ContractResolution::Possible(paths) => {
                    let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                    format!("possible {paths:?}")
                }
                ContractResolution::Unresolved(reason) => format!("unresolved {reason}"),
            };
            format!(
                "{}:{} {} {:?} partial={} {} -> {resolution}",
                c.file.display(),
                c.line,
                c.file_ref,
                c.pointer,
                c.partial,
                owner_name(&c.owner)
            )
        })
        .collect();
    got_contracts.sort();
    let mut want_contracts: Vec<String> = expected["contract"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let resolution = match c["tier"].as_str().unwrap() {
                "proven" => {
                    format!("proven {}:{}", c["target"].as_str().unwrap(), c["target_line"].as_integer().unwrap())
                }
                "possible" => format!("possible {:?}", str_list(&c["candidates"])),
                "unresolved" => format!("unresolved {}", c["reason"].as_str().unwrap()),
                other => panic!("unknown tier {other}"),
            };
            format!(
                "{}:{} {} {:?} partial={} {} -> {resolution}",
                c["file"].as_str().unwrap(),
                c["line"].as_integer().unwrap(),
                c["file_ref"].as_str().unwrap(),
                c.get("pointer").map(|p| p.as_str().unwrap().to_string()),
                c["partial"].as_bool().unwrap(),
                c["owner"].as_str().unwrap()
            )
        })
        .collect();
    want_contracts.sort();
    assert_eq!(got_contracts, want_contracts, "contract tags and resolutions (precision and recall)");

    let mut got_malformed: Vec<String> = tags
        .iter()
        .flat_map(|(file, t)| t.malformed.iter().map(move |(line, _)| format!("{}:{line}", file.display())))
        .collect();
    got_malformed.sort();
    let mut want_malformed: Vec<String> = expected["malformed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| format!("{}:{}", m["file"].as_str().unwrap(), m["line"].as_integer().unwrap()))
        .collect();
    want_malformed.sort();
    assert_eq!(got_malformed, want_malformed, "malformed tags");

    assert!(!tags.contains_key(Path::new("docs/api.md")), "Markdown is never scanned");
}

/// Prints the tag counts of TBD-Reforger and checks the fleet sender tags. Run with
/// `STUDIO_REAL_REPOS=/path/to/TBD-Reforger cargo test -p studio_parser --test tags -- --ignored --nocapture`.
#[test]
#[ignore]
fn real_repo_tags() {
    let Ok(repos) = std::env::var("STUDIO_REAL_REPOS") else { return };
    let Some(root) = repos.split(':').map(PathBuf::from).find(|p| p.ends_with("TBD-Reforger")) else { return };
    let started = std::time::Instant::now();
    let files = project_files(&root);
    let tags = scan_project(&root, &files);
    let contract_files = ContractFiles::new(files.iter().cloned());
    let read = |p: &Path| std::fs::read_to_string(root.join(p)).ok();

    let routes: usize = tags.values().map(|t| t.routes.len()).sum();
    let contracts: usize = tags.values().map(|t| t.contracts.len()).sum();
    let malformed: usize = tags.values().map(|t| t.malformed.len()).sum();
    let owned = |owner: &TagOwner| matches!(owner, TagOwner::Item(_));
    let routes_owned = tags.values().flat_map(|t| &t.routes).filter(|r| owned(&r.owner)).count();
    let contracts_owned = tags.values().flat_map(|t| &t.contracts).filter(|c| owned(&c.owner)).count();
    let partial = tags.values().flat_map(|t| &t.contracts).filter(|c| c.partial).count();
    let mut tiers: BTreeMap<&str, usize> = BTreeMap::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for contract in tags.values().flat_map(|t| &t.contracts) {
        match resolve_contract(contract, &contract_files, &read) {
            ContractResolution::Proven { .. } => *tiers.entry("proven").or_default() += 1,
            ContractResolution::Possible(_) => *tiers.entry("possible").or_default() += 1,
            ContractResolution::Unresolved(reason) => {
                *tiers.entry("unresolved").or_default() += 1;
                *reasons.entry(reason).or_default() += 1;
            }
        }
    }
    println!("files {} scanned {} in {:?}", files.len(), tags.len(), started.elapsed());
    println!("route tags {routes} (owned by an item {routes_owned}), contract tags {contracts} (owned {contracts_owned}), malformed {malformed}");
    println!("contract resolutions {tiers:?}, partial {partial}");
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (reason, n) in reasons.iter().take(12) {
        println!("  unresolved {n}x {reason}");
    }
    for (file, t) in &tags {
        for (line, reason) in &t.malformed {
            println!("  malformed {}:{line} {reason}", file.display());
        }
    }

    let sender = Path::new("mod/tbd-framework/Scripts/Game/TBD/API/FleetCommands/TBD_FleetCommandExecution.c");
    let file_tags = &tags[sender];
    for (line, last) in [(112, "executing"), (113, "result")] {
        let tag = file_tags.routes.iter().find(|r| r.line == line).unwrap_or_else(|| panic!("route tag at :{line}"));
        assert_eq!(tag.methods, ["POST"]);
        assert_eq!(tag.templates, [format!("/api/v1/fleet-executor/commands/{{id}}/{last}")]);
        assert_eq!(owner_name(&tag.owner), "SendReport", "owner of :{line}");
    }
    let contract = file_tags
        .contracts
        .iter()
        .find(|c| c.pointer.as_deref() == Some("/definitions/ExecutionStart"))
        .expect("ExecutionStart contract tag");
    assert_eq!(owner_name(&contract.owner), "SendReport");
    let schema = Path::new("contracts/definitions/fleet-command.schema.json");
    let key_line = std::fs::read_to_string(root.join(schema))
        .unwrap()
        .lines()
        .position(|l| l.trim_start().starts_with("\"ExecutionStart\":"))
        .expect("ExecutionStart key")
        + 1;
    println!("ExecutionStart key at {}:{key_line}", schema.display());
    assert_eq!(
        resolve_contract(contract, &contract_files, &read),
        ContractResolution::Proven {
            file: schema.to_path_buf(),
            pointer: Some("/definitions/ExecutionStart".to_string()),
            line: key_line
        }
    );
}
