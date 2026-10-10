//! Name lookup: module members, the scope of a module, and paths walked segment by segment.

use super::prelude::{primitive, std_prelude};
use super::{Body, Detail, Import, ItemKind, ItemRef, ModuleId, Ns, Resolution, RustIndex, UnresolvedReason};
use crate::analysis::crate_graph::TargetKind;
use crate::analysis::rust_facts::Vis;

/// What a name or path denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Def {
    Module(ModuleId),
    Item(usize),
    /// An enum variant: the enum item and the variant name.
    Variant(usize, String),
    /// A path into a crate outside the workspace sources.
    External(Vec<String>),
}

/// One binding found for a name.
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub def: Def,
    /// Reached through a `#[cfg(…)]`-gated item, import or module.
    pub gated: bool,
    /// The binding's visibility in the module it was found in.
    pub vis: Vis,
}

#[derive(Debug, Clone)]
pub(crate) enum Look {
    Found(Found),
    Ambiguous(Vec<Found>),
    Fail(UnresolvedReason),
    /// Nothing in the workspace provides the name, but glob imports of external paths (these bases) may.
    ExternalGlob(Vec<Vec<String>>),
    /// The lookup is already in progress further up (glob imports may import each other): this route provides
    /// nothing. Unlike `NotFound`, it does not make a glob base uncertain.
    Cycle,
}

type MemberKey = (ModuleId, String, Ns);

/// The member lookups in progress during one query (a cycle guard) and the finished ones (a memo: diamond-shaped
/// glob graphs would otherwise be walked once per route).
#[derive(Default)]
pub(crate) struct Stack {
    frames: Vec<MemberKey>,
    /// The lowest frame a cycle cut referred to while the current frame was computed.
    min_hit: usize,
    memo: std::collections::HashMap<MemberKey, Look>,
}

fn not_found() -> Look {
    Look::Fail(UnresolvedReason::NotFound)
}

/// One answer from a list of distinct candidates: unique → found; several → `Cfg` when any is gated (cfg twins),
/// else ambiguous.
fn pick(mut found: Vec<Found>) -> Look {
    let mut distinct: Vec<Found> = Vec::new();
    for f in found.drain(..) {
        match distinct.iter_mut().find(|d| d.def == f.def) {
            // The same definition through two routes: gated only if every route is.
            Some(d) => d.gated &= f.gated,
            None => distinct.push(f),
        }
    }
    match distinct.len() {
        0 => not_found(),
        1 => Look::Found(distinct.pop().unwrap_or_else(|| unreachable!())),
        _ if distinct.iter().any(|f| f.gated) => Look::Fail(UnresolvedReason::Cfg),
        _ => Look::Ambiguous(distinct),
    }
}

impl RustIndex {
    /// Resolves a path from a module scope in one name space (for the last segment).
    pub(crate) fn resolve_ns(&self, scope: ModuleId, path: &[String], leading_colon: bool, ns: Ns) -> Resolution {
        let look = self.resolve_path_from(scope, path, leading_colon, ns);
        self.to_resolution(look)
    }

    /// Resolves the path of a `use` declaration (edition 2015: from the crate root) in one name space.
    pub(crate) fn resolve_use_ns(&self, scope: ModuleId, path: &[String], leading_colon: bool, ns: Ns) -> Resolution {
        let look = self.path_from(scope, path, leading_colon, ns, true, &mut Stack::default());
        self.to_resolution(look)
    }

    pub(crate) fn resolve_path_from(&self, scope: ModuleId, path: &[String], leading_colon: bool, ns: Ns) -> Look {
        self.path_from(scope, path, leading_colon, ns, false, &mut Stack::default())
    }

    fn module(&self, m: ModuleId) -> &super::ModuleData {
        &self.modules[m.0 as usize]
    }

    /// `import` is true for the path of a `use` declaration (or a glob's base).
    fn path_from(
        &self,
        scope: ModuleId,
        path: &[String],
        leading_colon: bool,
        ns: Ns,
        import: bool,
        stack: &mut Stack,
    ) -> Look {
        let Some(first) = path.first() else { return Look::Fail(UnresolvedReason::Other("empty path".into())) };
        let krate = self.module(scope).krate;
        // Edition 2015: `use` paths and `::name` start at the crate root.
        let root_2015 = self.crates[krate].root.filter(|_| self.crates[krate].edition < 2018);
        let first_ns = if path.len() == 1 { ns } else { Ns::Type };
        let (start, rest) = if leading_colon {
            let found = match root_2015 {
                Some(root) => self.scope_name(root, first, first_ns, true, stack),
                None => match self.extern_crate(krate, first) {
                    Some((def, gated)) => Look::Found(Found { def, gated, vis: Vis::Public }),
                    None => not_found(),
                },
            };
            match found {
                Look::Found(f) => (f, &path[1..]),
                other => return other,
            }
        } else {
            match first.as_str() {
                "crate" => match self.crates[krate].root {
                    Some(root) => (Found { def: Def::Module(root), gated: false, vis: Vis::Public }, &path[1..]),
                    None => return not_found(),
                },
                "self" => (Found { def: Def::Module(scope), gated: false, vis: Vis::Public }, &path[1..]),
                "super" => {
                    let mut m = scope;
                    let mut n = 0;
                    while path.get(n).is_some_and(|s| s == "super") {
                        match self.module(m).parent {
                            Some(p) => m = p,
                            None => return not_found(),
                        }
                        n += 1;
                    }
                    (Found { def: Def::Module(m), gated: false, vis: Vis::Public }, &path[n..])
                }
                "Self" => return Look::Fail(UnresolvedReason::Other("Self outside an impl".into())),
                name => {
                    let base = match root_2015 {
                        Some(root) if import => root,
                        _ => scope,
                    };
                    match self.scope_name(base, name, first_ns, import, stack) {
                        Look::Found(f) => (f, &path[1..]),
                        other => return other,
                    }
                }
            }
        };
        self.walk(start, rest, ns, stack)
    }

    /// The crate root or external path an extern-prelude name denotes for crate `krate`, and whether it exists
    /// only under a condition: the crate's dependencies, `std`/`core`/`alloc`, then the `extern crate` items of the
    /// crate root (they join the extern prelude).
    fn extern_crate(&self, krate: usize, name: &str) -> Option<(Def, bool)> {
        if let Some(found) = self.extern_dep(krate, name) {
            return Some(found);
        }
        let root = self.crates[krate].root?;
        let mut found = Vec::new();
        for &id in self.module(root).names.get(name)? {
            let item = &self.items[id];
            let def = match &item.detail {
                Detail::ExternCrate(k) if k == "self" => Some((Def::Module(root), false)),
                Detail::ExternCrate(k) => self.extern_dep(krate, k),
                _ => None,
            };
            if let Some((def, gated)) = def {
                found.push(Found { def, gated: gated || item.gated, vis: Vis::Public });
            }
        }
        match pick(found) {
            Look::Found(f) => Some((f.def, f.gated)),
            // Two `extern crate … as name` twins: the name exists, which crate it is depends on the cfg.
            Look::Fail(UnresolvedReason::Cfg) | Look::Ambiguous(_) => {
                Some((Def::External(vec![name.to_string()]), true))
            }
            _ => None,
        }
    }

    /// An extern name from the dependency list or the always-present std crates.
    fn extern_dep(&self, krate: usize, name: &str) -> Option<(Def, bool)> {
        let c = &self.crates[krate];
        let gated = c.gated_deps.contains(name);
        if let Some(&dep) = c.deps.get(name) {
            return Some(match self.crates[dep].root {
                Some(root) => (Def::Module(root), gated),
                // A workspace crate whose root file could not be read: nothing exact is known about it.
                None => (Def::External(vec![self.crates[dep].name.clone()]), gated),
            });
        }
        if let Some(crate_name) = c.externals.get(name) {
            return Some((Def::External(vec![crate_name.clone()]), gated));
        }
        if gated {
            // One name for different crates under different conditions.
            return Some((Def::External(vec![name.to_string()]), true));
        }
        match name {
            "std" | "core" | "alloc" => Some((Def::External(vec![name.to_string()]), false)),
            "proc_macro" if c.kind == TargetKind::ProcMacro => Some((Def::External(vec![name.to_string()]), false)),
            _ => None,
        }
    }

    /// The first segment of a relative path: the module's own scope, then the extern prelude, the std prelude and
    /// the primitive types.
    ///
    /// When only glob imports of external paths could provide the name, a workspace crate of the same name is exact
    /// in a `use` path (rustc rejects a glob-vs-outer-scope ambiguity during import resolution, E0659), but not in
    /// an expression path, where a glob would silently shadow the extern prelude.
    fn scope_name(&self, m: ModuleId, name: &str, ns: Ns, import: bool, stack: &mut Stack) -> Look {
        let krate = self.module(m).krate;
        let edition = self.crates[krate].edition;
        let fallback = |ns: Ns| -> Option<(Def, bool)> {
            if ns == Ns::Type {
                if let Some(found) = self.extern_crate(krate, name) {
                    return Some(found);
                }
            }
            if let Some(path) = std_prelude(name, ns, edition) {
                return Some((Def::External(path.split("::").map(str::to_string).collect()), false));
            }
            if ns == Ns::Type && primitive(name) {
                return Some((Def::External(vec!["std".into(), "primitive".into(), name.into()]), false));
            }
            None
        };
        let found = |(def, gated): (Def, bool)| Look::Found(Found { def, gated, vis: Vis::Public });
        match self.member(m, name, ns, stack) {
            look @ (Look::Fail(UnresolvedReason::NotFound) | Look::Cycle) => fallback(ns).map_or(look, found),
            // A name item macros might define still resolves when the std or extern prelude gives an external
            // path. A workspace crate of that name is exact only in a `use` path (a macro-made item of the same
            // name would be ambiguous there, E0659); in an expression path the macro's item would shadow it.
            Look::Fail(UnresolvedReason::Macro) => match fallback(ns) {
                Some(f @ (Def::External(_), _)) => found(f),
                Some(f) if import => found(f),
                _ => Look::Fail(UnresolvedReason::Macro),
            },
            Look::ExternalGlob(bases) => match fallback(ns) {
                // A glob may shadow the extern prelude: only an external answer stays exact either way.
                Some((Def::Module(_), _)) if !import => Look::Fail(UnresolvedReason::Glob),
                Some(f) => found(f),
                None => external_from_globs(&bases, name),
            },
            other => other,
        }
    }

    /// A name as a member of module `m`: declared items and explicit imports, else glob imports.
    pub(crate) fn member(&self, m: ModuleId, name: &str, ns: Ns, stack: &mut Stack) -> Look {
        match &self.module(m).body {
            Body::Loaded { .. } => {}
            Body::PathAttr => return Look::Fail(UnresolvedReason::PathAttr),
            Body::Missing(reason) => return Look::Fail(UnresolvedReason::Other(reason.clone())),
        }
        let key = (m, name.to_string(), ns);
        if let Some(look) = stack.memo.get(&key) {
            return look.clone();
        }
        if let Some(pos) = stack.frames.iter().position(|k| *k == key) {
            // A cycle (glob imports may import each other): this route provides nothing.
            stack.min_hit = stack.min_hit.min(pos);
            return Look::Cycle;
        }
        let depth = stack.frames.len();
        let saved = std::mem::replace(&mut stack.min_hit, usize::MAX);
        stack.frames.push(key);
        let look = self.member_uncached(m, name, ns, stack);
        let key = stack.frames.pop().unwrap_or_else(|| unreachable!());
        if stack.min_hit >= depth {
            // No cycle cut reached below this frame: the answer does not depend on the lookups in progress.
            stack.memo.insert(key, look.clone());
            stack.min_hit = saved;
        } else {
            stack.min_hit = saved.min(stack.min_hit);
        }
        look
    }

    fn member_uncached(&self, m: ModuleId, name: &str, ns: Ns, stack: &mut Stack) -> Look {
        let md = self.module(m);
        let mut found = Vec::new();
        if let Some(ids) = md.names.get(name) {
            for &id in ids {
                let item = &self.items[id];
                if !item.in_ns(ns) {
                    continue;
                }
                let (def, dep_gated) = match &item.detail {
                    Detail::Mod(child) => (Def::Module(*child), false),
                    Detail::ExternCrate(k) if k == "self" => match self.crates[md.krate].root {
                        Some(root) => (Def::Module(root), false),
                        None => continue,
                    },
                    Detail::ExternCrate(k) => match self.extern_dep(md.krate, k) {
                        Some(found) => found,
                        None => continue,
                    },
                    _ => (Def::Item(id), false),
                };
                found.push(Found { def, gated: item.gated || dep_gated, vis: item.vis });
            }
        }
        let mut uncertain: Option<Look> = None;
        for imp in md.imports.iter().filter(|i| i.binding.as_deref() == Some(name)) {
            if imp.self_only && ns != Ns::Type {
                continue;
            }
            match self.import_target(m, imp, ns, stack) {
                Look::Found(mut f) => {
                    f.gated |= imp.gated;
                    f.vis = imp.vis;
                    found.push(f);
                }
                Look::Fail(UnresolvedReason::NotFound) | Look::Cycle => {}
                Look::ExternalGlob(bases) => {
                    if bases.len() == 1 {
                        let mut p = bases[0].clone();
                        p.push(imp.path.last().cloned().unwrap_or_default());
                        found.push(Found { def: Def::External(p), gated: imp.gated, vis: imp.vis });
                    } else {
                        uncertain.get_or_insert(Look::Fail(UnresolvedReason::Glob));
                    }
                }
                other => {
                    uncertain.get_or_insert(other);
                }
            }
        }
        if !found.is_empty() {
            return pick(found);
        }
        if let Some(look) = uncertain {
            return look;
        }
        if md.has_item_macros {
            // An item-level macro invocation may define the name explicitly, shadowing any glob.
            return Look::Fail(UnresolvedReason::Macro);
        }
        self.glob_member(m, name, ns, stack)
    }

    /// What an import's path denotes in one name space.
    fn import_target(&self, m: ModuleId, imp: &Import, ns: Ns, stack: &mut Stack) -> Look {
        self.path_from(m, &imp.path, imp.leading_colon, ns, true, stack)
    }

    /// A name provided by the glob imports of module `m`.
    fn glob_member(&self, m: ModuleId, name: &str, ns: Ns, stack: &mut Stack) -> Look {
        let md = self.module(m);
        let mut found = Vec::new();
        let mut uncertain: Option<UnresolvedReason> = None;
        let mut external: Vec<Vec<String>> = Vec::new();
        for g in &md.globs {
            let base = match self.path_from(m, &g.path, g.leading_colon, Ns::Type, true, stack) {
                Look::Found(b) => b,
                // The base needs the name being looked up: this glob cannot provide it.
                Look::Cycle => continue,
                // An unknown base may provide any name.
                Look::Fail(UnresolvedReason::NotFound) => {
                    uncertain.get_or_insert(UnresolvedReason::Glob);
                    continue;
                }
                Look::Fail(r) => {
                    uncertain.get_or_insert(r);
                    continue;
                }
                Look::Ambiguous(_) | Look::ExternalGlob(_) => {
                    uncertain.get_or_insert(UnresolvedReason::Glob);
                    continue;
                }
            };
            let gated = base.gated || g.gated;
            match base.def {
                Def::Module(gm) => {
                    let candidates = match self.member(gm, name, ns, stack) {
                        Look::Found(f) => vec![f],
                        Look::Ambiguous(list) => list,
                        Look::Fail(UnresolvedReason::NotFound) | Look::Cycle => Vec::new(),
                        Look::Fail(r) => {
                            uncertain.get_or_insert(r);
                            Vec::new()
                        }
                        Look::ExternalGlob(bases) => {
                            external.extend(bases);
                            Vec::new()
                        }
                    };
                    for mut f in candidates {
                        match self.visible(f.vis, gm, m) {
                            Some(true) => {}
                            Some(false) => continue,
                            None => {
                                uncertain.get_or_insert(UnresolvedReason::Glob);
                                continue;
                            }
                        }
                        f.gated |= gated;
                        f.vis = g.vis;
                        found.push(f);
                    }
                }
                Def::Item(id) => {
                    if let Detail::Enum(variants) = &self.items[id].detail {
                        if let Some((_, own_cfg)) = variants.iter().find(|(v, _)| v == name).filter(|_| ns != Ns::Macro)
                        {
                            found.push(Found {
                                def: Def::Variant(id, name.to_string()),
                                gated: gated || *own_cfg,
                                vis: g.vis,
                            });
                        }
                    }
                }
                Def::External(p) => external.push(p),
                Def::Variant(..) => {}
            }
        }
        let look = pick(found);
        match look {
            Look::Found(_) if uncertain.is_some() => Look::Fail(uncertain.unwrap_or(UnresolvedReason::Glob)),
            // An external glob cannot provide the name too: two globs giving different items for a used name is an
            // error (E0659), so in code that compiles the one workspace provider is the answer.
            Look::Found(_) | Look::Ambiguous(_) => look,
            Look::Fail(UnresolvedReason::Cfg) => look,
            _ => match uncertain {
                Some(r) => Look::Fail(r),
                None if !external.is_empty() => {
                    external.sort();
                    external.dedup();
                    Look::ExternalGlob(external)
                }
                None if md.macros.contains(name) => Look::Fail(UnresolvedReason::Macro),
                None => not_found(),
            },
        }
    }

    /// Walks the remaining segments of a path from a resolved start.
    fn walk(&self, start: Found, rest: &[String], ns: Ns, stack: &mut Stack) -> Look {
        let mut cur = start;
        for (i, seg) in rest.iter().enumerate() {
            let last = i + 1 == rest.len();
            let seg_ns = if last { ns } else { Ns::Type };
            let next = match &cur.def {
                Def::Module(mm) => match self.member(*mm, seg, seg_ns, stack) {
                    Look::ExternalGlob(bases) => external_from_globs(&bases, seg),
                    other => other,
                },
                Def::External(p) => {
                    let mut p = p.clone();
                    p.push(seg.clone());
                    Look::Found(Found { def: Def::External(p), gated: false, vis: Vis::Public })
                }
                Def::Item(id) => self.assoc(*id, seg, last),
                Def::Variant(..) => not_found(),
            };
            match next {
                Look::Found(f) => cur = Found { gated: cur.gated || f.gated, ..f },
                other => return other,
            }
        }
        Look::Found(cur)
    }

    /// `Type::name`, `Enum::Variant`, `Trait::name`.
    fn assoc(&self, id: usize, seg: &str, last: bool) -> Look {
        let item = &self.items[id];
        if let Detail::Enum(variants) = &item.detail {
            if let Some((_, own_cfg)) = variants.iter().find(|(v, _)| v == seg) {
                return if last {
                    Look::Found(Found { def: Def::Variant(id, seg.to_string()), gated: *own_cfg, vis: Vis::Public })
                } else {
                    not_found()
                };
            }
        }
        match &item.detail {
            Detail::Struct { .. } | Detail::Enum(_) | Detail::Union => {
                let Some(ids) = self.inherent.get(&id).and_then(|m| m.get(seg)) else {
                    // A trait may provide it: that needs types.
                    return Look::Fail(UnresolvedReason::Method);
                };
                if !last {
                    return Look::Fail(UnresolvedReason::Other("path through an associated item".into()));
                }
                pick(
                    ids.iter()
                        .map(|&a| Found { def: Def::Item(a), gated: self.items[a].gated, vis: self.items[a].vis })
                        .collect(),
                )
            }
            Detail::Trait(names) if last && names.iter().any(|(n, _)| n == seg) => {
                // Two cfg twins of one trait item count as gated, like any twin.
                let gated = names.iter().any(|(n, own_cfg)| n == seg && *own_cfg);
                Look::Found(Found { def: Def::Item(id), gated, vis: Vis::Public })
            }
            Detail::Trait(_) => Look::Fail(UnresolvedReason::Method),
            Detail::Type => Look::Fail(UnresolvedReason::Other("path through a type alias".into())),
            _ => not_found(),
        }
    }

    /// Whether a binding with visibility `vis` in module `owner` can be seen from module `viewer`; `None` when
    /// that depends on the path of a `pub(in path)` (not kept), seen from outside `owner`.
    pub(crate) fn visible(&self, vis: Vis, owner: ModuleId, viewer: ModuleId) -> Option<bool> {
        let same_crate = self.module(owner).krate == self.module(viewer).krate;
        Some(match vis {
            Vis::Public => true,
            Vis::Crate => same_crate,
            // The path names an ancestor of `owner`: visible inside `owner` for sure, beyond that it depends.
            Vis::Restricted if same_crate && !self.is_within(viewer, owner) => return None,
            Vis::Restricted => same_crate,
            Vis::Super => match self.module(owner).parent {
                Some(p) => same_crate && self.is_within(viewer, p),
                None => same_crate && self.is_within(viewer, owner),
            },
            Vis::Private => same_crate && self.is_within(viewer, owner),
        })
    }

    /// `m` is `ancestor` or a module inside it.
    fn is_within(&self, mut m: ModuleId, ancestor: ModuleId) -> bool {
        loop {
            if m == ancestor {
                return true;
            }
            match self.module(m).parent {
                Some(p) => m = p,
                None => return false,
            }
        }
    }

    /// The reference a module resolves to: its `mod` declaration, or line 1 of a crate root.
    fn module_ref(&self, m: ModuleId) -> ItemRef {
        let md = self.module(m);
        match md.decl_item {
            Some(id) => self.items[id].r.clone(),
            None => ItemRef {
                file: md.file.clone(),
                name: self.crates[md.krate].name.clone(),
                kind: ItemKind::Mod,
                line: 1,
                line_end: 1,
                module: m,
            },
        }
    }

    fn def_ref(&self, def: &Def) -> Option<ItemRef> {
        match def {
            Def::Module(m) => Some(self.module_ref(*m)),
            Def::Item(id) | Def::Variant(id, _) => Some(self.items[*id].r.clone()),
            Def::External(_) => None,
        }
    }

    pub(crate) fn to_resolution(&self, look: Look) -> Resolution {
        match look {
            Look::Found(f) if f.gated => Resolution::Unresolved(UnresolvedReason::Cfg),
            Look::Found(f) => match &f.def {
                Def::External(p) => Resolution::External(p.join("::")),
                def => self.def_ref(def).map_or(Resolution::Unresolved(UnresolvedReason::NotFound), Resolution::Item),
            },
            Look::Ambiguous(list) if list.iter().any(|f| f.gated) => Resolution::Unresolved(UnresolvedReason::Cfg),
            Look::Ambiguous(list) => {
                let mut refs: Vec<ItemRef> = list.iter().filter_map(|f| self.def_ref(&f.def)).collect();
                refs.sort();
                refs.dedup();
                Resolution::Ambiguous(refs)
            }
            Look::Fail(r) => Resolution::Unresolved(r),
            Look::ExternalGlob(_) => Resolution::Unresolved(UnresolvedReason::Glob),
            Look::Cycle => Resolution::Unresolved(UnresolvedReason::NotFound),
        }
    }
}

/// A name only external glob imports may provide: exact only when there is one such glob.
fn external_from_globs(bases: &[Vec<String>], name: &str) -> Look {
    match bases {
        [base] => {
            let mut p = base.clone();
            p.push(name.to_string());
            Look::Found(Found { def: Def::External(p), gated: false, vis: Vis::Public })
        }
        _ => Look::Fail(UnresolvedReason::Glob),
    }
}
