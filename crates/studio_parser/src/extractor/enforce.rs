use std::path::Path;
use super::types::{
    EnumItem, ExtractedFile, FieldInfo, FunctionItem, ImplItem, ItemVisibility, ParamInfo,
    StructItem, UseItem,
};

/// Checks if file content or path strongly suggests Bohemia Interactive's Enforce Script.
pub fn is_enforce_script(path: &Path, content: &str) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "ens" || ext == "es" || ext == "enforce" {
        return true;
    }

    if ext == "c" {
        // Enforce script typically contains `modded class`, `proto native`, `proto `,
        // or class declarations with `: BaseClass`, or standard DayZ / Arma calls.
        if content.contains("modded class")
            || content.contains("proto native")
            || content.contains("autoptr ")
            || content.contains("GetGame()")
            || content.contains("override void ")
            || content.contains("override bool ")
            || content.contains("override int ")
            || content.contains("override float ")
            || content.contains("ref array<")
            || content.contains("ref map<")
        {
            return true;
        }

        let path_str = path.to_string_lossy();
        if path_str.contains("Scripts/") || path_str.contains("scripts/")
            || path_str.contains("1_Core") || path_str.contains("2_GameLib")
            || path_str.contains("3_Game") || path_str.contains("4_World")
            || path_str.contains("5_Mission") || path_str.contains("DayZ")
            || path_str.contains("ArmaReforger") || path_str.contains("Enforce")
        {
            return true;
        }
    }

    false
}

/// Extracts classes, modded classes, methods, enums, and dependencies from an Enforce Script file.
pub fn extract_enforce_script_file(file_path: &Path, rel_path: &Path, content: &str) -> ExtractedFile {
    let module_name = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "script".to_string());

    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut impls = Vec::new();
    let mut functions = Vec::new();
    let mut uses = Vec::new();

    let mut current_class: Option<(String, usize, Vec<String>)> = None; // (name, start_line, derives)
    let mut current_class_methods = Vec::new();
    let mut current_class_fields = Vec::new();
    let mut brace_depth: i32 = 0;
    let mut class_brace_depth: i32 = 0;

    let lines: Vec<&str> = content.lines().collect();

    for (line_idx, raw_line) in lines.iter().enumerate() {
        let line_num = line_idx + 1;
        let line = raw_line.trim();

        // 1. Check for includes: #include "..."
        if line.starts_with("#include") {
            if let Some(start) = line.find('"') {
                if let Some(end) = line[start + 1..].find('"') {
                    let inc_path = &line[start + 1..start + 1 + end];
                    uses.push(UseItem {
                        path: inc_path.to_string(),
                        items: vec!["#include".to_string()],
                    });
                }
            }
            continue;
        }

        // Count braces for scope tracking
        let open_count = line.chars().filter(|&c| c == '{').count() as i32;
        let close_count = line.chars().filter(|&c| c == '}').count() as i32;

        // 2. Class Declaration: `modded class Name : Base` or `class Name : Base` or `class Name`
        if (line.starts_with("modded class ") || line.starts_with("class ") || line.starts_with("sealed class "))
            && !line.contains(';')
        {
            let is_modded = line.starts_with("modded class ");
            let line_clean = line.trim_start_matches("modded ")
                .trim_start_matches("sealed ")
                .trim_start_matches("class ")
                .trim();

            let (class_name, base_class) = if let Some((name_part, base_part)) = line_clean.split_once(':') {
                let name = name_part.trim().split_whitespace().next().unwrap_or(name_part.trim()).to_string();
                let base = base_part.trim().split_whitespace().next()
                    .unwrap_or(base_part.trim())
                    .trim_matches('{')
                    .trim()
                    .to_string();
                (name, Some(base))
            } else if let Some((name_part, base_part)) = line_clean.split_once("extends") {
                let name = name_part.trim().split_whitespace().next().unwrap_or(name_part.trim()).to_string();
                let base = base_part.trim().split_whitespace().next()
                    .unwrap_or(base_part.trim())
                    .trim_matches('{')
                    .trim()
                    .to_string();
                (name, Some(base))
            } else {
                let name = line_clean.split_whitespace().next()
                    .unwrap_or(line_clean)
                    .trim_matches('{')
                    .trim()
                    .to_string();
                (name, None)
            };

            let mut derives = Vec::new();
            if is_modded {
                derives.push("modded".to_string());
            }
            if let Some(base) = base_class {
                derives.push(base.clone());
                uses.push(UseItem {
                    path: base,
                    items: vec!["inherit".to_string()],
                });
            }

            // Flush previous class if any
            if let Some((prev_name, prev_line, prev_derives)) = current_class.take() {
                structs.push(StructItem {
                    name: prev_name.clone(),
                    visibility: if prev_derives.contains(&"modded".to_string()) {
                        ItemVisibility::Public
                    } else {
                        ItemVisibility::Public
                    },
                    fields: std::mem::take(&mut current_class_fields),
                    derives: prev_derives,
                    docs: String::new(),
                    line: prev_line,
                    source_code: format!("class {}", prev_name),
                });
                if !current_class_methods.is_empty() {
                    impls.push(ImplItem {
                        target_type: prev_name,
                        trait_name: None,
                        methods: std::mem::take(&mut current_class_methods),
                        line: prev_line,
                    });
                }
            }

            current_class = Some((class_name, line_num, derives));
            class_brace_depth = brace_depth + open_count - close_count;
            brace_depth += open_count - close_count;
            continue;
        }

        // 3. Enum Declaration: `enum EMyEnum`
        if line.starts_with("enum ") {
            let enum_name = line.trim_start_matches("enum ")
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('{')
                .to_string();

            if !enum_name.is_empty() {
                // Collect variants if on same line or within braces
                enums.push(EnumItem {
                    name: enum_name,
                    visibility: ItemVisibility::Public,
                    variants: Vec::new(),
                    docs: String::new(),
                    line: line_num,
                    source_code: raw_line.to_string(),
                });
            }
        }

        // 4. Method / Function Declaration
        // Examples: `override void MethodName(...)`, `proto native void Func(...)`, `void Func(...)`, `string GetName(...)`
        if is_enforce_method_declaration(line) {
            let method_item = parse_enforce_method(raw_line, line_num, &lines[line_idx..]);
            let calls = extract_enforce_calls(&method_item.source_code);
            let mut final_method = method_item;
            final_method.calls = calls;

            if current_class.is_some() {
                current_class_methods.push(final_method);
            } else {
                functions.push(final_method);
            }
        } else if current_class.is_some() && is_enforce_field_declaration(line) {
            // Field declaration inside class: `protected int m_Count;` or `ref array<string> m_Items;`
            let (field_name, field_type) = parse_enforce_field(line);
            if !field_name.is_empty() {
                current_class_fields.push(FieldInfo {
                    name: field_name,
                    type_str: field_type,
                    visibility: if line.starts_with("protected") {
                        ItemVisibility::Crate
                    } else if line.starts_with("private") {
                        ItemVisibility::Private
                    } else {
                        ItemVisibility::Public
                    },
                });
            }
        }

        brace_depth += open_count - close_count;

        // Check if class closed
        if current_class.is_some() {
            if brace_depth < class_brace_depth {
                if let Some((prev_name, prev_line, prev_derives)) = current_class.take() {
                    structs.push(StructItem {
                        name: prev_name.clone(),
                        visibility: ItemVisibility::Public,
                        fields: std::mem::take(&mut current_class_fields),
                        derives: prev_derives,
                        docs: String::new(),
                        line: prev_line,
                        source_code: format!("class {}", prev_name),
                    });
                    if !current_class_methods.is_empty() {
                        impls.push(ImplItem {
                            target_type: prev_name,
                            trait_name: None,
                            methods: std::mem::take(&mut current_class_methods),
                            line: prev_line,
                        });
                    }
                }
            }
        }
    }

    // Final flush if file ended before closing brace
    if let Some((prev_name, prev_line, prev_derives)) = current_class.take() {
        structs.push(StructItem {
            name: prev_name.clone(),
            visibility: ItemVisibility::Public,
            fields: current_class_fields,
            derives: prev_derives,
            docs: String::new(),
            line: prev_line,
            source_code: format!("class {}", prev_name),
        });
        if !current_class_methods.is_empty() {
            impls.push(ImplItem {
                target_type: prev_name,
                trait_name: None,
                methods: current_class_methods,
                line: prev_line,
            });
        }
    }

    ExtractedFile {
        file_path: file_path.to_path_buf(),
        relative_path: rel_path.to_path_buf(),
        module_name,
        functions,
        structs,
        enums,
        traits: Vec::new(),
        impls,
        uses,
        parse_error: None,
    }
}

fn is_enforce_method_declaration(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
        return false;
    }
    if !trimmed.contains('(') || !trimmed.contains(')') {
        return false;
    }
    if trimmed.starts_with("if ") || trimmed.starts_with("for ") || trimmed.starts_with("while ") || trimmed.starts_with("switch ") {
        return false;
    }

    trimmed.starts_with("override ")
        || trimmed.starts_with("proto ")
        || trimmed.starts_with("void ")
        || trimmed.starts_with("bool ")
        || trimmed.starts_with("int ")
        || trimmed.starts_with("float ")
        || trimmed.starts_with("string ")
        || trimmed.starts_with("vector ")
        || trimmed.starts_with("protected ")
        || trimmed.starts_with("private ")
        || trimmed.starts_with("static ")
}

fn parse_enforce_method(raw_line: &str, line_num: usize, remaining_lines: &[&str]) -> FunctionItem {
    let line = raw_line.trim();
    let is_override = line.starts_with("override ");
    let is_static = line.contains("static ");
    let paren_pos = line.find('(').unwrap_or(line.len());
    let sig_before_paren = &line[..paren_pos].trim();

    // The method name is the last token before '('
    let name = sig_before_paren.split_whitespace().last().unwrap_or("method").to_string();

    // Extract return type (the token before name)
    let tokens: Vec<&str> = sig_before_paren.split_whitespace().collect();
    let ret_type = if tokens.len() >= 2 {
        Some(tokens[tokens.len() - 2].to_string())
    } else {
        None
    };

    // Extract parameters
    let mut inputs = Vec::new();
    if let Some(close_paren) = line[paren_pos..].find(')') {
        let params_str = &line[paren_pos + 1..paren_pos + close_paren].trim();
        if !params_str.is_empty() {
            for param in params_str.split(',') {
                let p_parts: Vec<&str> = param.trim().split_whitespace().collect();
                if p_parts.len() >= 2 {
                    inputs.push(ParamInfo {
                        name: p_parts[p_parts.len() - 1].to_string(),
                        type_str: p_parts[..p_parts.len() - 1].join(" "),
                    });
                } else if !p_parts.is_empty() {
                    inputs.push(ParamInfo {
                        name: p_parts[0].to_string(),
                        type_str: "any".to_string(),
                    });
                }
            }
        }
    }

    // Collect method body snippet (up to 15 lines)
    let mut body_lines = Vec::new();
    let mut depth = 0;
    let mut opened = false;
    for l in remaining_lines.iter().take(15) {
        body_lines.push(*l);
        depth += l.chars().filter(|&c| c == '{').count() as i32;
        depth -= l.chars().filter(|&c| c == '}').count() as i32;
        if depth > 0 {
            opened = true;
        }
        if opened && depth <= 0 {
            break;
        }
    }
    let source_code = body_lines.join("\n");

    FunctionItem {
        name,
        visibility: if is_override {
            ItemVisibility::Public
        } else if line.starts_with("protected") {
            ItemVisibility::Crate
        } else if line.starts_with("private") {
            ItemVisibility::Private
        } else {
            ItemVisibility::Public
        },
        is_async: false,
        is_method: true,
        self_param: if is_static { None } else { Some("this".to_string()) },
        inputs,
        output: ret_type,
        calls: Vec::new(),
        docs: if is_override { "Override method".to_string() } else { String::new() },
        line: line_num,
        source_code,
    }
}

fn is_enforce_field_declaration(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with(';') && !trimmed.contains('(') && (
        trimmed.starts_with("protected ")
        || trimmed.starts_with("private ")
        || trimmed.starts_with("ref ")
        || trimmed.starts_with("autoptr ")
        || trimmed.starts_with("int ")
        || trimmed.starts_with("float ")
        || trimmed.starts_with("string ")
        || trimmed.starts_with("bool ")
    )
}

fn parse_enforce_field(line: &str) -> (String, String) {
    let clean = line.trim().trim_end_matches(';').trim();
    let tokens: Vec<&str> = clean.split_whitespace().collect();
    if tokens.len() >= 2 {
        let name = tokens.last().unwrap_or(&"").to_string();
        let type_str = tokens[..tokens.len() - 1].join(" ");
        (name, type_str)
    } else {
        (String::new(), String::new())
    }
}

fn extract_enforce_calls(source_code: &str) -> Vec<String> {
    let mut calls = Vec::new();
    for line in source_code.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }

        // Detect super.MethodName
        if let Some(super_idx) = trimmed.find("super.") {
            let after = &trimmed[super_idx + 6..];
            if let Some(paren) = after.find('(') {
                let call_name = after[..paren].trim().to_string();
                if !call_name.is_empty() && !calls.contains(&call_name) {
                    calls.push(format!("super.{}", call_name));
                    calls.push(call_name);
                }
            }
        }

        // Detect GetGame(), Print(), etc.
        for common_call in &["GetGame", "Print", "GetPlayer", "GetWorld", "CreateObject", "SetPosition"] {
            if trimmed.contains(common_call) && !calls.contains(&common_call.to_string()) {
                calls.push(common_call.to_string());
            }
        }
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let file = extract_enforce_script_file(
            Path::new("DayZPlayerImplement.c"),
            Path::new("DayZPlayerImplement.c"),
            code,
        );

        assert_eq!(file.structs.len(), 2, "Should parse 2 classes");
        assert_eq!(file.structs[0].name, "DayZPlayerImplement");
        assert!(file.structs[0].derives.contains(&"modded".to_string()));

        assert_eq!(file.structs[1].name, "CustomEntity");
        assert!(file.structs[1].derives.contains(&"Managed".to_string()));

        assert_eq!(file.impls.len(), 2, "Should have 2 impl blocks for the classes");
        let methods = &file.impls[0].methods;
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "ShowDeadScreen");
        assert!(methods[0].calls.contains(&"ShowDeadScreen".to_string()) || methods[0].calls.contains(&"Print".to_string()));
    }
}
