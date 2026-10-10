//! E2: exact palette result orders on a fixed fixture of files, folders, symbols, commands and
//! source data (`fixtures/palette/entries.txt`).

use std::path::Path;
use std::sync::Arc;

use studio_parser::search_index::{path_key, symbol_key};
use studio_parser::{Entry, EntryKind, Group, Query, SearchIndex, Segment};

fn kind(label: &str) -> EntryKind {
    *EntryKind::ALL.iter().find(|k| k.label().eq_ignore_ascii_case(label)).unwrap_or_else(|| panic!("kind {label}"))
}

/// The fixture's entries, in file order.
fn fixture() -> Vec<Entry> {
    let text =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/palette/entries.txt"))
            .expect("fixture");
    let lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    lines
        .enumerate()
        .map(|(i, line)| {
            let f: Vec<&str> = line.split('|').map(str::trim).collect();
            let [k, name, detail, path, line] = f[..] else { panic!("bad line {line}") };
            let kind = kind(k);
            let path = (!path.is_empty()).then(|| path.to_string());
            let p = path.clone().unwrap_or_default();
            let base = p.rsplit('/').next().unwrap_or_default().to_string();
            let (name, detail, key) = match kind {
                EntryKind::File => (base, p.clone(), path_key(&p)),
                EntryKind::Folder => (base, format!("{p} · {detail}"), path_key(&p)),
                EntryKind::Symbol => (name.to_string(), format!("{p}:{line}"), symbol_key(&p, i)),
                _ => (name.to_string(), detail.to_string(), i as u64),
            };
            Entry { kind, name, detail, path, line: line.parse().ok(), key }
        })
        .collect()
}

fn index() -> SearchIndex {
    let mut index = SearchIndex::default();
    index.push(Arc::new(Segment::new(fixture())));
    index
}

/// D4 display caps.
fn caps(kind: EntryKind) -> usize {
    match kind {
        EntryKind::File | EntryKind::Symbol => 8,
        _ => 5,
    }
}

/// One line per group: `Label total: hit | hit`, a hit being its path (files, folders), name and
/// line (symbols) or name.
fn render(index: &SearchIndex, groups: &[Group]) -> Vec<String> {
    groups
        .iter()
        .map(|g| {
            let hits: Vec<String> = g
                .hits
                .iter()
                .map(|h| {
                    let e = index.entry(h);
                    match e.kind {
                        EntryKind::File | EntryKind::Folder => e.path.clone().unwrap(),
                        EntryKind::Symbol => format!("{}:{}", e.name, e.line.unwrap()),
                        _ => e.name.clone(),
                    }
                })
                .collect();
            format!("{} {}: {}", g.kind.label(), g.total, hits.join(" | "))
        })
        .collect()
}

fn results(query: &str) -> Vec<String> {
    let index = index();
    render(&index, &index.search(&Query::parse(query), caps))
}

#[test]
fn objhud_finds_objective_hud_files_and_symbols() {
    assert_eq!(
        results("objhud"),
        [
            "File 4: src/ui/hud/objective_hud.ts | src/ui/hud/ObjectiveHud.tsx | scripts/Game/UI/HUD/SCR_ObjectiveHUDComponent.c | scripts/Game/UI/HUD/SCR_ObjectiveHudManager.c",
            "Symbol 5: objective_hud:4 | ObjectiveHud:8 | SCR_ObjectiveHUDComponent:5 | SCR_ObjectiveHUDComponent::OnInit:12 | SCR_ObjectiveHudManager:3",
            "Branch 1: feature/objective-hud",
        ]
    );
}

#[test]
fn a_path_query_finds_the_file_under_that_folder() {
    assert_eq!(
        results("ui/hud"),
        [
            "File 6: crates/ui/hud.rs | crates/ui/hud_layout.rs | src/ui/hud/ObjectiveHud.tsx | src/ui/hud/objective_hud.ts | scripts/Game/UI/HUD/SCR_ObjectiveHUDComponent.c | scripts/Game/UI/HUD/SCR_ObjectiveHudManager.c",
            "Symbol 7: HudLayout:5 | render_hud:10 | ObjectiveHud:8 | objective_hud:4 | SCR_ObjectiveHUDComponent:5 | SCR_ObjectiveHUDComponent::OnInit:12 | SCR_ObjectiveHudManager:3",
        ]
    );
}

#[test]
fn case_never_changes_the_order() {
    assert_eq!(results("HUD"), results("hud"));
    assert_eq!(
        results("HUD"),
        [
            "File 7: crates/game/HUD.rs | crates/ui/hud.rs | crates/ui/hud_layout.rs | src/ui/hud/objective_hud.ts | src/ui/hud/ObjectiveHud.tsx | scripts/Game/UI/HUD/SCR_ObjectiveHUDComponent.c | scripts/Game/UI/HUD/SCR_ObjectiveHudManager.c",
            "Symbol 7: HudLayout:5 | render_hud:10 | objective_hud:4 | ObjectiveHud:8 | SCR_ObjectiveHUDComponent:5 | SCR_ObjectiveHUDComponent::OnInit:12 | SCR_ObjectiveHudManager:3",
            "Setting 1: hud.scale",
            "Branch 1: feature/objective-hud",
        ]
    );
}

#[test]
fn at_searches_symbols_only() {
    assert_eq!(
        results("@parse"),
        ["Symbol 6: parse:3 | ParseError:40 | Parser:20 | parse_file:12 | extract_source:30 | load_folder_contents:78",]
    );
    assert_eq!(
        results("@hud"),
        [
            "Symbol 7: HudLayout:5 | render_hud:10 | objective_hud:4 | ObjectiveHud:8 | SCR_ObjectiveHUDComponent:5 | SCR_ObjectiveHUDComponent::OnInit:12 | SCR_ObjectiveHudManager:3",
        ]
    );
}

#[test]
fn greater_than_searches_commands_and_tools_only() {
    assert_eq!(results(">files"), ["Command 2: Files | Find in files",]);
    assert_eq!(results("> test"), ["Tool 1: cargo test",]);
}

#[test]
fn a_subsequence_runs_across_underscores() {
    assert_eq!(results("loadfc"), ["Symbol 1: load_folder_contents:78",]);
    assert_eq!(results("rndhud"), ["Symbol 1: render_hud:10",]);
}

#[test]
fn an_empty_query_and_a_query_that_matches_nothing_give_no_groups() {
    assert!(results("").is_empty());
    assert!(results("   ").is_empty());
    assert!(results("@").is_empty());
    assert!(results("zzqx").is_empty());
}

#[test]
fn non_ascii_names_and_paths_match() {
    assert_eq!(results("größe"), ["File 1: docs/Größe.md", "Symbol 1: GrößeBerechnen:2",]);
    assert_eq!(results("été/notes"), ["File 1: docs/été/notes.md",]);
}

#[test]
fn whitespace_in_the_query_is_ignored() {
    assert_eq!(results("objective hud"), results("objectivehud"));
    assert_eq!(
        results("objective hud"),
        [
            "File 4: src/ui/hud/ObjectiveHud.tsx | scripts/Game/UI/HUD/SCR_ObjectiveHUDComponent.c | scripts/Game/UI/HUD/SCR_ObjectiveHudManager.c | src/ui/hud/objective_hud.ts",
            "Symbol 5: ObjectiveHud:8 | SCR_ObjectiveHUDComponent:5 | SCR_ObjectiveHUDComponent::OnInit:12 | SCR_ObjectiveHudManager:3 | objective_hud:4",
            "Branch 1: feature/objective-hud",
        ]
    );
}

#[test]
fn heavy_folders_are_found_as_one_entry() {
    assert_eq!(results("target"), ["File 1: target",]);
    assert_eq!(results("node_mod"), ["File 1: web/node_modules",]);
}

#[test]
fn every_kind_is_searchable() {
    assert_eq!(results("cargo"), ["File 2: Cargo.toml | crates/ui/Cargo.toml", "Tool 1: cargo test",]);
    assert_eq!(results("main"), ["Branch 1: main",]);
    assert_eq!(results("api/files"), ["Route 1: GET /api/files",]);
    assert_eq!(
        results("otter"),
        [
            "File 2: scripts/Game/Objectives/SCR_ObjectiveManager.c | crates/studio_parser/src/extractor/parse.rs",
            "Symbol 1: extract_source:30",
            "Session 1: brave-otter",
        ]
    );
}

#[test]
fn exact_and_prefix_matches_rank_first() {
    assert_eq!(
        results("parse"),
        [
            "File 5: crates/studio_parser/src/extractor/parse.rs | crates/studio_parser/src/parse.rs | crates/studio_parser/src/parser.rs | crates/studio_parser/src/lib.rs | scripts/Game/Objectives/SCR_ObjectiveBase.c",
            "Symbol 6: parse:3 | ParseError:40 | Parser:20 | parse_file:12 | extract_source:30 | load_folder_contents:78",
            "Route 1: POST /api/parse",
            "Flow 1: Parse project",
        ]
    );
    assert_eq!(results("readme"), ["File 1: README.md",]);
}
