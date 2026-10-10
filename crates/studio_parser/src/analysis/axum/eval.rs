//! The router expression and statement evaluator behind [`super::evaluate_routes`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{Expr, Stmt};

use crate::analysis::route_template::RouteTemplate;
use crate::analysis::rust_resolver::{ItemKind, ModuleId, Resolution, RustIndex, UnresolvedReason};

/// A function by (file, name line): the same key R1's `ItemRef` gives.
pub(super) type Key = (PathBuf, usize);

/// A free function of a lib or bin module tree that builds a router or may serve one.
pub(super) struct FnSite {
    pub file: PathBuf,
    pub module: ModuleId,
    pub line: usize,
    pub item: syn::ItemFn,
    /// Its return type resolves to axum's `Router`.
    pub router: bool,
    /// The `#[cfg(…)]` conditions it is compiled under, as written: the modules around it (outermost first), then
    /// the function's own.
    pub conds: Vec<String>,
}

/// A route registered in a router value, relative to that router (no nest prefix of an outer router yet).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Pending {
    pub method: String,
    pub template: String,
    pub handler: Resolution,
    pub handler_text: String,
    /// Conditions of the enclosing `if`s, outermost first.
    pub conds: Vec<String>,
    pub defined_at: (PathBuf, usize),
}

/// What a router expression evaluates to.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Val {
    pub routes: Vec<Pending>,
    pub services: usize,
    /// The router functions folded into this value.
    pub refs: BTreeSet<Key>,
}

impl Val {
    /// Appends `other`, its templates nested under `prefix` when there is one.
    fn absorb(&mut self, other: Val, prefix: Option<&str>) {
        for mut p in other.routes {
            if let Some(prefix) = prefix {
                p.template = join_text(prefix, &p.template);
            }
            self.routes.push(p);
        }
        self.services += other.services;
        self.refs.extend(other.refs);
    }

    /// The value that is `t` when `cond` holds and `e` otherwise: a route both have (same registration, same
    /// conditions) stays as it is, the others carry `cond` or `!(cond)`. Services both have (`shared`) count once.
    fn either(t: &Val, e: &Val, cond: &str, shared: usize) -> Val {
        let common = t.routes.iter().zip(&e.routes).take_while(|(a, b)| a == b).count();
        let mut out = Val {
            routes: t.routes[..common].to_vec(),
            services: (t.services + e.services).saturating_sub(shared),
            refs: t.refs.union(&e.refs).cloned().collect(),
        };
        let mut used = vec![false; e.routes.len()];
        for p in &t.routes[common..] {
            match (common..e.routes.len()).find(|&j| !used[j] && e.routes[j] == *p) {
                Some(j) => {
                    used[j] = true;
                    out.routes.push(p.clone());
                }
                None => out.routes.push(p.under(&[cond.to_string()])),
            }
        }
        let not = [format!("!({cond})")];
        for (j, p) in e.routes.iter().enumerate().skip(common) {
            if !used[j] {
                out.routes.push(p.under(&not));
            }
        }
        out
    }

    /// Puts every route under `conds` (outermost first; a condition a route already carries is not repeated).
    fn gate(&mut self, conds: &[String]) {
        if !conds.is_empty() {
            for p in &mut self.routes {
                *p = p.under(conds);
            }
        }
    }
}

impl Pending {
    /// This route with `conds` in front of its own conditions (those it already carries are not repeated).
    fn under(&self, conds: &[String]) -> Pending {
        let mut p = self.clone();
        let new: Vec<String> = conds.iter().filter(|c| !self.conds.contains(c)).cloned().collect();
        p.conds.splice(0..0, new);
        p
    }
}

/// The text of a nested route: axum `nest` semantics (`/api` + `/` is `/api`).
fn join_text(prefix: &str, rest: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let joined = if rest.is_empty() || rest == "/" {
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    } else if rest.starts_with('/') {
        format!("{p}{rest}")
    } else {
        format!("{p}/{rest}")
    };
    debug_assert_eq!(
        RouteTemplate::parse(&joined),
        RouteTemplate::join(&RouteTemplate::parse(prefix), &RouteTemplate::parse(rest))
    );
    joined
}

/// A local router binding and the line of its `let`.
#[derive(Debug, Clone)]
struct Binding {
    val: Val,
    line: usize,
}

type Env = BTreeMap<String, Binding>;

/// How a statement list ends.
enum Outcome {
    /// With a router value (tail expression or `return`).
    Value(Val),
    /// With something the evaluator could not follow (already recorded in `skipped`).
    Unknown,
    /// Without a value.
    Nothing,
}

/// The function being evaluated.
struct Scope<'s> {
    file: &'s Path,
    module: ModuleId,
    text: &'s str,
    /// A serve scan (any function) rather than a router function's body.
    serve: bool,
}

/// What a method router expression registers.
#[derive(Default)]
struct MethodRouter {
    handlers: Vec<(String, Resolution, String)>,
    services: usize,
}

/// The `axum::routing` method functions and their HTTP method.
const METHODS: [&str; 9] = ["get", "post", "put", "delete", "patch", "head", "options", "trace", "connect"];
/// The paths `Router::new` resolves to when it is axum's.
const ROUTER_NEW: [&str; 2] = ["axum::Router::new", "axum::routing::Router::new"];
/// The paths `serve` resolves to when it is axum's.
const SERVE: [&str; 2] = ["axum::serve", "axum::serve::serve"];
/// How deep early returns may nest before a function is given up (each one evaluates the rest of the body twice).
const MAX_SPLITS: usize = 8;

pub(super) struct Evaluator<'a> {
    index: &'a RustIndex,
    fns: &'a BTreeMap<Key, FnSite>,
    sources: &'a BTreeMap<PathBuf, String>,
    /// Router functions evaluated so far (`None`: no router value could be followed).
    pub memo: BTreeMap<Key, Option<Val>>,
    active: BTreeSet<Key>,
    /// Serve sites: (file, line of the served binding's `let`, or of the served expression) → the router.
    pub serve_roots: BTreeMap<Key, Val>,
    pub skipped: BTreeSet<(PathBuf, usize, String)>,
    /// Every router function a call resolved to while evaluating (whatever came of it) → the calling sites.
    pub referenced: BTreeMap<Key, BTreeSet<(PathBuf, usize)>>,
    /// The `#[cfg]` conditions the code being evaluated is under (the function's, then gated statements'): a serve
    /// site found there serves its router only under them.
    guard: Vec<String>,
    /// Early returns being followed (see [`MAX_SPLITS`]).
    splits: usize,
}

impl<'a> Evaluator<'a> {
    pub fn new(index: &'a RustIndex, fns: &'a BTreeMap<Key, FnSite>, sources: &'a BTreeMap<PathBuf, String>) -> Self {
        Evaluator {
            index,
            fns,
            sources,
            memo: BTreeMap::new(),
            active: BTreeSet::new(),
            serve_roots: BTreeMap::new(),
            skipped: BTreeSet::new(),
            referenced: BTreeMap::new(),
            guard: Vec::new(),
            splits: 0,
        }
    }

    /// Evaluates a router function once (memoised by identity; nest prefixes are applied by the caller).
    pub fn eval_fn(&mut self, key: &Key) -> Option<Val> {
        if let Some(v) = self.memo.get(key) {
            return v.clone();
        }
        let fns = self.fns;
        let site = fns.get(key)?;
        if !self.active.insert(key.clone()) {
            self.skip(&site.file, site.line, "recursive router function".to_string());
            return None;
        }
        let sc = self.scope(site, false);
        let mut env = Env::new();
        let guard = std::mem::replace(&mut self.guard, site.conds.clone());
        let splits = std::mem::take(&mut self.splits);
        let result = match self.eval_stmts(&site.item.block.stmts, &mut env, &sc, true) {
            Outcome::Value(mut v) => {
                v.gate(&site.conds);
                Some(v)
            }
            Outcome::Unknown => None,
            Outcome::Nothing => {
                self.skip(&site.file, site.line, "no router value is returned that the evaluator follows".into());
                None
            }
        };
        self.guard = guard;
        self.splits = splits;
        self.active.remove(key);
        self.memo.insert(key.clone(), result.clone());
        result
    }

    /// Looks for routers handed to `axum::serve` / `.into_make_service*()` in a function's top-level statements.
    pub fn scan_serves(&mut self, key: &Key) {
        let fns = self.fns;
        let Some(site) = fns.get(key) else { return };
        let sc = self.scope(site, true);
        let mut env = Env::new();
        self.guard = site.conds.clone();
        self.eval_stmts(&site.item.block.stmts, &mut env, &sc, true);
        self.guard.clear();
    }

    fn scope(&self, site: &'a FnSite, serve: bool) -> Scope<'a> {
        let sources = self.sources;
        Scope {
            file: &site.file,
            module: site.module,
            text: sources.get(&site.file).map(String::as_str).unwrap_or(""),
            serve,
        }
    }

    fn skip(&mut self, file: &Path, line: usize, reason: String) {
        self.skipped.insert((file.to_path_buf(), line, reason));
    }

    /// Records a call (at `line` of `file`) that resolves to the router function `key`.
    fn reference(&mut self, key: &Key, file: &Path, line: usize) {
        self.referenced.entry(key.clone()).or_default().insert((file.to_path_buf(), line));
    }

    /// Records a served router (under the current `#[cfg]` guard); the first one found at a site wins.
    fn serve_root(&mut self, key: Key, mut val: Val) {
        val.gate(&self.guard);
        self.serve_roots.entry(key).or_insert(val);
    }

    fn resolve(&self, sc: &Scope, segs: &[String]) -> Resolution {
        let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
        self.index.resolve(sc.module, &segs)
    }

    // ---- statements -------------------------------------------------------------------------------------------

    fn eval_stmts(&mut self, stmts: &[Stmt], env: &mut Env, sc: &Scope, top: bool) -> Outcome {
        for (i, stmt) in stmts.iter().enumerate() {
            let last = i + 1 == stmts.len();
            let rest = &stmts[i + 1..];
            match stmt_cfg(stmt, sc.text) {
                // Compiled only for tests: not part of the served router.
                Cfg::Test => continue,
                Cfg::Conds(conds) if !conds.is_empty() => {
                    match self.gated_stmt(stmt, &conds.join(" && "), rest, last, env, sc, top) {
                        Some(out) => return out,
                        None => continue,
                    }
                }
                Cfg::Conds(_) => {}
            }
            // In a router function, a `return` decides the value: one the evaluator cannot put under a condition
            // makes the value unknown (what follows it would otherwise look unconditional).
            if !sc.serve && !matches!(stmt, Stmt::Expr(Expr::If(_) | Expr::Return(_), _)) && has_return(stmt) {
                let (line, kind) = match stmt {
                    Stmt::Expr(expr, _) => (line_of(expr.span()), kind_of(expr)),
                    _ => (line_of(stmt.span()), "`let`"),
                };
                self.skip(sc.file, line, format!("early `return` inside a {kind} the evaluator does not follow"));
                return Outcome::Unknown;
            }
            match stmt {
                Stmt::Local(local) => self.local(local, env, sc),
                Stmt::Item(_) => {}
                Stmt::Macro(m) => {
                    if mentions(m.to_token_stream(), env) {
                        self.skip(sc.file, line_of(m.span()), "router used in a macro".into());
                    }
                }
                Stmt::Expr(expr, semi) => match expr {
                    Expr::Assign(a) if assigned_binding(&a.left, env).is_some() => {
                        let name = assigned_binding(&a.left, env).unwrap_or_default();
                        match self.eval_router(&a.right, env, sc) {
                            Some(v) => {
                                if let Some(b) = env.get_mut(&name) {
                                    b.val = v;
                                }
                            }
                            None => {
                                if !self.find_serves(&a.right, env, sc) {
                                    self.skip(
                                        sc.file,
                                        line_of(a.span()),
                                        format!("router `{name}` reassigned from an expression the evaluator does not follow"),
                                    );
                                }
                            }
                        }
                    }
                    // `if c { …; return x; } rest` is `if c { …; return x; } else { rest }`: the rest of the body
                    // runs in both branches and the function's value is decided there.
                    Expr::If(ifx) if !sc.serve && has_return(stmt) => {
                        return self.eval_if(ifx, Some(rest), true, env, sc, top);
                    }
                    Expr::If(ifx) if mentions(ifx.to_token_stream(), env) => {
                        let tail = last && semi.is_none();
                        let out = self.eval_if(ifx, None, tail, env, sc, top);
                        if tail {
                            return out;
                        }
                    }
                    // A plain block: its own bindings stay inside, what it does to outer router bindings is kept.
                    Expr::Block(b) if b.label.is_none() && mentions(b.to_token_stream(), env) => {
                        let tail = last && semi.is_none();
                        let (inner, declared, out) = self.branch(&b.block.stmts, env, sc, top && tail);
                        for (name, binding) in env.iter_mut() {
                            if let Some(b) = inner.get(name).filter(|_| !declared.contains(name)) {
                                binding.val = b.val.clone();
                            }
                        }
                        if tail {
                            return out;
                        }
                    }
                    Expr::Return(ret) => {
                        return match &ret.expr {
                            Some(e) => self.tail(e, env, sc, true),
                            None => Outcome::Nothing,
                        };
                    }
                    _ if last && semi.is_none() => return self.tail(expr, env, sc, top),
                    _ => self.other(expr, env, sc),
                },
            }
        }
        Outcome::Nothing
    }

    /// A statement under `#[cfg(…)]` (`cond`, the attributes as written): it only exists under that condition, like
    /// the then branch of an `if` without `else`. Returns the outcome when the statement list ends here.
    #[allow(clippy::too_many_arguments)]
    fn gated_stmt(
        &mut self,
        stmt: &Stmt,
        cond: &str,
        rest: &[Stmt],
        last: bool,
        env: &mut Env,
        sc: &Scope,
        top: bool,
    ) -> Option<Outcome> {
        let stmt = strip_cfg(stmt);
        let line = line_of(stmt.span());
        if !sc.serve && has_return(&stmt) {
            // `#[cfg(x)] return r;` (or a gated statement holding a return): the rest runs only without it.
            return Some(self.split(line, cond, vec![stmt], Vec::new(), Some(rest), true, env, sc, top));
        }
        self.guard.push(cond.to_string());
        let out = match &stmt {
            Stmt::Local(local) => {
                self.gated_local(local, cond, env, sc);
                None
            }
            _ => {
                let out = self.split(line, cond, vec![stmt.clone()], Vec::new(), None, last, env, sc, top);
                last.then_some(out)
            }
        };
        self.guard.pop();
        out
    }

    /// `#[cfg(x)] let r = …;`: after it, `r` is the new value under `x` and what it was before otherwise (a name
    /// that was not a router binding exists only under `x`).
    fn gated_local(&mut self, local: &syn::Local, cond: &str, env: &mut Env, sc: &Scope) {
        let Some(name) = pat_ident(&local.pat) else {
            self.local(local, env, sc);
            return;
        };
        let old = env.get(&name).cloned();
        self.local(local, env, sc);
        let Some(new) = env.get_mut(&name) else { return };
        new.val = match old {
            Some(old) => Val::either(&new.val, &old.val, cond, old.val.services),
            None => {
                let mut v = std::mem::take(&mut new.val);
                v.gate(&[cond.to_string()]);
                v
            }
        };
    }

    fn local(&mut self, local: &syn::Local, env: &mut Env, sc: &Scope) {
        let name = pat_ident(&local.pat);
        let line = line_of(local.let_token.span);
        let Some(init) = &local.init else {
            if let Some(n) = &name {
                env.remove(n);
            }
            return;
        };
        let value = match (&name, &init.diverge) {
            (Some(_), None) => self.eval_router(&init.expr, env, sc),
            _ => None,
        };
        match value {
            Some(val) => {
                env.insert(name.unwrap_or_default(), Binding { val, line });
            }
            None => {
                // `let app = <router>.into_make_service*();` is a serve site at the `let`.
                let served = peel_served(&init.expr);
                let root = match std::ptr::eq(served, &*init.expr) {
                    true => None,
                    false => self.eval_router(served, env, sc),
                };
                match root {
                    Some(val) => self.serve_root((sc.file.to_path_buf(), line), val),
                    None => self.other(&init.expr, env, sc),
                }
                if let Some(n) = &name {
                    env.remove(n);
                }
            }
        }
    }

    /// The value a statement list ends with. At a router function's top level, a non-router tail is recorded.
    fn tail(&mut self, expr: &Expr, env: &Env, sc: &Scope, top: bool) -> Outcome {
        if let Some(v) = self.eval_router(expr, env, sc) {
            return Outcome::Value(v);
        }
        if top && !sc.serve {
            if !self.find_serves(expr, env, sc) {
                self.skip(
                    sc.file,
                    line_of(expr.span()),
                    format!("returns `{}`, not a router expression the evaluator follows", short(&self.text(expr, sc))),
                );
            }
            return Outcome::Unknown;
        }
        self.other(expr, env, sc);
        Outcome::Nothing
    }

    /// A statement that is not part of the router grammar: a serve site, or, when it touches a router, a skip.
    fn other(&mut self, expr: &Expr, env: &Env, sc: &Scope) {
        if self.find_serves(expr, env, sc) {
            return;
        }
        if mentions(expr.to_token_stream(), env) {
            self.skip(sc.file, line_of(expr.span()), format!("router used in a {}", kind_of(expr)));
        }
    }

    /// `if cond { … } else { … }`: each branch runs on a copy of the bindings; after it, a router binding is the
    /// then value under `cond` and the else value under `!(cond)` (routes both have stay unconditional). With `rest`
    /// (the statements after an `if` that returns), both branches go on with it and the `if` ends the list.
    fn eval_if(
        &mut self,
        ifx: &syn::ExprIf,
        rest: Option<&[Stmt]>,
        tail: bool,
        env: &mut Env,
        sc: &Scope,
        top: bool,
    ) -> Outcome {
        let line = line_of(ifx.if_token.span);
        let cond = self.text(&ifx.cond, sc);
        let then = ifx.then_branch.stmts.clone();
        let els = match ifx.else_branch.as_ref().map(|(_, e)| &**e) {
            Some(Expr::Block(b)) => b.block.stmts.clone(),
            Some(other) => vec![Stmt::Expr(other.clone(), None)],
            None => Vec::new(),
        };
        self.split(line, &cond, then, els, rest, tail, env, sc, top)
    }

    /// Runs `then` under `cond` and `els` under `!(cond)`, each followed by `rest` when given; `tail` when the
    /// branches end the statement list (their outcomes are then combined into the list's).
    #[allow(clippy::too_many_arguments)]
    fn split(
        &mut self,
        line: usize,
        cond: &str,
        mut then: Vec<Stmt>,
        mut els: Vec<Stmt>,
        rest: Option<&[Stmt]>,
        tail: bool,
        env: &mut Env,
        sc: &Scope,
        top: bool,
    ) -> Outcome {
        if let Some(rest) = rest {
            if self.splits >= MAX_SPLITS {
                self.skip(sc.file, line, format!("more than {MAX_SPLITS} nested early returns"));
                return Outcome::Unknown;
            }
            then.extend(rest.iter().cloned());
            els.extend(rest.iter().cloned());
            self.splits += 1;
        }
        let (then_env, then_decl, then_out) = self.branch(&then, env, sc, top && tail);
        let (else_env, else_decl, else_out) = self.branch(&els, env, sc, top && tail);
        if rest.is_some() {
            self.splits -= 1;
        }
        let names: Vec<String> = env.keys().cloned().collect();
        for name in names {
            let base = env[&name].val.clone();
            let pick = |branch_env: &Env, declared: &BTreeSet<String>| {
                if declared.contains(&name) {
                    base.clone()
                } else {
                    branch_env.get(&name).map_or_else(|| base.clone(), |b| b.val.clone())
                }
            };
            let merged = Val::either(&pick(&then_env, &then_decl), &pick(&else_env, &else_decl), cond, base.services);
            if let Some(b) = env.get_mut(&name) {
                b.val = merged;
            }
        }
        match (then_out, else_out) {
            (Outcome::Value(t), Outcome::Value(e)) => {
                let shared = t.services.min(e.services);
                Outcome::Value(Val::either(&t, &e, cond, shared))
            }
            (Outcome::Nothing, Outcome::Nothing) => Outcome::Nothing,
            (Outcome::Unknown, _) | (_, Outcome::Unknown) => Outcome::Unknown,
            _ => {
                if !sc.serve {
                    self.skip(sc.file, line, "a router value comes out of only one branch of an `if`".into());
                }
                Outcome::Unknown
            }
        }
    }

    /// Runs a branch on a copy of the bindings; returns them, the names the branch declares itself, and its end.
    fn branch(&mut self, stmts: &[Stmt], env: &Env, sc: &Scope, top: bool) -> (Env, BTreeSet<String>, Outcome) {
        let mut inner = env.clone();
        let declared: BTreeSet<String> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Local(l) => pat_ident(&l.pat),
                _ => None,
            })
            .collect();
        let out = self.eval_stmts(stmts, &mut inner, sc, top);
        (inner, declared, out)
    }

    // ---- serve sites ------------------------------------------------------------------------------------------

    /// Records the routers `expr` hands to `axum::serve` or `.into_make_service*()`; true when it found one.
    fn find_serves(&mut self, expr: &Expr, env: &Env, sc: &Scope) -> bool {
        if !any_ident(expr.to_token_stream(), &|id| id == "serve" || id.starts_with("into_make_service")) {
            return false;
        }
        let mut finder = ServeFinder { ev: &*self, sc, found: Vec::new() };
        finder.visit_expr(expr);
        let found = finder.found;
        let mut any = false;
        for served in found {
            let served = peel_served(served);
            let root = match served {
                Expr::Path(p) if p.qself.is_none() => p
                    .path
                    .get_ident()
                    .and_then(|id| env.get(&id.to_string()))
                    .map(|b| ((sc.file.to_path_buf(), b.line), b.val.clone())),
                _ => None,
            };
            let root = match root {
                Some(r) => Some(r),
                None => self.eval_router(served, env, sc).map(|v| ((sc.file.to_path_buf(), line_of(served.span())), v)),
            };
            if let Some((key, val)) = root {
                self.serve_root(key, val);
                any = true;
            }
        }
        any
    }

    // ---- expressions ------------------------------------------------------------------------------------------

    /// The router an expression builds, or `None` when it is not a router expression the evaluator knows.
    fn eval_router(&mut self, expr: &Expr, env: &Env, sc: &Scope) -> Option<Val> {
        match expr {
            Expr::Paren(p) => self.eval_router(&p.expr, env, sc),
            Expr::Group(g) => self.eval_router(&g.expr, env, sc),
            Expr::Path(p) if p.qself.is_none() => {
                p.path.get_ident().and_then(|id| env.get(&id.to_string())).map(|b| b.val.clone())
            }
            Expr::Call(c) => {
                let Expr::Path(fp) = &*c.func else { return None };
                let segs = path_segments(fp.qself.is_some(), &fp.path)?;
                match self.resolve(sc, &segs) {
                    Resolution::External(p) if ROUTER_NEW.contains(&p.as_str()) => Some(Val::default()),
                    Resolution::Item(item) if item.kind == ItemKind::Fn => {
                        let key = (item.file, item.line);
                        if !self.fns.get(&key).is_some_and(|f| f.router) {
                            return None;
                        }
                        self.reference(&key, sc.file, line_of(c.span()));
                        let mut v = self.eval_fn(&key).unwrap_or_default();
                        v.refs.insert(key);
                        Some(v)
                    }
                    // Not followed, but none of the candidates may become a root of its own.
                    Resolution::Ambiguous(list) => {
                        for item in list {
                            let key = (item.file, item.line);
                            if self.fns.get(&key).is_some_and(|f| f.router) {
                                self.reference(&key, sc.file, line_of(c.span()));
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            // A make-service is what gets served, no longer a router to build on.
            Expr::MethodCall(m) if m.method.to_string().starts_with("into_make_service") => None,
            Expr::MethodCall(m) => {
                let base = self.eval_router(&m.receiver, env, sc)?;
                Some(self.apply(base, m, env, sc))
            }
            _ => None,
        }
    }

    /// One builder method on a router value.
    fn apply(&mut self, mut base: Val, m: &syn::ExprMethodCall, env: &Env, sc: &Scope) -> Val {
        let method = m.method.to_string();
        let line = line_of(m.method.span());
        let args: Vec<&Expr> = m.args.iter().collect();
        match method.as_str() {
            "route" => {
                let [path, mr] = args[..] else {
                    self.skip(sc.file, line, "`.route` without two arguments".into());
                    return base;
                };
                let Some(path) = str_lit(path) else {
                    let text = short(&self.text(path, sc));
                    self.skip(sc.file, line, format!("`.route` path `{text}` is not a string literal"));
                    return base;
                };
                match self.method_router(mr, sc) {
                    Ok(found) => {
                        for (method, handler, handler_text) in found.handlers {
                            base.routes.push(Pending {
                                method,
                                template: path.clone(),
                                handler,
                                handler_text,
                                conds: Vec::new(),
                                defined_at: (sc.file.to_path_buf(), line),
                            });
                        }
                        base.services += found.services;
                    }
                    Err(reason) => self.skip(sc.file, line, reason),
                }
            }
            "nest" | "merge" => {
                let (prefix, sub) = match (method.as_str(), &args[..]) {
                    ("nest", [prefix, sub]) => match str_lit(prefix) {
                        Some(p) => (Some(p), *sub),
                        None => {
                            let text = short(&self.text(*prefix, sc));
                            self.skip(sc.file, line, format!("`.nest` prefix `{text}` is not a string literal"));
                            return base;
                        }
                    },
                    ("merge", [sub]) => (None, *sub),
                    _ => {
                        self.skip(sc.file, line, format!("`.{method}` with unexpected arguments"));
                        return base;
                    }
                };
                match self.eval_router(sub, env, sc) {
                    Some(v) => base.absorb(v, prefix.as_deref()),
                    None => {
                        let text = short(&self.text(sub, sc));
                        self.skip(
                            sc.file,
                            line,
                            format!("`.{method}` router `{text}` is not one the evaluator follows"),
                        );
                    }
                }
            }
            "layer" | "route_layer" | "with_state" | "without_v07_checks" | "reset_fallback" => {}
            "fallback" | "fallback_service" | "nest_service" | "route_service" | "method_not_allowed_fallback" => {
                base.services += 1;
            }
            other => self.skip(sc.file, line, format!("unknown method `.{other}` on a router")),
        }
        base
    }

    /// The methods and handlers a method router registers (`get(a).post(b)`, `on(MethodFilter::PUT, h)`, …).
    fn method_router(&mut self, expr: &Expr, sc: &Scope) -> Result<MethodRouter, String> {
        match expr {
            Expr::Paren(p) => self.method_router(&p.expr, sc),
            Expr::Group(g) => self.method_router(&g.expr, sc),
            Expr::Call(c) => {
                let not_axum = |what: String| format!("method router `{what}` does not resolve to axum::routing");
                let Expr::Path(fp) = &*c.func else { return Err(not_axum(short(&self.text(&c.func, sc)))) };
                let Some(segs) = path_segments(fp.qself.is_some(), &fp.path) else {
                    return Err(not_axum(short(&self.text(fp, sc))));
                };
                let res = self.resolve(sc, &segs);
                let name = match &res {
                    Resolution::External(p) => routing_fn(p),
                    _ => None,
                };
                let Some(name) = name else {
                    return Err(format!("{} ({})", not_axum(segs.join("::")), describe(&res)));
                };
                let args: Vec<&Expr> = c.args.iter().collect();
                self.method_args(name, &args, MethodRouter::default(), sc)
            }
            Expr::MethodCall(m) => {
                let inner = self.method_router(&m.receiver, sc)?;
                let name = m.method.to_string();
                match name.as_str() {
                    "layer" | "route_layer" | "with_state" => Ok(inner),
                    "fallback" | "fallback_service" => Ok(MethodRouter { services: inner.services + 1, ..inner }),
                    _ => {
                        let args: Vec<&Expr> = m.args.iter().collect();
                        self.method_args(&name, &args, inner, sc)
                    }
                }
            }
            _ => Err(format!("method router `{}` is not a call the evaluator follows", short(&self.text(expr, sc)))),
        }
    }

    /// Adds one method function's registrations (`get(h)`, `any(h)`, `on(filter, h)`, `get_service(s)`, …).
    fn method_args(
        &mut self,
        name: &str,
        args: &[&Expr],
        mut mr: MethodRouter,
        sc: &Scope,
    ) -> Result<MethodRouter, String> {
        let (method, handler) = match (name, args) {
            (n, [h]) if METHODS.contains(&n) => (n.to_ascii_uppercase(), *h),
            ("any", [h]) => ("ANY".to_string(), *h),
            ("on", [filter, h]) => (self.method_filter(filter, sc)?, *h),
            (n, _) if n.ends_with("_service") => {
                mr.services += 1;
                return Ok(mr);
            }
            _ => return Err(format!("unknown method router call `{name}`")),
        };
        let (res, text) = self.handler(handler, sc);
        mr.handlers.push((method, res, text));
        Ok(mr)
    }

    /// `MethodFilter::PUT` → "PUT" (a single constant only).
    fn method_filter(&self, expr: &Expr, sc: &Scope) -> Result<String, String> {
        if let Expr::Path(p) = expr {
            if let Some(segs) = path_segments(p.qself.is_some(), &p.path) {
                if let Resolution::External(path) = self.resolve(sc, &segs) {
                    let constant = path
                        .strip_prefix("axum::routing::MethodFilter::")
                        .or_else(|| path.strip_prefix("axum::routing::method_filter::MethodFilter::"));
                    if let Some(c) = constant.filter(|c| !c.contains("::")) {
                        return Ok(c.to_ascii_uppercase());
                    }
                }
            }
        }
        Err(format!("`on` filter `{}` is not a single MethodFilter constant", short(&self.text(expr, sc))))
    }

    /// A handler: a path resolves through R1 from the route's module; anything else stays unresolved.
    fn handler(&self, expr: &Expr, sc: &Scope) -> (Resolution, String) {
        let text = self.text(expr, sc);
        match expr {
            Expr::Paren(p) => self.handler(&p.expr, sc),
            Expr::Path(p) => match path_segments(p.qself.is_some(), &p.path) {
                Some(segs) => (self.resolve(sc, &segs), text),
                None => (Resolution::Unresolved(UnresolvedReason::Other("qualified path".into())), text),
            },
            Expr::Closure(_) => (Resolution::Unresolved(UnresolvedReason::Other("closure".into())), short(&text)),
            _ => (Resolution::Unresolved(UnresolvedReason::Other("expression".into())), short(&text)),
        }
    }

    /// The source text of a node, whitespace collapsed (tokens joined when the span cannot be sliced).
    fn text<T: Spanned + ToTokens>(&self, node: &T, sc: &Scope) -> String {
        span_text(sc.text, node.span()).unwrap_or_else(|| node.to_token_stream().to_string())
    }
}

/// Collects the routers handed to `axum::serve(_, r)` and `r.into_make_service*()`.
struct ServeFinder<'e, 'a, 's, 'x> {
    ev: &'e Evaluator<'a>,
    sc: &'e Scope<'s>,
    found: Vec<&'x Expr>,
}

impl<'x> Visit<'x> for ServeFinder<'_, '_, '_, 'x> {
    fn visit_expr_call(&mut self, c: &'x syn::ExprCall) {
        if let Expr::Path(fp) = &*c.func {
            if let Some(segs) = path_segments(fp.qself.is_some(), &fp.path) {
                if matches!(self.ev.resolve(self.sc, &segs), Resolution::External(p) if SERVE.contains(&p.as_str())) {
                    if let Some(arg) = c.args.iter().nth(1) {
                        self.found.push(arg);
                    }
                }
            }
        }
        syn::visit::visit_expr_call(self, c);
    }

    fn visit_expr_method_call(&mut self, m: &'x syn::ExprMethodCall) {
        if m.method.to_string().starts_with("into_make_service") {
            self.found.push(&m.receiver);
        }
        syn::visit::visit_expr_method_call(self, m);
    }

    fn visit_expr_closure(&mut self, _: &'x syn::ExprClosure) {}
}

/// What the `#[cfg]` attributes of a piece of code say.
pub(super) enum Cfg {
    /// Compiled only for tests (`#[cfg(test)]`).
    Test,
    /// Compiled under these conditions, each `cfg(…)` as written (`cfg(not(test))` is always true outside tests and
    /// is left out); empty when there is none.
    Conds(Vec<String>),
}

/// Reads the `#[cfg(…)]` attributes among `attrs` (`text` is the file the attributes are in).
pub(super) fn read_cfg(attrs: &[syn::Attribute], text: &str) -> Cfg {
    let mut conds = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("cfg")) {
        let syn::Meta::List(list) = &attr.meta else { continue };
        let compact: String = list.tokens.to_string().chars().filter(|c| !c.is_whitespace()).collect();
        match compact.as_str() {
            "test" => return Cfg::Test,
            "not(test)" => {}
            _ => conds.push(span_text(text, attr.meta.span()).unwrap_or_else(|| format!("cfg({})", list.tokens))),
        }
    }
    Cfg::Conds(conds)
}

/// The `#[cfg]`s of a statement.
fn stmt_cfg(stmt: &Stmt, text: &str) -> Cfg {
    read_cfg(stmt_attrs(stmt), text)
}

/// The outer attributes of a statement (for an expression statement, those of the expressions that carry them).
fn stmt_attrs(stmt: &Stmt) -> &[syn::Attribute] {
    match stmt {
        Stmt::Local(l) => &l.attrs,
        Stmt::Macro(m) => &m.attrs,
        Stmt::Item(_) => &[],
        Stmt::Expr(e, _) => match e {
            Expr::Assign(x) => &x.attrs,
            Expr::Async(x) => &x.attrs,
            Expr::Await(x) => &x.attrs,
            Expr::Block(x) => &x.attrs,
            Expr::Call(x) => &x.attrs,
            Expr::ForLoop(x) => &x.attrs,
            Expr::If(x) => &x.attrs,
            Expr::Loop(x) => &x.attrs,
            Expr::Macro(x) => &x.attrs,
            Expr::Match(x) => &x.attrs,
            Expr::MethodCall(x) => &x.attrs,
            Expr::Return(x) => &x.attrs,
            Expr::Try(x) => &x.attrs,
            Expr::Unsafe(x) => &x.attrs,
            Expr::While(x) => &x.attrs,
            _ => &[],
        },
    }
}

/// A statement without its `#[cfg]` attributes.
fn strip_cfg(stmt: &Stmt) -> Stmt {
    let mut stmt = stmt.clone();
    let attrs = match &mut stmt {
        Stmt::Local(l) => Some(&mut l.attrs),
        Stmt::Macro(m) => Some(&mut m.attrs),
        Stmt::Item(_) => None,
        Stmt::Expr(e, _) => match e {
            Expr::Assign(x) => Some(&mut x.attrs),
            Expr::Async(x) => Some(&mut x.attrs),
            Expr::Await(x) => Some(&mut x.attrs),
            Expr::Block(x) => Some(&mut x.attrs),
            Expr::Call(x) => Some(&mut x.attrs),
            Expr::ForLoop(x) => Some(&mut x.attrs),
            Expr::If(x) => Some(&mut x.attrs),
            Expr::Loop(x) => Some(&mut x.attrs),
            Expr::Macro(x) => Some(&mut x.attrs),
            Expr::Match(x) => Some(&mut x.attrs),
            Expr::MethodCall(x) => Some(&mut x.attrs),
            Expr::Return(x) => Some(&mut x.attrs),
            Expr::Try(x) => Some(&mut x.attrs),
            Expr::Unsafe(x) => Some(&mut x.attrs),
            Expr::While(x) => Some(&mut x.attrs),
            _ => None,
        },
    };
    if let Some(attrs) = attrs {
        attrs.retain(|a| !a.path().is_ident("cfg"));
    }
    stmt
}

/// True when the statement holds a `return` of the function it is in (closures, async blocks and nested items
/// return from themselves).
fn has_return(stmt: &Stmt) -> bool {
    struct Finder(bool);
    impl Visit<'_> for Finder {
        fn visit_expr_return(&mut self, _: &syn::ExprReturn) {
            self.0 = true;
        }
        fn visit_expr_closure(&mut self, _: &syn::ExprClosure) {}
        fn visit_expr_async(&mut self, _: &syn::ExprAsync) {}
        fn visit_item(&mut self, _: &syn::Item) {}
    }
    let mut finder = Finder(false);
    finder.visit_stmt(stmt);
    finder.0
}

/// `app.into_make_service()`, `(app)`, `&app` → `app`.
fn peel_served(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(p) => peel_served(&p.expr),
        Expr::Group(g) => peel_served(&g.expr),
        Expr::Reference(r) => peel_served(&r.expr),
        Expr::MethodCall(m) if m.method.to_string().starts_with("into_make_service") => peel_served(&m.receiver),
        _ => expr,
    }
}

/// The segments of a plain path (generic arguments dropped; a leading `::` kept as `"::"`); `None` with a qself.
pub(super) fn path_segments(qself: bool, path: &syn::Path) -> Option<Vec<String>> {
    if qself {
        return None;
    }
    let mut out = Vec::with_capacity(path.segments.len() + 1);
    if path.leading_colon.is_some() {
        out.push("::".to_string());
    }
    out.extend(path.segments.iter().map(|s| s.ident.to_string()));
    Some(out)
}

/// `axum::routing::get` / `axum::routing::method_routing::get` → `get`.
fn routing_fn(path: &str) -> Option<&str> {
    path.strip_prefix("axum::routing::method_routing::")
        .or_else(|| path.strip_prefix("axum::routing::"))
        .filter(|name| !name.contains("::"))
}

/// A resolution in a few words, for skip reasons.
fn describe(res: &Resolution) -> String {
    match res {
        Resolution::Item(i) => format!(
            "resolves to `{}`, {}:{}",
            i.name,
            i.file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
            i.line
        ),
        Resolution::External(p) => format!("resolves to {p}"),
        Resolution::Ambiguous(list) => format!("ambiguous between {} items", list.len()),
        Resolution::Unresolved(UnresolvedReason::Other(text)) => format!("unresolved: {text}"),
        Resolution::Unresolved(reason) => format!("unresolved: {reason:?}"),
    }
}

/// The name an assignment target binds, when it is a router binding.
fn assigned_binding(left: &Expr, env: &Env) -> Option<String> {
    match left {
        Expr::Path(p) if p.qself.is_none() => p.path.get_ident().map(|i| i.to_string()).filter(|n| env.contains_key(n)),
        _ => None,
    }
}

/// The name a `let` pattern binds when it is a single identifier (`r`, `mut r`, `r: Router`).
fn pat_ident(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(p) if p.subpat.is_none() => Some(p.ident.to_string()),
        syn::Pat::Type(t) => pat_ident(&t.pat),
        _ => None,
    }
}

fn str_lit(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
        Expr::Paren(p) => str_lit(&p.expr),
        Expr::Group(g) => str_lit(&g.expr),
        _ => None,
    }
}

/// A word for the kind of expression that touched a router.
fn kind_of(expr: &Expr) -> &'static str {
    match expr {
        Expr::ForLoop(_) | Expr::While(_) | Expr::Loop(_) => "loop",
        Expr::Macro(_) => "macro",
        Expr::Closure(_) => "closure",
        Expr::Match(_) => "match",
        Expr::If(_) => "if expression",
        Expr::MethodCall(_) => "method call whose result is not kept",
        Expr::Call(_) => "call",
        Expr::Block(_) | Expr::Unsafe(_) | Expr::Async(_) => "block",
        _ => "expression",
    }
}

/// True when the tokens name a router binding.
fn mentions(tokens: TokenStream, env: &Env) -> bool {
    !env.is_empty() && any_ident(tokens, &|id| env.contains_key(id))
}

/// True when any identifier in the tokens (at any depth) passes `pred`.
pub(super) fn any_ident(tokens: TokenStream, pred: &dyn Fn(&str) -> bool) -> bool {
    tokens.into_iter().any(|tt| match tt {
        TokenTree::Ident(id) => pred(&id.to_string()),
        TokenTree::Group(g) => any_ident(g.stream(), pred),
        _ => false,
    })
}

fn line_of(span: Span) -> usize {
    span.start().line
}

/// The source text a span covers, whitespace collapsed to single spaces.
pub(super) fn span_text(text: &str, span: Span) -> Option<String> {
    let (start, end) = (span.start(), span.end());
    if start.line == 0 || end.line < start.line || (end.line == start.line && end.column < start.column) {
        return None;
    }
    let lines: Vec<&str> = text.split('\n').skip(start.line - 1).take(end.line - start.line + 1).collect();
    if lines.len() != end.line - start.line + 1 {
        return None;
    }
    let mut out = String::new();
    let last = lines.len() - 1;
    for (i, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let from = if i == 0 { start.column } else { 0 };
        let to = if i == last { end.column } else { chars.len() };
        if from > to || to > chars.len() {
            return None;
        }
        out.extend(&chars[from..to]);
        out.push(' ');
    }
    Some(out.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A long expression cut to its first 60 characters.
fn short(text: &str) -> String {
    if text.chars().count() <= 60 {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(60).collect::<String>())
    }
}
