//! Packages: what each Cargo package builds (its targets) and which of the project's other
//! packages it depends on.
//!
//! Read with `cargo metadata --no-deps --offline` from each workspace root, which is exactly what
//! cargo itself sees. Packages cargo cannot read (no toolchain, a broken manifest, a package
//! outside every workspace) are read from their `Cargo.toml` with cargo's own rules for finding
//! targets (`src/main.rs`, `src/bin/*`, `examples/*`, `[[bin]]`, `autobins`, ...).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use studio_parser::analysis::crate_graph::normalize;

use crate::exec::{Command, Runner};

/// What a package builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    Lib,
    Bin,
    Example,
    Test,
    Bench,
    BuildScript,
}

impl TargetKind {
    pub fn label(self) -> &'static str {
        match self {
            TargetKind::Lib => "library",
            TargetKind::Bin => "binary",
            TargetKind::Example => "example",
            TargetKind::Test => "test",
            TargetKind::Bench => "benchmark",
            TargetKind::BuildScript => "build script",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub kind: TargetKind,
    pub name: String,
    pub src: PathBuf,
}

/// Where a package's facts come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSource {
    /// `cargo metadata`.
    Cargo,
    /// The manifest, read with cargo's rules, because cargo could not be asked.
    Manifest,
}

/// Which table a dependency is declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DependencyKind {
    /// `[dependencies]`: what the library and binaries compile against.
    Normal,
    /// `[dev-dependencies]`.
    Dev,
    /// `[build-dependencies]`.
    Build,
}

/// One dependency entry of a package, as cargo sees it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dependency {
    /// The depended-on package's name.
    pub name: String,
    /// The key the manifest binds it to when it renames it (`key = { package = "name" }`), as written.
    pub rename: Option<String>,
    pub kind: DependencyKind,
    /// The absolute folder of a path dependency (also through `workspace = true`).
    pub path: Option<PathBuf>,
    /// The platform of a `[target.'…'.dependencies]` entry.
    pub target: Option<String>,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub manifest: PathBuf,
    pub root: PathBuf,
    pub targets: Vec<Target>,
    /// Packages of this project it depends on (by name), from path and workspace dependencies.
    pub depends_on: Vec<String>,
    pub source: PackageSource,
    /// The Rust edition ("2021"), empty when the manifest names none.
    pub edition: String,
    /// The library is a procedural macro crate (`proc-macro = true`).
    pub proc_macro: bool,
    /// Every dependency entry, renames included, sorted.
    pub dependencies: Vec<Dependency>,
}

/// Every package found, and why cargo could not be asked where it was not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packages {
    pub packages: Vec<Package>,
    /// (workspace manifest, what went wrong) for each workspace read from its manifests instead.
    pub fallbacks: Vec<(PathBuf, String)>,
}

impl Packages {
    pub fn find(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }
}

/// The job that reads the packages of `manifests` for a [`crate::SourceHub`].
pub fn packages_job(manifests: Vec<PathBuf>) -> impl FnOnce(&crate::JobContext) -> Result<(), crate::JobError> + Send {
    move |ctx| {
        if manifests.is_empty() {
            return Err(crate::JobError::Unavailable("No Cargo packages here".to_string()));
        }
        let packages = read_packages(&ctx.runner, &manifests);
        ctx.send(crate::SourceEvent::Packages(std::sync::Arc::new(packages)));
        Ok(())
    }
}

/// Reads the packages of every `Cargo.toml` in `manifests` (as found on disk): each workspace
/// through cargo, everything cargo did not cover from its manifest.
pub fn read_packages(runner: &Runner, manifests: &[PathBuf]) -> Packages {
    let mut out = Packages::default();
    let mut covered: Vec<PathBuf> = Vec::new();
    let workspaces: Vec<&PathBuf> = manifests.iter().filter(|m| manifest_table(m, "workspace").is_some()).collect();
    for workspace in workspaces {
        match runner
            .run(&Command::cargo_metadata(workspace))
            .map_err(|e| e.to_string())
            .and_then(|o| parse_metadata(&o.stdout))
        {
            Ok(packages) => {
                covered.extend(packages.iter().map(|p| p.manifest.clone()));
                out.packages.extend(packages);
            }
            Err(why) => out.fallbacks.push((workspace.clone(), why)),
        }
    }
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let covered: Vec<PathBuf> = covered.iter().map(|p| canonical(p)).collect();
    for manifest in manifests {
        if covered.contains(&canonical(manifest)) {
            continue;
        }
        if let Some(package) = read_manifest(manifest) {
            out.packages.push(package);
        }
    }
    resolve_workspace_dependencies(&mut out.packages, manifests);
    out.packages.sort_by(|a, b| a.name.cmp(&b.name).then(a.manifest.cmp(&b.manifest)));
    out.packages.dedup_by(|a, b| a.manifest == b.manifest);
    out
}

/// Reads every `Cargo.toml` in `manifests` from the manifest alone, with cargo's rules, never asking cargo (the
/// fallback [`read_packages`] uses where cargo cannot be asked).
pub fn read_manifests(manifests: &[PathBuf]) -> Packages {
    let mut out =
        Packages { packages: manifests.iter().filter_map(|m| read_manifest(m)).collect(), ..Default::default() };
    resolve_workspace_dependencies(&mut out.packages, manifests);
    out.packages.sort_by(|a, b| a.name.cmp(&b.name).then(a.manifest.cmp(&b.manifest)));
    out.packages.dedup_by(|a, b| a.manifest == b.manifest);
    out
}

/// Packages from `cargo metadata --format-version 1` output.
fn parse_metadata(stdout: &[u8]) -> Result<Vec<Package>, String> {
    let json: Value = serde_json::from_slice(stdout).map_err(|e| format!("unreadable cargo metadata: {e}"))?;
    let packages = json.get("packages").and_then(Value::as_array).ok_or("cargo metadata without packages")?;
    let mut out = Vec::new();
    for p in packages {
        let text = |key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let manifest = PathBuf::from(text("manifest_path"));
        let root = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut targets = Vec::new();
        let mut proc_macro = false;
        for t in p.get("targets").and_then(Value::as_array).into_iter().flatten() {
            let kinds: Vec<&str> =
                t.get("kind").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            proc_macro |= kinds.contains(&"proc-macro");
            let kind = match kinds.first().copied() {
                Some("bin") => TargetKind::Bin,
                Some("example") => TargetKind::Example,
                Some("test") => TargetKind::Test,
                Some("bench") => TargetKind::Bench,
                Some("custom-build") => TargetKind::BuildScript,
                _ => TargetKind::Lib,
            };
            let name = t.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let src = PathBuf::from(t.get("src_path").and_then(Value::as_str).unwrap_or_default());
            targets.push(Target { kind, name, src });
        }
        targets.sort();
        let depends_on = p
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|d| d.get("path").is_some_and(|v| !v.is_null()))
            .filter_map(|d| d.get("name").and_then(Value::as_str).map(str::to_string))
            .collect();
        let mut dependencies: Vec<Dependency> = p
            .get("dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|d| {
                let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
                Dependency {
                    name: text("name").unwrap_or_default(),
                    rename: text("rename"),
                    kind: match d.get("kind").and_then(Value::as_str) {
                        Some("dev") => DependencyKind::Dev,
                        Some("build") => DependencyKind::Build,
                        _ => DependencyKind::Normal,
                    },
                    path: text("path").map(PathBuf::from),
                    target: text("target"),
                    optional: d.get("optional").and_then(Value::as_bool).unwrap_or(false),
                }
            })
            .collect();
        dependencies.sort();
        out.push(Package {
            name: text("name"),
            version: text("version"),
            manifest,
            root,
            targets,
            depends_on,
            source: PackageSource::Cargo,
            edition: text("edition"),
            proc_macro,
            dependencies,
        });
    }
    Ok(out)
}

fn read_toml(manifest: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(manifest).ok()?.parse().ok()
}

fn manifest_table(manifest: &Path, table: &str) -> Option<toml::Value> {
    read_toml(manifest)?.get(table).cloned()
}

/// A package read from its manifest with cargo's target rules.
fn read_manifest(manifest: &Path) -> Option<Package> {
    let toml = read_toml(manifest)?;
    let package = toml.get("package")?;
    let name = package.get("name")?.as_str()?.to_string();
    let version = package.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let root = manifest.parent()?.to_path_buf();
    let auto = |key: &str| package.get(key).and_then(|v| v.as_bool()).unwrap_or(true);
    let mut targets: BTreeMap<(TargetKind, String), PathBuf> = BTreeMap::new();

    let lib = toml.get("lib");
    let lib_path = lib.and_then(|l| l.get("path")).and_then(|p| p.as_str()).map(|p| root.join(p));
    if let Some(path) = lib_path.or_else(|| Some(root.join("src/lib.rs")).filter(|p| p.is_file())) {
        let lib_name = lib.and_then(|l| l.get("name")).and_then(|n| n.as_str()).unwrap_or(&name).replace('-', "_");
        targets.insert((TargetKind::Lib, lib_name), path);
    }
    if root.join("build.rs").is_file() && package.get("build").and_then(|b| b.as_bool()) != Some(false) {
        targets.insert((TargetKind::BuildScript, "build-script-build".to_string()), root.join("build.rs"));
    }
    // Targets found from the folder layout, unless turned off with `autobins = false` and friends.
    let discovered = [
        (TargetKind::Bin, "autobins", "src/bin"),
        (TargetKind::Example, "autoexamples", "examples"),
        (TargetKind::Test, "autotests", "tests"),
        (TargetKind::Bench, "autobenches", "benches"),
    ];
    if auto("autobins") && root.join("src/main.rs").is_file() {
        targets.insert((TargetKind::Bin, name.clone()), root.join("src/main.rs"));
    }
    for (kind, flag, folder) in discovered {
        if !auto(flag) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root.join(folder)) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            if path.extension().is_some_and(|e| e == "rs") {
                targets.insert((kind, stem), path);
            } else if path.join("main.rs").is_file() {
                targets.insert((kind, stem), path.join("main.rs"));
            }
        }
    }
    // Explicit `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]` entries come last and win.
    for (kind, key, default_dir) in [
        (TargetKind::Bin, "bin", "src/bin"),
        (TargetKind::Example, "example", "examples"),
        (TargetKind::Test, "test", "tests"),
        (TargetKind::Bench, "bench", "benches"),
    ] {
        for entry in toml.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
            let Some(target_name) = entry.get("name").and_then(|n| n.as_str()) else { continue };
            let path = entry
                .get("path")
                .and_then(|p| p.as_str())
                .map(|p| root.join(p))
                .unwrap_or_else(|| root.join(default_dir).join(format!("{target_name}.rs")));
            targets.insert((kind, target_name.to_string()), path);
        }
    }

    let workspace = workspace_of(manifest, &toml);
    let ws_package = |key: &str| {
        let (_, ws) = workspace.as_ref()?;
        ws.get("workspace")?.get("package")?.get(key)?.as_str().map(str::to_string)
    };
    let edition = match package.get("edition") {
        Some(toml::Value::String(e)) => e.clone(),
        Some(v) if v.get("workspace").and_then(|w| w.as_bool()) == Some(true) => {
            ws_package("edition").unwrap_or_default()
        }
        _ => String::new(),
    };
    let proc_macro = lib
        .and_then(|l| l.get("proc-macro").or_else(|| l.get("proc_macro")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let dependencies = manifest_dependencies(&toml, &root, workspace.as_ref());

    let mut depends_on = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for (dep, spec) in toml.get(table).and_then(|t| t.as_table()).into_iter().flatten() {
            if spec.get("path").is_some() || spec.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                let real = spec.get("package").and_then(|p| p.as_str()).unwrap_or(dep);
                depends_on.push(real.to_string());
            }
        }
    }
    depends_on.sort();
    depends_on.dedup();
    Some(Package {
        name,
        version,
        manifest: manifest.to_path_buf(),
        root,
        targets: targets.into_iter().map(|((kind, name), src)| Target { kind, name, src }).collect(),
        depends_on,
        source: PackageSource::Manifest,
        edition,
        proc_macro,
        dependencies,
    })
}

/// The workspace a manifest belongs to: its own `[workspace]`, `package.workspace = "…"`, or the nearest folder
/// above with a `Cargo.toml` that has one. Returns the root folder and the parsed root manifest.
fn workspace_of(manifest: &Path, toml: &toml::Value) -> Option<(PathBuf, toml::Value)> {
    let dir = manifest.parent()?;
    if toml.get("workspace").is_some() {
        return Some((dir.to_path_buf(), toml.clone()));
    }
    if let Some(explicit) = toml.get("package").and_then(|p| p.get("workspace")).and_then(|w| w.as_str()) {
        let root = dir.join(explicit);
        let t = read_toml(&root.join("Cargo.toml"))?;
        return Some((root, t));
    }
    let mut cur = dir.parent();
    while let Some(d) = cur {
        if let Some(t) = read_toml(&d.join("Cargo.toml")).filter(|t| t.get("workspace").is_some()) {
            return Some((d.to_path_buf(), t));
        }
        cur = d.parent();
    }
    None
}

/// Every dependency entry of a manifest, with renames (`package = …`) and `workspace = true` entries looked up in
/// the workspace's `[workspace.dependencies]`, sorted.
fn manifest_dependencies(
    toml: &toml::Value,
    root: &Path,
    workspace: Option<&(PathBuf, toml::Value)>,
) -> Vec<Dependency> {
    let kinds = [
        ("dependencies", DependencyKind::Normal),
        ("dev-dependencies", DependencyKind::Dev),
        ("build-dependencies", DependencyKind::Build),
    ];
    let mut tables: Vec<(&toml::Value, DependencyKind, Option<String>)> = Vec::new();
    for (key, kind) in kinds {
        if let Some(t) = toml.get(key) {
            tables.push((t, kind, None));
        }
    }
    for (platform, spec) in toml.get("target").and_then(|t| t.as_table()).into_iter().flatten() {
        for (key, kind) in kinds {
            if let Some(t) = spec.get(key) {
                tables.push((t, kind, Some(platform.clone())));
            }
        }
    }
    let ws_deps = workspace.and_then(|(dir, ws)| Some((dir, ws.get("workspace")?.get("dependencies")?)));
    let mut out = Vec::new();
    for (table, kind, target) in tables {
        for (key, spec) in table.as_table().into_iter().flatten() {
            let text = |v: &toml::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
            let mut package = text(spec, "package");
            let mut path = text(spec, "path").map(|p| normalize(&root.join(p)));
            let mut optional = spec.get("optional").and_then(|o| o.as_bool()) == Some(true);
            if spec.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                if let Some(entry) = ws_deps.and_then(|(dir, deps)| Some((dir, deps.get(key)?))) {
                    let (dir, entry) = entry;
                    package = package.or_else(|| text(entry, "package"));
                    path = text(entry, "path").map(|p| normalize(&dir.join(p)));
                    optional |= entry.get("optional").and_then(|o| o.as_bool()) == Some(true);
                }
            }
            let rename = package.as_ref().map(|_| key.clone());
            out.push(Dependency {
                name: package.unwrap_or_else(|| key.clone()),
                rename,
                kind,
                path,
                target: target.clone(),
                optional,
            });
        }
    }
    out.sort();
    out
}

/// Keeps only dependencies that are packages of this project (`workspace = true` entries can name
/// registry crates too).
fn resolve_workspace_dependencies(packages: &mut [Package], _manifests: &[PathBuf]) {
    let names: std::collections::BTreeSet<String> = packages.iter().map(|p| p.name.clone()).collect();
    for p in packages.iter_mut() {
        p.depends_on.retain(|d| names.contains(d) && d != &p.name);
        p.depends_on.sort();
        p.depends_on.dedup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CancelToken;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    /// A workspace with a library, a tool with two binaries (one declared), and a package
    /// outside the workspace.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "Cargo.toml", "[workspace]\nmembers = [\"core\", \"tool\"]\nresolver = \"2\"\n");
        write(r, "core/Cargo.toml", "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        write(r, "core/src/lib.rs", "");
        write(
            r,
            "tool/Cargo.toml",
            "[package]\nname = \"tool\"\nversion = \"0.2.0\"\nedition = \"2021\"\n\n[dependencies]\ncore = { path = \"../core\" }\n\n[[bin]]\nname = \"gen\"\npath = \"src/generate.rs\"\n",
        );
        write(r, "tool/src/main.rs", "fn main() {}");
        write(r, "tool/src/generate.rs", "fn main() {}");
        write(r, "tool/examples/demo.rs", "fn main() {}");
        write(
            r,
            "loose/Cargo.toml",
            "[package]\nname = \"loose\"\nversion = \"1.0.0\"\nedition = \"2021\"\nautobins = false\n",
        );
        write(r, "loose/src/main.rs", "fn main() {}");
        write(r, "loose/src/bin/hidden.rs", "fn main() {}");
        dir
    }

    fn manifests(root: &Path) -> Vec<PathBuf> {
        ["Cargo.toml", "core/Cargo.toml", "tool/Cargo.toml", "loose/Cargo.toml"].map(|m| root.join(m)).into()
    }

    fn targets(p: &Package) -> Vec<(TargetKind, &str)> {
        p.targets.iter().map(|t| (t.kind, t.name.as_str())).collect()
    }

    #[test]
    fn manifests_follow_cargos_target_rules() {
        let dir = fixture();
        let tool = read_manifest(&dir.path().join("tool/Cargo.toml")).unwrap();
        assert_eq!(
            targets(&tool),
            [(TargetKind::Bin, "gen"), (TargetKind::Bin, "tool"), (TargetKind::Example, "demo")]
        );
        assert_eq!(tool.depends_on, ["core"]);
        let loose = read_manifest(&dir.path().join("loose/Cargo.toml")).unwrap();
        assert!(targets(&loose).is_empty(), "autobins = false finds no binaries: {:?}", targets(&loose));
        assert!(read_manifest(&dir.path().join("Cargo.toml")).is_none(), "a workspace is not a package");
    }

    #[test]
    fn cargo_and_the_manifest_agree() {
        let dir = fixture();
        let runner = Runner::new(CancelToken::default());
        let read = read_packages(&runner, &manifests(dir.path()));
        let names: Vec<&str> = read.packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["core", "loose", "tool"]);
        let tool = read.find("tool").unwrap();
        if read.fallbacks.is_empty() {
            assert_eq!(tool.source, PackageSource::Cargo, "cargo read the workspace");
        }
        assert_eq!(targets(tool), [(TargetKind::Bin, "gen"), (TargetKind::Bin, "tool"), (TargetKind::Example, "demo")]);
        assert_eq!(tool.depends_on, ["core"]);
        assert_eq!(read.find("loose").unwrap().source, PackageSource::Manifest, "outside every workspace");
    }
}
