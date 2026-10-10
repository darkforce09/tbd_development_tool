//! R1 on the Code map: which Rust `use` imports and fn calls resolve to exactly one workspace item.
//!
//! [`analyze`] builds the crate graph from the package manifests the scan found and a [`RustIndex`] over it, then
//! takes the index's answers for every `use` leaf (file scope, [`RustIndex::resolved_uses`]) and every call site
//! ([`RustIndex::resolved_calls`]) of each Rust file in a module tree. The builder turns a unique, ungated resolution
//! into a Proven edge (`Basis::PathResolution`) to R1's target; everything else keeps the name match, which stays
//! Unresolved.
//!
//! What stays Unresolved: ambiguous names (two globs, or a `use` naming items in two files), macros, `#[path]`
//! modules, method calls, names reached through `#[cfg]`-gated items, and references written in `#[cfg]`-gated
//! code: a `use` under `#[cfg]`, a call in a gated statement, fn, `impl` or trait, or anywhere inside a gated module
//! (`#[cfg(…)] mod x;`, `#![cfg(…)]`). The index flags those (`UseLeaf::cfg_gated`, `CallSite::cfg_gated`). Test
//! code (`#[cfg(test)]`, `#[test]`) is not read by R1 and keeps the name match without being counted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::*;

use crate::analysis::crate_graph::{path_dependencies, CrateGraph, PathDep};
use crate::analysis::rust_resolver::{ItemKind, ItemRef, ModuleId, Resolution, RustIndex};
use crate::extractor::{ExtractedProject, UseItem};

/// What R1 did to the Code map, per load. Counters are per reference (one `use` leaf or one call site that R1
/// reads), not per edge: many references can share one file-to-file wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(derive(Debug))]
pub struct RustPathStats {
    /// Rust files resolved (files in a crate's module tree).
    pub files: usize,
    /// Unique target = the old name-match target: the same wire, now Proven.
    pub upgraded: usize,
    /// Unique target ≠ the old name-match target: the wire goes to R1's target, Proven; the name match is dropped.
    pub retargeted: usize,
    /// Unique target where the name match found nothing (or never looked, e.g. a glob `use`, a trait default
    /// method's call): a new Proven wire.
    pub added: usize,
    /// Resolves outside the workspace sources while the name match had linked a workspace file: that wire is removed.
    pub dropped: usize,
    /// Resolves outside the workspace sources and the name match found nothing either: no wire, before or after.
    pub external: usize,
    /// R1 could not decide (ambiguous, macro, `#[path]`, method, cfg-gated target, …): the name match is kept,
    /// Unresolved.
    pub unresolved: usize,
    /// R1 resolved it, but the reference is written in `#[cfg]`-gated code: the name match is kept, Unresolved.
    pub cfg_gated: usize,
    /// Proven path-resolution wires in the graph (file and member level).
    pub proven_edges: usize,
    /// Proven manifest wires: one per pair of packages joined by a normal or build path dependency.
    pub manifest_edges: usize,
    pub crate_graph_us: u64,
    /// Module trees, including reading and parsing every file in them.
    pub index_us: u64,
    /// Resolving every `use` leaf and call site.
    pub resolve_us: u64,
}

/// A unique workspace target: a file, and the item in it (`line` 0 for a module, which has no row of its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub file: PathBuf,
    pub name: String,
    pub line: usize,
}

/// What R1 says about one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Proven(Target),
    /// Into a crate outside the workspace sources.
    External,
    /// Resolved (to a target or outside), but written in cfg-gated code.
    Gated,
    Undecided,
}

/// One leaf of a file-level `use` tree, with the text today's name match looks up for it.
#[derive(Debug, Clone)]
pub(crate) struct UseRef {
    /// The path as written (a glob's base), to pair the leaf with the extractor's `use` declaration.
    pub path: Vec<String>,
    pub is_glob: bool,
    /// The last segment (`*` for a glob).
    pub last: String,
    /// The whole path joined with `::` (ending in `::*` for a glob).
    pub joined: String,
    pub outcome: Outcome,
}

/// One call site.
#[derive(Debug, Clone)]
pub(crate) struct CallRes {
    /// The enclosing fn or method: its bare name and the line of its name.
    pub caller: String,
    pub caller_line: usize,
    /// The called path's last segment (what the extractor records).
    pub name: String,
    pub outcome: Outcome,
}

/// R1's answers for one file.
#[derive(Debug, Clone, Default)]
pub(crate) struct FileRefs {
    /// Every leaf of the file-level `use` declarations outside test code, in source order.
    pub uses: Vec<UseRef>,
    /// Call sites outside test code, by line.
    pub calls: Vec<CallRes>,
}

impl FileRefs {
    /// Pairs the extractor's `use` declarations of the file with R1's leaves, in order: for each declaration,
    /// whether R1 read it (`false`: test code, which R1 skips and which keeps the name match). `None` when the two
    /// lists do not pair up, leaf for leaf.
    pub(crate) fn read_uses(&self, decls: &[UseItem]) -> Option<Vec<bool>> {
        let mut next = 0;
        let mut out = Vec::with_capacity(decls.len());
        for decl in decls {
            let leaves = use_text_leaves(&decl.path)?;
            let read = self.uses.get(next..next + leaves.len()).is_some_and(|refs| {
                refs.iter().zip(&leaves).all(|(r, (path, glob))| r.path == *path && r.is_glob == *glob)
            });
            if read {
                next += leaves.len();
            }
            out.push(read);
        }
        (next == self.uses.len()).then_some(out)
    }
}

/// R1's answers for every Rust file in a module tree.
#[derive(Debug, Default)]
pub struct RustPaths {
    pub(crate) files: HashMap<PathBuf, FileRefs>,
    /// Path dependencies between the packages, from every dependency table of their manifests.
    pub path_deps: Vec<PathDep>,
    pub stats: RustPathStats,
}

impl RustPaths {
    pub(crate) fn file(&self, path: &Path) -> Option<&FileRefs> {
        self.files.get(path)
    }
}

/// Resolves the Rust files of an extracted project. Empty when the project has no package manifests.
pub fn analyze(project: &ExtractedProject) -> RustPaths {
    let mut manifests: Vec<PathBuf> = project.crates.iter().filter_map(|c| c.manifest_path.clone()).collect();
    manifests.sort();
    if manifests.is_empty() {
        return RustPaths::default();
    }
    let mut stats = RustPathStats::default();
    let started = Instant::now();
    let graph = CrateGraph::from_manifests(&project.root_path, &manifests);
    let path_deps = path_dependencies(&project.root_path, &manifests);
    stats.crate_graph_us = started.elapsed().as_micros() as u64;

    let started = Instant::now();
    let read = |path: &Path| std::fs::read_to_string(path).ok();
    let index = RustIndex::build(&graph, &read);
    stats.index_us = started.elapsed().as_micros() as u64;

    let started = Instant::now();
    let modules = ModuleFiles::new(&index);
    let files: Vec<&Path> = index.files().collect();
    let resolved: Vec<(PathBuf, FileRefs)> =
        files.par_iter().filter_map(|&f| Some((f.to_path_buf(), resolve_file(&index, &modules, f)?))).collect();
    stats.resolve_us = started.elapsed().as_micros() as u64;
    stats.files = resolved.len();
    RustPaths { files: resolved.into_iter().collect(), path_deps, stats }
}

/// The file of each module, by the module's declaration as R1 reports it.
struct ModuleFiles {
    /// (file, line, name) of a module's `mod` item (or line 1 of a crate root) → the file its body is in, from
    /// [`RustIndex::module_file`]: `x.rs` / `x/mod.rs` for `mod x;`, the declaring file for an inline `mod x { … }`.
    /// A `#[path]` module, or one whose file is missing or does not parse, is not listed.
    decls: HashMap<(PathBuf, usize, String), PathBuf>,
}

impl ModuleFiles {
    fn new(index: &RustIndex) -> Self {
        let decls = (0..index.module_count())
            .filter_map(|i| {
                let module = ModuleId(i as u32);
                let file = index.module_file(module)?;
                // `self` in a module is the module: R1 answers with its declaration.
                match index.resolve(module, &["self"]) {
                    Resolution::Item(r) if r.kind == ItemKind::Mod => Some(((r.file, r.line, r.name), file)),
                    _ => None,
                }
            })
            .collect();
        ModuleFiles { decls }
    }

    /// The target a resolved item stands for on the map.
    fn target(&self, r: &ItemRef) -> Option<Target> {
        if r.kind != ItemKind::Mod {
            return Some(Target { file: r.file.clone(), name: r.name.clone(), line: r.line });
        }
        let file = self.decls.get(&(r.file.clone(), r.line, r.name.clone()))?;
        Some(Target { file: file.clone(), name: r.name.clone(), line: 0 })
    }

    fn outcome(&self, resolution: Resolution, gated: bool) -> Outcome {
        match resolution {
            Resolution::Item(_) | Resolution::External(_) if gated => Outcome::Gated,
            Resolution::Item(r) => self.target(&r).map_or(Outcome::Undecided, Outcome::Proven),
            Resolution::External(_) => Outcome::External,
            Resolution::Ambiguous(_) | Resolution::Unresolved(_) => Outcome::Undecided,
        }
    }
}

fn resolve_file(index: &RustIndex, modules: &ModuleFiles, file: &Path) -> Option<FileRefs> {
    let scope = index.module_of_file(file)?;
    let uses = index
        .resolved_uses(file)
        .into_iter()
        .map(|(leaf, resolution)| {
            // A leaf naming a module and a value declared next to it (`mod tidy;` + `fn tidy`) comes back as the
            // module, whose body can be in another file than the value: two files, no unique target.
            let resolution = match resolution {
                Resolution::Item(m)
                    if m.kind == ItemKind::Mod
                        && !leaf.is_glob
                        && modules.target(&m).is_some_and(|t| t.file != m.file) =>
                {
                    let path: Vec<&str> = leaf.path.iter().map(String::as_str).collect();
                    match index.resolve(scope, &path) {
                        Resolution::Item(value) if value.kind != ItemKind::Mod => Resolution::Ambiguous(vec![value, m]),
                        _ => Resolution::Item(m),
                    }
                }
                other => other,
            };
            let (last, joined) = match leaf.is_glob {
                false => (leaf.path.last().cloned().unwrap_or_default(), leaf.path.join("::")),
                true => ("*".to_string(), format!("{}::*", leaf.path.join("::"))),
            };
            UseRef {
                outcome: modules.outcome(resolution, leaf.cfg_gated),
                path: leaf.path,
                is_glob: leaf.is_glob,
                last,
                joined,
            }
        })
        .collect();
    let calls = index
        .resolved_calls(file)
        .into_iter()
        .filter_map(|(site, resolution)| {
            let caller = site.caller.as_ref()?;
            Some(CallRes {
                caller: caller.name.clone(),
                caller_line: caller.line,
                name: site.path.last().cloned().unwrap_or_default(),
                outcome: modules.outcome(resolution, site.cfg_gated),
            })
        })
        .collect();
    Some(FileRefs { uses, calls })
}

/// The leaves of a `use` tree as the extractor wrote it (`crate:: a:: { b, c as d, e::* }`), each as R1 lists them:
/// (path, is glob). `self` in a group stands for its base; a glob for its base; empty paths are left out. `None`
/// when the text is not a `use` tree.
pub(crate) fn use_text_leaves(text: &str) -> Option<Vec<(Vec<String>, bool)>> {
    let mut tokens: Vec<&str> = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let len = if rest.starts_with("::") {
            2
        } else if rest.starts_with(['{', '}', ',', '*']) {
            1
        } else {
            rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '#')).unwrap_or(rest.len())
        };
        if len == 0 {
            return None;
        }
        tokens.push(&rest[..len]);
        rest = rest[len..].trim_start();
    }
    let mut out = Vec::new();
    let mut pos = 0;
    tree(&tokens, &mut pos, &mut Vec::new(), &mut out)?;
    (pos == tokens.len()).then_some(out)
}

/// One use tree at `tokens[*pos]`: `a::tree`, `name`, `name as alias`, `*` or `{tree, …}`.
fn tree(tokens: &[&str], pos: &mut usize, prefix: &mut Vec<String>, out: &mut Vec<(Vec<String>, bool)>) -> Option<()> {
    match *tokens.get(*pos)? {
        "*" => {
            *pos += 1;
            if !prefix.is_empty() {
                out.push((prefix.clone(), true));
            }
        }
        "{" => {
            *pos += 1;
            while *tokens.get(*pos)? != "}" {
                tree(tokens, pos, prefix, out)?;
                match *tokens.get(*pos)? {
                    "," => *pos += 1,
                    "}" => {}
                    _ => return None,
                }
            }
            *pos += 1;
        }
        "::" | "}" | "," => return None,
        name => {
            *pos += 1;
            if tokens.get(*pos) == Some(&"::") {
                *pos += 1;
                prefix.push(name.to_string());
                tree(tokens, pos, prefix, out)?;
                prefix.pop();
                return Some(());
            }
            if tokens.get(*pos) == Some(&"as") {
                tokens.get(*pos + 1)?;
                *pos += 2;
            }
            let mut path = prefix.clone();
            if name != "self" || prefix.is_empty() {
                path.push(name.to_string());
            }
            out.push((path, false));
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(text: &str) -> Option<Vec<String>> {
        use_text_leaves(text).map(|l| {
            l.into_iter().map(|(p, glob)| if glob { format!("{}::*", p.join("::")) } else { p.join("::") }).collect()
        })
    }

    #[test]
    fn use_texts_split_into_the_leaves_r1_lists() {
        assert_eq!(leaves("std::fmt"), Some(vec!["std::fmt".into()]));
        assert_eq!(
            leaves("crate:: a:: { self, b as c, d:: { e, f::* }, r#type }"),
            Some(vec![
                "crate::a".into(),
                "crate::a::b".into(),
                "crate::a::d::e".into(),
                "crate::a::d::f::*".into(),
                "crate::a::r#type".into()
            ])
        );
        assert_eq!(leaves("{ alpha, beta:: g }"), Some(vec!["alpha".into(), "beta::g".into()]));
        assert_eq!(leaves("a:: {}"), Some(vec![]));
        assert_eq!(leaves("x as _"), Some(vec!["x".into()]));
        // Not a use tree: the file keeps the name match.
        assert_eq!(leaves("a:: {b"), None);
        assert_eq!(leaves("a::"), None);
        assert_eq!(leaves("\"x.h\""), None);
    }

    #[test]
    fn test_only_declarations_are_the_ones_r1_does_not_list() {
        let leaf = |path: &[&str]| UseRef {
            path: path.iter().map(|s| s.to_string()).collect(),
            is_glob: false,
            last: String::new(),
            joined: String::new(),
            outcome: Outcome::Undecided,
        };
        let decl = |path: &str| UseItem { path: path.to_string(), items: Vec::new() };
        let refs = FileRefs { uses: vec![leaf(&["a", "b"]), leaf(&["a", "c"]), leaf(&["d"])], calls: vec![] };
        let decls = [decl("a:: { b, c }"), decl("tests_only:: x"), decl("d")];
        assert_eq!(refs.read_uses(&decls), Some(vec![true, false, true]));
        // A leaf R1 lists that no declaration accounts for: the lists do not pair.
        assert_eq!(refs.read_uses(&decls[..2]), None);
        assert_eq!(refs.read_uses(&[decl("a:: { b, c }"), decl("e"), decl("f")]), None);
    }
}
