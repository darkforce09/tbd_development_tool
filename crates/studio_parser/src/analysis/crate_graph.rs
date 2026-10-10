//! The crates of a Cargo workspace and their dependencies, read from the `Cargo.toml` files alone.
//!
//! Targets follow cargo's rules: the library (`[lib]` name and path, default `src/lib.rs`, `proc-macro = true`), the
//! binaries (`src/main.rs` named after the package, `src/bin/*.rs`, `src/bin/*/main.rs`, `[[bin]]` entries, no
//! inferred binaries with `autobins = false`; an inferred binary whose path or name an explicit `[[bin]]` already
//! uses is dropped). Tests, benches, examples and build scripts are not crates here.
//!
//! Dependencies come from `[dependencies]` and `[target.*.dependencies]`: an entry with `path = …`, or
//! `workspace = true` resolved through the workspace root's `[workspace.dependencies]`, links to the package found
//! in that folder. The extern name used in paths is the dependency's library name, or the key (`-` → `_`) when
//! `package = "…"` renames it. Any other dependency is external: it is listed in [`CrateNode::external_deps`] and
//! resolution into it yields an external path. A dependency that exists only under a condition (a
//! `[target.'cfg(…)']` table, `optional = true`), or whose name points at different crates in different tables, is
//! listed in [`CrateNode::gated_deps`]. Every binary also depends on its own package's library. The edition is
//! `package.edition` (or the workspace's with `edition.workspace = true`), 2015 when absent. No process is spawned.
//!
//! [`path_dependencies`] lists, for the Code map's manifest wires, every path dependency between the packages, from
//! every table (normal, build and dev), with the line of its entry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// The kind of a crate target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    Lib,
    Bin,
    ProcMacro,
    Other,
}

/// One crate target of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateNode {
    /// The package name as written in `Cargo.toml`.
    pub package: String,
    /// The crate name used in paths: `[lib] name`, else the package name with `-` → `_`; a binary's own name.
    pub name: String,
    pub kind: TargetKind,
    /// The absolute crate root file.
    pub root_file: PathBuf,
    /// The absolute `Cargo.toml` path.
    pub manifest: PathBuf,
    /// Workspace dependencies: (extern name used in paths, index into [`CrateGraph::crates`]), sorted by name.
    pub deps: Vec<(String, usize)>,
    /// Dependencies outside the workspace: (extern name used in paths, crate name), sorted by extern name.
    pub external_deps: Vec<(String, String)>,
    /// The Rust edition (2015 when the manifest names none).
    pub edition: u16,
    /// Extern names that exist only under a condition (target-specific or optional dependencies), or that name
    /// different crates in different dependency tables (those are in neither `deps` nor `external_deps`). Paths
    /// through them are cfg-gated. Sorted.
    pub gated_deps: Vec<String>,
}

/// The crates of the workspace, sorted by (manifest, kind, name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrateGraph {
    pub crates: Vec<CrateNode>,
}

/// One package read from its manifest.
struct Package {
    name: String,
    manifest: PathBuf,
    dir: PathBuf,
    table: toml::Table,
    /// The workspace root's folder and its `[workspace]` table.
    workspace: Option<(PathBuf, toml::Table)>,
}

/// A crate's linked dependencies: (workspace deps, external deps, gated extern names).
type Links = (Vec<(String, usize)>, Vec<(String, String)>, Vec<String>);

/// A dependency entry before it is linked.
struct DepEntry {
    /// The key with `-` → `_`: the extern name when renamed (or external).
    key: String,
    /// `package = "…"` was given (here or in the workspace entry).
    renamed: bool,
    target: DepTarget,
    /// Declared in a `[target.'cfg(…)']` table, or `optional = true`.
    gated: bool,
}

enum DepTarget {
    Path { dir: PathBuf, package: String },
    External { crate_name: String },
}

/// What one dependency entry links a crate to.
#[derive(Clone, PartialEq, Eq)]
enum Link {
    Workspace(usize),
    External(String),
}

impl CrateGraph {
    /// Reads the given manifests (absolute, or relative to `project_root`); manifests without `[package]` (a
    /// virtual workspace root) add no crate. Unreadable or invalid manifests are skipped.
    pub fn from_manifests(project_root: &Path, manifests: &[PathBuf]) -> CrateGraph {
        let packages = read_packages(project_root, manifests);

        // Targets, then the order, then the dependency links (indices are only known after sorting).
        let mut crates = Vec::new();
        let mut targets_of: Vec<Vec<DepEntry>> = Vec::new();
        for (pi, pkg) in packages.iter().enumerate() {
            let deps = package_deps(pkg);
            let edition = package_edition(pkg);
            for (kind, name, root_file) in package_targets(pkg) {
                crates.push((
                    pi,
                    CrateNode {
                        package: pkg.name.clone(),
                        name,
                        kind,
                        root_file,
                        manifest: pkg.manifest.clone(),
                        deps: Vec::new(),
                        external_deps: Vec::new(),
                        edition,
                        gated_deps: Vec::new(),
                    },
                ));
            }
            targets_of.push(deps);
        }
        crates.sort_by(|(_, a), (_, b)| (&a.manifest, a.kind, &a.name).cmp(&(&b.manifest, b.kind, &b.name)));

        // The library crate of each package, by package folder.
        let mut lib_of_dir: BTreeMap<&Path, (usize, &str)> = BTreeMap::new();
        let mut lib_of_pkg: BTreeMap<usize, usize> = BTreeMap::new();
        for (ci, (pi, node)) in crates.iter().enumerate() {
            if matches!(node.kind, TargetKind::Lib | TargetKind::ProcMacro) {
                lib_of_dir.insert(&packages[*pi].dir, (ci, packages[*pi].name.as_str()));
                lib_of_pkg.insert(*pi, ci);
            }
        }
        let mut links: Vec<Links> = Vec::new();
        for (ci, (pi, node)) in crates.iter().enumerate() {
            // Every entry by the extern name it binds: (link, gated). One name may appear in several tables.
            let mut by_name: BTreeMap<String, Vec<(Link, bool)>> = BTreeMap::new();
            for entry in &targets_of[*pi] {
                let (name, link) = match &entry.target {
                    DepTarget::Path { dir, package } => match lib_of_dir.get(dir.as_path()) {
                        Some(&(lib, name)) if name == package && lib != ci => {
                            // Not renamed: paths use the dependency's library name (`[lib] name`).
                            let extern_name =
                                if entry.renamed { entry.key.clone() } else { crates[lib].1.name.clone() };
                            (extern_name, Link::Workspace(lib))
                        }
                        // A path dependency that is not a loaded package is outside the workspace sources.
                        None => (entry.key.clone(), Link::External(package.replace('-', "_"))),
                        Some(_) => continue,
                    },
                    DepTarget::External { crate_name } => (entry.key.clone(), Link::External(crate_name.clone())),
                };
                by_name.entry(name).or_default().push((link, entry.gated));
            }
            let mut deps: BTreeMap<String, usize> = BTreeMap::new();
            let mut external: BTreeMap<String, String> = BTreeMap::new();
            let mut gated: BTreeSet<String> = BTreeSet::new();
            for (name, entries) in by_name {
                let first = &entries[0].0;
                if entries.iter().any(|(link, _)| link != first) {
                    // The same name for different crates (cfg twins): no single answer.
                    gated.insert(name);
                    continue;
                }
                // Any unconditional entry makes the dependency unconditional.
                if entries.iter().all(|(_, g)| *g) {
                    gated.insert(name.clone());
                }
                match first {
                    Link::Workspace(lib) => {
                        deps.insert(name, *lib);
                    }
                    Link::External(crate_name) => {
                        external.insert(name, crate_name.clone());
                    }
                }
            }
            if node.kind == TargetKind::Bin {
                if let Some(&lib) = lib_of_pkg.get(pi) {
                    let name = &crates[lib].1.name;
                    if !deps.contains_key(name) && !external.contains_key(name) && !gated.contains(name) {
                        deps.insert(name.clone(), lib);
                    }
                }
            }
            links.push((deps.into_iter().collect(), external.into_iter().collect(), gated.into_iter().collect()));
        }
        let crates = crates
            .into_iter()
            .zip(links)
            .map(|((_, mut node), (deps, external, gated))| {
                node.deps = deps;
                node.external_deps = external;
                node.gated_deps = gated;
                node
            })
            .collect();
        CrateGraph { crates }
    }

    /// The crate whose root file is `file`, if any.
    pub fn crate_with_root(&self, file: &Path) -> Option<usize> {
        self.crates.iter().position(|c| c.root_file == file)
    }
}

/// The packages among `manifests` (absolute, or relative to `project_root`), sorted by manifest path. Manifests
/// without `[package]`, unreadable or invalid ones, and repeats are skipped.
fn read_packages(project_root: &Path, manifests: &[PathBuf]) -> Vec<Package> {
    let mut roots: BTreeMap<PathBuf, Option<toml::Table>> = BTreeMap::new();
    let mut packages = Vec::new();
    let mut seen = BTreeSet::new();
    for manifest in manifests {
        let manifest = normalize(&project_root.join(manifest));
        if !seen.insert(manifest.clone()) {
            continue;
        }
        let Some(table) = read_table(&manifest) else { continue };
        let Some(name) = table.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()) else {
            continue;
        };
        let dir = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
        let workspace = workspace_root(&manifest, &table, &mut roots);
        packages.push(Package { name: name.to_string(), manifest, dir, table, workspace });
    }
    packages.sort_by(|a, b| a.manifest.cmp(&b.manifest));
    packages
}

fn read_table(manifest: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(manifest).ok()?.parse::<toml::Table>().ok()
}

/// The workspace root of a package: `package.workspace = "…"`, else the nearest folder at or above the manifest
/// whose `Cargo.toml` has a `[workspace]` table. Returns that root's folder and its `[workspace]` table.
fn workspace_root(
    manifest: &Path,
    table: &toml::Table,
    cache: &mut BTreeMap<PathBuf, Option<toml::Table>>,
) -> Option<(PathBuf, toml::Table)> {
    let dir = manifest.parent()?;
    let workspace_of =
        |t: &toml::Table| -> toml::Table { t.get("workspace").and_then(|w| w.as_table()).cloned().unwrap_or_default() };
    if table.contains_key("workspace") {
        return Some((dir.to_path_buf(), workspace_of(table)));
    }
    if let Some(explicit) = table.get("package").and_then(|p| p.get("workspace")).and_then(|w| w.as_str()) {
        let root_dir = normalize(&dir.join(explicit));
        let t = read_table(&root_dir.join("Cargo.toml"))?;
        return Some((root_dir, workspace_of(&t)));
    }
    let mut cur = dir.parent();
    while let Some(d) = cur {
        let candidate = d.join("Cargo.toml");
        let t = cache.entry(candidate.clone()).or_insert_with(|| read_table(&candidate));
        if let Some(t) = t {
            if t.contains_key("workspace") {
                return Some((d.to_path_buf(), workspace_of(t)));
            }
        }
        cur = d.parent();
    }
    None
}

/// The edition of a package: `package.edition`, or the workspace's `[workspace.package] edition` with
/// `edition.workspace = true`; 2015 when absent or not a known edition.
fn package_edition(pkg: &Package) -> u16 {
    let value = pkg.table.get("package").and_then(|p| p.get("edition"));
    let text = match value {
        Some(toml::Value::String(s)) => Some(s.as_str()),
        Some(v) if v.get("workspace").and_then(|w| w.as_bool()) == Some(true) => pkg
            .workspace
            .as_ref()
            .and_then(|(_, ws)| ws.get("package"))
            .and_then(|p| p.get("edition"))
            .and_then(|e| e.as_str()),
        _ => None,
    };
    match text {
        Some("2018") => 2018,
        Some("2021") => 2021,
        Some("2024") => 2024,
        _ => 2015,
    }
}

/// The dependency entries of a package (normal and target-specific; dev and build dependencies are not code the
/// library or binaries compile against).
fn package_deps(pkg: &Package) -> Vec<DepEntry> {
    let tables = dep_tables(pkg, DepKind::Normal);
    tables
        .iter()
        .flat_map(|t| t.entries.iter().map(|(key, value)| dep_entry(pkg, key, value, t.target.is_some())))
        .collect()
}

/// One dependency table of a manifest: its entries, the `[target.…]` key it sits under, and its name.
struct DepTable<'a> {
    entries: &'a toml::Table,
    target: Option<&'a str>,
    name: &'static str,
}

/// The tables of one kind: top level first, then each `[target.…]` table in key order.
fn dep_tables(pkg: &Package, kind: DepKind) -> Vec<DepTable<'_>> {
    let mut out = Vec::new();
    for &name in kind.table_names() {
        if let Some(entries) = pkg.table.get(name).and_then(|d| d.as_table()) {
            out.push(DepTable { entries, target: None, name });
        }
    }
    if let Some(targets) = pkg.table.get("target").and_then(|t| t.as_table()) {
        for (target, spec) in targets {
            for &name in kind.table_names() {
                if let Some(entries) = spec.get(name).and_then(|d| d.as_table()) {
                    out.push(DepTable { entries, target: Some(target), name });
                }
            }
        }
    }
    out
}

/// One dependency entry, with `workspace = true` resolved through the workspace root's `[workspace.dependencies]`.
fn dep_entry(pkg: &Package, key: &str, value: &toml::Value, in_target: bool) -> DepEntry {
    let mut package = value.get("package").and_then(|p| p.as_str()).map(str::to_string);
    let mut path = value.get("path").and_then(|p| p.as_str()).map(|p| normalize(&pkg.dir.join(p)));
    let mut optional = value.get("optional").and_then(|o| o.as_bool()) == Some(true);
    if value.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
        let ws_entry = pkg.workspace.as_ref().and_then(|(root, ws)| Some((root, ws.get("dependencies")?.get(key)?)));
        if let Some((root, entry)) = ws_entry {
            if package.is_none() {
                package = entry.get("package").and_then(|p| p.as_str()).map(str::to_string);
            }
            path = entry.get("path").and_then(|p| p.as_str()).map(|p| normalize(&root.join(p)));
            optional |= entry.get("optional").and_then(|o| o.as_bool()) == Some(true);
        }
    }
    let renamed = package.is_some();
    let package = package.unwrap_or_else(|| key.to_string());
    let target = match path {
        Some(dir) => DepTarget::Path { dir, package },
        None => DepTarget::External { crate_name: package.replace('-', "_") },
    };
    DepEntry { key: key.replace('-', "_"), renamed, target, gated: in_target || optional }
}

/// Which table of a manifest a dependency is declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepKind {
    /// `[dependencies]`.
    Normal,
    /// `[build-dependencies]`: the build script's.
    Build,
    /// `[dev-dependencies]`: tests', examples' and benches'.
    Dev,
}

impl DepKind {
    /// The table names cargo reads for the kind (the `_` spellings are cargo's older ones).
    fn table_names(self) -> &'static [&'static str] {
        match self {
            DepKind::Normal => &["dependencies"],
            DepKind::Build => &["build-dependencies", "build_dependencies"],
            DepKind::Dev => &["dev-dependencies", "dev_dependencies"],
        }
    }
}

/// A path dependency of one loaded package on another, as its manifest declares it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathDep {
    /// The absolute manifest that declares the dependency.
    pub manifest: PathBuf,
    /// The absolute manifest of the package depended on.
    pub target: PathBuf,
    pub kind: DepKind,
    /// The 1-based line of the entry in `manifest`, when the manifest's spans give it.
    pub line: Option<usize>,
}

/// Every path dependency between the packages among `manifests` (absolute, or relative to `project_root`), from
/// every dependency table (target-specific and optional entries included), sorted. A path to a folder that holds
/// no loaded package, or a package of another name, links nothing; neither does a package naming itself.
pub fn path_dependencies(project_root: &Path, manifests: &[PathBuf]) -> Vec<PathDep> {
    let packages = read_packages(project_root, manifests);
    let by_dir: BTreeMap<&Path, &Package> = packages.iter().map(|p| (p.dir.as_path(), p)).collect();
    let mut out = BTreeSet::new();
    for pkg in &packages {
        let lines = DepLines::read(&pkg.manifest);
        for kind in [DepKind::Normal, DepKind::Build, DepKind::Dev] {
            for table in dep_tables(pkg, kind) {
                for (key, value) in table.entries {
                    let DepTarget::Path { dir, package } = dep_entry(pkg, key, value, false).target else { continue };
                    let Some(dep) = by_dir.get(dir.as_path()).filter(|d| d.name == package && d.dir != pkg.dir) else {
                        continue;
                    };
                    let line = lines.line(table.target, table.name, key);
                    out.insert(PathDep { manifest: pkg.manifest.clone(), target: dep.manifest.clone(), kind, line });
                }
            }
        }
    }
    out.into_iter().collect()
}

/// One dependency table, with the span of each entry's key.
type EntrySpans = BTreeMap<toml::Spanned<String>, serde::de::IgnoredAny>;

/// The dependency tables of a manifest, with the span of each entry.
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct TableSpans {
    dependencies: EntrySpans,
    #[serde(rename = "build-dependencies")]
    build: EntrySpans,
    #[serde(rename = "build_dependencies")]
    build_old: EntrySpans,
    #[serde(rename = "dev-dependencies")]
    dev: EntrySpans,
    #[serde(rename = "dev_dependencies")]
    dev_old: EntrySpans,
    target: BTreeMap<String, TableSpans>,
}

impl TableSpans {
    fn table(&self, name: &str) -> Option<&EntrySpans> {
        match name {
            "dependencies" => Some(&self.dependencies),
            "build-dependencies" => Some(&self.build),
            "build_dependencies" => Some(&self.build_old),
            "dev-dependencies" => Some(&self.dev),
            "dev_dependencies" => Some(&self.dev_old),
            _ => None,
        }
    }
}

/// Where each dependency entry of a manifest starts.
#[derive(Default)]
struct DepLines {
    text: String,
    spans: TableSpans,
}

impl DepLines {
    fn read(manifest: &Path) -> DepLines {
        let text = std::fs::read_to_string(manifest).unwrap_or_default();
        let spans = toml::from_str(&text).unwrap_or_default();
        DepLines { text, spans }
    }

    /// The 1-based line of entry `key` in table `name` (under `[target.<target>]` when given).
    fn line(&self, target: Option<&str>, name: &str, key: &str) -> Option<usize> {
        let tables = match target {
            Some(t) => self.spans.target.get(t)?,
            None => &self.spans,
        };
        let start = tables.table(name)?.keys().find(|k| k.get_ref() == key)?.span().start;
        Some(self.text.get(..start)?.matches('\n').count() + 1)
    }
}

/// The lib and bin targets of a package: (kind, crate name, absolute root file).
fn package_targets(pkg: &Package) -> Vec<(TargetKind, String, PathBuf)> {
    let mut out = Vec::new();
    let lib = pkg.table.get("lib");
    let lib_path = lib
        .and_then(|l| l.get("path"))
        .and_then(|p| p.as_str())
        .map(|p| normalize(&pkg.dir.join(p)))
        .or_else(|| Some(pkg.dir.join("src/lib.rs")).filter(|p| p.is_file()));
    if let Some(path) = lib_path {
        let name = lib
            .and_then(|l| l.get("name"))
            .and_then(|n| n.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| pkg.name.replace('-', "_"));
        let proc_macro = lib
            .and_then(|l| l.get("proc-macro").or_else(|| l.get("proc_macro")))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push((if proc_macro { TargetKind::ProcMacro } else { TargetKind::Lib }, name, path));
    }

    let mut bins: Vec<(String, PathBuf)> = Vec::new();
    if let Some(list) = pkg.table.get("bin").and_then(|b| b.as_array()) {
        for entry in list {
            let Some(name) = entry.get("name").and_then(|n| n.as_str()) else { continue };
            let path = match entry.get("path").and_then(|p| p.as_str()) {
                Some(p) => Some(normalize(&pkg.dir.join(p))),
                None => {
                    let mut candidates = vec![
                        pkg.dir.join(format!("src/bin/{name}.rs")),
                        pkg.dir.join(format!("src/bin/{name}/main.rs")),
                    ];
                    if name == pkg.name {
                        candidates.push(pkg.dir.join("src/main.rs"));
                    }
                    candidates.into_iter().find(|p| p.is_file())
                }
            };
            if let Some(path) = path {
                bins.push((name.to_string(), path));
            }
        }
    }
    let autobins = pkg.table.get("package").and_then(|p| p.get("autobins")).and_then(|v| v.as_bool()).unwrap_or(true);
    if autobins {
        let mut inferred = Vec::new();
        let main = pkg.dir.join("src/main.rs");
        if main.is_file() {
            inferred.push((pkg.name.clone(), main));
        }
        if let Ok(entries) = std::fs::read_dir(pkg.dir.join("src/bin")) {
            let mut found: Vec<(String, PathBuf)> = Vec::new();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        found.push((stem.to_string(), path.clone()));
                    }
                } else if path.is_dir() && path.join("main.rs").is_file() {
                    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                        found.push((name.to_string(), path.join("main.rs")));
                    }
                }
            }
            found.sort();
            inferred.extend(found);
        }
        for (name, path) in inferred {
            let path = normalize(&path);
            if !bins.iter().any(|(n, p)| *n == name || *p == path) {
                bins.push((name, path));
            }
        }
    }
    out.extend(bins.into_iter().map(|(name, path)| (TargetKind::Bin, name, path)));
    out
}

/// Removes `.` and folds `..` lexically (no filesystem access, symlinks are not followed).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
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
    fn normalize_folds_parent_components() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
    }

    #[test]
    fn explicit_bins_replace_inferred_ones_and_autobins_off_drops_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "p/Cargo.toml",
            "[package]\nname = \"my-pkg\"\n\n[[bin]]\nname = \"tool\"\npath = \"src/bin/tool_main.rs\"\n",
        );
        write(root, "p/src/lib.rs", "");
        write(root, "p/src/main.rs", "fn main() {}");
        write(root, "p/src/bin/tool_main.rs", "fn main() {}");
        write(root, "p/src/bin/other/main.rs", "fn main() {}");
        write(root, "q/Cargo.toml", "[package]\nname = \"q\"\nautobins = false\n[lib]\nproc-macro = true\n");
        write(root, "q/src/lib.rs", "");
        write(root, "q/src/main.rs", "fn main() {}");
        let g = CrateGraph::from_manifests(root, &[PathBuf::from("q/Cargo.toml"), PathBuf::from("p/Cargo.toml")]);
        let got: Vec<(String, TargetKind, String)> = g
            .crates
            .iter()
            .map(|c| (c.name.clone(), c.kind, c.root_file.strip_prefix(root).unwrap().display().to_string()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("my_pkg".to_string(), TargetKind::Lib, "p/src/lib.rs".to_string()),
                ("my-pkg".to_string(), TargetKind::Bin, "p/src/main.rs".to_string()),
                ("other".to_string(), TargetKind::Bin, "p/src/bin/other/main.rs".to_string()),
                ("tool".to_string(), TargetKind::Bin, "p/src/bin/tool_main.rs".to_string()),
                ("q".to_string(), TargetKind::ProcMacro, "q/src/lib.rs".to_string()),
            ]
        );
        // Every binary depends on its own library.
        assert_eq!(g.crates[1].deps, vec![("my_pkg".to_string(), 0)]);
    }

    #[test]
    fn path_and_target_dependencies_link_and_others_are_external() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "a/Cargo.toml",
            "[package]\nname = \"a\"\n[dependencies]\nserde-json = { version = \"1\", package = \"serde_json\" }\n\
             [target.'cfg(unix)'.dependencies]\nbee = { path = \"../b\", package = \"b-crate\" }\n",
        );
        write(root, "a/src/lib.rs", "");
        write(root, "b/Cargo.toml", "[package]\nname = \"b-crate\"\n");
        write(root, "b/src/lib.rs", "");
        let g = CrateGraph::from_manifests(root, &[root.join("a/Cargo.toml"), root.join("b/Cargo.toml")]);
        assert_eq!(g.crates[0].deps, vec![("bee".to_string(), 1)]);
        assert_eq!(g.crates[0].external_deps, vec![("serde_json".to_string(), "serde_json".to_string())]);
        assert_eq!(g.crates[1].name, "b_crate");
        // Only declared for `cfg(unix)`.
        assert_eq!(g.crates[0].gated_deps, vec!["bee".to_string()]);
    }

    #[test]
    fn one_name_for_two_crates_is_gated_and_one_crate_in_two_tables_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "a/Cargo.toml",
            "[package]\nname = \"a\"\n[dependencies]\nb = { path = \"../b\" }\nlog = \"0.4\"\nopt = { version = \"1\", optional = true }\n\
             [target.'cfg(unix)'.dependencies]\nb = { path = \"../b\", features = [\"x\"] }\ntwin = { path = \"../b\", package = \"b\" }\n\
             [target.'cfg(windows)'.dependencies]\ntwin = { path = \"../c\", package = \"c\" }\nlog = { path = \"../c\", package = \"c\" }\n",
        );
        write(root, "a/src/lib.rs", "");
        write(root, "b/Cargo.toml", "[package]\nname = \"b\"\n");
        write(root, "b/src/lib.rs", "");
        write(root, "c/Cargo.toml", "[package]\nname = \"c\"\n");
        write(root, "c/src/lib.rs", "");
        let g = CrateGraph::from_manifests(
            root,
            &[root.join("a/Cargo.toml"), root.join("b/Cargo.toml"), root.join("c/Cargo.toml")],
        );
        let a = &g.crates[0];
        // `b` twice for the same crate stays unconditional; `twin` and `log` name two crates: neither is linked.
        assert_eq!(a.deps, vec![("b".to_string(), 1)]);
        assert_eq!(a.external_deps, vec![("opt".to_string(), "opt".to_string())]);
        assert_eq!(a.gated_deps, vec!["log".to_string(), "opt".to_string(), "twin".to_string()]);
    }

    #[test]
    fn path_dependencies_come_from_every_table_with_their_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"a\", \"b\", \"c\", \"d\"]\n[workspace.dependencies]\nd = { path = \"d\" }\n",
        );
        write(
            root,
            "a/Cargo.toml",
            "[package]\nname = \"a\"\n\n[dependencies]\nlog = \"0.4\"\nb = { path = \"../b\" }\nd.workspace = true\n\n\
             [dependencies.c]\npath = \"../c\"\n\n[build_dependencies]\nb = { path = \"../b\" }\n\
             [target.'cfg(unix)'.dev-dependencies]\nc = { path = \"../c\" }\nnone = { path = \"../missing\" }\n",
        );
        write(
            root,
            "b/Cargo.toml",
            "[package]\nname = \"b\"\n[dependencies]\nself-b = { path = \".\", package = \"b\" }\n",
        );
        write(
            root,
            "c/Cargo.toml",
            "[package]\nname = \"c\"\n[dev-dependencies]\nwrong = { path = \"../b\", package = \"x\" }\n",
        );
        write(root, "d/Cargo.toml", "[package]\nname = \"d\"\n");
        let manifests: Vec<PathBuf> =
            ["a", "b", "c", "d", ""].iter().map(|d| root.join(d).join("Cargo.toml")).collect();
        let got: Vec<(String, String, DepKind, Option<usize>)> = path_dependencies(root, &manifests)
            .into_iter()
            .map(|d| {
                let rel = |p: &Path| p.parent().unwrap().strip_prefix(root).unwrap().display().to_string();
                (rel(&d.manifest), rel(&d.target), d.kind, d.line)
            })
            .collect();
        let row = |a: &str, b: &str, kind, line| (a.to_string(), b.to_string(), kind, Some(line));
        assert_eq!(
            got,
            vec![
                row("a", "b", DepKind::Normal, 6),
                row("a", "b", DepKind::Build, 13),
                row("a", "c", DepKind::Normal, 9),
                row("a", "c", DepKind::Dev, 15),
                row("a", "d", DepKind::Normal, 7),
            ]
        );
    }

    #[test]
    fn extern_names_follow_the_lib_name_unless_renamed_and_editions_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "Cargo.toml", "[workspace]\nmembers = [\"a\", \"b\"]\n[workspace.package]\nedition = \"2021\"\n");
        write(
            root,
            "a/Cargo.toml",
            "[package]\nname = \"a\"\nedition.workspace = true\n[dependencies]\nb-pkg = { path = \"../b\" }\n\
             again = { path = \"../b\", package = \"b-pkg\" }\n",
        );
        write(root, "a/src/lib.rs", "");
        write(root, "b/Cargo.toml", "[package]\nname = \"b-pkg\"\n[lib]\nname = \"b_core\"\n");
        write(root, "b/src/lib.rs", "");
        let g = CrateGraph::from_manifests(root, &[root.join("a/Cargo.toml"), root.join("b/Cargo.toml")]);
        assert_eq!(g.crates[0].deps, vec![("again".to_string(), 1), ("b_core".to_string(), 1)]);
        assert_eq!((g.crates[0].edition, g.crates[1].edition), (2021, 2015));
    }
}
