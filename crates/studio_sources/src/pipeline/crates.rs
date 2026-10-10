//! The crate graph of the pipeline, built from what cargo reported ([`Packages`]) instead of reading the manifests
//! again. The rules are those of [`CrateGraph::from_manifests`]: library and binary targets only, the library's
//! name with `-` → `_`, normal dependencies (dev and build dependencies are not code the targets compile against),
//! a renamed dependency bound to its key, a path dependency linked to the package in that folder, a target-specific
//! or optional dependency listed as gated, one name bound to different crates gated and unlinked, and every binary
//! depending on its own package's library.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use studio_parser::analysis::crate_graph::{normalize, CrateGraph, CrateNode, TargetKind};

use crate::packages::{DependencyKind, Packages, TargetKind as PackageTarget};

/// What one dependency entry links a crate to.
#[derive(Clone, PartialEq, Eq)]
enum Link {
    Workspace(usize),
    External(String),
}

/// The crate graph of `packages`, sorted by (manifest, kind, name) like [`CrateGraph::from_manifests`].
pub fn crate_graph(packages: &Packages) -> CrateGraph {
    let mut pkgs: Vec<_> = packages.packages.iter().collect();
    pkgs.sort_by(|a, b| a.manifest.cmp(&b.manifest));
    pkgs.dedup_by(|a, b| a.manifest == b.manifest);

    let mut crates: Vec<(usize, CrateNode)> = Vec::new();
    for (pi, pkg) in pkgs.iter().enumerate() {
        let edition = match pkg.edition.as_str() {
            "2018" => 2018,
            "2021" => 2021,
            "2024" => 2024,
            _ => 2015,
        };
        for target in &pkg.targets {
            let (kind, name) = match target.kind {
                PackageTarget::Lib if pkg.proc_macro => (TargetKind::ProcMacro, target.name.replace('-', "_")),
                PackageTarget::Lib => (TargetKind::Lib, target.name.replace('-', "_")),
                PackageTarget::Bin => (TargetKind::Bin, target.name.clone()),
                _ => continue,
            };
            crates.push((
                pi,
                CrateNode {
                    package: pkg.name.clone(),
                    name,
                    kind,
                    root_file: normalize(&target.src),
                    manifest: normalize(&pkg.manifest),
                    deps: Vec::new(),
                    external_deps: Vec::new(),
                    edition,
                    gated_deps: Vec::new(),
                },
            ));
        }
    }
    crates.sort_by(|(_, a), (_, b)| (&a.manifest, a.kind, &a.name).cmp(&(&b.manifest, b.kind, &b.name)));

    // The library crate of each package, by package folder and by package index.
    let mut lib_of_dir: BTreeMap<PathBuf, (usize, &str)> = BTreeMap::new();
    let mut lib_of_pkg: BTreeMap<usize, usize> = BTreeMap::new();
    for (ci, (pi, node)) in crates.iter().enumerate() {
        if matches!(node.kind, TargetKind::Lib | TargetKind::ProcMacro) {
            lib_of_dir.insert(normalize(&pkgs[*pi].root), (ci, pkgs[*pi].name.as_str()));
            lib_of_pkg.insert(*pi, ci);
        }
    }

    let mut links = Vec::with_capacity(crates.len());
    for (ci, (pi, node)) in crates.iter().enumerate() {
        let mut by_name: BTreeMap<String, Vec<(Link, bool)>> = BTreeMap::new();
        for dep in pkgs[*pi].dependencies.iter().filter(|d| d.kind == DependencyKind::Normal) {
            let key = dep.rename.as_deref().unwrap_or(&dep.name).replace('-', "_");
            let gated = dep.target.is_some() || dep.optional;
            let (name, link) = match &dep.path {
                Some(dir) => match lib_of_dir.get(&normalize(dir)) {
                    Some(&(lib, package)) if package == dep.name && lib != ci => {
                        let name = if dep.rename.is_some() { key } else { crates[lib].1.name.clone() };
                        (name, Link::Workspace(lib))
                    }
                    None => (key, Link::External(dep.name.replace('-', "_"))),
                    Some(_) => continue,
                },
                None => (key, Link::External(dep.name.replace('-', "_"))),
            };
            by_name.entry(name).or_default().push((link, gated));
        }
        let mut deps: BTreeMap<String, usize> = BTreeMap::new();
        let mut external: BTreeMap<String, String> = BTreeMap::new();
        let mut gated: BTreeSet<String> = BTreeSet::new();
        for (name, entries) in by_name {
            let first = &entries[0].0;
            if entries.iter().any(|(link, _)| link != first) {
                gated.insert(name);
                continue;
            }
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
        links.push((deps, external, gated));
    }
    let crates = crates
        .into_iter()
        .zip(links)
        .map(|((_, mut node), (deps, external, gated))| {
            node.deps = deps.into_iter().collect();
            node.external_deps = external.into_iter().collect();
            node.gated_deps = gated.into_iter().collect();
            node
        })
        .collect();
    CrateGraph { crates }
}

/// The manifests of `packages`, for [`CrateGraph::from_manifests`].
pub fn manifests(packages: &Packages) -> Vec<PathBuf> {
    packages.packages.iter().map(|p| p.manifest.clone()).collect()
}

/// Every difference between two crate graphs, one line each (empty when they agree).
pub fn differences(a: &CrateGraph, b: &CrateGraph, root: &Path) -> Vec<String> {
    let key = |c: &CrateNode| (c.manifest.clone(), c.kind, c.name.clone());
    let show = |c: &CrateNode| {
        let manifest = c.manifest.strip_prefix(root).unwrap_or(&c.manifest);
        format!("{} {:?} {}", manifest.display(), c.kind, c.name)
    };
    let names = |g: &CrateGraph, deps: &[(String, usize)]| -> Vec<String> {
        deps.iter().map(|(n, i)| format!("{n}->{}", g.crates[*i].name)).collect()
    };
    let in_b: BTreeMap<_, _> = b.crates.iter().map(|c| (key(c), c)).collect();
    let in_a: BTreeMap<_, _> = a.crates.iter().map(|c| (key(c), c)).collect();
    let mut out = Vec::new();
    for (k, ca) in &in_a {
        let Some(cb) = in_b.get(k) else {
            out.push(format!("only in first: {}", show(ca)));
            continue;
        };
        if ca.root_file != cb.root_file {
            out.push(format!("{}: root {} vs {}", show(ca), ca.root_file.display(), cb.root_file.display()));
        }
        if ca.edition != cb.edition {
            out.push(format!("{}: edition {} vs {}", show(ca), ca.edition, cb.edition));
        }
        if names(a, &ca.deps) != names(b, &cb.deps) {
            out.push(format!("{}: deps {:?} vs {:?}", show(ca), names(a, &ca.deps), names(b, &cb.deps)));
        }
        if ca.external_deps != cb.external_deps {
            out.push(format!("{}: external {:?} vs {:?}", show(ca), ca.external_deps, cb.external_deps));
        }
        if ca.gated_deps != cb.gated_deps {
            out.push(format!("{}: gated {:?} vs {:?}", show(ca), ca.gated_deps, cb.gated_deps));
        }
        if ca.package != cb.package {
            out.push(format!("{}: package {} vs {}", show(ca), ca.package, cb.package));
        }
    }
    for (k, cb) in &in_b {
        if !in_a.contains_key(k) {
            out.push(format!("only in second: {}", show(cb)));
        }
    }
    out
}
