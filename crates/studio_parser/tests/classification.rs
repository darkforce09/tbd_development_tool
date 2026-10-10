//! The classification table: `docs/CLASSIFICATION.md` mirrors `classify::RULES` and
//! `classify::HEAVY_MARKERS` row for row, and the fixture (`fixtures/classification/`) gets
//! exactly the classes, rule rows and heavy folders in its `expected.toml`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use studio_parser::classify::{self, ClassInputs, FileClass, HEAVY_MARKERS, RULES};
use studio_parser::tree::{self, ProjectTree, ScanOptions};

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The cells of every table row under the heading `## <section>` (header and rule line skipped).
fn table(doc: &str, section: &str) -> Vec<Vec<String>> {
    let heading = format!("## {section}\n");
    let start = doc.find(&heading).unwrap_or_else(|| panic!("no section {section}")) + heading.len();
    let body = &doc[start..doc[start..].find("\n## ").map_or(doc.len(), |e| start + e)];
    body.lines()
        .filter(|l| l.starts_with('|'))
        .skip(2)
        .map(|l| l.trim_matches('|').split(" | ").map(|c| c.trim().trim_matches('`').to_string()).collect())
        .collect()
}

#[test]
fn the_doc_mirrors_the_rules_and_markers_row_for_row() {
    let doc =
        std::fs::read_to_string(manifest_dir().join("../../docs/CLASSIFICATION.md")).expect("docs/CLASSIFICATION.md");
    let rules: Vec<Vec<String>> = RULES
        .iter()
        .enumerate()
        .map(|(i, r)| {
            vec![
                (i + 1).to_string(),
                r.kind.label().to_string(),
                r.pattern.to_string(),
                r.class.label().to_string(),
                r.basis.to_string(),
                r.note.to_string(),
            ]
        })
        .collect();
    assert_eq!(table(&doc, "Rules"), rules);
    let markers: Vec<Vec<String>> = HEAVY_MARKERS
        .iter()
        .map(|m| vec![m.file.to_string(), m.reason.label().to_string(), m.tool.to_string(), m.source.to_string()])
        .collect();
    assert_eq!(table(&doc, "Heavy folders"), markers);
    let classes: Vec<String> = table(&doc, "Classes").into_iter().map(|row| row[0].clone()).collect();
    assert_eq!(classes, FileClass::ALL.map(|c| c.label().to_string()));
}

#[test]
fn every_tool_convention_cites_its_source() {
    let doc = std::fs::read_to_string(manifest_dir().join("../../docs/CLASSIFICATION.md")).unwrap();
    let cited: Vec<String> =
        table(&doc, "Tool conventions and their sources").into_iter().map(|r| r[0].clone()).collect();
    for rule in RULES.iter().filter(|r| r.kind == classify::RuleKind::Convention) {
        assert!(cited.contains(&rule.basis.to_string()), "{} has no source", rule.basis);
    }
}

#[derive(serde::Deserialize)]
struct Expected {
    setup: Setup,
    markers: BTreeMap<String, String>,
    heavy: BTreeMap<String, String>,
    classes: BTreeMap<String, ExpectedClass>,
}

#[derive(serde::Deserialize)]
struct Setup {
    gitignore: String,
    gitattributes: String,
}

#[derive(serde::Deserialize, Debug, PartialEq)]
struct ExpectedClass {
    class: String,
    rule: usize,
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in walkdir::WalkDir::new(from).sort_by_file_name() {
        let entry = entry.unwrap();
        let target = to.join(entry.path().strip_prefix(from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn slash(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// The Linguist attribute sets git reports for every listed file (what the viewer passes in).
fn linguist_sets(root: &Path, tree: &ProjectTree, inputs: &mut ClassInputs) {
    let paths: String = tree.files.iter().map(|f| slash(&f.rel) + "\0").collect();
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-attr", "--stdin", "-z", "linguist-generated", "linguist-vendored", "linguist-documentation"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(paths.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap().stdout;
    let fields: Vec<&str> = out.split(|&b| b == 0).filter_map(|f| std::str::from_utf8(f).ok()).collect();
    for c in fields.chunks(3).filter(|c| c.len() == 3 && matches!(c[2], "set" | "true")) {
        let set = match c[1] {
            "linguist-generated" => &mut inputs.generated,
            "linguist-vendored" => &mut inputs.vendored,
            _ => &mut inputs.documentation,
        };
        set.insert(c[0].to_string());
    }
}

#[test]
fn the_fixture_gets_exactly_its_expected_classes_and_heavy_folders() {
    let fixture = manifest_dir().join("tests/fixtures/classification");
    let expected: Expected = toml::from_str(&std::fs::read_to_string(fixture.join("expected.toml")).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    copy_tree(&fixture.join("tree"), &root);
    std::fs::write(root.join(".gitignore"), &expected.setup.gitignore).unwrap();
    std::fs::write(root.join(".gitattributes"), &expected.setup.gitattributes).unwrap();
    for (folder, marker) in &expected.markers {
        let folder = root.join(folder);
        std::fs::create_dir_all(&folder).unwrap();
        let content: &[u8] = if marker == "CACHEDIR.TAG" { classify::CACHEDIR_SIGNATURE } else { b"" };
        std::fs::write(folder.join(marker), content).unwrap();
        std::fs::write(folder.join("inside.js"), "module.exports = 1;\n").unwrap();
    }
    let git_ok = Command::new("git").arg("-C").arg(&root).args(["init", "-q"]).status().is_ok_and(|s| s.success());
    if !git_ok {
        return;
    }

    let tree = tree::scan_tree(&root, ScanOptions::default());
    let heavy: BTreeMap<String, String> =
        tree.heavy_dirs().map(|(_, d, i)| (slash(&d.rel), i.reason.label().to_string())).collect();
    assert_eq!(heavy, expected.heavy);

    let mut inputs = tree::class_inputs(&tree);
    // The scan classifies with what it knows; the same table with the same inputs agrees.
    for f in &tree.files {
        let (class, rule) = classify::classify(&slash(&f.rel), &inputs);
        assert_eq!((f.class, f.rule), (class, rule), "{:?}", f.rel);
    }
    linguist_sets(&root, &tree, &mut inputs);
    let row = |rule: &classify::Rule| RULES.iter().position(|r| std::ptr::eq(r, rule)).unwrap() + 1;
    let classes: BTreeMap<String, ExpectedClass> = tree
        .files
        .iter()
        .map(|f| {
            let rel = slash(&f.rel);
            let (class, rule) = classify::classify(&rel, &inputs);
            (rel, ExpectedClass { class: class.label().to_string(), rule: row(rule) })
        })
        .collect();
    assert_eq!(classes, expected.classes);
    let mut covered: Vec<usize> = expected.classes.values().map(|c| c.rule).collect();
    covered.sort();
    covered.dedup();
    assert_eq!(covered, (1..=RULES.len()).collect::<Vec<_>>(), "every rule row is covered");
    assert_eq!(expected.markers.len(), HEAVY_MARKERS.len(), "every marker is covered");
}

#[test]
fn map_cards_count_files_by_class_and_show_a_conventions_basis() {
    let dir = tempfile::tempdir().unwrap();
    let write = |rel: &str, text: &str| {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write("src/lib.rs", "pub fn a() {}\n");
    write("tests/it.rs", "#[test]\nfn it() {}\n");
    write("tests/data.json", "{}\n");
    write("other/tests/plain.rs", "fn plain() {}\n");
    let (graph, _) = studio_parser::load_rust_project(dir.path()).unwrap();
    let subtitle = |folder: &str| {
        let id = studio_parser::folder_cluster_id(Path::new(folder));
        graph.clusters.iter().find(|c| c.id == id).and_then(|c| c.subtitle.clone()).unwrap()
    };
    assert_eq!(subtitle("tests"), "2 files · 1 test · 2 test files (tool convention: Cargo)");
    assert_eq!(subtitle("other/tests"), "1 file · 1 code", "a bare tests/ folder is code");
    assert_eq!(subtitle("."), "5 files · 1 test · 2 code, 2 test files (tool convention: Cargo), 1 config");
}
