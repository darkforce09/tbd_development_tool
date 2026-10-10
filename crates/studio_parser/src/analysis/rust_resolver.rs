//! Deterministic Rust path resolution over the crates of a [`CrateGraph`] (R1).
//!
//! [`RustIndex::build`] walks every crate's module tree from its root file (`mod x;` → `x.rs` or `x/mod.rs`, a
//! non-`mod.rs` file `foo.rs` keeps its children in `foo/`, inline `mod x { … }` at any depth), parsing each file
//! once (in parallel) into [`FileFacts`]. Test-only code (`#[cfg(test)]`, `#[test]`) is left out entirely.
//!
//! Resolution is rustc-like but conservative; it answers only when the answer is unique and exact:
//!
//! - path roots: `crate`, `self`, `super` (repeatable), `::name` (an extern crate), then the module's own scope
//!   (declared items and explicit `use` imports first, glob imports second), then the extern prelude (the crate's
//!   dependencies, `std`, `core`, `alloc`), then the std prelude (`Vec`, `Some`, …) and the primitive types;
//! - `pub use` re-exports are followed transitively across modules and crates, with a cycle guard;
//! - a glob import gives a name only when exactly one glob provides it and nothing explicit does (several distinct
//!   providers → [`Resolution::Ambiguous`]); globs respect visibility (a private item is seen only from its module
//!   and the modules inside it);
//! - `Enum::Variant` resolves to the enum item; `Type::name` resolves to the associated function or constant's own
//!   span when an inherent `impl` in the workspace defines it, and is `Unresolved(Method)` otherwise (a trait may
//!   provide it, which needs types); `Trait::name` resolves to the trait when the trait declares `name`;
//! - anything reached through a `#[cfg(…)]`-gated item, variant, trait item, import, module or dependency (a
//!   target-specific or optional one), and any name with two cfg twins, is `Unresolved(Cfg)`; a `#[path]` module is
//!   `Unresolved(PathAttr)`; a `macro_rules!` name, a name missing from a module whose item-level macro invocations
//!   may define it, and a call whose first segment a statement-level macro of the body names in its input (any
//!   macro but the std expression macros: it may declare that item in the block) is `Unresolved(Macro)`; method calls and `Self::` in a trait or a blanket impl over a type
//!   parameter are `Unresolved(Method)`; a module whose file does not parse is `Unresolved(Other)`;
//! - a glob whose base cannot be resolved makes every name it might provide uncertain (`Unresolved(Glob)`), and a
//!   `pub(in path)` item seen through a glob from outside its module is uncertain too;
//! - edition 2015 crates resolve `use` paths and `::name` from the crate root;
//! - a path into a crate outside the workspace sources is `External("axum::Router")`, built from the canonical path
//!   after use-expansion; std prelude names are `External("std::…")`.

mod lookup;
mod prelude;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

#[cfg(doc)]
use super::crate_graph::CrateNode;
use super::crate_graph::{normalize, CrateGraph, TargetKind};
use super::rust_facts::{parse_rust_facts, AssocKind, CallFact, FactKind, FileFacts, ModBody, Vis};

/// A module of one crate in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(pub u32);

/// What a resolved item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Fn,
    Struct,
    Enum,
    Trait,
    Const,
    Static,
    Type,
    Mod,
    Macro,
    /// A re-export (part of the contract; resolution follows re-exports to their target, so it is not produced).
    Use,
}

/// A workspace item a path resolves to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemRef {
    pub file: PathBuf,
    pub name: String,
    pub kind: ItemKind,
    /// 1-based line of the item's name (line 1 for a crate root).
    pub line: usize,
    pub line_end: usize,
    /// The module the item is declared in (a crate root is its own module).
    pub module: ModuleId,
}

/// Why a path has no exact target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnresolvedReason {
    NotFound,
    Macro,
    Cfg,
    PathAttr,
    Glob,
    Method,
    ExternalUnknown,
    Other(String),
}

/// The answer for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Unique and exact.
    Item(ItemRef),
    /// Into a crate outside the workspace sources, e.g. `"axum::Router"`, `"std::vec::Vec::new"`.
    External(String),
    /// Several candidates (e.g. two globs providing the name).
    Ambiguous(Vec<ItemRef>),
    Unresolved(UnresolvedReason),
}

/// One call site of a function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// The function or method the call is written in.
    pub caller: Option<ItemRef>,
    /// The path as written (`["handlers", "a", "f"]`), or `[method]` for a method call.
    pub path: Vec<String>,
    pub line: usize,
    pub is_method: bool,
    /// Written in cfg-gated code: the statement or expression, the enclosing fn, its `impl` or trait, or a module
    /// on the way up (its `mod` item or `#![cfg]`) is under a `#[cfg(…)]` other than `test`.
    pub cfg_gated: bool,
}

/// One leaf of a file-level `use` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseLeaf {
    /// The path as written, without a leading `::` (`["crate", "a", "f"]`); for a glob, its base.
    pub path: Vec<String>,
    /// The name bound when it differs from the last segment (`as g`, `as _`).
    pub alias: Option<String>,
    /// 1-based line of the `use` keyword.
    pub line: usize,
    pub is_glob: bool,
    /// The declaration or a module on the way up is under a `#[cfg(…)]` other than `test`.
    pub cfg_gated: bool,
}

/// Name spaces of Rust items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Ns {
    Type,
    Value,
    Macro,
}

/// Where a module's body is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Body {
    /// Scope `scope` of `file`'s facts.
    Loaded { scope: usize },
    /// Behind a `#[path]` attribute (not followed).
    PathAttr,
    /// The module file was not found (or both `x.rs` and `x/mod.rs` exist).
    Missing(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Import {
    pub path: Vec<String>,
    pub leading_colon: bool,
    /// The bound name (`None` for a glob).
    pub binding: Option<String>,
    pub self_only: bool,
    pub vis: Vis,
    pub gated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleData {
    pub krate: usize,
    pub parent: Option<ModuleId>,
    pub file: PathBuf,
    pub body: Body,
    /// The `mod` item that declares this module (`None` for a crate root).
    pub decl_item: Option<usize>,
    /// Declared items by name (all name spaces).
    pub names: BTreeMap<String, Vec<usize>>,
    pub imports: Vec<Import>,
    pub globs: Vec<Import>,
    pub macros: BTreeSet<String>,
    pub has_item_macros: bool,
    /// The module, or one around it, is declared under a `#[cfg(…)]` (its `mod` item or a `#![cfg]` body).
    pub gated: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum Detail {
    Fn,
    Struct {
        value: bool,
    },
    Union,
    /// Variants: (name, under its own `#[cfg(…)]`).
    Enum(Vec<(String, bool)>),
    /// Associated functions and constants: (name, under its own `#[cfg(…)]`).
    Trait(Vec<(String, bool)>),
    Const,
    Static,
    Type,
    Mod(ModuleId),
    MacroRules,
    ExternCrate(String),
    /// An associated function or constant of an inherent `impl`.
    Assoc,
}

#[derive(Debug, Clone)]
pub(crate) struct ItemData {
    pub r: ItemRef,
    pub vis: Vis,
    /// Declared under a `#[cfg(…)]` other than `test` / `not(test)`.
    pub gated: bool,
    pub detail: Detail,
}

impl ItemData {
    pub(crate) fn in_ns(&self, ns: Ns) -> bool {
        match (&self.detail, ns) {
            (Detail::Fn | Detail::Const | Detail::Static, Ns::Value) => true,
            (Detail::Struct { value }, Ns::Value) => *value,
            (
                Detail::Struct { .. }
                | Detail::Union
                | Detail::Enum(_)
                | Detail::Trait(_)
                | Detail::Type
                | Detail::Mod(_)
                | Detail::ExternCrate(_),
                Ns::Type,
            ) => true,
            (Detail::MacroRules, Ns::Macro) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CrateData {
    pub name: String,
    pub kind: TargetKind,
    pub root: Option<ModuleId>,
    /// Workspace dependencies by extern name → crate index.
    pub deps: BTreeMap<String, usize>,
    /// External dependencies by extern name → crate name.
    pub externals: BTreeMap<String, String>,
    /// Extern names that exist only under a condition, or name two crates (see [`CrateNode::gated_deps`]).
    pub gated_deps: BTreeSet<String>,
    pub edition: u16,
}

#[derive(Debug, Clone)]
struct FileData {
    facts: Arc<FileFacts>,
    /// The first module (in crate order) each scope of the file defines.
    scope_modules: Vec<Option<ModuleId>>,
}

/// The module trees of every crate in a [`CrateGraph`], ready to resolve paths.
#[derive(Debug, Clone, Default)]
pub struct RustIndex {
    pub(crate) crates: Vec<CrateData>,
    pub(crate) modules: Vec<ModuleData>,
    pub(crate) items: Vec<ItemData>,
    files: BTreeMap<PathBuf, FileData>,
    /// Inherent associated items: type item → name → assoc item ids.
    pub(crate) inherent: BTreeMap<usize, BTreeMap<String, Vec<usize>>>,
}

/// The deepest module nesting followed (guards against pathological trees).
const MAX_DEPTH: usize = 64;

impl RustIndex {
    /// Builds the index; `read` returns a file's text (`None` when it does not exist or cannot be read).
    pub fn build(graph: &CrateGraph, read: &(dyn Fn(&Path) -> Option<String> + Sync)) -> RustIndex {
        let parsed = parse_all(graph, read);
        let mut index = RustIndex::default();
        for node in &graph.crates {
            index.crates.push(CrateData {
                name: node.name.clone(),
                kind: node.kind,
                root: None,
                deps: node.deps.iter().cloned().collect(),
                externals: node.external_deps.iter().cloned().collect(),
                gated_deps: node.gated_deps.iter().cloned().collect(),
                edition: node.edition,
            });
        }
        for (ci, node) in graph.crates.iter().enumerate() {
            let root = normalize(&node.root_file);
            let Some(Some(facts)) = parsed.get(&root) else { continue };
            if facts.scopes[0].test_only {
                continue;
            }
            let dir = root.parent().map(Path::to_path_buf);
            let m = index.add_module(&parsed, ci, None, &root, Body::Loaded { scope: 0 }, dir, None, 0);
            index.crates[ci].root = Some(m);
        }
        index.index_inherent_impls();
        index
    }

    /// The module a file defines (its file scope), in the first crate that owns it.
    pub fn module_of_file(&self, file: &Path) -> Option<ModuleId> {
        self.files.get(file).and_then(|f| f.scope_modules.first().copied().flatten())
    }

    /// Resolves a path from a module scope. The last segment is looked up as a value first (a function, constant,
    /// unit or tuple struct, variant), then as a type. A leading `""` or `"::"` segment stands for a leading `::`.
    pub fn resolve(&self, scope: ModuleId, path: &[&str]) -> Resolution {
        if scope.0 as usize >= self.modules.len() {
            return Resolution::Unresolved(UnresolvedReason::Other("unknown module".into()));
        }
        let (leading_colon, segs) = match path.first() {
            Some(&"") | Some(&"::") => (true, &path[1..]),
            _ => (false, path),
        };
        let segs: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
        let value = self.resolve_ns(scope, &segs, leading_colon, Ns::Value);
        if value == Resolution::Unresolved(UnresolvedReason::NotFound) {
            let ty = self.resolve_ns(scope, &segs, leading_colon, Ns::Type);
            if ty != Resolution::Unresolved(UnresolvedReason::NotFound) {
                return ty;
            }
        }
        value
    }

    /// Every call site in the function bodies of `file` (test-only code excluded) with its resolution, in source
    /// order. A call is resolved from the module scope of its enclosing function, in the value name space.
    pub fn resolved_calls(&self, file: &Path) -> Vec<(CallSite, Resolution)> {
        let Some(fd) = self.files.get(file) else { return Vec::new() };
        let mut out = Vec::new();
        for call in &fd.facts.calls {
            if call.test_only {
                continue;
            }
            let Some(Some(module)) = fd.scope_modules.get(call.scope).copied() else { continue };
            let caller = ItemRef {
                file: file.to_path_buf(),
                name: call.caller.name.clone(),
                kind: ItemKind::Fn,
                line: call.caller.line,
                line_end: call.caller.line_end,
                module,
            };
            let cfg_gated = call.cfg_gated || self.modules[module.0 as usize].gated || caller_gated(&fd.facts, call);
            let site = CallSite {
                caller: Some(caller),
                path: call.path.clone(),
                line: call.line,
                is_method: call.is_method,
                cfg_gated,
            };
            out.push((site, self.resolve_call(&fd.facts, module, call)));
        }
        out.sort_by_key(|(site, _)| site.line);
        out
    }

    /// The functions and methods (with a body) declared in `file`, test-only code excluded, by line.
    pub fn functions_in(&self, file: &Path) -> Vec<ItemRef> {
        let Some(fd) = self.files.get(file) else { return Vec::new() };
        let mut out = Vec::new();
        for (si, scope) in fd.facts.scopes.iter().enumerate() {
            let Some(Some(module)) = fd.scope_modules.get(si).copied() else { continue };
            if scope.test_only {
                continue;
            }
            let mk = |name: &str, line: usize, line_end: usize| ItemRef {
                file: file.to_path_buf(),
                name: name.to_string(),
                kind: ItemKind::Fn,
                line,
                line_end,
                module,
            };
            for item in scope.items.iter().filter(|i| !i.test_only) {
                match &item.kind {
                    FactKind::Fn => out.push(mk(&item.name, item.line, item.line_end)),
                    FactKind::Trait { items } => out.extend(
                        items
                            .iter()
                            .filter(|a| a.kind == AssocKind::Fn && a.has_body && !a.test_only)
                            .map(|a| mk(&a.name, a.line, a.line_end)),
                    ),
                    _ => {}
                }
            }
            for imp in scope.impls.iter().filter(|i| !i.test_only) {
                out.extend(
                    imp.items
                        .iter()
                        .filter(|a| a.kind == AssocKind::Fn && !a.test_only)
                        .map(|a| mk(&a.name, a.line, a.line_end)),
                );
            }
        }
        out.sort_by(|a, b| (a.line, &a.name).cmp(&(b.line, &b.name)));
        out
    }

    /// Every file the index parsed into a module tree, sorted.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().filter(|(_, f)| f.scope_modules.iter().any(Option::is_some)).map(|(p, _)| p.as_path())
    }

    /// The number of modules (crate roots, file modules and inline modules) in every crate.
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// The leaves of the file-level `use` declarations of `file` (test-only ones excluded), in source order, each
    /// resolved from the file's module the way rustc reads a `use` path.
    ///
    /// A named leaf imports every name space: it is resolved as a value and as a type. One hit (or the same item
    /// twice) is the answer. Two different items in the same file give the type-side item (a module's `mod`
    /// line, a struct …): the file an import edge points at is the same either way. Two different items in
    /// different files are [`Resolution::Ambiguous`]; a workspace item on one side and an external path or an
    /// unresolved reason on the other is unresolved. A glob leaf resolves its base (a module or an enum).
    pub fn resolved_uses(&self, file: &Path) -> Vec<(UseLeaf, Resolution)> {
        let Some(fd) = self.files.get(file) else { return Vec::new() };
        let Some(Some(module)) = fd.scope_modules.first().copied() else { return Vec::new() };
        let Some(scope) = fd.facts.scopes.first() else { return Vec::new() };
        let module_gated = self.modules[module.0 as usize].gated;
        let mut out = Vec::new();
        for u in scope.uses.iter().filter(|u| !u.test_only) {
            for e in u.entries.iter().filter(|e| !e.path.is_empty()) {
                let is_glob = e.binding.is_none();
                let alias = e.binding.clone().filter(|b| Some(b) != e.path.last());
                let leaf = UseLeaf {
                    path: e.path.clone(),
                    alias,
                    line: u.line,
                    is_glob,
                    cfg_gated: module_gated || !u.cfg.is_empty(),
                };
                let resolution = if is_glob || e.self_only {
                    self.resolve_use_ns(module, &e.path, u.leading_colon, Ns::Type)
                } else {
                    let value = self.resolve_use_ns(module, &e.path, u.leading_colon, Ns::Value);
                    let ty = self.resolve_use_ns(module, &e.path, u.leading_colon, Ns::Type);
                    both_name_spaces(value, ty)
                };
                out.push((leaf, resolution));
            }
        }
        out
    }

    /// The file a module's own body is in: `x.rs`, `x/mod.rs` or the crate root; for an inline `mod x { … }` the
    /// file that contains it. `None` for a `#[path]` module, a module whose file is missing or does not parse, and
    /// an unknown id.
    pub fn module_file(&self, module: ModuleId) -> Option<PathBuf> {
        let m = self.modules.get(module.0 as usize)?;
        matches!(m.body, Body::Loaded { .. }).then(|| m.file.clone())
    }

    fn resolve_call(&self, facts: &FileFacts, module: ModuleId, call: &CallFact) -> Resolution {
        if call.is_method || call.qualified_self {
            return Resolution::Unresolved(UnresolvedReason::Method);
        }
        if call.local {
            return Resolution::Unresolved(UnresolvedReason::Other("local binding or block item".into()));
        }
        let mut path = call.path.clone();
        if path.first().is_some_and(|s| s == "Self") {
            if call.caller.in_trait {
                return Resolution::Unresolved(UnresolvedReason::Method);
            }
            let imp = call.caller.impl_index.and_then(|i| facts.scopes[call.scope].impls.get(i));
            match imp.and_then(|imp| imp.self_ty.as_ref().map(|ty| (ty, &imp.generics))) {
                // `impl<T> Trait for T`: `Self` is whatever type the impl is used with.
                Some((ty, generics)) if ty.first().is_some_and(|first| generics.contains(first)) => {
                    return Resolution::Unresolved(UnresolvedReason::Method);
                }
                Some((ty, _)) => {
                    path.splice(0..1, ty.iter().cloned());
                }
                None => return Resolution::Unresolved(UnresolvedReason::Other("Self type is not a path".into())),
            }
        }
        let rooted = call.leading_colon || matches!(path.first().map(String::as_str), Some("crate" | "self" | "super"));
        if call.block_glob && !rooted {
            return Resolution::Unresolved(UnresolvedReason::Glob);
        }
        if call.block_macro && !rooted {
            return Resolution::Unresolved(UnresolvedReason::Macro);
        }
        self.resolve_ns(module, &path, call.leading_colon, Ns::Value)
    }

    /// Adds a module and, recursively, the modules it declares; returns its id.
    #[allow(clippy::too_many_arguments)]
    fn add_module(
        &mut self,
        parsed: &BTreeMap<PathBuf, Option<Arc<FileFacts>>>,
        krate: usize,
        parent: Option<ModuleId>,
        file: &Path,
        body: Body,
        dir: Option<PathBuf>,
        decl_item: Option<usize>,
        depth: usize,
    ) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        self.modules.push(ModuleData {
            krate,
            parent,
            file: file.to_path_buf(),
            body: body.clone(),
            decl_item,
            names: BTreeMap::new(),
            imports: Vec::new(),
            globs: Vec::new(),
            macros: BTreeSet::new(),
            has_item_macros: false,
            gated: parent.is_some_and(|p| self.modules[p.0 as usize].gated)
                || decl_item.is_some_and(|d| self.items[d].gated),
        });
        let Body::Loaded { scope } = body else { return id };
        let Some(Some(facts)) = parsed.get(file) else { return id };
        let facts = Arc::clone(facts);
        let fd = self
            .files
            .entry(file.to_path_buf())
            .or_insert_with(|| FileData { facts: Arc::clone(&facts), scope_modules: vec![None; facts.scopes.len()] });
        if fd.scope_modules[scope].is_none() {
            fd.scope_modules[scope] = Some(id);
        }
        if let Some(error) = &facts.error {
            // Nothing is known about a file that does not parse: its names are not "missing", they are unknown.
            self.modules[id.0 as usize].body = Body::Missing(error.clone());
            return id;
        }
        let sf = &facts.scopes[scope];
        if sf.test_only {
            return id;
        }
        if parent.is_none() && !sf.inner_cfg.is_empty() {
            // `#![cfg(…)]` on a crate root.
            self.modules[id.0 as usize].gated = true;
        }
        // A `#[path]` on an inline module hides where its file children live.
        let dir = if sf.path_attr.is_some() { None } else { dir };

        for item in sf.items.iter().filter(|i| !i.test_only) {
            let kind = match &item.kind {
                FactKind::Fn => ItemKind::Fn,
                FactKind::Struct { .. } | FactKind::Union => ItemKind::Struct,
                FactKind::Enum { .. } => ItemKind::Enum,
                FactKind::Trait { .. } => ItemKind::Trait,
                FactKind::Const => ItemKind::Const,
                FactKind::Static => ItemKind::Static,
                FactKind::Type => ItemKind::Type,
                FactKind::Mod(_) | FactKind::ExternCrate { .. } => ItemKind::Mod,
                FactKind::MacroRules => ItemKind::Macro,
            };
            let item_id = self.items.len();
            let detail = match &item.kind {
                FactKind::Fn => Detail::Fn,
                FactKind::Struct { value } => Detail::Struct { value: *value },
                FactKind::Union => Detail::Union,
                FactKind::Enum { variants } => {
                    Detail::Enum(variants.iter().map(|v| (v.clone(), item.gated_variants.contains(v))).collect())
                }
                FactKind::Trait { items } => Detail::Trait(
                    items
                        .iter()
                        .filter(|a| !a.test_only && a.kind != AssocKind::Type)
                        .map(|a| (a.name.clone(), !a.cfg.is_empty()))
                        .collect(),
                ),
                FactKind::Const => Detail::Const,
                FactKind::Static => Detail::Static,
                FactKind::Type => Detail::Type,
                FactKind::Mod(_) => Detail::Mod(ModuleId(u32::MAX)),
                FactKind::MacroRules => Detail::MacroRules,
                FactKind::ExternCrate { krate } => Detail::ExternCrate(krate.clone()),
            };
            let mut gated = !item.cfg.is_empty();
            // Where a child module's body is, before the item is added (a test-only file adds nothing).
            let child = match &item.kind {
                FactKind::Mod(ModBody::Inline(child_scope)) => Some((
                    file.to_path_buf(),
                    Body::Loaded { scope: *child_scope },
                    dir.as_ref().map(|d| d.join(&item.name)),
                )),
                FactKind::Mod(ModBody::File { path_attr: Some(_) }) => Some((file.to_path_buf(), Body::PathAttr, None)),
                FactKind::Mod(ModBody::File { path_attr: None }) => match &dir {
                    None => Some((file.to_path_buf(), Body::PathAttr, None)),
                    Some(d) => {
                        let flat = d.join(format!("{}.rs", item.name));
                        let nested = d.join(&item.name).join("mod.rs");
                        let child_dir = Some(d.join(&item.name));
                        match (parsed.get(&flat), parsed.get(&nested)) {
                            (Some(Some(_)), Some(Some(_))) => Some((
                                file.to_path_buf(),
                                Body::Missing(format!("both {0}.rs and {0}/mod.rs exist", item.name)),
                                None,
                            )),
                            (Some(Some(f)), _) => Some((flat.clone(), Body::Loaded { scope: 0 }, child_dir))
                                .filter(|_| !f.scopes[0].test_only),
                            (_, Some(Some(f))) => Some((nested.clone(), Body::Loaded { scope: 0 }, child_dir))
                                .filter(|_| !f.scopes[0].test_only),
                            _ => Some((
                                file.to_path_buf(),
                                Body::Missing(format!("module file for `{}` not found", item.name)),
                                None,
                            )),
                        }
                    }
                },
                _ => None,
            };
            if matches!(item.kind, FactKind::Mod(_)) {
                let Some((child_file, child_body, _)) = &child else { continue };
                if let (Body::Loaded { scope: 0 }, Some(Some(f))) = (child_body, parsed.get(child_file)) {
                    gated |= !f.scopes[0].inner_cfg.is_empty();
                }
            }
            self.items.push(ItemData {
                r: ItemRef {
                    file: file.to_path_buf(),
                    name: item.name.clone(),
                    kind,
                    line: item.line,
                    line_end: item.line_end,
                    module: id,
                },
                vis: item.vis,
                gated,
                detail,
            });
            self.modules[id.0 as usize].names.entry(item.name.clone()).or_default().push(item_id);
            if matches!(item.kind, FactKind::MacroRules) {
                self.modules[id.0 as usize].macros.insert(item.name.clone());
            }
            if let Some((child_file, child_body, child_dir)) = child {
                let child_id = if depth >= MAX_DEPTH {
                    self.add_module(
                        parsed,
                        krate,
                        Some(id),
                        &child_file,
                        Body::Missing("too deep".into()),
                        None,
                        Some(item_id),
                        depth + 1,
                    )
                } else {
                    self.add_module(
                        parsed,
                        krate,
                        Some(id),
                        &child_file,
                        child_body,
                        child_dir,
                        Some(item_id),
                        depth + 1,
                    )
                };
                self.items[item_id].detail = Detail::Mod(child_id);
            }
        }
        let m = &mut self.modules[id.0 as usize];
        m.has_item_macros = !sf.item_macros.is_empty();
        for u in sf.uses.iter().filter(|u| !u.test_only) {
            for e in &u.entries {
                let import = Import {
                    path: e.path.clone(),
                    leading_colon: u.leading_colon,
                    binding: e.binding.clone(),
                    self_only: e.self_only,
                    vis: u.vis,
                    gated: !u.cfg.is_empty(),
                };
                match &e.binding {
                    None => m.globs.push(import),
                    Some(b) if b == "_" => {}
                    Some(_) => m.imports.push(import),
                }
            }
        }
        id
    }

    /// Registers the associated functions and constants of every inherent `impl` under the type it names.
    fn index_inherent_impls(&mut self) {
        let mut adds: Vec<(usize, ItemData)> = Vec::new();
        for (mi, m) in self.modules.iter().enumerate() {
            let Body::Loaded { scope } = m.body else { continue };
            let Some(fd) = self.files.get(&m.file) else { continue };
            let sf = &fd.facts.scopes[scope];
            if sf.test_only {
                continue;
            }
            for imp in sf.impls.iter().filter(|i| !i.test_only && i.trait_path.is_none()) {
                let Some(ty) = &imp.self_ty else { continue };
                let lookup::Look::Found(found) = self.resolve_path_from(ModuleId(mi as u32), ty, false, Ns::Type)
                else {
                    continue;
                };
                let lookup::Def::Item(type_id) = found.def else { continue };
                if !matches!(self.items[type_id].detail, Detail::Struct { .. } | Detail::Enum(_) | Detail::Union) {
                    continue;
                }
                for a in imp.items.iter().filter(|a| !a.test_only && a.kind != AssocKind::Type) {
                    adds.push((
                        type_id,
                        ItemData {
                            r: ItemRef {
                                file: m.file.clone(),
                                name: a.name.clone(),
                                kind: if a.kind == AssocKind::Fn { ItemKind::Fn } else { ItemKind::Const },
                                line: a.line,
                                line_end: a.line_end,
                                module: ModuleId(mi as u32),
                            },
                            vis: a.vis,
                            gated: found.gated || !imp.cfg.is_empty() || !a.cfg.is_empty(),
                            detail: Detail::Assoc,
                        },
                    ));
                }
            }
        }
        for (type_id, data) in adds {
            let id = self.items.len();
            let name = data.r.name.clone();
            self.items.push(data);
            self.inherent.entry(type_id).or_default().entry(name).or_default().push(id);
        }
    }
}

/// Combines the value-side and type-side answers of a `use` leaf (see [`RustIndex::resolved_uses`]).
fn both_name_spaces(value: Resolution, ty: Resolution) -> Resolution {
    let not_found = Resolution::Unresolved(UnresolvedReason::NotFound);
    match (value, ty) {
        (v, t) if v == t => v,
        (v, t) if t == not_found => v,
        (v, t) if v == not_found => t,
        (Resolution::Item(v), Resolution::Item(t)) if v.file == t.file => Resolution::Item(t),
        (Resolution::Item(v), Resolution::Item(t)) => {
            let mut both = vec![v, t];
            both.sort();
            Resolution::Ambiguous(both)
        }
        (Resolution::Unresolved(r), _) | (_, Resolution::Unresolved(r)) => Resolution::Unresolved(r),
        _ => Resolution::Unresolved(UnresolvedReason::Other("names a workspace item and another path".into())),
    }
}

/// Whether the fn a call is written in is cfg-gated itself: the fn, its `impl` (or the method in it), or its trait
/// (or the method in it).
fn caller_gated(facts: &FileFacts, call: &CallFact) -> bool {
    let Some(scope) = facts.scopes.get(call.scope) else { return false };
    let c = &call.caller;
    let assoc_gated = |items: &[super::rust_facts::AssocFact]| {
        items.iter().any(|a| a.line == c.line && a.name == c.name && !a.cfg.is_empty())
    };
    match (c.impl_index, c.in_trait) {
        (Some(i), _) => scope.impls.get(i).is_some_and(|imp| !imp.cfg.is_empty() || assoc_gated(&imp.items)),
        (None, true) => scope.items.iter().any(|it| match &it.kind {
            FactKind::Trait { items } if items.iter().any(|a| a.line == c.line && a.name == c.name) => {
                !it.cfg.is_empty() || assoc_gated(items)
            }
            _ => false,
        }),
        (None, false) => scope
            .items
            .iter()
            .any(|it| it.kind == FactKind::Fn && it.line == c.line && it.name == c.name && !it.cfg.is_empty()),
    }
}

/// Parses every file reachable from the crate roots by the module rules, in parallel waves (one wave per module
/// depth). Both candidate files of a `mod x;` are tried; a missing file maps to `None`.
fn parse_all(
    graph: &CrateGraph,
    read: &(dyn Fn(&Path) -> Option<String> + Sync),
) -> BTreeMap<PathBuf, Option<Arc<FileFacts>>> {
    let mut parsed: BTreeMap<PathBuf, Option<Arc<FileFacts>>> = BTreeMap::new();
    let mut seen: BTreeSet<(PathBuf, PathBuf)> = BTreeSet::new();
    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for node in &graph.crates {
        let root = normalize(&node.root_file);
        let dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
        if seen.insert((root.clone(), dir.clone())) {
            jobs.push((root, dir));
        }
    }
    for _ in 0..MAX_DEPTH {
        if jobs.is_empty() {
            break;
        }
        let todo: BTreeSet<&PathBuf> = jobs.iter().map(|(f, _)| f).filter(|f| !parsed.contains_key(*f)).collect();
        let fresh: Vec<(PathBuf, Option<Arc<FileFacts>>)> =
            todo.into_par_iter().map(|f| (f.clone(), read(f).map(|text| Arc::new(parse_rust_facts(&text))))).collect();
        parsed.extend(fresh);
        let mut next = Vec::new();
        for (file, dir) in &jobs {
            let Some(Some(facts)) = parsed.get(file) else { continue };
            let mut dirs: Vec<Option<PathBuf>> = Vec::with_capacity(facts.scopes.len());
            for (si, scope) in facts.scopes.iter().enumerate() {
                let d = match scope.parent {
                    None => Some(dir.clone()),
                    Some(p) => dirs[p].as_ref().map(|d| d.join(&scope.name)).filter(|_| scope.path_attr.is_none()),
                };
                dirs.push(d);
                let Some(d) = &dirs[si] else { continue };
                if scope.test_only {
                    continue;
                }
                for item in scope.items.iter().filter(|i| !i.test_only) {
                    if let FactKind::Mod(ModBody::File { path_attr: None }) = item.kind {
                        let child_dir = d.join(&item.name);
                        for candidate in [d.join(format!("{}.rs", item.name)), child_dir.join("mod.rs")] {
                            if seen.insert((candidate.clone(), child_dir.clone())) {
                                next.push((candidate, child_dir.clone()));
                            }
                        }
                    }
                }
            }
        }
        jobs = next;
    }
    parsed
}
