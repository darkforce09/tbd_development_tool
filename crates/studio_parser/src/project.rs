use std::path::{Path, PathBuf};

use crate::tree::{scan_tree, FileKind, ProjectTree, ScanOptions};

/// A package (Cargo, npm, Python or Go) and the text files it holds; or, with no manifest and an
/// empty name, the project's files that belong to no package.
#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    pub manifest_path: Option<PathBuf>,
    pub root_path: PathBuf,
    pub source_files: Vec<PathBuf>,
}

impl CrateInfo {
    /// Whether this is a real package rather than the files outside every package.
    pub fn is_package(&self) -> bool {
        self.manifest_path.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct RustProject {
    pub name: String,
    pub root_path: PathBuf,
    /// Parseable text files grouped by the innermost package that holds them.
    pub crates: Vec<CrateInfo>,
    /// Every file and folder on disk.
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

/// Scans a directory holding any kind of project. Every file is listed; text files are grouped by
/// the innermost package that holds them (see [`group_by_package`]), whatever the folder layout.
pub fn scan_project(path: impl AsRef<Path>) -> Result<RustProject, ProjectError> {
    let root = path
        .as_ref()
        .canonicalize()
        .map_err(|e| ProjectError::NotFound(format!("{}: {}", path.as_ref().display(), e)))?;

    if !root.is_dir() {
        return Err(ProjectError::NotFound(format!("Not a directory: {}", root.display())));
    }

    let tree = scan_tree(&root, ScanOptions::default());
    let crates = group_by_package(&tree);
    let name = crates
        .iter()
        .find(|c| c.is_package() && c.root_path == root)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| folder_name(&root));
    Ok(RustProject { name, root_path: root, crates, tree })
}

fn folder_name(root: &Path) -> String {
    root.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "Project".to_string())
}

/// Groups the tree's text files by the innermost package whose folder holds them. A package is a
/// folder with a manifest that names it: `Cargo.toml` with `[package]`, `package.json` with a
/// `name`, `pyproject.toml` with `[project]` or `[tool.poetry]`, or `go.mod`. Manifests inside
/// heavy folders (dependencies, build output) are never seen. Files in no package form one last
/// group with an empty name and no manifest.
pub fn group_by_package(tree: &ProjectTree) -> Vec<CrateInfo> {
    let mut packages: Vec<(PathBuf, PathBuf, String)> = tree
        .files
        .iter()
        .filter(|f| f.kind == FileKind::Text)
        .filter_map(|f| {
            let manifest = tree.abs(&f.rel);
            let name = package_name(&manifest)?;
            let dir = f.rel.parent().unwrap_or(Path::new("")).to_path_buf();
            Some((dir, manifest, name))
        })
        .collect();
    // One package per folder: Cargo, then npm, then Python, then Go, as `package_name` ranks them.
    packages.sort_by(|a, b| a.0.cmp(&b.0).then(manifest_rank(&a.1).cmp(&manifest_rank(&b.1))));
    packages.dedup_by(|a, b| a.0 == b.0);

    let mut files: Vec<Vec<PathBuf>> = vec![Vec::new(); packages.len()];
    let mut outside = Vec::new();
    for f in tree.files.iter().filter(|f| f.kind == FileKind::Text) {
        // Innermost first: the longest package folder that is a prefix of the file's path.
        let owner = packages
            .iter()
            .enumerate()
            .filter(|(_, (dir, _, _))| f.rel.starts_with(dir))
            .max_by_key(|(_, (dir, _, _))| dir.components().count())
            .map(|(i, _)| i);
        match owner {
            Some(i) => files[i].push(tree.abs(&f.rel)),
            None => outside.push(tree.abs(&f.rel)),
        }
    }

    let mut crates: Vec<CrateInfo> = packages
        .into_iter()
        .zip(files)
        .filter(|(_, files)| !files.is_empty())
        .map(|((dir, manifest, name), source_files)| CrateInfo {
            name,
            manifest_path: Some(manifest),
            root_path: tree.abs(&dir),
            source_files,
        })
        .collect();
    crates.sort_by(|a, b| a.name.cmp(&b.name).then(a.root_path.cmp(&b.root_path)));
    if !outside.is_empty() {
        crates.push(CrateInfo {
            name: String::new(),
            manifest_path: None,
            root_path: tree.root.clone(),
            source_files: outside,
        });
    }
    crates
}

/// Which manifest wins when one folder has several.
fn manifest_rank(manifest: &Path) -> usize {
    match manifest.file_name().and_then(|n| n.to_str()) {
        Some("Cargo.toml") => 0,
        Some("package.json") => 1,
        Some("pyproject.toml") => 2,
        _ => 3,
    }
}

/// The package a manifest declares, if `manifest` is one.
fn package_name(manifest: &Path) -> Option<String> {
    let file_name = manifest.file_name()?.to_str()?;
    if !matches!(file_name, "Cargo.toml" | "package.json" | "pyproject.toml" | "go.mod") {
        return None;
    }
    let text = std::fs::read_to_string(manifest).ok()?;
    let name = match file_name {
        "Cargo.toml" => {
            let toml: toml::Value = text.parse().ok()?;
            toml.get("package")?.get("name")?.as_str()?.to_string()
        }
        "package.json" => {
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            json.get("name")?.as_str()?.to_string()
        }
        "pyproject.toml" => {
            let toml: toml::Value = text.parse().ok()?;
            let project = toml.get("project").and_then(|p| p.get("name"));
            let poetry = || toml.get("tool")?.get("poetry")?.get("name");
            project.or_else(poetry)?.as_str()?.to_string()
        }
        _ => {
            let module = text.lines().find_map(|l| l.trim().strip_prefix("module "))?;
            let module = module.trim().trim_matches('"');
            module.rsplit('/').next()?.to_string()
        }
    };
    (!name.trim().is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn group(root: &Path) -> Vec<(String, Vec<String>)> {
        let tree = scan_tree(root, ScanOptions { gitignored_heavy: false });
        group_by_package(&tree)
            .into_iter()
            .map(|c| {
                let rels = c
                    .source_files
                    .iter()
                    .map(|f| f.strip_prefix(&tree.root).unwrap().to_string_lossy().replace('\\', "/"))
                    .collect();
                (c.name, rels)
            })
            .collect()
    }

    #[test]
    fn files_belong_to_the_innermost_package_in_any_layout() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "Cargo.toml", "[workspace]\nmembers = [\"engine\"]\n");
        write(r, "engine/Cargo.toml", "[package]\nname = \"engine\"\n");
        write(r, "engine/src/lib.rs", "");
        write(r, "engine/tools/gen/Cargo.toml", "[package]\nname = \"gen\"\n");
        write(r, "engine/tools/gen/main.rs", "");
        write(r, "web/package.json", "{\"name\": \"@studio/web\", \"version\": \"1.0.0\"}");
        write(r, "web/src/app.ts", "");
        write(r, "py/pyproject.toml", "[project]\nname = \"pyth\"\n");
        write(r, "py/m.py", "");
        write(r, "svc/go.mod", "module example.com/team/svc\n\ngo 1.22\n");
        write(r, "svc/main.go", "");
        write(r, "docs/guide.md", "");
        write(r, "node_modules/dep/package.json", "{\"name\": \"dep\"}");

        let groups = group(r);
        let find = |name: &str| groups.iter().find(|(n, _)| n == name).map(|(_, f)| f.clone()).unwrap_or_default();
        assert_eq!(find("engine"), ["engine/Cargo.toml", "engine/src/lib.rs"]);
        assert_eq!(find("gen"), ["engine/tools/gen/Cargo.toml", "engine/tools/gen/main.rs"], "innermost wins");
        assert_eq!(find("@studio/web"), ["web/package.json", "web/src/app.ts"]);
        assert_eq!(find("pyth"), ["py/m.py", "py/pyproject.toml"]);
        assert_eq!(find("svc"), ["svc/go.mod", "svc/main.go"]);
        assert_eq!(find(""), ["Cargo.toml", "docs/guide.md"], "files outside every package are still parsed");
        assert!(groups.iter().all(|(n, _)| n != "dep"), "manifests in dependency folders are not packages");
    }

    #[test]
    fn the_project_is_named_by_its_root_package_or_folder() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Cargo.toml", "[package]\nname = \"rooted\"\n");
        write(dir.path(), "src/main.rs", "fn main() {}");
        assert_eq!(scan_project(dir.path()).unwrap().name, "rooted");

        let plain = tempfile::tempdir().unwrap();
        write(plain.path(), "notes.md", "# Notes");
        let project = scan_project(plain.path()).unwrap();
        assert_eq!(project.name, folder_name(&plain.path().canonicalize().unwrap()));
        assert!(project.crates.iter().all(|c| !c.is_package()));
    }
}
