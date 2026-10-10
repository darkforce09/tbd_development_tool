//! axum route tables, evaluated from the router code itself (no build, no macro expansion).
//!
//! [`evaluate_routes`] finds the functions of the lib and bin module trees (never `tests/`, benches, examples or
//! `#[cfg(test)]` code, which [`RustIndex`] leaves out) whose return type resolves through R1 to `axum::Router`, and
//! the places that hand a router to `axum::serve(…)` or `.into_make_service*()`. It then evaluates their bodies over
//! `syn`:
//!
//! - `Router::new()` (resolved, so a local `Router` is not axum's), then the builder chain: `.route(path, m)`,
//!   `.nest(prefix, r)`, `.merge(r)`, pass-through layers and state, and `fallback` / `*_service` mounts, which are
//!   counted as services, never entries;
//! - a router argument may be a call to another router function (resolved through R1 from the calling module and
//!   evaluated once), a local binding or an inline chain;
//! - statements: `let r = …;`, `let mut r = …; r = r.route(…);`, plain blocks, `if cond { … } else { … }` (routes
//!   registered inside carry the condition text, nested conditions joined with " && ", the else branch as
//!   `!(cond)`), the tail expression or a `return` (after an `if` that returns, the rest of the body runs under
//!   `!(cond)`; a `return` anywhere else makes the function's value unknown);
//! - `#[cfg(test)]` statements are left out; a statement under another `#[cfg(…)]` is conditional on it (the
//!   attribute as written), and so is every route of a function whose modules or own item are under one;
//! - method routers `get(h)`, `post(h)`, …, `any(h)` ("ANY"), `on(MethodFilter::X, h)` and their chains, only when
//!   the function resolves to `axum::routing` (a local `get` is not axum's).
//!
//! Anything else that touches a router (a loop, a macro, a closure, an unknown call or method) is listed in
//! [`RouteTable::skipped`] with its line and a reason; nothing is guessed.
//!
//! Roots are the router functions nothing references (no call resolves to them, in any function, whatever came of
//! the call), plus the serve sites. Each root is evaluated once and every entry remembers the root it came from. A
//! referenced router function that no evaluated root folds in is never a root: its routes are listed in
//! [`RouteTable::skipped`] with the referencing site, since its place in the URL space is not known.

mod eval;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quote::ToTokens;

use super::crate_graph::{CrateGraph, TargetKind};
use super::route_template::RouteTemplate;
use super::rust_resolver::{ModuleId, Resolution, RustIndex};
use eval::{read_cfg, Cfg, Evaluator, FnSite, Key};

/// One evaluated route: a method and full path, and the handler that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEntry {
    /// "GET", "POST", … or "ANY".
    pub method: String,
    /// The full path after every nest, as written (`{commandId}` kept).
    pub template: String,
    /// R1's resolution of the handler expression (`Unresolved(Other("closure"))` for a closure).
    pub handler: Resolution,
    /// The handler as written.
    pub handler_text: String,
    /// The conditions of the enclosing `if`s, joined with " && " (`!(cond)` for an else branch).
    pub conditional: Option<String>,
    /// The `.route(` line.
    pub defined_at: (PathBuf, usize),
    /// The root this entry was evaluated from: a router function (its name line) or a serve site (the `let` line).
    pub root: (PathBuf, usize),
}

/// Every route of a workspace, sorted by (canonical template, method, defined_at).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteTable {
    pub entries: Vec<RouteEntry>,
    /// `fallback`, `fallback_service`, `nest_service`, `route_service` and `*_service` method routers.
    pub services: usize,
    /// Router code the evaluator does not follow: (file, line, reason), sorted.
    pub skipped: Vec<(PathBuf, usize, String)>,
}

/// The paths `Router` resolves to when it is axum's.
const ROUTER_TYPES: [&str; 2] = ["axum::Router", "axum::routing::Router"];

/// Evaluates the axum routes of every lib and bin crate in `graph`. `read` returns a file's text.
pub fn evaluate_routes(index: &RustIndex, graph: &CrateGraph, read: &dyn Fn(&Path) -> Option<String>) -> RouteTable {
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut fns: BTreeMap<Key, FnSite> = BTreeMap::new();
    let mut module_conds = ModuleConds::default();
    for file in index.files() {
        let Some(text) = read(file) else { continue };
        let serve_hint = has_word(&text, "serve") || text.contains("into_make_service");
        if !has_word(&text, "Router") && !serve_hint {
            continue;
        }
        let Ok(ast) = syn::parse_file(&text) else { continue };
        // The functions R1 kept (test-only code is not among them), by (name, name line).
        let known: BTreeMap<(String, usize), ModuleId> =
            index.functions_in(file).into_iter().map(|f| ((f.name, f.line), f.module)).collect();
        let mut found = Vec::new();
        collect_fns(&ast.items, &mut found);
        let mut any = false;
        for item in found {
            let line = item.sig.ident.span().start().line;
            let Some(&module) = known.get(&(item.sig.ident.to_string(), line)) else { continue };
            if !in_lib_or_bin(index, graph, module) {
                continue;
            }
            let router = returns_router(index, module, &item.sig);
            let serves = serve_hint && mentions_serve(&item.block);
            if !router && !serves {
                continue;
            }
            any = true;
            let mut conds = module_conds.of(index, module, read);
            if let Cfg::Conds(own) = read_cfg(&item.attrs, &text) {
                conds.extend(own);
            }
            fns.insert(
                (file.to_path_buf(), line),
                FnSite { file: file.to_path_buf(), module, line, item, router, conds },
            );
        }
        if any {
            sources.insert(file.to_path_buf(), text);
        }
    }

    let mut ev = Evaluator::new(index, &fns, &sources);
    for (key, site) in &fns {
        if site.router {
            ev.eval_fn(key);
        }
    }
    for (key, site) in &fns {
        if !site.router {
            ev.scan_serves(key);
        }
    }

    // Every call that resolves to a router function, evaluated or not: such a function is never a root.
    let mut referenced = std::mem::take(&mut ev.referenced);
    add_call_sites(index, &fns, read, &mut referenced);
    let mut roots: Vec<(&Key, &eval::Val)> = ev
        .memo
        .iter()
        .filter(|(k, _)| !referenced.contains_key(*k))
        .filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
        .chain(ev.serve_roots.iter())
        .collect();
    roots.sort_by(|a, b| a.0.cmp(b.0));

    // Referenced router functions no root folds in: their routes are skipped at the referencing site (those of a
    // function folded into another such function are reported with the outer one).
    let folded: BTreeSet<&Key> = roots.iter().flat_map(|(_, v)| v.refs.iter()).collect();
    let outside: Vec<&Key> = referenced.keys().filter(|k| !folded.contains(k)).collect();
    let inner: BTreeSet<&Key> = outside
        .iter()
        .filter_map(|k| ev.memo.get(*k).and_then(Option::as_ref).map(|v| (k, v)))
        .flat_map(|(k, v)| v.refs.iter().filter(move |r| **r != **k))
        .collect();
    let mut skipped = std::mem::take(&mut ev.skipped);
    for key in outside.into_iter().filter(|k| !inner.contains(k)) {
        let (Some(Some(val)), Some(site), Some((at, line))) =
            (ev.memo.get(key), fns.get(key), referenced.get(key).and_then(|s| s.first()))
        else {
            continue;
        };
        for p in &val.routes {
            skipped.insert((
                p.defined_at.0.clone(),
                p.defined_at.1,
                format!(
                    "`{} {}` left out: router fn `{}` is referenced at {}:{line} but not folded into an evaluated root",
                    p.method,
                    p.template,
                    site.item.sig.ident,
                    at.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                ),
            ));
        }
    }

    let mut table = RouteTable::default();
    for (root, val) in roots {
        table.services += val.services;
        for p in &val.routes {
            table.entries.push(RouteEntry {
                method: p.method.clone(),
                template: p.template.clone(),
                handler: p.handler.clone(),
                handler_text: p.handler_text.clone(),
                conditional: (!p.conds.is_empty()).then(|| p.conds.join(" && ")),
                defined_at: p.defined_at.clone(),
                root: root.clone(),
            });
        }
    }
    table.entries.sort_by_cached_key(|e| {
        (
            RouteTemplate::parse(&e.template).canonical(),
            e.method.clone(),
            e.defined_at.clone(),
            e.template.clone(),
            e.root.clone(),
        )
    });
    table.skipped = skipped.into_iter().collect();
    table
}

/// Adds the call sites R1 resolves to a router function, in every function of the index (the evaluator only sees
/// the calls in what it evaluates). Only files that name a router function are looked at.
fn add_call_sites(
    index: &RustIndex,
    fns: &BTreeMap<Key, FnSite>,
    read: &dyn Fn(&Path) -> Option<String>,
    referenced: &mut BTreeMap<Key, BTreeSet<(PathBuf, usize)>>,
) {
    let names: BTreeSet<String> = fns.values().filter(|f| f.router).map(|f| f.item.sig.ident.to_string()).collect();
    if names.is_empty() {
        return;
    }
    let is_router = |key: &Key| fns.get(key).is_some_and(|f| f.router);
    for file in index.files() {
        let Some(text) = read(file) else { continue };
        if !names.iter().any(|name| calls_word(&text, name)) {
            continue;
        }
        for (site, res) in index.resolved_calls(file) {
            let targets: Vec<Key> = match res {
                Resolution::Item(item) => vec![(item.file, item.line)],
                Resolution::Ambiguous(list) => list.into_iter().map(|i| (i.file, i.line)).collect(),
                _ => Vec::new(),
            };
            for key in targets.into_iter().filter(|k| is_router(k)) {
                referenced.entry(key).or_default().insert((file.to_path_buf(), site.line));
            }
        }
    }
}

/// The `#[cfg(…)]` conditions of modules (memoised), read from the `mod` items and `#![cfg]` bodies on the way up.
#[derive(Default)]
struct ModuleConds {
    by_module: BTreeMap<u32, Vec<String>>,
    files: BTreeMap<PathBuf, Option<(String, syn::File)>>,
}

impl ModuleConds {
    /// The conditions a module is compiled under, outermost first (R1 marks the gated ones; others have none).
    fn of(&mut self, index: &RustIndex, module: ModuleId, read: &dyn Fn(&Path) -> Option<String>) -> Vec<String> {
        if let Some(conds) = self.by_module.get(&module.0) {
            return conds.clone();
        }
        let Some(data) = index.modules.get(module.0 as usize) else { return Vec::new() };
        if !data.gated {
            return Vec::new();
        }
        let mut conds = match data.parent {
            Some(parent) => self.of(index, parent, read),
            None => Vec::new(),
        };
        let decl = data.decl_item.and_then(|d| index.items.get(d)).map(|d| (d.r.file.clone(), d.r.line));
        if let Some((file, line)) = &decl {
            // The `mod` item's attributes (an inline module's `#![cfg]` is among them).
            if let Some((text, ast)) = self.file(file, read) {
                if let Some(Cfg::Conds(own)) = find_mod(&ast.items, *line).map(|m| read_cfg(&m.attrs, text)) {
                    conds.extend(own);
                }
            }
        }
        if decl.as_ref().is_none_or(|(file, _)| *file != data.file) {
            // A module file of its own (or a crate root): its `#![cfg]`.
            let file = data.file.clone();
            if let Some((text, ast)) = self.file(&file, read) {
                if let Cfg::Conds(own) = read_cfg(&ast.attrs, text) {
                    conds.extend(own);
                }
            }
        }
        self.by_module.insert(module.0, conds.clone());
        conds
    }

    fn file(&mut self, file: &Path, read: &dyn Fn(&Path) -> Option<String>) -> Option<&(String, syn::File)> {
        self.files
            .entry(file.to_path_buf())
            .or_insert_with(|| read(file).and_then(|text| syn::parse_file(&text).ok().map(|ast| (text, ast))))
            .as_ref()
    }
}

/// The `mod` item whose name is on `line`, at any depth of inline modules.
fn find_mod(items: &[syn::Item], line: usize) -> Option<&syn::ItemMod> {
    items.iter().find_map(|item| match item {
        syn::Item::Mod(m) if m.ident.span().start().line == line => Some(m),
        syn::Item::Mod(m) => m.content.as_ref().and_then(|(_, inner)| find_mod(inner, line)),
        _ => None,
    })
}

/// Free functions of a module and of the inline modules inside it, at any depth (impl methods are not router
/// builders here: calling one needs types).
fn collect_fns(items: &[syn::Item], out: &mut Vec<syn::ItemFn>) {
    for item in items {
        match item {
            syn::Item::Fn(f) => out.push(f.clone()),
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_fns(inner, out);
                }
            }
            _ => {}
        }
    }
}

/// True when the module belongs to a lib or bin crate (proc-macro and other targets hold no served routes).
fn in_lib_or_bin(index: &RustIndex, graph: &CrateGraph, module: ModuleId) -> bool {
    index
        .modules
        .get(module.0 as usize)
        .and_then(|m| graph.crates.get(m.krate))
        .is_some_and(|c| matches!(c.kind, TargetKind::Lib | TargetKind::Bin))
}

/// True when the function's return type resolves to axum's `Router` (any generic arguments).
fn returns_router(index: &RustIndex, module: ModuleId, sig: &syn::Signature) -> bool {
    let syn::ReturnType::Type(_, ty) = &sig.output else { return false };
    let syn::Type::Path(tp) = &**ty else { return false };
    let Some(segs) = eval::path_segments(tp.qself.is_some(), &tp.path) else { return false };
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    matches!(index.resolve(module, &segs), Resolution::External(p) if ROUTER_TYPES.contains(&p.as_str()))
}

/// True when the body names `serve` or `into_make_service*` anywhere (a cheap filter before the serve scan).
fn mentions_serve(block: &syn::Block) -> bool {
    eval::any_ident(block.to_token_stream(), &|id| id == "serve" || id.starts_with("into_make_service"))
}

/// True when `word` occurs in `text` as a whole identifier followed by a call (`word(`, `word::<`), maybe with
/// whitespace between (a cheap filter before resolving a file's calls).
fn calls_word(text: &str, word: &str) -> bool {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(word).any(|(at, _)| {
        let after = text[at + word.len()..].trim_start();
        !ident(text[..at].chars().next_back()) && (after.starts_with('(') || after.starts_with("::<"))
    })
}

/// True when `word` occurs in `text` as a whole identifier (a cheap filter before parsing).
fn has_word(text: &str, word: &str) -> bool {
    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(word)
        .any(|(at, _)| !ident(text[..at].chars().next_back()) && !ident(text[at + word.len()..].chars().next()))
}
