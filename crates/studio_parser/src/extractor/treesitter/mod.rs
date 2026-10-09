//! Tree-sitter based extraction for every non-Rust, non-Markdown, non-Enforce language.
//!
//! Each language has a query file in `queries/` that tags syntax nodes with a shared set of
//! capture names; this driver turns the captures into the common [`ExtractedFile`] model.
//!
//! | capture | meaning |
//! |---|---|
//! | `@class.def` `@class.name` `@class.base` | class / struct / object / module and its bases |
//! | `@impl.def` `@impl.name` | block adding methods to an existing type (Swift `extension`) |
//! | `@iface.def` `@iface.name` | interface / protocol / trait |
//! | `@enum.def` `@enum.name` `@enum.variant` | enum and its cases |
//! | `@func.def` `@func.name` `@func.params` `@func.param` `@func.return` | function or method |
//! | `@func.receiver` | type a function is attached to outside its body (Go receiver, Lua `T:m`) |
//! | `@field.def` `@field.name` `@field.type` | field / property, attached to the enclosing type |
//! | `@import.def` `@import.path` `@import.item` | import, include, require, using |
//! | `@call.name` | callee name at a call site |
//!
//! Functions nested inside other functions are folded into the outer one (their calls count
//! for it). Methods are functions whose innermost enclosing definition is a type.

mod registry;

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator};

pub use registry::CodeLang;

use super::types::{
    EnumItem, ExtractedFile, FieldInfo, FunctionItem, ImplItem, ItemVisibility, ParamInfo, StructItem,
    TraitItem, UseItem,
};

/// Max lines kept per item snippet, mirroring the UI's preview budget.
const MAX_SNIPPET_LINES: usize = 400;

pub fn extract_code_file(lang: CodeLang, file_path: &Path, rel_path: &Path, content: &str) -> ExtractedFile {
    let mut file = ExtractedFile::empty(file_path, rel_path, super::lang::SourceLang::Code(lang), None);
    if is_minified(content) {
        return file;
    }

    let Some(tree) = PARSERS.with(|parsers| {
        let mut parsers = parsers.borrow_mut();
        let parser = parsers.entry(lang).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&lang.language()).expect("grammar ABI is compatible");
            p
        });
        parser.parse(content, None)
    }) else {
        file.parse_error = Some("tree-sitter could not parse this file".to_string());
        return file;
    };

    let root = tree.root_node();
    if root.has_error() {
        file.parse_error = Some("Syntax errors; extracted what could be recovered".to_string());
    }

    let collected = collect(lang, root, content);
    assemble(lang, &mut file, collected, content);
    file
}

thread_local! {
    static PARSERS: RefCell<HashMap<CodeLang, Parser>> = RefCell::new(HashMap::new());
}

/// Compiled query for a language, built on first use (compiling all of them costs ~0.6s).
pub fn query(lang: CodeLang) -> &'static Query {
    static QUERIES: [OnceLock<Query>; CodeLang::ALL.len()] = [const { OnceLock::new() }; CodeLang::ALL.len()];
    QUERIES[lang as usize].get_or_init(|| {
        Query::new(&lang.language(), lang.query_source())
            .unwrap_or_else(|e| panic!("invalid {} query: {e}", lang.name()))
    })
}

fn is_minified(content: &str) -> bool {
    let sample: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).take(10).collect();
    !sample.is_empty() && sample.iter().map(|l| l.len()).sum::<usize>() / sample.len() > 400
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Class,
    Impl,
    Iface,
    Enum,
    Func,
    Field,
}

#[derive(Debug, Default)]
struct Def {
    range: Range<usize>,
    /// Start of the snippet (includes decorators / export wrappers).
    source_start: usize,
    name: String,
    name_start: usize,
    bases: Vec<String>,
    variants: Vec<String>,
    params: Vec<String>,
    ret: Option<String>,
    receiver: Option<String>,
    field_type: Option<String>,
    docs: String,
}

/// (statement range, path, (offset, item) pairs)
type RawImport = (Range<usize>, String, Vec<(usize, String)>);

#[derive(Default)]
struct Collected {
    defs: HashMap<(Kind, usize, usize), Def>,
    imports: Vec<RawImport>,
    calls: Vec<(usize, String)>,
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

fn collect(lang: CodeLang, root: Node, src: &str) -> Collected {
    let query = query(lang);
    let names = query.capture_names();
    let mut out = Collected::default();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, src.as_bytes());

    while let Some(m) = matches.next() {
        let mut caps: HashMap<&str, Vec<Node>> = HashMap::new();
        for c in m.captures() {
            caps.entry(names[c.index as usize]).or_default().push(c.node);
        }
        let first = |name: &str| caps.get(name).and_then(|v| v.first()).copied();

        if let Some(call) = first("call.name") {
            out.calls.push((call.start_byte(), last_segment(text(call, src)).to_string()));
            continue;
        }
        if let Some(def) = first("import.def") {
            let path = first("import.path").map(|n| text(n, src)).unwrap_or_else(|| text(def, src));
            let path = clean_import_path(path);
            if path.is_empty() {
                continue;
            }
            let items: Vec<(usize, String)> = caps
                .get("import.item")
                .map(|v| v.iter().map(|n| (n.start_byte(), text(*n, src).to_string())).collect())
                .unwrap_or_default();
            match out.imports.iter_mut().find(|(r, p, _)| *r == def.byte_range() && *p == path) {
                Some((_, _, existing)) => {
                    for item in items {
                        if !existing.contains(&item) {
                            existing.push(item);
                        }
                    }
                }
                None => out.imports.push((def.byte_range(), path, items)),
            }
            continue;
        }

        let kinds = [
            ("class", Kind::Class),
            ("impl", Kind::Impl),
            ("iface", Kind::Iface),
            ("enum", Kind::Enum),
            ("func", Kind::Func),
            ("field", Kind::Field),
        ];
        for (prefix, kind) in kinds {
            let Some(def_node) = first(&format!("{prefix}.def")) else { continue };
            let Some(name_node) = first(&format!("{prefix}.name")) else { continue };
            let name = clean_name(text(name_node, src));
            if name.is_empty() {
                continue;
            }
            let range = def_node.byte_range();
            let key = (kind, range.start, range.end);
            let def = out.defs.entry(key).or_insert_with(|| Def {
                source_start: snippet_start(def_node),
                range: range.clone(),
                name,
                name_start: name_node.start_byte(),
                docs: leading_comments(def_node, src),
                ..Default::default()
            });
            let texts = |cap: &str| -> Vec<String> {
                caps.get(cap).map(|v| v.iter().map(|n| text(*n, src).to_string()).collect()).unwrap_or_default()
            };
            for base in texts(&format!("{prefix}.base")) {
                let base = clean_base(&base);
                if !base.is_empty() && !def.bases.contains(&base) {
                    def.bases.push(base);
                }
            }
            for v in texts("enum.variant") {
                let v = clean_name(&v);
                if !v.is_empty() && !def.variants.contains(&v) {
                    def.variants.push(v);
                }
            }
            if def.params.is_empty() {
                if let Some(params) = first("func.params") {
                    def.params = param_texts(params, src);
                } else {
                    def.params = texts("func.param");
                }
            }
            if def.ret.is_none() {
                def.ret = first("func.return").map(|n| clean_type(text(n, src))).filter(|r| !r.is_empty());
            }
            if def.receiver.is_none() {
                def.receiver = first("func.receiver").map(|n| clean_base(text(n, src))).filter(|r| !r.is_empty());
            }
            if def.field_type.is_none() {
                def.field_type = first("field.type").map(|n| clean_type(text(n, src))).filter(|t| !t.is_empty());
            }
            break;
        }
    }
    out
}

/// Turns collected definitions into the extractor model, resolving nesting by byte range.
fn assemble(lang: CodeLang, file: &mut ExtractedFile, collected: Collected, src: &str) {
    let Collected { defs, imports, calls } = collected;
    let mut defs: Vec<(Kind, Def)> = defs.into_iter().map(|((k, _, _), d)| (k, d)).collect();
    // A node tagged as both class and interface/enum (Kotlin `interface`, `enum class`) is the specific kind.
    let specific: Vec<Range<usize>> = defs
        .iter()
        .filter(|(k, _)| matches!(k, Kind::Iface | Kind::Enum))
        .map(|(_, d)| d.range.clone())
        .collect();
    defs.retain(|(k, d)| *k != Kind::Class || !specific.contains(&d.range));
    defs.sort_by(|a, b| a.1.range.start.cmp(&b.1.range.start).then(b.1.range.end.cmp(&a.1.range.end)));

    let line_starts: Vec<usize> =
        std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect();
    let line_of = |offset: usize| line_starts.partition_point(|&s| s <= offset);

    // Innermost definition (other than itself) enclosing each definition.
    let parent_of = |idx: usize| -> Option<usize> {
        let r = &defs[idx].1.range;
        (0..defs.len())
            .filter(|&j| j != idx && matches!(defs[j].0, Kind::Class | Kind::Impl | Kind::Iface | Kind::Enum | Kind::Func))
            .filter(|&j| {
                let o = &defs[j].1.range;
                o.start <= r.start && r.end <= o.end && (o.start, o.end) != (r.start, r.end)
            })
            .min_by_key(|&j| defs[j].1.range.len())
    };
    let parents: Vec<Option<usize>> = (0..defs.len()).map(parent_of).collect();

    let container_name = |j: usize| -> String { defs[j].1.name.clone() };

    // Functions that are kept (top-level or methods); nested functions fold into their parent.
    let mut kept_funcs: Vec<usize> = (0..defs.len())
        .filter(|&i| defs[i].0 == Kind::Func)
        .filter(|&i| parents[i].is_none_or(|p| defs[p].0 != Kind::Func))
        .collect();
    kept_funcs.sort_by_key(|&i| defs[i].1.range.start);

    // Calls → innermost kept function containing the call site.
    let mut calls_by_func: HashMap<usize, Vec<String>> = HashMap::new();
    for (pos, name) in calls {
        let idx = kept_funcs.partition_point(|&i| defs[i].1.range.start <= pos);
        let owner = kept_funcs[..idx].iter().rev().find(|&&i| defs[i].1.range.contains(&pos)).copied();
        if let Some(f) = owner {
            if name != defs[f].1.name {
                let list = calls_by_func.entry(f).or_default();
                if !list.contains(&name) {
                    list.push(name);
                }
            }
        }
    }

    let mut impls: Vec<ImplItem> = Vec::new();
    let add_method = |target: String, line: usize, f: FunctionItem, impls: &mut Vec<ImplItem>| match impls
        .iter_mut()
        .find(|i| i.target_type == target)
    {
        Some(i) => i.methods.push(f),
        None => impls.push(ImplItem { target_type: target, trait_name: None, methods: vec![f], line }),
    };

    let mut struct_fields: HashMap<usize, Vec<FieldInfo>> = HashMap::new();
    let mut iface_methods: HashMap<usize, Vec<String>> = HashMap::new();

    for (i, (kind, def)) in defs.iter().enumerate() {
        match kind {
            Kind::Field => {
                let owner = std::iter::successors(parents[i], |&p| parents[p])
                    .find(|&p| matches!(defs[p].0, Kind::Class | Kind::Impl | Kind::Enum));
                if let Some(owner) = owner {
                    let fields = struct_fields.entry(owner).or_default();
                    if !fields.iter().any(|f| f.name == def.name) {
                        fields.push(FieldInfo {
                            name: def.name.clone(),
                            type_str: def.field_type.clone().unwrap_or_default(),
                            visibility: visibility(lang, def, src),
                        });
                    }
                }
            }
            Kind::Func if kept_funcs.contains(&i) => {
                let mut f = function_item(lang, def, src, line_of(def.range.start), &line_starts);
                f.calls = calls_by_func.remove(&i).unwrap_or_default();
                let parent = parents[i];
                if let Some(receiver) = &def.receiver {
                    f.is_method = true;
                    add_method(receiver.clone(), f.line, f, &mut impls);
                } else if let Some(p) = parent {
                    match defs[p].0 {
                        Kind::Iface => iface_methods.entry(p).or_default().push(f.name.clone()),
                        _ => {
                            f.is_method = true;
                            let line = line_of(defs[p].1.range.start);
                            add_method(container_name(p), line, f, &mut impls);
                        }
                    }
                } else {
                    f.is_method = false;
                    f.self_param = None;
                    file.functions.push(f);
                }
            }
            _ => {}
        }
    }

    for (i, (kind, def)) in defs.iter().enumerate() {
        let line = line_of(def.range.start);
        let source_code = snippet(src, &line_starts, def.source_start, def.range.end);
        match kind {
            Kind::Class => {
                let fields = struct_fields.remove(&i).unwrap_or_default();
                if lang == CodeLang::Python && def.bases.iter().any(|b| PY_ENUM_BASES.contains(&b.as_str())) {
                    file.enums.push(EnumItem {
                        name: def.name.clone(),
                        visibility: visibility(lang, def, src),
                        variants: fields.into_iter().map(|f| f.name).collect(),
                        docs: def.docs.clone(),
                        line,
                        source_code,
                    });
                    continue;
                }
                file.structs.push(StructItem {
                    name: def.name.clone(),
                    visibility: visibility(lang, def, src),
                    fields,
                    derives: def.bases.clone(),
                    docs: def.docs.clone(),
                    line,
                    source_code,
                });
            }
            Kind::Enum => file.enums.push(EnumItem {
                name: def.name.clone(),
                visibility: visibility(lang, def, src),
                variants: def.variants.clone(),
                docs: def.docs.clone(),
                line,
                source_code,
            }),
            Kind::Iface => file.traits.push(TraitItem {
                name: def.name.clone(),
                visibility: visibility(lang, def, src),
                methods: iface_methods.remove(&i).unwrap_or_default(),
                docs: def.docs.clone(),
                line,
                source_code,
            }),
            Kind::Impl | Kind::Func | Kind::Field => {}
        }
    }

    file.impls = impls;
    let mut imports = imports;
    imports.sort_by_key(|(range, _, _)| range.start);
    file.uses = imports
        .into_iter()
        .map(|(_, path, mut items)| {
            items.sort_by_key(|(pos, _)| *pos);
            let mut items: Vec<String> = items.into_iter().map(|(_, item)| item).collect();
            if items.is_empty() {
                items.push(last_segment(&path).to_string());
            }
            if matches!(lang, CodeLang::C | CodeLang::Cpp) {
                items = vec![file_stem(&path).to_string(), "#include".to_string()];
            }
            UseItem { path, items }
        })
        .collect();
}

const PY_ENUM_BASES: &[&str] = &["Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "enum.Enum", "enum.IntEnum"];

fn function_item(lang: CodeLang, def: &Def, src: &str, line: usize, line_starts: &[usize]) -> FunctionItem {
    let mut self_param = None;
    let inputs = def
        .params
        .iter()
        .filter_map(|p| {
            let param = parse_param(p)?;
            if matches!(param.name.as_str(), "self" | "&self" | "&mut self" | "cls" | "this") {
                self_param = Some(param.name);
                return None;
            }
            Some(param)
        })
        .collect();
    let head = &src[def.range.start..def.name_start.max(def.range.start)];
    FunctionItem {
        name: def.name.clone(),
        visibility: visibility(lang, def, src),
        is_async: head.split_whitespace().any(|t| t == "async"),
        is_method: false,
        self_param,
        inputs,
        output: def.ret.clone(),
        calls: Vec::new(),
        docs: def.docs.clone(),
        line,
        source_code: snippet(src, line_starts, def.source_start, def.range.end),
    }
}

fn visibility(lang: CodeLang, def: &Def, src: &str) -> ItemVisibility {
    let head = &src[def.range.start.min(def.name_start)..def.name_start.max(def.range.start)];
    let words: Vec<&str> = head.split(|c: char| !c.is_alphanumeric() && c != '_').collect();
    if words.contains(&"private") || def.name.starts_with('#') {
        return ItemVisibility::Private;
    }
    if words.iter().any(|w| matches!(*w, "protected" | "internal" | "fileprivate")) {
        return ItemVisibility::Crate;
    }
    if lang.private_by_underscore() && def.name.starts_with('_') && !def.name.starts_with("__") {
        return ItemVisibility::Private;
    }
    if lang == CodeLang::Go && def.name.chars().next().is_some_and(|c| c.is_lowercase()) {
        return ItemVisibility::Crate;
    }
    ItemVisibility::Public
}

/// Decorators (Python) and export wrappers are part of the item's source.
fn snippet_start(def: Node) -> usize {
    let mut start = def.start_byte();
    let mut node = def;
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "decorated_definition" | "export_statement") {
            start = parent.start_byte();
            node = parent;
        } else {
            break;
        }
    }
    start
}

/// Whole lines from the line containing `start` through the line containing `end`, verbatim
/// (CRLF normalized to LF so snippets match the editor's line endings).
fn snippet(src: &str, line_starts: &[usize], start: usize, end: usize) -> String {
    let first = line_starts.partition_point(|&s| s <= start).saturating_sub(1);
    let line_start = line_starts[first];
    let last_line = line_starts.partition_point(|&s| s < end.max(start + 1)).saturating_sub(1);
    let last_line = last_line.min(first + MAX_SNIPPET_LINES);
    let line_end = line_starts.get(last_line + 1).map_or(src.len(), |&s| s - 1);
    let text = &src[line_start..line_end.max(line_start)];
    text.trim_end_matches('\r').replace("\r\n", "\n")
}

/// Comment lines directly above a definition, without comment markers.
fn leading_comments(def: Node, src: &str) -> String {
    let mut lines = Vec::new();
    let mut node = def;
    // Decorated definitions: comments sit above the decorator wrapper.
    while let Some(parent) = node.parent().filter(|p| matches!(p.kind(), "decorated_definition" | "export_statement")) {
        node = parent;
    }
    let mut expected_row = node.start_position().row;
    let mut prev = node.prev_sibling();
    while let Some(p) = prev {
        if !p.kind().contains("comment") || p.end_position().row + 1 < expected_row {
            break;
        }
        lines.push(clean_comment(text(p, src)));
        expected_row = p.start_position().row;
        prev = p.prev_sibling();
    }
    lines.reverse();
    lines.join("\n").trim().to_string()
}

fn clean_comment(c: &str) -> String {
    c.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches("/**")
                .trim_start_matches("/*")
                .trim_end_matches("*/")
                .trim_start_matches("///")
                .trim_start_matches("//")
                .trim_start_matches('#')
                .trim_start_matches("--")
                .trim_start_matches('*')
                .trim()
        })
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn param_texts(params: Node, src: &str) -> Vec<String> {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|c| !c.kind().contains("comment"))
        .map(|c| text(c, src).to_string())
        .collect()
}

fn parse_param(p: &str) -> Option<ParamInfo> {
    let p = p.split('=').next().unwrap_or(p).trim().trim_end_matches(',');
    if p.is_empty() {
        return None;
    }
    let p = p
        .split_whitespace()
        .filter(|w| !matches!(*w, "public" | "private" | "protected" | "readonly" | "final" | "val" | "var" | "const" | "in" | "out" | "ref" | "params" | "override"))
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((name, ty)) = p.split_once(':') {
        let name = name.trim().trim_end_matches('?');
        let name = name.split_whitespace().last().unwrap_or(name);
        return Some(ParamInfo { name: name.to_string(), type_str: ty.trim().to_string() });
    }
    let mut parts: Vec<&str> = p.split_whitespace().collect();
    let name = parts.pop()?.trim_start_matches(['*', '&']);
    let ty = parts.join(" ");
    Some(ParamInfo {
        name: name.to_string(),
        type_str: if ty.is_empty() { "any".to_string() } else { ty },
    })
}

fn clean_name(s: &str) -> String {
    s.trim().trim_matches(['"', '\'', '`']).to_string()
}

fn clean_type(s: &str) -> String {
    s.trim().trim_start_matches("->").trim_start_matches(':').trim().to_string()
}

/// `extends Foo<T>` / `: Bar(1)` / `Baz` → `Foo` / `Bar` / `Baz`.
fn clean_base(s: &str) -> String {
    let s = s.trim();
    let s = ["extends ", "implements ", "with ", ":", "<"]
        .iter()
        .fold(s, |acc, kw| acc.strip_prefix(kw).unwrap_or(acc))
        .trim();
    let end = s.find(['<', '(', '[', '{', ' ', ',']).unwrap_or(s.len());
    s[..end].trim_start_matches('*').trim_start_matches('&').to_string()
}

fn clean_import_path(s: &str) -> String {
    let s = s.trim();
    let s = ["import ", "using ", "use ", "require ", "include "]
        .iter()
        .fold(s, |acc, kw| acc.strip_prefix(kw).unwrap_or(acc));
    s.trim()
        .trim_end_matches(';')
        .trim()
        .trim_matches(['"', '\'', '`', '<', '>'])
        .trim()
        .to_string()
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['.', ':', '/', '\\']).find(|s| !s.is_empty()).unwrap_or(path)
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

#[cfg(test)]
mod tests;
