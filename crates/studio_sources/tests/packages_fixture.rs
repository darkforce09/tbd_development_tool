//! `read_packages` (through `cargo metadata --offline --no-deps`) and `read_manifests` (the TOML fallback) on
//! `tests/fixtures/packages`, both checked against its `expected.toml`.

use std::path::{Path, PathBuf};

use studio_sources::packages::{read_manifests, Dependency, DependencyKind};
use studio_sources::{read_packages, CancelToken, Package, PackageSource, Packages, Runner};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages")
}

fn expected() -> toml::Table {
    std::fs::read_to_string(fixture().join("expected.toml")).unwrap().parse().unwrap()
}

fn strings(value: &toml::Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

/// The fixture copied into a new temporary folder (cargo then writes nothing next to the sources), and that
/// folder with symlinks resolved, as cargo reports it.
fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let from = fixture();
    for entry in walkdir::WalkDir::new(&from).min_depth(1) {
        let entry = entry.unwrap();
        let to = dir.path().join(entry.path().strip_prefix(&from).unwrap());
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&to).unwrap();
        } else if entry.file_name() != "expected.toml" {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
    let root = dir.path().canonicalize().unwrap();
    (dir, root)
}

/// Every `Cargo.toml` under `root`, sorted, as the folder scan finds them.
fn manifests(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() == "Cargo.toml")
        .map(|e| e.into_path())
        .collect();
    found.sort();
    found
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or_else(|_| panic!("{} outside the fixture", path.display())).display().to_string()
}

fn dependency(root: &Path, d: &Dependency) -> String {
    let kind = match d.kind {
        DependencyKind::Normal => "normal",
        DependencyKind::Dev => "dev",
        DependencyKind::Build => "build",
    };
    let rename = d.rename.as_ref().map(|r| format!(" as {r}")).unwrap_or_default();
    let path = d.path.as_ref().map(|p| format!(" path={}", rel(root, p))).unwrap_or_default();
    let target = d.target.as_ref().map(|t| format!(" target={t}")).unwrap_or_default();
    let optional = if d.optional { " optional" } else { "" };
    format!("{}{rename} {kind}{path}{target}{optional}", d.name)
}

/// One table per package, as `expected.toml` writes it.
fn describe(root: &Path, p: &Package) -> toml::Value {
    let list = |items: Vec<String>| toml::Value::Array(items.into_iter().map(toml::Value::String).collect());
    let mut t = toml::Table::new();
    t.insert("version".into(), p.version.clone().into());
    t.insert("edition".into(), p.edition.clone().into());
    t.insert("manifest".into(), rel(root, &p.manifest).into());
    t.insert("root".into(), rel(root, &p.root).into());
    t.insert("proc_macro".into(), p.proc_macro.into());
    let targets = p.targets.iter().map(|t| format!("{} {} {}", t.kind.label(), t.name, rel(root, &t.src))).collect();
    t.insert("targets".into(), list(targets));
    t.insert("depends_on".into(), list(p.depends_on.clone()));
    t.insert("dependencies".into(), list(p.dependencies.iter().map(|d| dependency(root, d)).collect()));
    toml::Value::Table(t)
}

fn check(root: &Path, read: &Packages, source: PackageSource) {
    let expected = expected();
    let names: Vec<String> = read.packages.iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, strings(&expected["packages"]), "{source:?}");
    let described: toml::Table = read.packages.iter().map(|p| (p.name.clone(), describe(root, p))).collect();
    let wanted = expected["package"].as_table().unwrap();
    for name in &names {
        assert_eq!(described[name], wanted[name], "{name} ({source:?})");
    }
    assert_eq!(&described, wanted, "{source:?}");
    assert!(read.packages.iter().all(|p| p.source == source), "every package from {source:?}");
    assert!(read.fallbacks.is_empty(), "{:?}", read.fallbacks);
}

fn cargo_installed() -> bool {
    std::process::Command::new("cargo").arg("--version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn cargo_metadata_reads_the_fixture_as_expected() {
    if !cargo_installed() {
        eprintln!("cargo is not installed: the cargo metadata half is skipped (the manifest half still runs)");
        return;
    }
    let (_dir, root) = project();
    let read = read_packages(&Runner::new(CancelToken::default()), &manifests(&root));
    check(&root, &read, PackageSource::Cargo);
}

#[test]
fn the_manifest_fallback_reads_the_fixture_as_expected() {
    let (_dir, root) = project();
    check(&root, &read_manifests(&manifests(&root)), PackageSource::Manifest);
}
