//! The facts the Rust path resolver needs from one file, read with `syn`.
//!
//! A file is split into scopes: scope 0 is the file itself and every inline `mod x { … }` (at any depth) adds one
//! scope, in source order. Each scope lists its items (name, kind, visibility, 1-based ident line and last line),
//! its `use` declarations as flat entries (groups, renames, `self` in groups and globs expanded, the visibility and
//! a leading `::` kept), its `impl` blocks and the names of its `macro_rules!` macros. `#[cfg(…)]` predicates are
//! kept as text; `#[cfg(test)]`, `#[test]` and everything inside a `#[cfg(test)] mod` are marked test-only.
//!
//! Items of `extern { … }` blocks count as items of their scope. Call sites are read from function bodies (free
//! functions, `impl` methods and trait default methods, closures and nested blocks included) with their full path.
//! A call written inside a macro invocation is not visible: macro tokens are never parsed or guessed into.

use std::collections::BTreeSet;

use syn::spanned::Spanned;
use syn::visit::Visit;

/// The facts of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileFacts {
    /// Scope 0 is the file; inline modules follow in source order.
    pub scopes: Vec<ScopeFacts>,
    /// Call sites inside function bodies, in source order.
    pub calls: Vec<CallFact>,
    /// The syntax error when the file does not parse (then `scopes` holds one empty scope).
    pub error: Option<String>,
}

/// One module body: the file itself or an inline `mod x { … }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeFacts {
    /// The enclosing scope (`None` for the file scope).
    pub parent: Option<usize>,
    /// The inline module's name (empty for the file scope).
    pub name: String,
    /// `#![cfg(…)]` predicates written inside the body.
    pub inner_cfg: Vec<String>,
    /// A `#[path]` attribute on an inline module (its `mod x;` children are then not found by the module rules).
    pub path_attr: Option<String>,
    /// The body is compiled only for tests (`#[cfg(test)] mod`, `#![cfg(test)]`, or inside such a module).
    pub test_only: bool,
    pub items: Vec<ItemFact>,
    pub uses: Vec<UseFact>,
    pub impls: Vec<ImplFact>,
    /// Item-level macro invocations other than `macro_rules!` (their expansion may define names).
    pub item_macros: Vec<(String, usize)>,
}

/// An item's visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Vis {
    Public,
    Crate,
    Super,
    /// `pub(in path)`.
    Restricted,
    /// No visibility, or `pub(self)`.
    Private,
}

/// One item declared in a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFact {
    pub name: String,
    pub kind: FactKind,
    pub vis: Vis,
    /// 1-based line of the item's name.
    pub line: usize,
    /// 1-based last line of the item.
    pub line_end: usize,
    /// `#[cfg(…)]` predicates other than `test` / `not(test)`, as written.
    pub cfg: Vec<String>,
    /// Compiled only for tests (`#[cfg(test)]`, `#[test]`, or inside a test-only scope).
    pub test_only: bool,
    /// For an enum: the variants under their own `#[cfg(…)]` (other than `test` / `not(test)`).
    pub gated_variants: Vec<String>,
}

/// What an item is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactKind {
    Fn,
    /// A struct; `value` is true for unit and tuple structs (they also name a constructor value).
    Struct {
        value: bool,
    },
    Union,
    Enum {
        variants: Vec<String>,
    },
    Trait {
        items: Vec<AssocFact>,
    },
    Const,
    Static,
    /// A type alias.
    Type,
    Mod(ModBody),
    MacroRules,
    /// `extern crate krate as name;` (`krate` may be `self`).
    ExternCrate {
        krate: String,
    },
}

/// Where a module's body is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModBody {
    /// `mod x { … }`: the scope index in this file.
    Inline(usize),
    /// `mod x;`: a file found by the module rules, unless a `#[path]` attribute says otherwise.
    File { path_attr: Option<String> },
}

/// An associated item of an `impl` block or a trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssocFact {
    pub name: String,
    pub kind: AssocKind,
    pub vis: Vis,
    pub line: usize,
    pub line_end: usize,
    pub cfg: Vec<String>,
    pub test_only: bool,
    /// A function with a body (always true in an `impl`; a trait method with a default body).
    pub has_body: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssocKind {
    Fn,
    Const,
    Type,
}

/// An `impl` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplFact {
    /// The self type's path segments when it is a plain path (`Thing`, `crate::a::Thing<T>` → `[crate, a, Thing]`).
    pub self_ty: Option<Vec<String>>,
    /// The implemented trait's path segments (`None` for an inherent impl).
    pub trait_path: Option<Vec<String>>,
    pub line: usize,
    pub cfg: Vec<String>,
    pub test_only: bool,
    pub items: Vec<AssocFact>,
    /// The impl's type parameters (`impl<T> Trait for T` → `[T]`).
    pub generics: Vec<String>,
}

/// One `use` declaration, flattened into entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseFact {
    pub vis: Vis,
    pub line: usize,
    pub cfg: Vec<String>,
    pub test_only: bool,
    /// The path starts with `::` (an extern crate).
    pub leading_colon: bool,
    pub entries: Vec<UseEntry>,
}

/// One name a `use` tree imports (or one glob).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseEntry {
    /// The full path: for a named import it ends with the imported name, for a glob it is the glob's base.
    pub path: Vec<String>,
    /// The name bound in the scope (the rename after `as`, `_` for an underscore import; `None` for a glob).
    pub binding: Option<String>,
    /// `self` inside a group (`use a::{self}`): imports the module name only (type namespace).
    pub self_only: bool,
}

/// The function a call site is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerFact {
    pub name: String,
    pub line: usize,
    pub line_end: usize,
    /// The `impl` block index (in the scope's `impls`) for a method.
    pub impl_index: Option<usize>,
    /// A trait's default method.
    pub in_trait: bool,
}

/// One call site inside a function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFact {
    /// The scope (module body) the enclosing function is declared in.
    pub scope: usize,
    pub caller: CallerFact,
    /// The called path's segments (`["a", "b", "f"]`), or `[method]` for a method call.
    pub path: Vec<String>,
    pub leading_colon: bool,
    /// 1-based line of the path's first segment (the method name for a method call).
    pub line: usize,
    /// `x.f()`.
    pub is_method: bool,
    /// `<T as Trait>::f()`: needs types.
    pub qualified_self: bool,
    /// Written inside a statement or expression under `#[cfg(…)]` other than `test` (a `#[cfg(test)]` statement's
    /// calls are left out). Like a call in a cfg-gated function, it is still resolved: the condition gates the
    /// call, not the target.
    pub cfg_gated: bool,
    /// The first segment is a local binding, a generic parameter, an item or import declared inside the body, or
    /// `Self` inside an `impl` nested in the body: the module scope does not decide it.
    pub local: bool,
    /// A glob `use` inside the body may shadow the module scope's names.
    pub block_glob: bool,
    /// The enclosing function or its scope is compiled only for tests.
    pub test_only: bool,
    /// A statement-level macro invocation in the body (other than the std expression macros) names the path's
    /// first segment in its input: its expansion may declare an item of that name, shadowing the module scope.
    pub block_macro: bool,
}

/// Reads the facts of one Rust file.
pub fn parse_rust_facts(text: &str) -> FileFacts {
    let file = match syn::parse_file(text) {
        Ok(file) => file,
        Err(e) => {
            return FileFacts {
                scopes: vec![ScopeFacts::default()],
                calls: Vec::new(),
                error: Some(format!("Syntax error: {e}")),
            }
        }
    };
    let mut facts = FileFacts::default();
    let attrs = Attrs::read(&file.attrs);
    facts.scopes.push(ScopeFacts { inner_cfg: attrs.cfg, test_only: attrs.test_only, ..ScopeFacts::default() });
    read_items(&mut facts, 0, &file.items);
    facts
}

/// The attributes the resolver cares about.
#[derive(Debug, Default)]
struct Attrs {
    cfg: Vec<String>,
    test_only: bool,
    path_attr: Option<String>,
}

impl Attrs {
    fn read(attrs: &[syn::Attribute]) -> Attrs {
        let mut out = Attrs::default();
        for attr in attrs {
            let path = attr.path();
            if path.is_ident("cfg") {
                if let syn::Meta::List(list) = &attr.meta {
                    let text = list.tokens.to_string();
                    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                    match compact.as_str() {
                        "test" => out.test_only = true,
                        "not(test)" => {}
                        _ => out.cfg.push(text),
                    }
                }
            } else if path.is_ident("path") {
                if let syn::Meta::NameValue(nv) = &attr.meta {
                    if let syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) = &nv.value {
                        out.path_attr = Some(s.value());
                    }
                }
            } else if path.is_ident("cfg_attr") {
                if let syn::Meta::List(list) = &attr.meta {
                    let text = list.tokens.to_string();
                    if text.split(',').skip(1).any(|part| part.trim_start().starts_with("path")) {
                        out.path_attr = Some(format!("cfg_attr({text})"));
                    }
                }
            } else if path.segments.last().is_some_and(|s| s.ident == "test" || s.ident == "bench") {
                out.test_only = true;
            }
        }
        out
    }
}

fn vis_of(vis: &syn::Visibility) -> Vis {
    match vis {
        syn::Visibility::Public(_) => Vis::Public,
        syn::Visibility::Inherited => Vis::Private,
        syn::Visibility::Restricted(r) => {
            if r.in_token.is_none() && r.path.is_ident("crate") {
                Vis::Crate
            } else if r.in_token.is_none() && r.path.is_ident("super") {
                Vis::Super
            } else if r.path.is_ident("self") {
                Vis::Private
            } else {
                Vis::Restricted
            }
        }
    }
}

fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line
}

fn end_line_of(span: proc_macro2::Span, start: usize) -> usize {
    span.end().line.max(start)
}

/// The plain path of a type (`a::b::C<T>` → `[a, b, C]`); `None` for references, tuples, `dyn` and the like.
fn type_path(ty: &syn::Type) -> Option<Vec<String>> {
    match ty {
        syn::Type::Path(tp) if tp.qself.is_none() => Some(path_segments(&tp.path)),
        syn::Type::Group(g) => type_path(&g.elem),
        syn::Type::Paren(p) => type_path(&p.elem),
        _ => None,
    }
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}

fn read_items(facts: &mut FileFacts, scope: usize, items: &[syn::Item]) {
    let scope_test = facts.scopes[scope].test_only;
    for item in items {
        read_item(facts, scope, scope_test, item);
    }
}

fn item_fact(
    name: String,
    kind: FactKind,
    vis: Vis,
    ident_span: proc_macro2::Span,
    item_span: proc_macro2::Span,
) -> ItemFact {
    let line = line_of(ident_span);
    ItemFact {
        name,
        kind,
        vis,
        line,
        line_end: end_line_of(item_span, line),
        cfg: Vec::new(),
        test_only: false,
        gated_variants: Vec::new(),
    }
}

/// The functions, statics and types an `extern { … }` block declares, as items of `scope`.
fn read_foreign_items(facts: &mut FileFacts, scope: usize, scope_test: bool, block: &syn::ItemForeignMod) {
    let outer = Attrs::read(&block.attrs);
    for fi in &block.items {
        let (name, kind, vis, ident_span, attrs) = match fi {
            syn::ForeignItem::Fn(f) => (f.sig.ident.to_string(), FactKind::Fn, &f.vis, f.sig.ident.span(), &f.attrs),
            syn::ForeignItem::Static(s) => (s.ident.to_string(), FactKind::Static, &s.vis, s.ident.span(), &s.attrs),
            syn::ForeignItem::Type(t) => (t.ident.to_string(), FactKind::Type, &t.vis, t.ident.span(), &t.attrs),
            _ => continue,
        };
        let a = Attrs::read(attrs);
        let mut fact = item_fact(name, kind, vis_of(vis), ident_span, fi.span());
        fact.cfg = outer.cfg.iter().chain(&a.cfg).cloned().collect();
        fact.test_only = scope_test || outer.test_only || a.test_only;
        facts.scopes[scope].items.push(fact);
    }
}

fn read_item(facts: &mut FileFacts, scope: usize, scope_test: bool, item: &syn::Item) {
    let attrs = match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        syn::Item::ForeignMod(block) => return read_foreign_items(facts, scope, scope_test, block),
        _ => return,
    };
    let a = Attrs::read(attrs);
    let test_only = scope_test || a.test_only;
    let span = item.span();
    let mut fact = match item {
        syn::Item::Fn(f) => {
            let fact = item_fact(f.sig.ident.to_string(), FactKind::Fn, vis_of(&f.vis), f.sig.ident.span(), span);
            let caller = CallerFact {
                name: fact.name.clone(),
                line: fact.line,
                line_end: fact.line_end,
                impl_index: None,
                in_trait: false,
            };
            read_body_calls(facts, scope, caller, test_only, &[&f.sig.generics], &f.sig, &f.block);
            fact
        }
        syn::Item::Struct(s) => {
            let value = !matches!(s.fields, syn::Fields::Named(_));
            item_fact(s.ident.to_string(), FactKind::Struct { value }, vis_of(&s.vis), s.ident.span(), span)
        }
        syn::Item::Union(u) => item_fact(u.ident.to_string(), FactKind::Union, vis_of(&u.vis), u.ident.span(), span),
        syn::Item::Enum(e) => {
            let variants = e.variants.iter().map(|v| v.ident.to_string()).collect();
            let mut fact =
                item_fact(e.ident.to_string(), FactKind::Enum { variants }, vis_of(&e.vis), e.ident.span(), span);
            fact.gated_variants = e
                .variants
                .iter()
                .filter(|v| !Attrs::read(&v.attrs).cfg.is_empty())
                .map(|v| v.ident.to_string())
                .collect();
            fact
        }
        syn::Item::Trait(t) => {
            let mut assoc = Vec::new();
            let mut fact = item_fact(
                t.ident.to_string(),
                FactKind::Trait { items: Vec::new() },
                vis_of(&t.vis),
                t.ident.span(),
                span,
            );
            for ti in &t.items {
                let (name, kind, ident_span, attrs, has_body) = match ti {
                    syn::TraitItem::Fn(f) => {
                        (f.sig.ident.to_string(), AssocKind::Fn, f.sig.ident.span(), &f.attrs, f.default.is_some())
                    }
                    syn::TraitItem::Const(c) => {
                        (c.ident.to_string(), AssocKind::Const, c.ident.span(), &c.attrs, false)
                    }
                    syn::TraitItem::Type(t) => (t.ident.to_string(), AssocKind::Type, t.ident.span(), &t.attrs, false),
                    _ => continue,
                };
                let ia = Attrs::read(attrs);
                let line = line_of(ident_span);
                let af = AssocFact {
                    name,
                    kind,
                    vis: Vis::Public,
                    line,
                    line_end: end_line_of(ti.span(), line),
                    cfg: ia.cfg,
                    test_only: test_only || ia.test_only,
                    has_body,
                };
                if let syn::TraitItem::Fn(f) = ti {
                    if let Some(block) = &f.default {
                        let caller = CallerFact {
                            name: af.name.clone(),
                            line: af.line,
                            line_end: af.line_end,
                            impl_index: None,
                            in_trait: true,
                        };
                        read_body_calls(
                            facts,
                            scope,
                            caller,
                            af.test_only,
                            &[&t.generics, &f.sig.generics],
                            &f.sig,
                            block,
                        );
                    }
                }
                assoc.push(af);
            }
            fact.kind = FactKind::Trait { items: assoc };
            fact
        }
        syn::Item::Const(c) => item_fact(c.ident.to_string(), FactKind::Const, vis_of(&c.vis), c.ident.span(), span),
        syn::Item::Static(s) => item_fact(s.ident.to_string(), FactKind::Static, vis_of(&s.vis), s.ident.span(), span),
        syn::Item::Type(t) => item_fact(t.ident.to_string(), FactKind::Type, vis_of(&t.vis), t.ident.span(), span),
        syn::Item::ExternCrate(e) => {
            let (name, ident_span) = match &e.rename {
                Some((_, rename)) => (rename.to_string(), rename.span()),
                None => (e.ident.to_string(), e.ident.span()),
            };
            item_fact(name, FactKind::ExternCrate { krate: e.ident.to_string() }, vis_of(&e.vis), ident_span, span)
        }
        syn::Item::Macro(m) => {
            if m.mac.path.is_ident("macro_rules") {
                match &m.ident {
                    Some(ident) => item_fact(ident.to_string(), FactKind::MacroRules, Vis::Private, ident.span(), span),
                    None => return,
                }
            } else {
                if !test_only {
                    let name = path_segments(&m.mac.path).join("::");
                    facts.scopes[scope].item_macros.push((name, line_of(m.mac.path.span())));
                }
                return;
            }
        }
        syn::Item::Mod(m) => {
            let body = match &m.content {
                Some((_, items)) => {
                    let index = facts.scopes.len();
                    // `syn` keeps an inline module's inner `#![…]` attributes in the same list, so `test_only`
                    // and the item's `cfg` already include them.
                    facts.scopes.push(ScopeFacts {
                        parent: Some(scope),
                        name: m.ident.to_string(),
                        path_attr: a.path_attr.clone(),
                        test_only,
                        ..ScopeFacts::default()
                    });
                    read_items(facts, index, items);
                    ModBody::Inline(index)
                }
                None => ModBody::File { path_attr: a.path_attr.clone() },
            };
            item_fact(m.ident.to_string(), FactKind::Mod(body), vis_of(&m.vis), m.ident.span(), span)
        }
        syn::Item::Impl(i) => {
            let impl_index = facts.scopes[scope].impls.len();
            let trait_path = i.trait_.as_ref().map(|(_, p, _)| path_segments(p));
            let mut fact = ImplFact {
                self_ty: type_path(&i.self_ty),
                trait_path,
                line: line_of(i.impl_token.span),
                cfg: a.cfg.clone(),
                test_only,
                items: Vec::new(),
                generics: i
                    .generics
                    .params
                    .iter()
                    .filter_map(|p| match p {
                        syn::GenericParam::Type(t) => Some(t.ident.to_string()),
                        _ => None,
                    })
                    .collect(),
            };
            for ii in &i.items {
                let (name, kind, vis, ident_span, attrs) = match ii {
                    syn::ImplItem::Fn(f) => {
                        (f.sig.ident.to_string(), AssocKind::Fn, &f.vis, f.sig.ident.span(), &f.attrs)
                    }
                    syn::ImplItem::Const(c) => {
                        (c.ident.to_string(), AssocKind::Const, &c.vis, c.ident.span(), &c.attrs)
                    }
                    syn::ImplItem::Type(t) => (t.ident.to_string(), AssocKind::Type, &t.vis, t.ident.span(), &t.attrs),
                    _ => continue,
                };
                let ia = Attrs::read(attrs);
                let line = line_of(ident_span);
                let af = AssocFact {
                    name,
                    kind,
                    vis: vis_of(vis),
                    line,
                    line_end: end_line_of(ii.span(), line),
                    cfg: ia.cfg,
                    test_only: test_only || ia.test_only,
                    has_body: kind == AssocKind::Fn,
                };
                if let syn::ImplItem::Fn(f) = ii {
                    let caller = CallerFact {
                        name: af.name.clone(),
                        line: af.line,
                        line_end: af.line_end,
                        impl_index: Some(impl_index),
                        in_trait: false,
                    };
                    read_body_calls(
                        facts,
                        scope,
                        caller,
                        af.test_only,
                        &[&i.generics, &f.sig.generics],
                        &f.sig,
                        &f.block,
                    );
                }
                fact.items.push(af);
            }
            facts.scopes[scope].impls.push(fact);
            return;
        }
        syn::Item::Use(u) => {
            let mut entries = Vec::new();
            flatten_use(&u.tree, &mut Vec::new(), &mut entries);
            facts.scopes[scope].uses.push(UseFact {
                vis: vis_of(&u.vis),
                line: line_of(u.use_token.span),
                cfg: a.cfg.clone(),
                test_only,
                leading_colon: u.leading_colon.is_some(),
                entries,
            });
            return;
        }
        _ => return,
    };
    fact.cfg = a.cfg;
    fact.test_only = test_only;
    facts.scopes[scope].items.push(fact);
}

fn flatten_use(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<UseEntry>) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => named_entry(prefix, n.ident.to_string(), None, out),
        syn::UseTree::Rename(r) => named_entry(prefix, r.ident.to_string(), Some(r.rename.to_string()), out),
        syn::UseTree::Glob(_) => out.push(UseEntry { path: prefix.clone(), binding: None, self_only: false }),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                flatten_use(t, prefix, out);
            }
        }
    }
}

fn named_entry(prefix: &[String], name: String, rename: Option<String>, out: &mut Vec<UseEntry>) {
    if name == "self" && !prefix.is_empty() {
        let binding = rename.unwrap_or_else(|| prefix.last().cloned().unwrap_or_default());
        out.push(UseEntry { path: prefix.to_vec(), binding: Some(binding), self_only: true });
    } else {
        let mut path = prefix.to_vec();
        path.push(name.clone());
        out.push(UseEntry { path, binding: Some(rename.unwrap_or(name)), self_only: false });
    }
}

/// Reads the call sites of one function body.
fn read_body_calls(
    facts: &mut FileFacts,
    scope: usize,
    caller: CallerFact,
    test_only: bool,
    generics: &[&syn::Generics],
    sig: &syn::Signature,
    block: &syn::Block,
) {
    let mut locals = LocalNames::default();
    for g in generics {
        locals.visit_generics(g);
    }
    locals.visit_signature(sig);
    locals.visit_block(block);
    let mut calls = CallCollector { nested_impl: 0, nested_mod: 0, gated: 0, out: Vec::new() };
    calls.visit_block(block);
    for raw in calls.out {
        let first = raw.path.first().cloned().unwrap_or_default();
        // Inside a `mod` declared in the body, only `crate::` and `::` paths mean what they mean outside.
        let local = raw.in_nested_impl && first == "Self"
            || (raw.in_nested_mod && !raw.leading_colon && first != "crate")
            || (!raw.is_method && !raw.leading_colon && locals.names.contains(&first));
        facts.calls.push(CallFact {
            scope,
            caller: caller.clone(),
            path: raw.path,
            leading_colon: raw.leading_colon,
            line: raw.line,
            is_method: raw.is_method,
            qualified_self: raw.qualified_self,
            cfg_gated: raw.cfg_gated,
            local,
            block_glob: locals.block_glob,
            test_only,
            block_macro: !raw.leading_colon && locals.macro_idents.contains(&first),
        });
    }
}

/// Every name a body binds or declares (over-approximated: all patterns anywhere in the body).
#[derive(Default)]
struct LocalNames {
    names: BTreeSet<String>,
    block_glob: bool,
    /// Identifiers in the input of statement-level macros other than the std expression macros.
    macro_idents: BTreeSet<String>,
}

/// The identifiers of a macro's input (groups included) that could name a declared item: an identifier written
/// as a path qualifier (`name::…`), a path tail (`…::name`) or a field or method (`.name`) is a use, not a name.
fn collect_idents(tokens: proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
    use proc_macro2::TokenTree;
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let punct = |i: usize, c: char| matches!(tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == c);
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(ident) => {
                let qualifier = punct(i + 1, ':') && punct(i + 2, ':');
                let tail = i >= 2 && punct(i - 1, ':') && punct(i - 2, ':');
                let member = i >= 1 && punct(i - 1, '.');
                if !(qualifier || tail || member) {
                    out.insert(ident.to_string());
                }
            }
            TokenTree::Group(group) => collect_idents(group.stream(), out),
            _ => {}
        }
    }
}

/// The std macros that expand to an expression or statement and never declare an item (by bare name or through
/// `std::` / `core::` / `alloc::`).
fn is_std_expression_macro(path: &syn::Path) -> bool {
    let segs = path_segments(path);
    let name = match segs.as_slice() {
        [name] => name,
        [krate, name] if matches!(krate.as_str(), "std" | "core" | "alloc") => name,
        _ => return false,
    };
    matches!(
        name.as_str(),
        "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "write"
            | "writeln"
            | "format"
            | "format_args"
            | "panic"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "vec"
            | "dbg"
            | "matches"
    )
}

impl<'ast> Visit<'ast> for LocalNames {
    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        if !is_std_expression_macro(&node.mac.path) {
            collect_idents(node.mac.tokens.clone(), &mut self.macro_idents);
        }
        syn::visit::visit_stmt_macro(self, node);
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        self.names.insert(node.ident.to_string());
        syn::visit::visit_pat_ident(self, node);
    }

    fn visit_generic_param(&mut self, node: &'ast syn::GenericParam) {
        match node {
            syn::GenericParam::Type(t) => {
                self.names.insert(t.ident.to_string());
            }
            syn::GenericParam::Const(c) => {
                self.names.insert(c.ident.to_string());
            }
            syn::GenericParam::Lifetime(_) => {}
        }
        syn::visit::visit_generic_param(self, node);
    }

    fn visit_item(&mut self, node: &'ast syn::Item) {
        let name = match node {
            syn::Item::Const(i) => Some(i.ident.to_string()),
            syn::Item::Enum(i) => Some(i.ident.to_string()),
            syn::Item::ExternCrate(i) => Some(i.rename.as_ref().map_or(&i.ident, |(_, r)| r).to_string()),
            syn::Item::Fn(i) => Some(i.sig.ident.to_string()),
            syn::Item::Macro(i) => {
                if i.ident.is_none() && !is_std_expression_macro(&i.mac.path) {
                    collect_idents(i.mac.tokens.clone(), &mut self.macro_idents);
                }
                i.ident.as_ref().map(|i| i.to_string())
            }
            syn::Item::ForeignMod(block) => {
                for fi in &block.items {
                    match fi {
                        syn::ForeignItem::Fn(f) => self.names.insert(f.sig.ident.to_string()),
                        syn::ForeignItem::Static(s) => self.names.insert(s.ident.to_string()),
                        syn::ForeignItem::Type(t) => self.names.insert(t.ident.to_string()),
                        _ => false,
                    };
                }
                None
            }
            syn::Item::Mod(i) => Some(i.ident.to_string()),
            syn::Item::Static(i) => Some(i.ident.to_string()),
            syn::Item::Struct(i) => Some(i.ident.to_string()),
            syn::Item::Trait(i) => Some(i.ident.to_string()),
            syn::Item::Type(i) => Some(i.ident.to_string()),
            syn::Item::Union(i) => Some(i.ident.to_string()),
            syn::Item::Use(u) => {
                let mut entries = Vec::new();
                flatten_use(&u.tree, &mut Vec::new(), &mut entries);
                for e in entries {
                    match e.binding {
                        Some(b) => {
                            self.names.insert(b);
                        }
                        None => self.block_glob = true,
                    }
                }
                None
            }
            _ => None,
        };
        if let Some(name) = name {
            self.names.insert(name);
        }
        syn::visit::visit_item(self, node);
    }
}

struct RawCall {
    path: Vec<String>,
    leading_colon: bool,
    line: usize,
    is_method: bool,
    qualified_self: bool,
    in_nested_impl: bool,
    in_nested_mod: bool,
    cfg_gated: bool,
}

struct CallCollector {
    nested_impl: usize,
    /// Depth of `mod` items declared inside the body.
    nested_mod: usize,
    /// Depth of statements and expressions under a `#[cfg(…)]` other than `test` / `not(test)`.
    gated: usize,
    out: Vec<RawCall>,
}

impl CallCollector {
    /// Visits a statement, arm or expression with its attributes: `#[cfg(test)]` code is skipped, other `#[cfg]`
    /// code marks its calls gated.
    fn with_attrs(&mut self, attrs: &[syn::Attribute], visit: impl FnOnce(&mut Self)) {
        let a = Attrs::read(attrs);
        if a.test_only {
            return;
        }
        let gated = !a.cfg.is_empty();
        self.gated += usize::from(gated);
        visit(self);
        self.gated -= usize::from(gated);
    }
}

/// The outer attributes of the expressions that commonly carry `#[cfg]`.
fn expr_attrs(e: &syn::Expr) -> &[syn::Attribute] {
    match e {
        syn::Expr::Assign(x) => &x.attrs,
        syn::Expr::Async(x) => &x.attrs,
        syn::Expr::Await(x) => &x.attrs,
        syn::Expr::Block(x) => &x.attrs,
        syn::Expr::Call(x) => &x.attrs,
        syn::Expr::Closure(x) => &x.attrs,
        syn::Expr::ForLoop(x) => &x.attrs,
        syn::Expr::If(x) => &x.attrs,
        syn::Expr::Let(x) => &x.attrs,
        syn::Expr::Loop(x) => &x.attrs,
        syn::Expr::Macro(x) => &x.attrs,
        syn::Expr::Match(x) => &x.attrs,
        syn::Expr::MethodCall(x) => &x.attrs,
        syn::Expr::Return(x) => &x.attrs,
        syn::Expr::Struct(x) => &x.attrs,
        syn::Expr::Try(x) => &x.attrs,
        syn::Expr::Unsafe(x) => &x.attrs,
        syn::Expr::While(x) => &x.attrs,
        _ => &[],
    }
}

impl<'ast> Visit<'ast> for CallCollector {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*node.func {
            self.out.push(RawCall {
                path: path_segments(&p.path),
                leading_colon: p.path.leading_colon.is_some(),
                line: line_of(p.path.span()),
                is_method: false,
                qualified_self: p.qself.is_some(),
                in_nested_impl: self.nested_impl > 0,
                in_nested_mod: self.nested_mod > 0,
                cfg_gated: self.gated > 0,
            });
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.out.push(RawCall {
            path: vec![node.method.to_string()],
            leading_colon: false,
            line: line_of(node.method.span()),
            is_method: true,
            qualified_self: false,
            in_nested_impl: self.nested_impl > 0,
            in_nested_mod: self.nested_mod > 0,
            cfg_gated: self.gated > 0,
        });
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        self.with_attrs(expr_attrs(node), |v| syn::visit::visit_expr(v, node));
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.with_attrs(&node.attrs, |v| syn::visit::visit_local(v, node));
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        self.with_attrs(&node.attrs, |v| syn::visit::visit_arm(v, node));
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.with_attrs(&node.attrs, |v| syn::visit::visit_stmt_macro(v, node));
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        self.nested_impl += 1;
        syn::visit::visit_item_impl(self, node);
        self.nested_impl -= 1;
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.nested_impl += 1;
        syn::visit::visit_item_trait(self, node);
        self.nested_impl -= 1;
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        self.nested_mod += 1;
        syn::visit::visit_item_mod(self, node);
        self.nested_mod -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_trees_flatten_with_groups_renames_self_and_globs() {
        let f = parse_rust_facts("pub use ::a::{self, b as c, d::*};\nuse e;\n");
        let u = &f.scopes[0].uses[0];
        assert!(u.leading_colon);
        assert_eq!(u.vis, Vis::Public);
        let entries: Vec<(Vec<String>, Option<String>, bool)> =
            u.entries.iter().map(|e| (e.path.clone(), e.binding.clone(), e.self_only)).collect();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            entries,
            vec![
                (s(&["a"]), Some("a".to_string()), true),
                (s(&["a", "b"]), Some("c".to_string()), false),
                (s(&["a", "d"]), None, false),
            ]
        );
        assert_eq!(f.scopes[0].uses[1].entries[0].binding.as_deref(), Some("e"));
    }

    #[test]
    fn items_at_any_depth_with_cfg_test_and_path_attributes() {
        let text = "mod outer {\n    #[cfg(feature = \"x\")]\n    pub(crate) fn f() {}\n    mod deeper {}\n}\n\
                    #[path = \"p.rs\"]\nmod p;\n#[cfg(test)]\nmod tests {\n    fn helper() {}\n}\n\
                    macro_rules! m { () => {} }\nlazy! { X }\n";
        let f = parse_rust_facts(text);
        assert_eq!(f.scopes.len(), 4);
        let outer = &f.scopes[1];
        assert_eq!((outer.name.as_str(), outer.parent), ("outer", Some(0)));
        assert_eq!(outer.items[0].name, "f");
        assert_eq!(outer.items[0].vis, Vis::Crate);
        assert_eq!(outer.items[0].cfg, vec!["feature = \"x\"".to_string()]);
        assert_eq!((outer.items[0].line, outer.items[0].line_end), (3, 3));
        let root = &f.scopes[0];
        assert_eq!(root.items[1].kind, FactKind::Mod(ModBody::File { path_attr: Some("p.rs".into()) }));
        assert!(root.items[2].test_only && f.scopes[3].test_only && f.scopes[3].items[0].test_only);
        assert_eq!(root.items[3].kind, FactKind::MacroRules);
        assert_eq!(root.item_macros, vec![("lazy".to_string(), 13)]);
    }

    #[test]
    fn calls_in_bodies_closures_and_methods_but_not_in_macros() {
        let text = "impl S {\n    fn m(&self) {\n        let c = || helper();\n        self.go();\n        \
                    vec![hidden()];\n        Self::new();\n    }\n}\n";
        let f = parse_rust_facts(text);
        let calls: Vec<(String, usize, bool)> =
            f.calls.iter().map(|c| (c.path.join("::"), c.line, c.is_method)).collect();
        assert_eq!(
            calls,
            vec![("helper".to_string(), 3, false), ("go".to_string(), 4, true), ("Self::new".to_string(), 6, false)]
        );
        assert_eq!(f.calls[0].caller.impl_index, Some(0));
        assert_eq!(f.scopes[0].impls[0].self_ty, Some(vec!["S".to_string()]));
    }
}
