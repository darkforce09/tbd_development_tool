use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
    pub crates: Vec<CrateInfo>,
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

/// Checks whether a given path is a supported source, script, layout, config or documentation file.
/// True binary files (images, archives, compiled objects) are excluded.
pub fn is_supported_source_file(path: &Path) -> bool {
    let file_name = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };

    // Skip temporary editor cruft and system files
    if file_name.ends_with(".swp")
        || file_name.ends_with(".swo")
        || file_name == ".DS_Store"
        || file_name == "Thumbs.db"
        || file_name.ends_with(".min.js")
        || file_name.ends_with(".min.css")
        || file_name.ends_with(".bundle.js")
        || file_name.ends_with(".bundle.min.js")
        || file_name.ends_with(".map")
    {
        return false;
    }

    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e.to_lowercase(),
        None => {
            // Extensionless files (e.g. Dockerfile, Makefile, LICENSE, .editorconfig, baselines)
            return true;
        }
    };

    // Exclude true binary asset, image, archive and compiled formats
    if matches!(
        ext.as_str(),
        // Images & textures
        "png" | "jpg" | "jpeg" | "gif" | "ico" | "webp" | "bmp" | "tiff" | "tga" | "dds"
        // Archives & compressed files
        | "gz" | "zip" | "tar" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst"
        // Machine binaries & compiled objects
        | "exe" | "dll" | "so" | "dylib" | "o" | "a" | "lib" | "pyc" | "class" | "jar" | "wasm"
        | "rmeta" | "rlib" | "d" | "timestamp" | "map" | "lock" | "pdb" | "idb" | "ipdb" | "iobj"
        // Documents & fonts
        | "pdf" | "ttf" | "otf" | "woff" | "woff2"
        // Raw game terrain binaries / LFS payload types
        | "dem" | "tbd-sat" | "tbd-bath" | "r16"
    ) {
        return false;
    }

    // Safety guard: skip files larger than 1.5 MB (almost certainly minified bundles, data dumps, or binary blobs)
    if let Ok(meta) = path.metadata() {
        if meta.len() > 1_500_000 {
            return false;
        }
    }

    true
}

/// Discovers all candidate repository files.
/// First attempts ultra-fast discovery using `git ls-files` (takes ~4ms on 18k files),
/// falling back to recursive WalkDir if git is unavailable or errors.
pub fn discover_all_repository_files(root: &Path) -> Vec<PathBuf> {
    if let Some(files) = git_discover_files(root) {
        if !files.is_empty() {
            return files;
        }
    }
    walkdir_discover_files(root)
}

fn git_discover_files(root: &Path) -> Option<Vec<PathBuf>> {
    if !root.join(".git").exists() {
        return None;
    }

    let mut files = Vec::new();

    // 1. Tracked files
    let tracked_output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .ok()?;

    if !tracked_output.status.success() {
        return None;
    }

    for chunk in tracked_output.stdout.split(|&b| b == 0) {
        if !chunk.is_empty() {
            if let Ok(rel) = std::str::from_utf8(chunk) {
                let path = root.join(rel);
                if is_supported_source_file(&path) {
                    files.push(path);
                }
            }
        }
    }

    // 2. Untracked files (excluding gitignored)
    if let Ok(untracked_output) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
    {
        if untracked_output.status.success() {
            for chunk in untracked_output.stdout.split(|&b| b == 0) {
                if !chunk.is_empty() {
                    if let Ok(rel) = std::str::from_utf8(chunk) {
                        let path = root.join(rel);
                        if is_supported_source_file(&path) {
                            files.push(path);
                        }
                    }
                }
            }
        }
    }

    files.sort();
    files.dedup();
    Some(files)
}

fn walkdir_discover_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(should_descend)
        .flatten()
    {
        if entry.file_type().is_file() {
            let path = entry.path();
            if is_supported_source_file(path) {
                files.push(path.to_path_buf());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Scans a directory for any codebase:
/// - Cargo workspaces or packages (Rust)
/// - Enforce Script projects (DayZ / Arma Reforger / Enfusion mods)
/// - Markdown documentation folders & wikis
/// - Polyglot or loose code files across any language
pub fn scan_project(path: impl AsRef<Path>) -> Result<RustProject, ProjectError> {
    let root = path.as_ref().canonicalize().map_err(|e| {
        ProjectError::NotFound(format!("{}: {}", path.as_ref().display(), e))
    })?;

    if !root.is_dir() {
        return Err(ProjectError::NotFound(format!(
            "Not a directory: {}",
            root.display()
        )));
    }

    let cargo_toml_path = root.join("Cargo.toml");
    if cargo_toml_path.is_file() {
        if let Ok(manifest_content) = std::fs::read_to_string(&cargo_toml_path) {
            if let Ok(toml_val) = manifest_content.parse::<toml::Value>() {
                if let Ok(project) = parse_cargo_workspace_or_package(&root, &cargo_toml_path, &toml_val) {
                    if project.total_files() > 0 {
                        return Ok(project);
                    }
                }
            }
        }
    }

    // Fallback: Scan general directory tree (Enforce script, Markdown, polyglot)
    scan_loose_files(&root)
}

fn parse_cargo_workspace_or_package(
    root: &Path,
    cargo_toml_path: &Path,
    toml_val: &toml::Value,
) -> Result<RustProject, ProjectError> {
    let project_name = toml_val
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            toml_val
                .get("workspace")
                .and_then(|w| w.get("package"))
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| {
            root.file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| "Project".to_string())
        });

    // 1. Discover all repository files across the entire workspace tree
    let all_files = discover_all_repository_files(root);
    if all_files.is_empty() {
        return Err(ProjectError::NoSourceFiles(root.display().to_string()));
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
    for file in all_files {
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
            crates.push(CrateInfo {
                name,
                manifest_path: manifest_opt,
                root_path: crate_root,
                source_files: files,
            });
        }
    }

    // 5. Group unclaimed non-Cargo directories & root files into dedicated subsystems
    group_unclaimed_into_crates(root, &project_name, unclaimed_files, &mut crates);

    if crates.is_empty() || crates.iter().all(|c| c.source_files.is_empty()) {
        return scan_loose_files(root);
    }

    Ok(RustProject {
        name: project_name,
        root_path: root.to_path_buf(),
        crates,
    })
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

        groups
            .entry(group_name)
            .or_insert_with(|| (group_root, Vec::new()))
            .1
            .push(file);
    }

    for (name, (group_root, mut files)) in groups {
        files.sort();
        crates.push(CrateInfo {
            name,
            manifest_path: None,
            root_path: group_root,
            source_files: files,
        });
    }
}

fn resolve_workspace_member_roots(
    root: &Path,
    pattern: &str,
    crates: &mut Vec<(PathBuf, Option<PathBuf>, String)>,
) {
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
                if sub_path.is_dir()
                    && file_name != ".git"
                    && file_name != "target"
                    && file_name != "node_modules"
                {
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
    toml.get("package")?
        .get("name")?
        .as_str()
        .map(|s| s.to_string())
}

fn scan_loose_files(root: &Path) -> Result<RustProject, ProjectError> {
    let project_name = root
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "Project".to_string());

    let all_files = discover_all_repository_files(root);
    if all_files.is_empty() {
        return Err(ProjectError::NoSourceFiles(root.display().to_string()));
    }

    let mut crates = Vec::new();
    group_unclaimed_into_crates(root, &project_name, all_files, &mut crates);

    if crates.is_empty() || crates.iter().all(|c| c.source_files.is_empty()) {
        return Err(ProjectError::NoSourceFiles(root.display().to_string()));
    }

    Ok(RustProject {
        name: project_name,
        root_path: root.to_path_buf(),
        crates,
    })
}

fn should_descend(entry: &walkdir::DirEntry) -> bool {
    let file_name = entry.file_name().to_string_lossy();
    if entry.file_type().is_dir()
        && (file_name == ".git"
            || file_name == "target"
            || file_name == "node_modules"
            || file_name == "dist"
            || file_name == "build"
            || file_name == "out"
            || file_name == "bin"
            || file_name == "obj"
            || file_name == "vendor"
            || file_name == ".cache"
            || file_name == ".studio"
            || file_name == "__pycache__"
            || file_name == ".gradle"
            || file_name == ".cargo"
            || file_name == "bazel-out"
            || file_name == "coverage"
            || file_name == ".svn"
            || file_name == ".hg")
    {
        return false;
    }
    true
}
