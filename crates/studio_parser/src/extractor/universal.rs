use std::path::Path;
use super::types::{
    EnumItem, ExtractedFile, FieldInfo, FunctionItem, ItemVisibility, ParamInfo, StructItem, TraitItem,
    UseItem,
};

/// Universal extractor for any and all types of code (Python, JS/TS, C/C++, C#, Go, Java, Shell, PHP, Kotlin, etc.).
pub fn extract_universal_file(file_path: &Path, rel_path: &Path, content: &str) -> ExtractedFile {
    let module_name = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "code".to_string());

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    let mut structs = Vec::new();
    let mut functions = Vec::new();
    let mut enums = Vec::new();
    let mut traits = Vec::new();
    let mut uses = Vec::new();

    let lines: Vec<&str> = content.lines().collect();

    // Guard against minified or data files:
    // If first 10 non-empty lines have avg length > 400, skip AST extraction to prevent bloat
    let non_empty_first: Vec<&str> = lines.iter().copied().filter(|l| !l.trim().is_empty()).take(10).collect();
    if !non_empty_first.is_empty() {
        let avg_len: usize = non_empty_first.iter().map(|l| l.len()).sum::<usize>() / non_empty_first.len();
        if avg_len > 400 {
            return ExtractedFile {
                file_path: file_path.to_path_buf(),
                relative_path: rel_path.to_path_buf(),
                module_name,
                functions: Vec::new(),
                structs: Vec::new(),
                enums: Vec::new(),
                traits: Vec::new(),
                impls: Vec::new(),
                uses: Vec::new(),
                parse_error: None,
            };
        }
    }

    for (line_idx, raw_line) in lines.iter().enumerate() {
        let line_num = line_idx + 1;
        let line = raw_line.trim();

        if line.is_empty()
            || line.starts_with("//")
            || (line.starts_with('#') && !line.starts_with("#include") && !matches!(ext.as_str(), "py" | "sh" | "bash"))
            || line.starts_with("/*")
            || line.starts_with('*')
        {
            continue;
        }

        // 1. Imports / Dependencies across languages
        if let Some(use_item) = parse_universal_import(line, &ext) {
            uses.push(use_item);
            continue;
        }

        // 2. Enums across languages
        if let Some(enum_item) = parse_generic_enum(line, line_num, &lines, &ext) {
            enums.push(enum_item);
            continue;
        }

        // 3. Interfaces across languages
        if let Some(trait_item) = parse_generic_interface(line, line_num, &lines, &ext) {
            traits.push(trait_item);
            continue;
        }

        // 4. Class / Struct Definitions
        if let Some(mut struct_item) = parse_generic_class_or_type(line, raw_line, line_num, &ext) {
            struct_item.source_code = extract_universal_source_block(&lines, line_num, &ext);
            extract_class_fields(&mut struct_item, &ext);
            structs.push(struct_item);
            continue;
        }

        // 5. Function / Method Definitions across languages
        if let Some(mut func_item) = parse_generic_function(line, raw_line, line_num, &ext) {
            func_item.source_code = extract_universal_source_block(&lines, line_num, &ext);
            func_item.calls = extract_calls_from_source(&func_item.source_code, &func_item.name);
            functions.push(func_item);
        }
    }

    ExtractedFile {
        file_path: file_path.to_path_buf(),
        relative_path: rel_path.to_path_buf(),
        module_name,
        functions,
        structs,
        enums,
        traits,
        impls: Vec::new(),
        uses,
        parse_error: None,
    }
}

/// Slices multi-line code for functions, classes, and interfaces across languages.
/// Strictly clamped to 200 lines and 16 KB max length to eliminate memory bloat.
pub fn extract_universal_source_block(lines: &[&str], start_line_1_based: usize, ext: &str) -> String {
    if lines.is_empty() || start_line_1_based == 0 || start_line_1_based > lines.len() {
        return String::new();
    }
    let mut start_idx = start_line_1_based - 1;
    while start_idx > 0 {
        let prev = lines[start_idx - 1].trim();
        if prev.starts_with('@') || prev.starts_with("///") || prev.starts_with("//") || prev.starts_with("#[") {
            start_idx -= 1;
        } else {
            break;
        }
    }

    let mut collected = Vec::new();
    let mut total_bytes: usize = 0;
    const MAX_LINES: usize = 200;
    const MAX_BYTES: usize = 16_384;
    const MAX_LINE_LEN: usize = 800;

    let sanitize_line = |l: &str| -> String {
        if l.len() > MAX_LINE_LEN {
            let mut end = 200.min(l.len());
            while !l.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            format!("{}...", &l[..end])
        } else {
            l.to_string()
        }
    };

    if ext == "py" {
        let base_line = lines[start_line_1_based - 1];
        let base_indent = base_line.chars().take_while(|c| c.is_whitespace()).count();

        for i in start_idx..start_line_1_based - 1 {
            let s = sanitize_line(lines[i]);
            total_bytes += s.len() + 1;
            collected.push(s);
        }
        let base_s = sanitize_line(base_line);
        total_bytes += base_s.len() + 1;
        collected.push(base_s);

        let mut trailing_empty = 0;
        for &line in &lines[start_line_1_based..] {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                trailing_empty += 1;
                continue;
            }
            let indent = line.chars().take_while(|c| c.is_whitespace()).count();
            if indent <= base_indent {
                break;
            }
            while trailing_empty > 0 {
                collected.push(String::new());
                trailing_empty -= 1;
            }
            let s = sanitize_line(line);
            total_bytes += s.len() + 1;
            collected.push(s);

            if collected.len() >= MAX_LINES || total_bytes >= MAX_BYTES {
                break;
            }
        }
        return collected.join("\n");
    }

    // Braced languages
    let mut brace_depth: i32 = 0;
    let mut found_open_brace = false;

    for &line in &lines[start_idx..] {
        let s = sanitize_line(line);
        total_bytes += s.len() + 1;
        collected.push(s);

        for ch in line.chars() {
            if ch == '{' {
                brace_depth += 1;
                found_open_brace = true;
            } else if ch == '}' {
                brace_depth -= 1;
            }
        }

        if found_open_brace && brace_depth <= 0 {
            break;
        }
        if !found_open_brace && line.trim().ends_with(';') {
            break;
        }
        if collected.len() >= MAX_LINES || total_bytes >= MAX_BYTES {
            break;
        }
    }

    if !collected.is_empty() {
        collected.join("\n")
    } else {
        sanitize_line(lines[start_line_1_based - 1])
    }
}

/// Robust regex/tokenizer scanner extracting function call targets across all languages.
pub fn extract_calls_from_source(source: &str, self_name: &str) -> Vec<String> {
    let mut calls = Vec::new();
    let mut seen = std::collections::HashSet::new();

    const KEYWORDS: &[&str] = &[
        "if", "else", "for", "while", "do", "switch", "case", "catch", "try",
        "return", "throw", "yield", "await", "sizeof", "typeof", "instanceof",
        "assert", "import", "require", "from", "export", "class", "struct",
        "interface", "enum", "fn", "def", "func", "function", "match", "super",
        "new", "delete", "const", "let", "var", "print", "println", "printf",
        "format", "panic", "len", "range", "make", "append", "type", "with",
        "except", "finally", "lambda", "cast", "decltype", "synchronized",
    ];

    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip string literals
        if chars[i] == '"' || chars[i] == '\'' || chars[i] == '`' {
            let quote = chars[i];
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }

        // Skip comments
        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '/' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            continue;
        }
        if chars[i] == '#' {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }

        // Match identifiers
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();

            // Look ahead for '(' after optional whitespace
            let mut peek = i;
            while peek < len && (chars[peek] == ' ' || chars[peek] == '\t') {
                peek += 1;
            }

            if peek < len && chars[peek] == '(' {
                if ident != self_name && !KEYWORDS.contains(&ident.as_str()) && ident.len() > 1 {
                    if seen.insert(ident.clone()) {
                        calls.push(ident);
                    }
                }
            }
            continue;
        }

        i += 1;
    }

    calls
}

fn parse_universal_import(line: &str, ext: &str) -> Option<UseItem> {
    // 1. C / C++: #include
    if line.starts_with("#include") {
        let target = line
            .trim_start_matches("#include")
            .trim()
            .trim_matches('<')
            .trim_matches('>')
            .trim_matches('"')
            .to_string();
        if !target.is_empty() {
            let file_stem = Path::new(&target)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(&target)
                .to_string();
            return Some(UseItem {
                path: target,
                items: vec![file_stem, "#include".to_string()],
            });
        }
        return None;
    }

    // 2. TypeScript / JavaScript:
    // import { a, b } from './view';
    // import defaultItem from '../path';
    // import * as x from 'path';
    // import './styles.css';
    // const { a, b } = require('./lib');
    if matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") {
        if line.starts_with("import ") || line.starts_with("export * from") || line.contains("require(") {
            let mut path = String::new();
            let mut items = Vec::new();

            if let Some(first_quote) = line.find(['\'', '"', '`']) {
                let after = &line[first_quote + 1..];
                if let Some(close_quote) = after.find(['\'', '"', '`']) {
                    path = after[..close_quote].trim().to_string();
                }
            }

            if line.contains("from ") {
                let before_from = line.split("from ").next().unwrap_or("").trim_start_matches("import ").trim();
                if before_from.starts_with('{') {
                    if let Some(close_brace) = before_from.find('}') {
                        let inside = &before_from[1..close_brace];
                        for item in inside.split(',') {
                            let sym = item.split(" as ").last().unwrap_or("").trim();
                            if !sym.is_empty() && sym.chars().all(|c| c.is_alphanumeric() || c == '_') {
                                items.push(sym.to_string());
                            }
                        }
                    }
                } else if before_from.starts_with("* as ") {
                    let sym = before_from.trim_start_matches("* as ").trim();
                    if !sym.is_empty() {
                        items.push(sym.to_string());
                    }
                } else if !before_from.is_empty() {
                    let sym = before_from.split(',').next().unwrap_or("").trim();
                    if !sym.is_empty() && sym.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        items.push(sym.to_string());
                    }
                }
            }

            if !path.is_empty() {
                if items.is_empty() {
                    let stem = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or(&path).to_string();
                    items.push(stem);
                }
                return Some(UseItem { path, items });
            }
        }
    }

    // 3. Python:
    // from foo.bar import a, b
    // from .utils import helper
    // import module
    if ext == "py" {
        if line.starts_with("from ") {
            let after_from = line.trim_start_matches("from ").trim();
            let mut parts = after_from.split("import ");
            let module_path = parts.next().unwrap_or("").trim().to_string();
            let mut items = Vec::new();
            if let Some(imported_str) = parts.next() {
                for item in imported_str.split(',') {
                    let sym = item.split(" as ").next().unwrap_or("").trim();
                    if !sym.is_empty() && sym.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        items.push(sym.to_string());
                    }
                }
            }
            if !module_path.is_empty() {
                return Some(UseItem { path: module_path, items });
            }
        } else if line.starts_with("import ") {
            let after_import = line.trim_start_matches("import ").trim();
            for mod_entry in after_import.split(',') {
                let mod_name = mod_entry.split(" as ").next().unwrap_or("").trim().to_string();
                if !mod_name.is_empty() {
                    return Some(UseItem { path: mod_name.clone(), items: vec![mod_name] });
                }
            }
        }
    }

    // 4. Go:
    // import "fmt"
    if ext == "go" && line.starts_with("import ") {
        if let Some(first_quote) = line.find('"') {
            let after = &line[first_quote + 1..];
            if let Some(close_quote) = after.find('"') {
                let pkg_path = after[..close_quote].trim().to_string();
                let pkg_name = pkg_path.split('/').last().unwrap_or(&pkg_path).to_string();
                return Some(UseItem {
                    path: pkg_path,
                    items: vec![pkg_name],
                });
            }
        }
    }

    // 5. Java / Kotlin / Scala:
    if matches!(ext, "java" | "kt" | "scala") && line.starts_with("import ") {
        let clean = line.trim_start_matches("import ").trim_end_matches(';').trim();
        let target_sym = clean.split('.').last().unwrap_or(clean).to_string();
        if !clean.is_empty() {
            return Some(UseItem {
                path: clean.to_string(),
                items: vec![target_sym],
            });
        }
    }

    // 6. C#:
    if ext == "cs" && line.starts_with("using ") && line.ends_with(';') {
        let clean = line.trim_start_matches("using ").trim_end_matches(';').trim();
        let target_sym = clean.split('.').last().unwrap_or(clean).to_string();
        if !clean.is_empty() {
            return Some(UseItem {
                path: clean.to_string(),
                items: vec![target_sym],
            });
        }
    }

    // 7. PHP:
    if ext == "php" && line.starts_with("use ") && line.ends_with(';') {
        let clean = line.trim_start_matches("use ").trim_end_matches(';').trim();
        let target_sym = clean.split('\\').last().unwrap_or(clean).to_string();
        if !clean.is_empty() {
            return Some(UseItem {
                path: clean.to_string(),
                items: vec![target_sym],
            });
        }
    }

    None
}

fn parse_generic_enum(line: &str, line_num: usize, lines: &[&str], ext: &str) -> Option<EnumItem> {
    // 1. Python: `class Color(Enum):` or `class Color(IntEnum):`
    if ext == "py"
        && line.starts_with("class ")
        && (line.contains("(Enum)")
            || line.contains("(IntEnum)")
            || line.contains("(str, Enum)")
            || line.contains("(Flag)")
            || line.contains("(IntFlag)"))
    {
        let after_cls = line.trim_start_matches("class ").trim();
        let name_end = after_cls.find('(').unwrap_or(after_cls.len());
        let name = after_cls[..name_end].trim().to_string();
        if !name.is_empty() {
            let source_code = extract_universal_source_block(lines, line_num, ext);
            let mut variants = Vec::new();
            for block_line in source_code.lines().skip(1) {
                let trimmed = block_line.trim();
                if let Some(eq_pos) = trimmed.find('=') {
                    let var_name = trimmed[..eq_pos].trim();
                    if !var_name.is_empty() && var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        variants.push(var_name.to_string());
                    }
                }
            }
            return Some(EnumItem {
                name,
                visibility: ItemVisibility::Public,
                variants,
                docs: String::new(),
                line: line_num,
                source_code,
            });
        }
    }

    // 2. TypeScript / JS / Java / C# / C++: `enum Name ...`
    let is_enum = line.starts_with("enum ")
        || line.starts_with("export enum ")
        || line.starts_with("export const enum ")
        || line.starts_with("const enum ")
        || line.starts_with("public enum ")
        || line.starts_with("private enum ")
        || line.starts_with("protected enum ")
        || line.starts_with("enum class ")
        || line.starts_with("enum struct ")
        || line.starts_with("public enum class ");

    if is_enum {
        let clean = line
            .trim_start_matches("export ")
            .trim_start_matches("public ")
            .trim_start_matches("private ")
            .trim_start_matches("protected ")
            .trim_start_matches("const ")
            .trim_start_matches("enum class ")
            .trim_start_matches("enum struct ")
            .trim_start_matches("enum ")
            .trim();

        let name_end = clean.find(|c: char| c == '{' || c == ':' || c.is_whitespace()).unwrap_or(clean.len());
        let name = clean[..name_end].trim().to_string();

        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let source_code = extract_universal_source_block(lines, line_num, ext);
            let mut variants = Vec::new();

            let inside = if let Some(open) = source_code.find('{') {
                let close = source_code.rfind('}').unwrap_or(source_code.len());
                if close > open {
                    &source_code[open + 1..close]
                } else {
                    &source_code[open + 1..]
                }
            } else {
                ""
            };

            for token in inside.split([',', ';', '\n']) {
                let t = token.trim();
                let var_name = t.split('=').next().unwrap_or("").trim();
                if !var_name.is_empty() && var_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    variants.push(var_name.to_string());
                }
            }

            let is_public = !line.starts_with("private ") && !line.starts_with("protected ");
            return Some(EnumItem {
                name,
                visibility: if is_public { ItemVisibility::Public } else { ItemVisibility::Private },
                variants,
                docs: String::new(),
                line: line_num,
                source_code,
            });
        }
    }

    None
}

fn parse_generic_interface(line: &str, line_num: usize, lines: &[&str], ext: &str) -> Option<TraitItem> {
    let is_interface = line.starts_with("interface ")
        || line.starts_with("export interface ")
        || line.starts_with("export default interface ")
        || line.starts_with("public interface ")
        || line.starts_with("internal interface ")
        || (ext == "go" && line.starts_with("type ") && line.contains("interface"));

    if is_interface {
        let name = if ext == "go" && line.starts_with("type ") {
            line.split_whitespace().nth(1).unwrap_or("").trim().to_string()
        } else {
            let clean = line
                .trim_start_matches("export ")
                .trim_start_matches("default ")
                .trim_start_matches("public ")
                .trim_start_matches("internal ")
                .trim_start_matches("interface ")
                .trim();
            let name_end = clean.find(|c: char| c == '{' || c == '<' || c == ':' || c.is_whitespace()).unwrap_or(clean.len());
            clean[..name_end].trim().to_string()
        };

        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let source_code = extract_universal_source_block(lines, line_num, ext);
            let mut methods = Vec::new();

            for block_line in source_code.lines().skip(1) {
                let trimmed = block_line.trim();
                if trimmed.contains('(') && !trimmed.starts_with("//") && !trimmed.starts_with("/*") {
                    let before_paren = trimmed.split('(').next().unwrap_or("").trim();
                    let method_name = before_paren.split_whitespace().last().unwrap_or("").trim_start_matches('*');
                    if !method_name.is_empty() && method_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        methods.push(method_name.to_string());
                    }
                }
            }

            return Some(TraitItem {
                name,
                visibility: ItemVisibility::Public,
                methods,
                docs: String::new(),
                line: line_num,
                source_code,
            });
        }
    }

    None
}

fn parse_generic_class_or_type(line: &str, raw_line: &str, line_num: usize, ext: &str) -> Option<StructItem> {
    // Go: `type Name struct`
    if ext == "go" && line.starts_with("type ") && line.contains("struct") {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let name = parts[1].to_string();
            return Some(StructItem {
                name,
                visibility: ItemVisibility::Public,
                fields: Vec::new(),
                derives: Vec::new(),
                docs: String::new(),
                line: line_num,
                source_code: raw_line.to_string(),
            });
        }
    }

    // Python / JS / TS / C# / Java / C++: `class Name...` or `struct Name...`
    let is_class = line.starts_with("class ")
        || line.starts_with("export class ")
        || line.starts_with("public class ")
        || line.starts_with("default class ")
        || line.starts_with("abstract class ")
        || line.starts_with("final class ");

    let is_struct = line.starts_with("struct ")
        || line.starts_with("export struct ")
        || line.starts_with("typedef struct ");

    if is_class || is_struct {
        let clean = line
            .trim_start_matches("export ")
            .trim_start_matches("public ")
            .trim_start_matches("default ")
            .trim_start_matches("abstract ")
            .trim_start_matches("final ")
            .trim_start_matches("typedef ")
            .trim();

        let keyword = if clean.starts_with("class ") { "class " } else { "struct " };
        let after_kw = clean.trim_start_matches(keyword).trim();

        let name_end = after_kw
            .find(|c: char| c == '(' || c == ':' || c == '<' || c == '{' || c.is_whitespace())
            .unwrap_or(after_kw.len());
        let name = after_kw[..name_end].trim().to_string();

        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let mut derives = Vec::new();
            if let Some(open_p) = after_kw.find('(') {
                if let Some(close_p) = after_kw[open_p..].find(')') {
                    let base_str = &after_kw[open_p + 1..open_p + close_p];
                    for base in base_str.split(',') {
                        let b = base.trim();
                        if !b.is_empty() {
                            derives.push(b.to_string());
                        }
                    }
                }
            } else if let Some(pos) = after_kw.find("extends") {
                let after_ext = after_kw[pos + 7..].trim();
                let base = after_ext.split(['{', ' ', ',']).next().unwrap_or("").trim();
                if !base.is_empty() {
                    derives.push(base.to_string());
                }
            } else if let Some(pos) = after_kw.find(':') {
                let after_colon = after_kw[pos + 1..].trim();
                for base in after_colon.split(['{', ',']) {
                    let b = base.split_whitespace().last().unwrap_or("").trim();
                    if !b.is_empty() && b.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        derives.push(b.to_string());
                    }
                }
            }

            return Some(StructItem {
                name,
                visibility: ItemVisibility::Public,
                fields: Vec::new(),
                derives,
                docs: String::new(),
                line: line_num,
                source_code: raw_line.to_string(),
            });
        }
    }

    None
}

fn extract_class_fields(item: &mut StructItem, ext: &str) {
    if item.source_code.is_empty() {
        return;
    }

    for line in item.source_code.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.contains('(') {
            continue;
        }

        // Python self.field = ...
        if ext == "py" && trimmed.starts_with("self.") && trimmed.contains('=') {
            let field_name = trimmed.trim_start_matches("self.").split('=').next().unwrap_or("").trim();
            if !field_name.is_empty() && field_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                if !item.fields.iter().any(|f| f.name == field_name) {
                    item.fields.push(FieldInfo {
                        name: field_name.to_string(),
                        type_str: "Any".to_string(),
                        visibility: ItemVisibility::Public,
                    });
                }
            }
        } else if matches!(ext, "ts" | "tsx" | "js") && trimmed.contains(':') && trimmed.ends_with(';') {
            // TypeScript: `field: Type;`
            let mut parts = trimmed.trim_end_matches(';').split(':');
            let name_part = parts.next().unwrap_or("").split_whitespace().last().unwrap_or("").trim_matches('?');
            let type_part = parts.next().unwrap_or("any").trim();
            if !name_part.is_empty() && name_part.chars().all(|c| c.is_alphanumeric() || c == '_') {
                item.fields.push(FieldInfo {
                    name: name_part.to_string(),
                    type_str: type_part.to_string(),
                    visibility: ItemVisibility::Public,
                });
            }
        } else if ext == "go" {
            // Go struct field: `Name Type`
            let tokens: Vec<&str> = trimmed.split_whitespace().collect();
            if tokens.len() >= 2 && tokens[0].chars().all(|c| c.is_alphanumeric() || c == '_') {
                item.fields.push(FieldInfo {
                    name: tokens[0].to_string(),
                    type_str: tokens[1].to_string(),
                    visibility: ItemVisibility::Public,
                });
            }
        }
    }
}

fn parse_params_and_ret(sig_part: &str) -> (Vec<ParamInfo>, Option<String>) {
    let mut inputs = Vec::new();
    let mut output = None;

    if let Some(open) = sig_part.find('(') {
        if let Some(close) = sig_part[open..].find(')') {
            let params_str = &sig_part[open + 1..open + close];
            for p in params_str.split(',') {
                let p_trim = p.trim();
                if p_trim.is_empty() {
                    continue;
                }
                if let Some(colon) = p_trim.find(':') {
                    let name = p_trim[..colon].trim().trim_matches('*').trim_matches('&');
                    let t = p_trim[colon + 1..].split('=').next().unwrap_or("").trim();
                    if !name.is_empty() {
                        inputs.push(ParamInfo {
                            name: name.to_string(),
                            type_str: t.to_string(),
                        });
                    }
                } else {
                    let tokens: Vec<&str> = p_trim.split_whitespace().collect();
                    if tokens.len() >= 2 {
                        inputs.push(ParamInfo {
                            name: tokens.last().unwrap_or(&"").trim_matches('*').trim_matches('&').to_string(),
                            type_str: tokens[..tokens.len() - 1].join(" "),
                        });
                    } else if !p_trim.is_empty() {
                        inputs.push(ParamInfo {
                            name: p_trim.to_string(),
                            type_str: "any".to_string(),
                        });
                    }
                }
            }

            let after_close = sig_part[open + close + 1..].trim();
            if let Some(arrow) = after_close.find("->") {
                let ret = after_close[arrow + 2..].split(['{', ':']).next().unwrap_or("").trim();
                if !ret.is_empty() {
                    output = Some(ret.to_string());
                }
            } else if let Some(colon) = after_close.find(':') {
                let ret = after_close[colon + 1..].split('{').next().unwrap_or("").trim();
                if !ret.is_empty() {
                    output = Some(ret.to_string());
                }
            }
        }
    }

    (inputs, output)
}

fn parse_generic_function(line: &str, raw_line: &str, line_num: usize, ext: &str) -> Option<FunctionItem> {
    if !line.contains('(') {
        return None;
    }

    // Ignore control flow statements
    if line.starts_with("if ") || line.starts_with("if(")
        || line.starts_with("for ") || line.starts_with("for(")
        || line.starts_with("while ") || line.starts_with("while(")
        || line.starts_with("switch ") || line.starts_with("switch(")
        || line.starts_with("catch ") || line.starts_with("catch(")
    {
        return None;
    }

    // Python: `def foo(...)` or `async def foo(...)`
    if ext == "py" {
        if line.starts_with("def ") || line.starts_with("async def ") {
            let is_async = line.starts_with("async def ");
            let clean = if is_async { &line[10..] } else { &line[4..] }.trim();
            let name_end = clean.find('(').unwrap_or(clean.len());
            let name = clean[..name_end].trim().to_string();
            if !name.is_empty() {
                let (inputs, output) = parse_params_and_ret(clean);
                return Some(FunctionItem {
                    name,
                    visibility: if clean.starts_with('_') { ItemVisibility::Private } else { ItemVisibility::Public },
                    is_async,
                    is_method: line.contains("(self") || line.contains("(cls"),
                    self_param: if line.contains("(self") { Some("self".to_string()) } else { None },
                    inputs,
                    output,
                    calls: Vec::new(),
                    docs: String::new(),
                    line: line_num,
                    source_code: raw_line.to_string(),
                });
            }
        }
        return None;
    }

    // Go: `func Foo(...)` or `func (r *Recv) Foo(...)`
    if ext == "go" {
        if line.starts_with("func ") {
            let clean = line[5..].trim();
            if clean.starts_with('(') {
                if let Some(close_recv) = clean.find(')') {
                    let after = clean[close_recv + 1..].trim();
                    let name_end = after.find('(').unwrap_or(after.len());
                    let name = after[..name_end].trim().to_string();
                    if !name.is_empty() {
                        let (inputs, output) = parse_params_and_ret(after);
                        return Some(FunctionItem {
                            name,
                            visibility: ItemVisibility::Public,
                            is_async: false,
                            is_method: true,
                            self_param: Some("receiver".to_string()),
                            inputs,
                            output,
                            calls: Vec::new(),
                            docs: String::new(),
                            line: line_num,
                            source_code: raw_line.to_string(),
                        });
                    }
                }
            } else {
                let name_end = clean.find('(').unwrap_or(clean.len());
                let name = clean[..name_end].trim().to_string();
                if !name.is_empty() {
                    let (inputs, output) = parse_params_and_ret(clean);
                    return Some(FunctionItem {
                        name,
                        visibility: ItemVisibility::Public,
                        is_async: false,
                        is_method: false,
                        self_param: None,
                        inputs,
                        output,
                        calls: Vec::new(),
                        docs: String::new(),
                        line: line_num,
                        source_code: raw_line.to_string(),
                    });
                }
            }
        }
        return None;
    }

    // Shell: `func_name() {` or `function func_name {`
    if ext == "sh" || ext == "bash" {
        if line.contains("()") || line.starts_with("function ") {
            let clean = line.trim_start_matches("function ").trim();
            let name_end = clean.find('(').or_else(|| clean.find('{')).unwrap_or(clean.len());
            let name = clean[..name_end].trim().to_string();
            if !name.is_empty() {
                return Some(FunctionItem {
                    name,
                    visibility: ItemVisibility::Public,
                    is_async: false,
                    is_method: false,
                    self_param: None,
                    inputs: Vec::new(),
                    output: None,
                    calls: Vec::new(),
                    docs: String::new(),
                    line: line_num,
                    source_code: raw_line.to_string(),
                });
            }
        }
        return None;
    }

    // Kotlin: `fun name(...)` or PHP: `function name(...)` or Scala: `def name(...)`
    if (ext == "kt" && line.contains("fun ")) || (ext == "php" && line.contains("function ")) || (ext == "scala" && line.contains("def ")) {
        let kw = if ext == "kt" { "fun " } else if ext == "php" { "function " } else { "def " };
        if let Some(pos) = line.find(kw) {
            let after = line[pos + kw.len()..].trim();
            let name_end = after.find('(').unwrap_or(after.len());
            let name = after[..name_end].trim().to_string();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let (inputs, output) = parse_params_and_ret(after);
                return Some(FunctionItem {
                    name,
                    visibility: ItemVisibility::Public,
                    is_async: line.contains("async") || line.contains("suspend"),
                    is_method: line.contains("this"),
                    self_param: None,
                    inputs,
                    output,
                    calls: Vec::new(),
                    docs: String::new(),
                    line: line_num,
                    source_code: raw_line.to_string(),
                });
            }
        }
    }

    // JS/TS: `function foo(...)`, `export function foo(...)`, `const foo = (...) =>`, `async function`
    let is_js_func = line.starts_with("function ")
        || line.starts_with("export function ")
        || line.starts_with("async function ")
        || line.starts_with("export async function ")
        || (line.contains(" = (") && line.contains("=>"))
        || (line.contains(" = async (") && line.contains("=>"));

    if is_js_func {
        let is_async = line.contains("async");
        let name = if line.contains(" = ") {
            line.split('=').next().unwrap_or("")
                .trim_start_matches("export ")
                .trim_start_matches("const ")
                .trim_start_matches("let ")
                .trim_start_matches("var ")
                .trim()
                .to_string()
        } else {
            let after_fn = if let Some(pos) = line.find("function") {
                line[pos + 8..].trim()
            } else {
                line
            };
            let name_end = after_fn.find('(').unwrap_or(after_fn.len());
            after_fn[..name_end].trim().to_string()
        };

        if !name.is_empty() {
            let (inputs, output) = parse_params_and_ret(line);
            return Some(FunctionItem {
                name,
                visibility: ItemVisibility::Public,
                is_async,
                is_method: false,
                self_param: None,
                inputs,
                output,
                calls: Vec::new(),
                docs: String::new(),
                line: line_num,
                source_code: raw_line.to_string(),
            });
        }
    }

    // JS/TS class method: `renderFrame() {` or `async renderFrame() {`
    if (ext == "js" || ext == "ts" || ext == "jsx" || ext == "tsx") && !line.ends_with(';') {
        let paren_pos = line.find('(').unwrap_or(0);
        let before_paren = line[..paren_pos].trim();
        let tokens: Vec<&str> = before_paren.split_whitespace().collect();
        if tokens.len() == 1 || (tokens.len() == 2 && (tokens[0] == "async" || tokens[0] == "static" || tokens[0] == "private" || tokens[0] == "public")) {
            let name = tokens.last().unwrap_or(&"").to_string();
            if !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && name != "if" && name != "for" && name != "while" && name != "switch" && name != "catch" && name != "return"
            {
                let (inputs, output) = parse_params_and_ret(line);
                return Some(FunctionItem {
                    name,
                    visibility: ItemVisibility::Public,
                    is_async: tokens.contains(&"async"),
                    is_method: true,
                    self_param: Some("this".to_string()),
                    inputs,
                    output,
                    calls: Vec::new(),
                    docs: String::new(),
                    line: line_num,
                    source_code: raw_line.to_string(),
                });
            }
        }
    }

    // C / C++ / C# / Java: `type name(...)` or `public void name(...)`
    let paren_pos = line.find('(').unwrap_or(0);
    let before_paren = line[..paren_pos].trim();
    let tokens: Vec<&str> = before_paren.split_whitespace().collect();

    if tokens.len() >= 2 && !line.ends_with(';') {
        let name = tokens.last().unwrap_or(&"").trim_start_matches('*').trim_start_matches('&').to_string();
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let is_public = tokens.iter().any(|&t| t == "public" || t == "export");
            let is_static = tokens.iter().any(|&t| t == "static");
            let is_async = tokens.iter().any(|&t| t == "async");
            let (inputs, _) = parse_params_and_ret(line);

            return Some(FunctionItem {
                name,
                visibility: if is_public { ItemVisibility::Public } else { ItemVisibility::Private },
                is_async,
                is_method: !is_static,
                self_param: None,
                inputs,
                output: Some(tokens[tokens.len() - 2].to_string()),
                calls: Vec::new(),
                docs: String::new(),
                line: line_num,
                source_code: raw_line.to_string(),
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_python_file() {
        let code = r#"
import os
from pathlib import Path
from services.auth import verify_token

class Status(Enum):
    ACTIVE = 1
    INACTIVE = 2

class Pipeline(BaseEngine):
    def __init__(self):
        self.endpoint = "http://localhost"

    def run(self):
        verify_token()
        self.fetch_data("http://example.com")

async def fetch_data(url: str) -> dict:
    os.getenv("KEY")
"#;
        let file = extract_universal_file(
            Path::new("pipeline.py"),
            Path::new("pipeline.py"),
            code,
        );

        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "Pipeline");
        assert_eq!(file.structs[0].derives[0], "BaseEngine");
        assert_eq!(file.structs[0].fields[0].name, "endpoint");

        assert_eq!(file.enums.len(), 1);
        assert_eq!(file.enums[0].name, "Status");
        assert_eq!(file.enums[0].variants, vec!["ACTIVE", "INACTIVE"]);

        assert_eq!(file.functions.len(), 3);
        assert_eq!(file.functions[1].name, "run");
        assert_eq!(file.functions[1].calls, vec!["verify_token", "fetch_data"]);

        assert_eq!(file.functions[2].name, "fetch_data");
        assert!(file.functions[2].is_async);
        assert_eq!(file.functions[2].calls, vec!["getenv"]);

        assert_eq!(file.uses.len(), 3);
        assert_eq!(file.uses[2].items, vec!["verify_token"]);
    }

    #[test]
    fn test_extract_typescript_file() {
        let code = r#"
import { render, clear } from './view';

export enum EngineState {
    Ready,
    Running,
    Stopped,
}

export interface CanvasProps {
    width: number;
    renderFrame(): void;
}

export class CanvasEngine extends BaseEngine {
    renderFrame() {
        render();
        clear();
    }
}

export function startEngine() {
    render();
}
"#;
        let file = extract_universal_file(
            Path::new("engine.ts"),
            Path::new("engine.ts"),
            code,
        );

        assert_eq!(file.enums.len(), 1);
        assert_eq!(file.enums[0].name, "EngineState");
        assert_eq!(file.enums[0].variants, vec!["Ready", "Running", "Stopped"]);

        assert_eq!(file.traits.len(), 1);
        assert_eq!(file.traits[0].name, "CanvasProps");
        assert_eq!(file.traits[0].methods, vec!["renderFrame"]);

        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "CanvasEngine");
        assert_eq!(file.structs[0].derives[0], "BaseEngine");

        assert_eq!(file.functions.len(), 2);
        assert_eq!(file.functions[0].name, "renderFrame");
        assert_eq!(file.functions[0].calls, vec!["render", "clear"]);

        assert_eq!(file.functions[1].name, "startEngine");
        assert_eq!(file.functions[1].calls, vec!["render"]);

        assert_eq!(file.uses.len(), 1);
        assert_eq!(file.uses[0].path, "./view");
        assert_eq!(file.uses[0].items, vec!["render", "clear"]);
    }

    #[test]
    fn test_extract_cpp_file() {
        let code = r#"
#include "renderer.h"
#include <vector>

enum class Quality { Low, High };

class Scene : public BaseScene {
public:
    void render() {
        drawGeometry();
        present();
    }
};
"#;
        let file = extract_universal_file(
            Path::new("scene.cpp"),
            Path::new("scene.cpp"),
            code,
        );

        assert_eq!(file.enums.len(), 1);
        assert_eq!(file.enums[0].name, "Quality");
        assert_eq!(file.enums[0].variants, vec!["Low", "High"]);

        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "Scene");
        assert_eq!(file.structs[0].derives, vec!["BaseScene"]);

        assert_eq!(file.functions.len(), 1);
        assert_eq!(file.functions[0].name, "render");
        assert_eq!(file.functions[0].calls, vec!["drawGeometry", "present"]);

        assert_eq!(file.uses.len(), 2);
        assert_eq!(file.uses[0].path, "renderer.h");
    }

    #[test]
    fn test_extract_go_file() {
        let code = r#"
package engine

import "fmt"

type Dispatcher interface {
    Dispatch()
}

type Worker struct {
    ID int
}

func (w *Worker) Process() {
    fmt.Println(w.ID)
}
"#;
        let file = extract_universal_file(
            Path::new("worker.go"),
            Path::new("worker.go"),
            code,
        );

        assert_eq!(file.traits.len(), 1);
        assert_eq!(file.traits[0].name, "Dispatcher");
        assert_eq!(file.traits[0].methods, vec!["Dispatch"]);

        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "Worker");
        assert_eq!(file.structs[0].fields[0].name, "ID");

        assert_eq!(file.functions.len(), 1);
        assert_eq!(file.functions[0].name, "Process");
        assert_eq!(file.functions[0].calls, vec!["Println"]);
    }

    #[test]
    fn test_extract_csharp_and_java_file() {
        let code = r#"
using System.Collections.Generic;

public enum OrderState {
    Pending,
    Shipped,
    Delivered
}

public class OrderService {
    public void ProcessOrder(int orderId) {
        ValidateOrder(orderId);
        NotifyUser();
    }
}
"#;
        let file = extract_universal_file(
            Path::new("OrderService.cs"),
            Path::new("OrderService.cs"),
            code,
        );

        assert_eq!(file.uses.len(), 1);
        assert_eq!(file.uses[0].path, "System.Collections.Generic");

        assert_eq!(file.enums.len(), 1);
        assert_eq!(file.enums[0].name, "OrderState");
        assert_eq!(file.enums[0].variants, vec!["Pending", "Shipped", "Delivered"]);

        assert_eq!(file.structs.len(), 1);
        assert_eq!(file.structs[0].name, "OrderService");

        assert_eq!(file.functions.len(), 1);
        assert_eq!(file.functions[0].name, "ProcessOrder");
        assert_eq!(file.functions[0].calls, vec!["ValidateOrder", "NotifyUser"]);
    }
}
