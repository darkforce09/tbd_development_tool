//! Settings: the files that configure how a project builds, formats, lints and is versioned,
//! read into what they say, each fact with the line it comes from. Nothing is written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A `.gitignore` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRuleLine {
    pub pattern: String,
    pub line: usize,
    /// The comment lines directly above it.
    pub note: Option<String>,
}

/// A group of rules under one heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreGroup {
    pub title: String,
    pub line: usize,
    pub rules: Vec<IgnoreRuleLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreFile {
    pub file: PathBuf,
    pub groups: Vec<IgnoreGroup>,
}

impl IgnoreFile {
    pub fn patterns(&self) -> usize {
        self.groups.iter().map(|g| g.rules.len()).sum()
    }
}

/// A `key = value` fact with its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// One settings file, as facts under headings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactSheet {
    pub title: String,
    pub file: PathBuf,
    pub summary: String,
    pub sections: Vec<(String, Vec<Fact>)>,
}

/// A JSON Schema file and the shape it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaShape {
    pub file: PathBuf,
    pub title: Option<String>,
    /// `object`, `array`, ... of the root.
    pub kind: String,
    pub properties: usize,
    pub required: usize,
    pub definitions: usize,
    pub refs: usize,
    pub formats: Vec<String>,
}

/// Everything the settings files say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub gitignore: Option<IgnoreFile>,
    pub sheets: Vec<FactSheet>,
    pub schemas: Vec<SchemaShape>,
}

/// The job for a [`crate::SourceHub`].
pub fn settings_job(
    manifests: Vec<PathBuf>,
    json_files: Vec<PathBuf>,
) -> impl FnOnce(&crate::JobContext) -> Result<(), crate::JobError> + Send {
    move |ctx| {
        let settings = read_settings(&ctx.root, &manifests, &json_files);
        ctx.send(crate::SourceEvent::Settings(std::sync::Arc::new(settings)));
        Ok(())
    }
}

/// Reads the settings files at the project's root, the workspace manifest (with `manifests` to
/// see which packages use each shared dependency) and every JSON Schema in `json_files`.
pub fn read_settings(root: &Path, manifests: &[PathBuf], json_files: &[PathBuf]) -> Settings {
    let mut sheets = Vec::new();
    if let Some(sheet) = workspace_sheet(root, manifests) {
        sheets.push(sheet);
    }
    for (title, names) in [
        ("Toolchain", &["rust-toolchain.toml", "rust-toolchain"][..]),
        ("Cargo settings", &[".cargo/config.toml", ".cargo/config"][..]),
        ("Lints", &["clippy.toml", ".clippy.toml"][..]),
        ("Formatting", &["rustfmt.toml", ".rustfmt.toml"][..]),
    ] {
        if let Some(path) = names.iter().map(|n| root.join(n)).find(|p| p.is_file()) {
            if let Some(sheet) = toml_sheet(title, &path) {
                sheets.push(sheet);
            }
        }
    }
    if let Some(sheet) = editorconfig_sheet(&root.join(".editorconfig")) {
        sheets.push(sheet);
    }
    if let Some(sheet) = attributes_sheet(&root.join(".gitattributes")) {
        sheets.push(sheet);
    }
    let mut schemas: Vec<SchemaShape> = json_files.iter().filter_map(|f| schema_shape(f)).collect();
    schemas.sort_by(|a, b| a.file.cmp(&b.file));
    Settings { gitignore: read_gitignore(&root.join(".gitignore")), sheets, schemas }
}

/// A comment line of only symbols, like `# ======`.
fn is_rule_line(text: &str) -> bool {
    let body = text.trim().trim_start_matches('#').trim();
    body.len() >= 3 && !body.chars().any(char::is_alphanumeric)
}

/// Reads a `.gitignore`. Groups start at banners (a `# ====` line next to a titled comment) when
/// the file has them; otherwise at each comment that follows a blank line.
pub fn read_gitignore(path: &Path) -> Option<IgnoreFile> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let banners = lines.iter().any(|l| l.starts_with('#') && is_rule_line(l));
    let mut groups: Vec<IgnoreGroup> = Vec::new();
    let mut note: Vec<String> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            note.clear();
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            if is_rule_line(line) {
                continue;
            }
            // A title sits right after a banner's opening rule (one that follows a blank line or
            // the start), not after its closing one.
            let opening_rule = |j: usize| {
                lines[j].starts_with('#') && is_rule_line(lines[j]) && (j == 0 || lines[j - 1].trim().is_empty())
            };
            let after_rule = i > 0 && opening_rule(i - 1);
            let after_blank = i == 0 || lines[i - 1].trim().is_empty();
            if (banners && after_rule) || (!banners && after_blank) {
                groups.push(IgnoreGroup { title: comment.trim().to_string(), line: i + 1, rules: Vec::new() });
                note.clear();
            } else {
                note.push(comment.trim().to_string());
            }
            continue;
        }
        if groups.is_empty() {
            groups.push(IgnoreGroup { title: "Rules".to_string(), line: i + 1, rules: Vec::new() });
        }
        let rule_note = (!note.is_empty()).then(|| note.join(" "));
        note.clear();
        groups.last_mut()?.rules.push(IgnoreRuleLine { pattern: line.to_string(), line: i + 1, note: rule_note });
    }
    groups.retain(|g| !g.rules.is_empty());
    Some(IgnoreFile { file: path.to_path_buf(), groups })
}

/// The 1-based line where `key` is set, at or after the line of its table's header.
fn line_of_key(text: &str, table: &str, key: &str) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let start =
        if table.is_empty() { 0 } else { lines.iter().position(|l| l.trim() == format!("[{table}]")).unwrap_or(0) };
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, l)| {
            let t = l.trim_start();
            t.starts_with(key) && t[key.len()..].trim_start().starts_with(['=', '.'])
        })
        .map_or(start + 1, |(i, _)| i + 1)
}

fn show(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(a) => a.iter().map(show).collect::<Vec<_>>().join(", "),
        toml::Value::Table(t) => t.iter().map(|(k, v)| format!("{k} = {}", show(v))).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// A TOML file as facts: top-level keys first, then one section per table.
fn toml_sheet(title: &str, path: &Path) -> Option<FactSheet> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = text.parse().ok()?;
    let table = value.as_table()?;
    let mut top = Vec::new();
    let mut sections = Vec::new();
    for (key, v) in table {
        match v {
            toml::Value::Table(inner) => {
                let facts = inner
                    .iter()
                    .map(|(k, v)| Fact { key: k.clone(), value: show(v), line: line_of_key(&text, key, k) })
                    .collect();
                sections.push((key.clone(), facts));
            }
            _ => top.push(Fact { key: key.clone(), value: show(v), line: line_of_key(&text, "", key) }),
        }
    }
    if !top.is_empty() {
        sections.insert(0, (String::new(), top));
    }
    let count: usize = sections.iter().map(|(_, f)| f.len()).sum();
    Some(FactSheet {
        title: title.to_string(),
        file: path.to_path_buf(),
        summary: format!("{count} settings"),
        sections,
    })
}

/// The workspace manifest: its members, the dependencies its packages share (with how many use
/// each) and its build profiles.
fn workspace_sheet(root: &Path, manifests: &[PathBuf]) -> Option<FactSheet> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    let value: toml::Value = text.parse().ok()?;
    let workspace = value.get("workspace")?;
    let mut sections = Vec::new();
    let members: Vec<Fact> = workspace
        .get("members")
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_str())
        .map(|m| Fact {
            key: m.to_string(),
            value: String::new(),
            line: text.lines().position(|l| l.contains(&format!("\"{m}\""))).map_or(1, |i| i + 1),
        })
        .collect();
    let member_count = members.len();
    sections.push(("Members".to_string(), members));

    // How many packages use each shared dependency (`name.workspace = true` or
    // `name = { workspace = true }`).
    let mut users: BTreeMap<String, usize> = BTreeMap::new();
    for manifest in manifests.iter().filter(|m| m.as_path() != path) {
        let Some(toml) = std::fs::read_to_string(manifest).ok().and_then(|t| t.parse::<toml::Value>().ok()) else {
            continue;
        };
        for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
            for (name, spec) in toml.get(table).and_then(|t| t.as_table()).into_iter().flatten() {
                if spec.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                    *users.entry(name.clone()).or_default() += 1;
                }
            }
        }
    }
    let shared: Vec<Fact> = workspace
        .get("dependencies")
        .and_then(|d| d.as_table())
        .into_iter()
        .flatten()
        .map(|(name, spec)| {
            let version = match spec {
                toml::Value::String(v) => v.clone(),
                other => other
                    .get("version")
                    .map(show)
                    .or_else(|| other.get("path").map(|p| format!("path {}", show(p))))
                    .unwrap_or_default(),
            };
            let used = users.get(name).copied().unwrap_or(0);
            Fact {
                key: name.clone(),
                value: format!("{version} · used by {used}"),
                line: line_of_key(&text, "workspace.dependencies", name),
            }
        })
        .collect();
    let shared_count = shared.len();
    if !shared.is_empty() {
        sections.push(("Shared dependencies".to_string(), shared));
    }
    for (profile, settings) in value.get("profile").and_then(|p| p.as_table()).into_iter().flatten() {
        let header = format!("profile.{profile}");
        let facts = settings
            .as_table()
            .into_iter()
            .flatten()
            .map(|(k, v)| Fact { key: k.clone(), value: show(v), line: line_of_key(&text, &header, k) })
            .collect();
        sections.push((format!("Profile {profile}"), facts));
    }
    Some(FactSheet {
        title: "Cargo workspace".to_string(),
        file: path,
        summary: format!("{member_count} member patterns · {shared_count} shared dependencies"),
        sections,
    })
}

/// `.editorconfig`: one section per file pattern.
fn editorconfig_sheet(path: &Path) -> Option<FactSheet> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut sections: Vec<(String, Vec<Fact>)> = vec![(String::new(), Vec::new())];
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some(pattern) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((pattern.to_string(), Vec::new()));
        } else if let Some((k, v)) = line.split_once('=') {
            sections.last_mut()?.1.push(Fact { key: k.trim().to_string(), value: v.trim().to_string(), line: i + 1 });
        }
    }
    sections.retain(|(_, f)| !f.is_empty());
    let count = sections.len();
    Some(FactSheet {
        title: "Editor settings".to_string(),
        file: path.to_path_buf(),
        summary: format!("{count} file patterns"),
        sections,
    })
}

/// `.gitattributes`: each pattern and its attributes, LFS rules first.
fn attributes_sheet(path: &Path) -> Option<FactSheet> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lfs = Vec::new();
    let mut other = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pattern) = parts.next() else { continue };
        let attrs: Vec<&str> = parts.collect();
        let fact = Fact { key: pattern.to_string(), value: attrs.join(" "), line: i + 1 };
        if attrs.contains(&"filter=lfs") {
            lfs.push(fact);
        } else {
            other.push(fact);
        }
    }
    let summary = format!("{} LFS rules · {} other", lfs.len(), other.len());
    let mut sections = Vec::new();
    if !lfs.is_empty() {
        sections.push(("Kept in Git LFS".to_string(), lfs));
    }
    if !other.is_empty() {
        sections.push(("Other attributes".to_string(), other));
    }
    Some(FactSheet { title: "Git attributes".to_string(), file: path.to_path_buf(), summary, sections })
}

/// A JSON file that declares `$schema`: the shape of what it describes.
fn schema_shape(path: &Path) -> Option<SchemaShape> {
    if path.extension().is_none_or(|e| e != "json") {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if !text.contains("\"$schema\"") {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("$schema")?;
    let kind = match json.get("type") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" | "),
        _ if json.get("oneOf").or(json.get("anyOf")).is_some() => "one of".to_string(),
        _ => "any".to_string(),
    };
    let count_keys = |key: &str| json.get(key).and_then(|v| v.as_object()).map_or(0, |o| o.len());
    let mut refs = 0;
    let mut formats = std::collections::BTreeSet::new();
    fn walk(v: &serde_json::Value, refs: &mut usize, formats: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::Object(o) => {
                if o.get("$ref").is_some() {
                    *refs += 1;
                }
                if let Some(f) = o.get("format").and_then(|f| f.as_str()) {
                    formats.insert(f.to_string());
                }
                o.values().for_each(|v| walk(v, refs, formats));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, refs, formats)),
            _ => {}
        }
    }
    walk(&json, &mut refs, &mut formats);
    Some(SchemaShape {
        file: path.to_path_buf(),
        title: json.get("title").and_then(|t| t.as_str()).map(str::to_string),
        kind,
        properties: count_keys("properties"),
        required: json.get("required").and_then(|r| r.as_array()).map_or(0, |r| r.len()),
        definitions: count_keys("$defs").max(count_keys("definitions")),
        refs,
        formats: formats.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn gitignore_groups_follow_its_banners() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".gitignore",
            "# =====\n# 1. Secrets\n# =====\n.env\n!.env.example\n\n# =====\n# 2. Build\n# =====\n# all build output\n/target/\n\n# old folders\n/target-ci/\n",
        );
        let file = read_gitignore(&dir.path().join(".gitignore")).unwrap();
        let titles: Vec<&str> = file.groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(
            titles,
            ["1. Secrets", "2. Build"],
            "a comment after a blank line is a note, not a group, when there are banners"
        );
        assert_eq!(file.patterns(), 4);
        let target = &file.groups[1].rules[0];
        assert_eq!(
            (target.pattern.as_str(), target.line, target.note.as_deref()),
            ("/target/", 11, Some("all build output"))
        );

        write(dir.path(), "plain/.gitignore", "*.log\n\n# Editors\n.idea/\n.vscode/\n");
        let plain = read_gitignore(&dir.path().join("plain/.gitignore")).unwrap();
        assert_eq!(plain.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Rules", "Editors"]);
    }

    #[test]
    fn settings_files_become_facts_with_their_lines() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(
            r,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\nserde = \"1.0\"\ntokio = { version = \"1.40\", features = [\"full\"] }\n\n[profile.release]\nlto = true\n",
        );
        write(
            r,
            "crates/a/Cargo.toml",
            "[package]\nname = \"a\"\n[dependencies]\nserde = { workspace = true }\ntokio.workspace = true\n",
        );
        write(r, "crates/b/Cargo.toml", "[package]\nname = \"b\"\n[dependencies]\nserde.workspace = true\n");
        write(r, "rust-toolchain.toml", "[toolchain]\nchannel = \"1.95\"\ncomponents = [\"clippy\", \"rustfmt\"]\n");
        write(r, ".gitattributes", "*.png filter=lfs diff=lfs merge=lfs -text\n*.rs text eol=lf\n");
        write(r, "contracts/user.schema.json", "{\"$schema\": \"https://json-schema.org/draft/2020-12/schema\", \"type\": \"object\", \"properties\": {\"id\": {\"type\": \"string\", \"format\": \"uuid\"}, \"team\": {\"$ref\": \"team.schema.json\"}}, \"required\": [\"id\"]}");
        write(r, "contracts/data.json", "{\"id\": 1}");
        let manifests: Vec<PathBuf> =
            ["Cargo.toml", "crates/a/Cargo.toml", "crates/b/Cargo.toml"].map(|m| r.join(m)).into();
        let json: Vec<PathBuf> = ["contracts/user.schema.json", "contracts/data.json"].map(|f| r.join(f)).into();
        let settings = read_settings(r, &manifests, &json);
        let sheet =
            |title: &str| settings.sheets.iter().find(|s| s.title == title).unwrap_or_else(|| panic!("{title}"));
        let fact = |s: &FactSheet, section: &str, key: &str| {
            s.sections
                .iter()
                .find(|(t, _)| t == section)
                .and_then(|(_, f)| f.iter().find(|f| f.key == key))
                .cloned()
                .unwrap()
        };

        let ws = sheet("Cargo workspace");
        assert_eq!(fact(ws, "Shared dependencies", "serde").value, "1.0 · used by 2");
        assert_eq!(fact(ws, "Shared dependencies", "tokio").value, "1.40 · used by 1");
        assert_eq!(fact(ws, "Shared dependencies", "tokio").line, 6);
        assert_eq!(fact(ws, "Profile release", "lto").line, 9);
        assert_eq!(fact(sheet("Toolchain"), "toolchain", "channel").value, "1.95");
        let attrs = sheet("Git attributes");
        assert_eq!(attrs.summary, "1 LFS rules · 1 other");
        assert_eq!(settings.schemas.len(), 1, "only files that declare $schema");
        let shape = &settings.schemas[0];
        assert_eq!((shape.kind.as_str(), shape.properties, shape.required, shape.refs), ("object", 2, 1, 1));
        assert_eq!(shape.formats, ["uuid"]);
    }
}
