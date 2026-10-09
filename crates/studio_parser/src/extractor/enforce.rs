use super::types::{
    EnumItem, ExtractedFile, FieldInfo, FunctionItem, ImplItem, ItemVisibility, ParamInfo, StructItem, UseItem,
};
use std::path::Path;

/// Extracts classes, modded classes, methods, fields, enums, and dependencies from an Enforce Script file.
///
/// Comments, strings and preprocessor lines are blanked out first (byte offsets preserved), so brace
/// matching and declaration scanning only ever see code. Item sources are verbatim slices of the file.
pub fn extract_enforce_script_file(file_path: &Path, rel_path: &Path, content: &str) -> ExtractedFile {
    let module_name =
        file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "script".to_string());

    let masked = mask_non_code(content);
    let mut scanner =
        Scanner { src: content, code: &masked, line_starts: line_starts(content), out: Extracted::default() };
    scanner.out.uses = collect_includes(content);
    scanner.scan_scope(0, masked.len(), None);
    let out = scanner.out;

    ExtractedFile {
        file_path: file_path.to_path_buf(),
        relative_path: rel_path.to_path_buf(),
        module_name,
        functions: out.functions,
        structs: out.structs,
        enums: out.enums,
        traits: Vec::new(),
        impls: out.impls,
        uses: out.uses,
        parse_error: None,
        language: super::lang::SourceLang::Enforce,
    }
}

#[derive(Default)]
struct Extracted {
    functions: Vec<FunctionItem>,
    structs: Vec<StructItem>,
    enums: Vec<EnumItem>,
    impls: Vec<ImplItem>,
    uses: Vec<UseItem>,
}

struct ClassCtx {
    fields: Vec<FieldInfo>,
    methods: Vec<FunctionItem>,
}

const MODIFIERS: &[&str] = &[
    "override",
    "proto",
    "native",
    "external",
    "static",
    "private",
    "protected",
    "event",
    "sealed",
    "volatile",
    "notnull",
    "owned",
    "reference",
    "const",
    "ref",
    "autoptr",
    "local",
    "out",
    "inout",
];

const NOT_CALLS: &[&str] = &[
    "if", "for", "foreach", "while", "switch", "return", "new", "delete", "sizeof", "typename", "super", "this",
    "case", "else", "thread", "Class", "array", "set", "map",
];

struct Scanner<'a> {
    src: &'a str,
    code: &'a str,
    line_starts: Vec<usize>,
    out: Extracted,
}

impl Scanner<'_> {
    /// Walks declarations between `start..end`. `class` is set when scanning a class body.
    fn scan_scope(&mut self, start: usize, end: usize, mut class: Option<&mut ClassCtx>) {
        let bytes = self.code.as_bytes();
        let mut header_start = start;
        let mut i = start;
        while i < end {
            match bytes[i] {
                b';' => {
                    if let Some(ctx) = class.as_deref_mut() {
                        self.class_statement(header_start, i, ctx);
                    }
                    i += 1;
                    header_start = i;
                }
                b'{' => {
                    let close = matching_brace(bytes, i, end);
                    self.block_declaration(header_start, i, close, class.as_deref_mut());
                    i = (close + 1).min(end);
                    // `class X {...};` — swallow the trailing semicolon
                    while i < end && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < end && bytes[i] == b';' {
                        i += 1;
                    }
                    header_start = i;
                }
                b'}' => {
                    i += 1;
                    header_start = i;
                }
                // Attribute groups may contain braces: `[Attr(modules: {"A", "B"})]`
                b'[' => i = matching_bracket(bytes, i, end) + 1,
                _ => i += 1,
            }
        }
    }

    /// A `{ ... }` block preceded by a header: class, enum, or function/method.
    fn block_declaration(&mut self, header_start: usize, open: usize, close: usize, class: Option<&mut ClassCtx>) {
        let (attr_end, raw_header) = strip_attributes(self.code, header_start, open);
        let spanned: Vec<(usize, &str)> = token_spans(raw_header).map(|(off, t)| (attr_end + off, t)).collect();
        if spanned.is_empty() {
            return;
        }
        let body_end = (close + 1).min(self.src.len());

        // `class`/`enum` declarations: only `modded`/`sealed` may precede the keyword. Anything
        // earlier is stray text (e.g. a file starting inside a comment) and is ignored.
        let keyword = spanned.iter().rposition(|(_, t)| *t == "class" || *t == "enum");
        if let Some(pos) = keyword.filter(|&p| p + 1 < spanned.len() && class.is_none()) {
            let mut first = pos;
            while first > 0 && matches!(spanned[first - 1].1, "modded" | "sealed") {
                first -= 1;
            }
            let decl_start = spanned[first].0;
            let tokens: Vec<&str> = spanned[first..].iter().map(|(_, t)| *t).collect();
            let kw = pos - first;
            let line = self.line_of(decl_start);
            if tokens[kw] == "class" {
                self.class_declaration(&tokens, kw, decl_start, open, close, line, body_end);
            } else {
                self.enum_declaration(&tokens, kw, decl_start, open, close, line, body_end);
            }
            return;
        }

        let decl_start = spanned[0].0;
        let header = self.code[decl_start..open].trim();
        let line = self.line_of(decl_start);
        if header.contains('(') {
            if let Some(mut f) = self.function(header, decl_start, line, body_end) {
                f.calls = calls_in(&self.code[open + 1..close], &f.name);
                match class {
                    Some(ctx) => ctx.methods.push(f),
                    None => self.out.functions.push(f),
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enum_declaration(
        &mut self,
        tokens: &[&str],
        pos: usize,
        decl_start: usize,
        open: usize,
        close: usize,
        line: usize,
        body_end: usize,
    ) {
        let name = tokens[pos + 1].split(':').next().unwrap_or_default().to_string();
        let variants = enum_variants(&self.code[open + 1..close]);
        self.out.enums.push(EnumItem {
            name,
            visibility: ItemVisibility::Public,
            variants,
            docs: self.docs_above(decl_start),
            line,
            source_code: self.slice(decl_start, body_end),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn class_declaration(
        &mut self,
        tokens: &[&str],
        pos: usize,
        decl_start: usize,
        open: usize,
        close: usize,
        line: usize,
        body_end: usize,
    ) {
        let Some(raw_name) = tokens.get(pos + 1) else { return };
        let (name, inline_base) = match raw_name.split_once(':') {
            Some((n, b)) => (n.to_string(), (!b.is_empty()).then(|| b.to_string())),
            None => (raw_name.to_string(), None),
        };
        let base = inline_base.or_else(|| {
            let rest = &tokens[pos + 2..];
            match rest.first() {
                Some(&":") | Some(&"extends") => rest.get(1).map(|b| b.to_string()),
                Some(t) if t.starts_with(':') => Some(t.trim_start_matches(':').to_string()),
                _ => None,
            }
        });

        let mut derives = Vec::new();
        if tokens[..pos].contains(&"modded") {
            derives.push("modded".to_string());
        }
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            derives.push(base.clone());
            self.out.uses.push(UseItem { path: base, items: vec!["inherit".to_string()] });
        }

        let mut ctx = ClassCtx { fields: Vec::new(), methods: Vec::new() };
        self.scan_scope(open + 1, close, Some(&mut ctx));

        self.out.structs.push(StructItem {
            name: name.clone(),
            visibility: ItemVisibility::Public,
            fields: ctx.fields,
            derives,
            docs: self.docs_above(decl_start),
            line,
            source_code: self.slice(decl_start, body_end),
        });
        if !ctx.methods.is_empty() {
            self.out.impls.push(ImplItem { target_type: name, trait_name: None, methods: ctx.methods, line });
        }
    }

    /// A `;`-terminated statement in a class body: a field or a body-less (proto) method.
    fn class_statement(&mut self, header_start: usize, semi: usize, ctx: &mut ClassCtx) {
        let (decl_start, header) = strip_attributes(self.code, header_start, semi);
        let header = header.trim();
        if header.is_empty() || header.starts_with("typedef") {
            return;
        }
        let line = self.line_of(decl_start);
        let is_method = match (header.find('('), header.find('=')) {
            (Some(paren), Some(eq)) => paren < eq,
            (Some(_), None) => true,
            _ => false,
        };
        if is_method {
            if let Some(f) = self.function(header, decl_start, line, semi + 1) {
                ctx.methods.push(f);
            }
            return;
        }
        let decl = header.split('=').next().unwrap_or(header);
        let decl = decl.split(',').next().unwrap_or(decl);
        let tokens: Vec<&str> = decl.split_whitespace().collect();
        let Some((name, type_tokens)) = tokens.split_last() else { return };
        let type_str = type_tokens.iter().filter(|t| !is_access_modifier(t)).copied().collect::<Vec<_>>().join(" ");
        if type_str.is_empty() || !is_identifier(name.trim_end_matches(']').split('[').next().unwrap_or(name)) {
            return;
        }
        ctx.fields.push(FieldInfo {
            name: name.split('[').next().unwrap_or(name).to_string(),
            type_str,
            visibility: visibility(type_tokens),
        });
    }

    /// Parses `[modifiers] ReturnType Name(params)` from a (possibly multi-line) header.
    fn function(&self, header: &str, decl_start: usize, line: usize, source_end: usize) -> Option<FunctionItem> {
        let paren = header.find('(')?;
        let close = header.rfind(')').filter(|&c| c > paren)?;
        let before: Vec<&str> = header[..paren].split_whitespace().collect();
        let (name, prefix) = before.split_last()?;
        // Constructors/destructors have no return type: `void Foo()`, `Foo()`, `~Foo()`
        let name = name.trim_start_matches('~');
        if !is_identifier(name) || NOT_CALLS.contains(&name) {
            return None;
        }
        let ret: Vec<&str> = prefix.iter().filter(|t| !MODIFIERS.contains(t)).copied().collect();
        let is_override = prefix.contains(&"override");

        let inputs = split_params(&header[paren + 1..close])
            .into_iter()
            .filter_map(|p| {
                let p = p.split('=').next().unwrap_or(&p).trim().to_string();
                let parts: Vec<&str> = p.split_whitespace().collect();
                let (pname, ptype) = parts.split_last()?;
                Some(ParamInfo {
                    name: pname.to_string(),
                    type_str: if ptype.is_empty() { "any".to_string() } else { ptype.join(" ") },
                })
            })
            .collect();

        Some(FunctionItem {
            name: name.to_string(),
            visibility: visibility(prefix),
            is_async: false,
            is_method: true,
            self_param: if prefix.contains(&"static") { None } else { Some("this".to_string()) },
            inputs,
            output: (!ret.is_empty() && ret != ["void"]).then(|| ret.join(" ")),
            calls: Vec::new(),
            docs: {
                let docs = self.docs_above(decl_start);
                if docs.is_empty() && is_override {
                    "Override method".to_string()
                } else {
                    docs
                }
            },
            line,
            source_code: self.slice(decl_start, source_end),
        })
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.src[start..end.min(self.src.len())].replace("\r\n", "\n")
    }

    fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= offset)
    }

    /// Contiguous `//` comment lines directly above the declaration.
    fn docs_above(&self, decl_start: usize) -> String {
        let line_idx = self.line_of(decl_start) - 1;
        let mut docs = Vec::new();
        for idx in (0..line_idx).rev() {
            let start = self.line_starts[idx];
            let end = self.line_starts.get(idx + 1).copied().unwrap_or(self.src.len());
            let text = self.src[start..end].trim();
            match text.strip_prefix("//") {
                Some(rest) => docs.push(rest.trim_start_matches(['!', '/']).trim().to_string()),
                None => break,
            }
        }
        docs.reverse();
        docs.join("\n")
    }
}

fn is_access_modifier(t: &str) -> bool {
    matches!(t, "private" | "protected" | "static" | "const")
}

fn visibility(tokens: &[&str]) -> ItemVisibility {
    if tokens.contains(&"private") {
        ItemVisibility::Private
    } else if tokens.contains(&"protected") {
        ItemVisibility::Crate
    } else {
        ItemVisibility::Public
    }
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Strips leading `[Attribute(...)]` groups; returns the declaration start offset and header text.
fn strip_attributes(code: &str, start: usize, end: usize) -> (usize, &str) {
    let bytes = code.as_bytes();
    let mut i = start;
    loop {
        while i < end && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < end && bytes[i] == b'[' {
            let mut depth = 0;
            while i < end {
                match bytes[i] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        } else {
            break;
        }
    }
    (i, &code[i..end])
}

fn matching_bracket(bytes: &[u8], open: usize, end: usize) -> usize {
    let mut depth = 0;
    for (i, &b) in bytes.iter().enumerate().take(end).skip(open) {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    // Unbalanced: the group runs to the end of the scope.
    end.max(open + 1)
}

/// Whitespace-separated tokens with their byte offsets.
fn token_spans(s: &str) -> impl Iterator<Item = (usize, &str)> {
    s.split_whitespace().map(move |t| (t.as_ptr() as usize - s.as_ptr() as usize, t))
}

fn matching_brace(bytes: &[u8], open: usize, end: usize) -> usize {
    let mut depth = 0;
    for (i, &b) in bytes.iter().enumerate().take(end).skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    // Unbalanced: the group runs to the end of the scope.
    end.max(open + 1)
}

fn split_params(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in params.chars() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn enum_variants(body: &str) -> Vec<String> {
    body.split(',')
        .filter_map(|v| v.split('=').next())
        .map(str::trim)
        .filter(|v| is_identifier(v))
        .map(str::to_string)
        .collect()
}

/// Identifiers followed by `(` in a function body, in first-seen order. `super.X(` also yields `super.X`.
fn calls_in(body: &str, own_name: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut calls: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &body[start..i];
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let preceded_by_new = body[..start].trim_end().ends_with("new");
            let is_super = body[..start].ends_with("super.");
            if j < bytes.len()
                && bytes[j] == b'('
                && !NOT_CALLS.contains(&ident)
                && !preceded_by_new
                && (is_super || ident != own_name)
            {
                if is_super {
                    let sup = format!("super.{}", ident);
                    if !calls.contains(&sup) {
                        calls.push(sup);
                    }
                }
                if !calls.iter().any(|c| c == ident) {
                    calls.push(ident.to_string());
                }
            }
        } else {
            i += 1;
        }
    }
    calls
}

fn collect_includes(content: &str) -> Vec<UseItem> {
    content
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#include"))
        .filter_map(|rest| {
            let start = rest.find('"')?;
            let end = rest[start + 1..].find('"')?;
            Some(UseItem { path: rest[start + 1..start + 1 + end].to_string(), items: vec!["#include".to_string()] })
        })
        .collect()
}

fn line_starts(content: &str) -> Vec<usize> {
    std::iter::once(0).chain(content.match_indices('\n').map(|(i, _)| i + 1)).collect()
}

/// Copy of `src` with comments, string/char literals and preprocessor lines replaced by spaces.
/// Newlines and byte offsets are preserved.
fn mask_non_code(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    let mut i = 0;
    let mut line_start = true;
    while i < bytes.len() {
        let b = bytes[i];
        if line_start && b == b'#' {
            let end = src[i..].find('\n').map_or(bytes.len(), |e| i + e);
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let end = src[i..].find('\n').map_or(bytes.len(), |e| i + e);
            blank(&mut out, i, end);
            i = end;
        } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = src[i + 2..].find("*/").map_or(bytes.len(), |e| i + 2 + e + 2);
            blank(&mut out, i, end);
            i = end;
        } else if b == b'"' || b == b'\'' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b && bytes[j] != b'\n' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            // keep the quotes so `Print("x")` still reads as a call with an argument;
            // an unterminated literal stops before the newline
            let closed = j < bytes.len() && bytes[j] == b;
            blank(&mut out, i + 1, j.min(bytes.len()));
            i = if closed { j + 1 } else { j.min(bytes.len()) };
        } else {
            if b == b'\n' {
                line_start = true;
            } else if !b.is_ascii_whitespace() {
                line_start = false;
            }
            i += 1;
        }
    }
    // Only ASCII bytes were replaced with ASCII spaces inside comments/strings; multi-byte UTF-8
    // sequences there are blanked byte-by-byte, which still yields valid UTF-8 (all spaces).
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(code: &str) -> ExtractedFile {
        extract_enforce_script_file(Path::new("Test.c"), Path::new("Test.c"), code)
    }

    #[test]
    fn test_extract_enforce_script_modded_class() {
        let code = r#"
modded class DayZPlayerImplement
{
    protected int m_CustomCounter;

    override void ShowDeadScreen(bool show, float duration)
    {
        super.ShowDeadScreen(show, duration);
        Print("Player is dead, showing custom UI.");
    }
}

class CustomEntity : Managed
{
    void InitCustom()
    {
        GetGame().GetWorld();
    }
}
"#;
        let file = extract(code);

        assert_eq!(file.structs.len(), 2, "Should parse 2 classes");
        assert_eq!(file.structs[0].name, "DayZPlayerImplement");
        assert!(file.structs[0].derives.contains(&"modded".to_string()));
        assert_eq!(file.structs[0].fields.len(), 1);
        assert_eq!(file.structs[0].fields[0].name, "m_CustomCounter");

        assert_eq!(file.structs[1].name, "CustomEntity");
        assert!(file.structs[1].derives.contains(&"Managed".to_string()));

        assert_eq!(file.impls.len(), 2, "Should have 2 impl blocks for the classes");
        let methods = &file.impls[0].methods;
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "ShowDeadScreen");
        assert_eq!(methods[0].calls, vec!["super.ShowDeadScreen", "ShowDeadScreen", "Print"]);
        assert_eq!(file.impls[1].methods[0].calls, vec!["GetGame", "GetWorld"]);
    }

    #[test]
    fn braces_in_strings_and_comments_do_not_break_scopes() {
        let code = r#"
class A
{
    void One()
    {
        Print("{ not a brace");
        // } also not a brace
        /* { */
    }

    void Two() {}
}

class B {}
"#;
        let file = extract(code);
        let names: Vec<_> = file.structs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        let methods: Vec<_> = file.impls[0].methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(methods, ["One", "Two"]);
    }

    #[test]
    fn multi_line_signatures_custom_return_types_and_modifiers() {
        let code = r#"
[EntityEditorProps(category: "Test")]
sealed class SCR_Thing : ScriptComponent
{
    [Attribute("1", desc: "Speed")]
    protected float m_fSpeed = 1.0;
    ref array<string> m_aNames;

    //! Finds the owner
    IEntity FindOwner(
        notnull IEntity root,
        int depth = 3)
    {
        int count = CountChildren(root);
        return root;
    }

    static proto native void NativeThing(int a);
    protected array<ref SCR_Item> GetItems() { return null; }
}
"#;
        let file = extract(code);
        let class = &file.structs[0];
        assert_eq!(class.name, "SCR_Thing");
        assert_eq!(class.derives, vec!["ScriptComponent"]);
        assert_eq!(class.line, 3, "line of the class keyword, after attributes");
        assert!(class.source_code.starts_with("sealed class SCR_Thing"));
        assert!(class.source_code.ends_with('}'));
        let fields: Vec<_> = class.fields.iter().map(|f| (f.name.as_str(), f.type_str.as_str())).collect();
        assert_eq!(fields, [("m_fSpeed", "float"), ("m_aNames", "ref array<string>")]);

        let methods = &file.impls[0].methods;
        let names: Vec<_> = methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["FindOwner", "NativeThing", "GetItems"], "body statements are not methods");
        let find = &methods[0];
        assert_eq!(find.output.as_deref(), Some("IEntity"));
        assert_eq!(find.inputs.len(), 2);
        assert_eq!(find.inputs[1].name, "depth");
        assert_eq!(find.docs, "Finds the owner");
        assert_eq!(find.calls, vec!["CountChildren"]);
        assert!(find.source_code.contains("return root;"), "full multi-line body is kept");
        assert!(methods[1].self_param.is_none(), "static");
        assert_eq!(methods[2].output.as_deref(), Some("array<ref SCR_Item>"));
        assert_eq!(methods[2].visibility, ItemVisibility::Crate);
    }

    #[test]
    fn enums_includes_and_free_functions() {
        let code = "#include \"scripts/Game/base.c\"\n#ifdef DEBUG\nenum EState\n{\n    IDLE,\n    RUNNING = 2,\n    DONE\n};\n#endif\nvoid Helper()\n{\n    Other();\n}\n";
        let file = extract(code);
        assert_eq!(file.uses[0].path, "scripts/Game/base.c");
        assert_eq!(file.enums[0].name, "EState");
        assert_eq!(file.enums[0].variants, ["IDLE", "RUNNING", "DONE"]);
        assert_eq!(file.functions.len(), 1);
        assert_eq!(file.functions[0].name, "Helper");
        assert_eq!(file.functions[0].calls, ["Other"]);
    }

    #[test]
    fn attributes_with_braces_modded_enum_and_stray_text() {
        let code = r#"modify, this script is generated
*/

[WorkbenchPluginAttribute(name: "Oracle", wbModules: {"ResourceManager", "WorldEditor"})]
class Plugin : WorkbenchPlugin
{
    [Attribute("", UIWidgets.EditBox, desc: "id {x}")]
    protected string m_sId;

    override void Run() { Go(); }
}

modded enum ChimeraMenuPreset
{
    TBD_Screen,
}
"#;
        let file = extract(code);
        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "Plugin");
        assert!(file.structs[0].source_code.starts_with("class Plugin"), "stray text excluded");
        assert_eq!(file.structs[0].fields[0].name, "m_sId");
        assert_eq!(file.impls[0].methods[0].name, "Run");
        assert_eq!(file.enums[0].name, "ChimeraMenuPreset");
        assert_eq!(file.enums[0].variants, ["TBD_Screen"]);
        assert!(file.enums[0].source_code.starts_with("modded enum"));
    }

    #[test]
    fn unbalanced_input_does_not_panic() {
        for code in ["class A\n{\n    void F() {", "enum E\n{", "class B : C { [Attr(", "}}}} class D {}", "class"] {
            extract(code);
        }
        let file = extract("class A\n{\n    int x;\n");
        assert_eq!(file.structs[0].name, "A");
    }

    #[test]
    fn snippets_are_verbatim_file_text() {
        let code = "class A\n{\n    void Run()\n    {\n        Go();\n    }\n}\n";
        let file = extract(code);
        assert!(code.contains(&file.structs[0].source_code));
        assert!(code.contains(&file.impls[0].methods[0].source_code));
        assert_eq!(file.impls[0].methods[0].line, 3);
    }
}
