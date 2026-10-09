use std::path::{Path, PathBuf};

use crate::tree::{scan_tree, ProjectTree, ScanOptions};

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    pub manifest_path: Option<PathBuf>,
    pub root_path: PathBuf,
    pub source_files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RustProject {
    pub name: String,
    pub root_path: PathBuf,
    /// Parseable text files grouped by crate / top-level folder (Items and Modules views).
    pub crates: Vec<CrateInfo>,
    /// Every file and folder on disk (Files view).
    pub tree: ProjectTree,
}

pub type SourceProject = RustProject;
pub type Project = RustProject;

impl RustProject {
    pub fn total_files(&self) -> usize {
        self.crates.iter().map(|c| c.source_files.len()).sum()
    }
}

#[derive(Debug, Clone)]
pub enum ProjectError {
    NotFound(String),
    IoError(String),
    NoSourceFiles(String),
    #[allow(dead_code)]
    NoRustFiles(String),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::NotFound(p) => write!(f, "Path does not exist: {}", p),
            ProjectError::IoError(e) => write!(f, "IO error: {}", e),
            ProjectError::NoSourceFiles(p) | ProjectError::NoRustFiles(p) => {
                write!(f, "No source or markdown files found in {}", p)
            }
        }
    }
}

impl std::error::Error for ProjectError {}

impl From<std::io::Error> for ProjectError {
    fn from(e: std::io::Error) -> Self {
        ProjectError::IoError(e.to_string())
    }
}

/// Absolute paths of every file outside heavy (collapsed) folders.
pub fn discover_all_repository_files(root: &Path) -> Vec<PathBuf> {
    let tree = scan_tree(root, ScanOptions::default());
    tree.files.iter().map(|f| tree.root.join(&f.rel)).collect()
}

/// Scans a directory for any codebase:
/// - Cargo workspaces or packages (Rust)
/// - Enforce Script projects (DayZ / Arma Reforger / Enfusion mods)
/// - Markdown documentation folders & wikis
/// - Polyglot or loose code files across any language
pub fn scan_project(path: impl AsRef<Path>) -> Result<RustProject, ProjectError> {
    let root = path
        .as_ref()
        .canonicalize()
        .map_err(|e| ProjectError::NotFound(format!("{}: {}", path.as_ref().display(), e)))?;

    if !root.is_dir() {
        return Err(ProjectError::NotFound(format!("Not a directory: {}", root.display())));
    }

    let tree = scan_tree(&root, ScanOptions::default());
    let text_files: Vec<PathBuf> = tree.text_files().collect();

    let cargo_toml_path = root.join("Cargo.toml");
    if cargo_toml_path.is_file() {
        if let Ok(manifest_content) = std::fs::read_to_string(&cargo_toml_path) {
            if let Ok(toml_val) = manifest_content.parse::<toml::Value>() {
                if let Some(crates) = parse_cargo_workspace_or_package(&root, &cargo_toml_path, &toml_val, &text_files)
                {
                    return Ok(RustProject {
                        name: cargo_project_name(&root, &toml_val),
                        root_path: root,
                        crates,
                        tree,
                    });
                }
            }
        }
    }

    // Fallback: group by top-level folder (Enforce script, Markdown, polyglot, or empty projects)
    let name = folder_name(&root);
    let mut crates = Vec::new();
    group_unclaimed_into_crates(&root, &name, text_files, &mut crates);
    Ok(RustProject { name, root_path: root, crates, tree })
}

fn folder_name(root: &Path) -> String {
    root.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Project".to_string())
}

fn cargo_project_name(root: &Path, toml_val: &toml::Value) -> String {
    toml_val
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .or_else(|| toml_val.get("workspace")?.get("package")?.get("name")?.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| folder_name(root))
}

fn parse_cargo_workspace_or_package(
    root: &Path,
    cargo_toml_path: &Path,
    toml_val: &toml::Value,
    all_files: &[PathBuf],
) -> Option<Vec<CrateInfo>> {
    let project_name = cargo_project_name(root, toml_val);
    if all_files.is_empty() {
        return None;
    }

    // 2. Identify Cargo workspace member crate roots
    let mut cargo_crate_metas = Vec::new();
    if let Some(workspace) = toml_val.get("workspace") {
        if let Some(members) = workspace.get("members").and_then(|m| m.as_array()) {
            for member in members {
                if let Some(pattern) = member.as_str() {
                    resolve_workspace_member_roots(root, pattern, &mut cargo_crate_metas);
                }
            }
        }
    }

    // Check if root itself is also a package
    if toml_val.get("package").is_some() && !cargo_crate_metas.iter().any(|(p, _, _)| p == root) {
        cargo_crate_metas.push((root.to_path_buf(), Some(cargo_toml_path.to_path_buf()), project_name.clone()));
    }

    // Sort cargo crates by root path length descending (longest path / innermost crate matches first)
    cargo_crate_metas.sort_by_key(|meta| std::cmp::Reverse(meta.0.as_os_str().len()));

    let mut crates = Vec::new();
    let mut crate_file_map: std::collections::HashMap<PathBuf, Vec<PathBuf>> = std::collections::HashMap::new();
    for (crate_root, _, _) in &cargo_crate_metas {
        crate_file_map.insert(crate_root.clone(), Vec::new());
    }

    let mut unclaimed_files = Vec::new();

    // 3. Partition discovered files between Cargo crates and unclaimed repository files
    for file in all_files.iter().cloned() {
        let mut matched_root = None;
        for (crate_root, _, _) in &cargo_crate_metas {
            if file.starts_with(crate_root) {
                matched_root = Some(crate_root);
                break;
            }
        }
        if let Some(c_root) = matched_root {
            if let Some(list) = crate_file_map.get_mut(c_root) {
                list.push(file);
            }
        } else {
            unclaimed_files.push(file);
        }
    }

    // 4. Register Cargo crates (alphabetical by name for consistent UI display)
    cargo_crate_metas.sort_by(|a, b| a.2.cmp(&b.2));
    for (crate_root, manifest_opt, name) in cargo_crate_metas {
        let mut files = crate_file_map.remove(&crate_root).unwrap_or_default();
        if !files.is_empty() {
            files.sort();
            crates.push(CrateInfo { name, manifest_path: manifest_opt, root_path: crate_root, source_files: files });
        }
    }

    // 5. Group unclaimed non-Cargo directories & root files into dedicated subsystems
    group_unclaimed_into_crates(root, &project_name, unclaimed_files, &mut crates);

    if crates.is_empty() || crates.iter().all(|c| c.source_files.is_empty()) {
        return None;
    }
    Some(crates)
}

fn group_unclaimed_into_crates(
    root: &Path,
    project_name: &str,
    unclaimed_files: Vec<PathBuf>,
    crates: &mut Vec<CrateInfo>,
) {
    if unclaimed_files.is_empty() {
        return;
    }

    let mut groups: std::collections::BTreeMap<String, (PathBuf, Vec<PathBuf>)> = std::collections::BTreeMap::new();

    for file in unclaimed_files {
        let Ok(rel) = file.strip_prefix(root) else { continue };
        let mut components = rel.components();
        let first = match components.next() {
            Some(c) => c.as_os_str().to_string_lossy().to_string(),
            None => continue,
        };

        let (group_name, group_root) = if components.as_path().as_os_str().is_empty() {
            // Root-level file (README.md, Cargo.toml, CLAUDE.md, etc.)
            (format!("{}_root", project_name), root.to_path_buf())
        } else if first == "apps" {
            // Subdirectories inside apps/ that are not Cargo crates (e.g. apps/mod)
            if let Some(second) = components.next() {
                let second_str = second.as_os_str().to_string_lossy().to_string();
                let sub_dir = root.join("apps").join(&second_str);
                if sub_dir.is_dir() {
                    (format!("apps: {}", second_str), sub_dir)
                } else {
                    ("apps_root".to_string(), root.join("apps"))
                }
            } else {
                ("apps_root".to_string(), root.join("apps"))
            }
        } else if first == "crates" || first == "tools" {
            // Loose files directly under crates/ or tools/
            (format!("{}_root", first), root.join(&first))
        } else {
            // Any other top-level directory: documentation, .ai, assets, contracts, deploy, .cursor, .github, etc.
            let dir_path = root.join(&first);
            (first, dir_path)
        };

        groups.entry(group_name).or_insert_with(|| (group_root, Vec::new())).1.push(file);
    }

    for (name, (group_root, mut files)) in groups {
        files.sort();
        crates.push(CrateInfo { name, manifest_path: None, root_path: group_root, source_files: files });
    }
}

fn resolve_workspace_member_roots(root: &Path, pattern: &str, crates: &mut Vec<(PathBuf, Option<PathBuf>, String)>) {
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    resolve_glob_segments_roots(root, &segments, crates);
}

fn resolve_glob_segments_roots(
    current: &Path,
    segments: &[&str],
    crates: &mut Vec<(PathBuf, Option<PathBuf>, String)>,
) {
    if segments.is_empty() {
        if current.is_dir() {
            let manifest = current.join("Cargo.toml");
            let manifest_opt = if manifest.is_file() { Some(manifest) } else { None };
            let name = manifest_opt
                .as_deref()
                .and_then(get_package_name_from_manifest)
                .unwrap_or_else(|| current.file_name().unwrap_or_default().to_string_lossy().to_string());
            if !crates.iter().any(|(p, _, _)| p == current) {
                crates.push((current.to_path_buf(), manifest_opt, name));
            }
        }
        return;
    }

    let head = segments[0];
    let tail = &segments[1..];

    if head == "*" {
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.flatten() {
                let sub_path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();
                if sub_path.is_dir() && file_name != ".git" && file_name != "target" && file_name != "node_modules" {
                    resolve_glob_segments_roots(&sub_path, tail, crates);
                }
            }
        }
    } else {
        let next_path = current.join(head);
        if next_path.is_dir() {
            resolve_glob_segments_roots(&next_path, tail, crates);
        }
    }
}

fn get_package_name_from_manifest(manifest: &Path) -> Option<String> {
    let content = std::fs::read_to_string(manifest).ok()?;
    let toml: toml::Value = content.parse().ok()?;
    toml.get("package")?.get("name")?.as_str().map(|s| s.to_string())
}
