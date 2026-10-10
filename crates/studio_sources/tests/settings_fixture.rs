//! `read_settings` on `tests/fixtures/settings`, checked against its `expected.toml`: every sheet's facts with
//! their lines, the `.gitignore` groups and rules, and every schema's shape.

use std::path::{Path, PathBuf};

use studio_sources::{read_settings, FactSheet, IgnoreFile, SchemaShape};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/settings")
}

fn expected() -> toml::Table {
    std::fs::read_to_string(fixture().join("expected.toml")).unwrap().parse().unwrap()
}

fn strings(value: &toml::Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap().display().to_string()
}

/// Every file under `root` with `extension`, sorted.
fn files_with(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == extension))
        .map(|e| e.into_path())
        .collect();
    found.sort();
    found
}

/// A sheet as `expected.toml` writes it: a table with its file, summary and one line per fact.
fn sheet(root: &Path, s: &FactSheet) -> toml::Value {
    let facts = s
        .sections
        .iter()
        .flat_map(|(section, facts)| {
            facts.iter().map(move |f| format!("[{section}] {} = {} @{}", f.key, f.value, f.line))
        })
        .map(toml::Value::String)
        .collect();
    let mut t = toml::Table::new();
    t.insert("title".into(), s.title.clone().into());
    t.insert("file".into(), rel(root, &s.file).into());
    t.insert("summary".into(), s.summary.clone().into());
    t.insert("facts".into(), toml::Value::Array(facts));
    toml::Value::Table(t)
}

fn gitignore(file: &IgnoreFile) -> (Vec<String>, Vec<String>) {
    let groups = file.groups.iter().map(|g| format!("{} @{}: {} patterns", g.title, g.line, g.rules.len())).collect();
    let rules = file
        .groups
        .iter()
        .flat_map(|g| {
            g.rules.iter().map(move |r| {
                let note = r.note.as_ref().map(|n| format!(" ({n})")).unwrap_or_default();
                format!("{} | {} @{}{note}", g.title, r.pattern, r.line)
            })
        })
        .collect();
    (groups, rules)
}

fn schema(root: &Path, s: &SchemaShape) -> String {
    format!(
        "{} title={} kind={} properties={} required={} definitions={} refs={} formats={}",
        rel(root, &s.file),
        s.title.as_deref().unwrap_or("-"),
        s.kind,
        s.properties,
        s.required,
        s.definitions,
        s.refs,
        s.formats.join(",")
    )
}

#[test]
fn the_fixture_settings_read_as_expected() {
    let root = fixture();
    let expected = expected();
    let manifests: Vec<PathBuf> = files_with(&root, "toml").into_iter().filter(|f| f.ends_with("Cargo.toml")).collect();
    assert_eq!(manifests.len(), 4);
    let settings = read_settings(&root, &manifests, &files_with(&root, "json"));

    let sheets: Vec<toml::Value> = settings.sheets.iter().map(|s| sheet(&root, s)).collect();
    let wanted = expected["sheet"].as_array().unwrap();
    for (got, want) in sheets.iter().zip(wanted) {
        assert_eq!(got, want);
    }
    assert_eq!(&sheets, wanted);

    let ignore = settings.gitignore.as_ref().expect("a .gitignore");
    assert_eq!(rel(&root, &ignore.file), ".gitignore");
    assert_eq!(ignore.patterns() as i64, expected["gitignore"]["patterns"].as_integer().unwrap());
    let (groups, rules) = gitignore(ignore);
    assert_eq!(groups, strings(&expected["gitignore"]["groups"]));
    assert_eq!(rules, strings(&expected["gitignore"]["rules"]));

    let schemas: Vec<String> = settings.schemas.iter().map(|s| schema(&root, s)).collect();
    assert_eq!(schemas, strings(&expected["schemas"]));
}
