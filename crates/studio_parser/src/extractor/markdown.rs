use super::types::{ExtractedFile, FunctionItem, ItemVisibility, LinkItem, StructItem};
use std::path::Path;

/// Extracts sections, headings, links, and code blocks from a Markdown document.
pub fn extract_markdown_file(file_path: &Path, rel_path: &Path, content: &str) -> ExtractedFile {
    let module_name =
        file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "document".to_string());

    let mut structs = Vec::new(); // Used for Headings (H1, H2, H3, etc.)
    let mut functions = Vec::new(); // Used for code blocks
    let mut links = Vec::new();

    let mut in_code_block = false;
    let mut code_block_lang = String::new();
    let mut code_block_start_line = 0;
    let mut code_block_lines = Vec::new();

    for (line_idx, raw_line) in content.lines().enumerate() {
        let line_num = line_idx + 1;
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        // Check for fenced code block toggle
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_code_block {
                // End code block
                let code_content = code_block_lines.join("\n");
                let lang_label = if code_block_lang.is_empty() { "code".to_string() } else { code_block_lang.clone() };
                functions.push(FunctionItem {
                    name: format!("block:{}", lang_label),
                    visibility: ItemVisibility::Public,
                    is_async: false,
                    is_method: false,
                    self_param: None,
                    inputs: Vec::new(),
                    output: None,
                    calls: Vec::new(),
                    docs: format!("{} code block ({} lines)", lang_label, code_block_lines.len()),
                    line: code_block_start_line,
                    line_end: line_num,
                    source_code: code_content,
                });
                in_code_block = false;
                code_block_lines.clear();
            } else {
                // Start code block
                in_code_block = true;
                code_block_start_line = line_num;
                code_block_lang = trimmed.trim_start_matches('`').trim_start_matches('~').trim().to_string();
                code_block_lines.clear();
            }
            continue;
        }

        if in_code_block {
            code_block_lines.push(raw_line.to_string());
            continue;
        }

        // Check for Markdown headings: # H1, ## H2, ### H3, #### H4
        if trimmed.starts_with('#') {
            let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
            if hash_count <= 6 {
                let rest = trimmed[hash_count..].trim();
                if !rest.is_empty() {
                    let level_tag = format!("H{}", hash_count);
                    structs.push(StructItem {
                        name: rest.to_string(),
                        visibility: ItemVisibility::Public,
                        fields: Vec::new(),
                        derives: vec![level_tag],
                        docs: format!("Heading level {}", hash_count),
                        line: line_num,
                        line_end: line_num,
                        source_code: raw_line.to_string(),
                    });
                }
            }
        }

        // Extract Markdown links: [label](target) or ['label'](target) or [[wiki]]
        extract_links_from_line(raw_line, line_num, &mut links);
    }

    ExtractedFile {
        file_path: file_path.to_path_buf(),
        relative_path: rel_path.to_path_buf(),
        module_name,
        functions,
        structs,
        enums: Vec::new(),
        traits: Vec::new(),
        impls: Vec::new(),
        uses: Vec::new(),
        links,
        tests: 0,
        parse_error: None,
        language: super::lang::SourceLang::Markdown,
    }
}

fn extract_links_from_line(line: &str, line_num: usize, links: &mut Vec<LinkItem>) {
    // 1. Standard Markdown links: [label](target) or ['label'](target)
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Find closing ']'
            if let Some(close_bracket) = line[i + 1..].find(']') {
                let label_start = i + 1;
                let label_end = i + 1 + close_bracket;
                let raw_label = &line[label_start..label_end];
                let label = raw_label.trim().trim_matches('\'').trim_matches('"');

                // Check for immediate '(' target
                let after_bracket = &line[label_end + 1..];
                if let Some(after_paren) = after_bracket.strip_prefix('(') {
                    if let Some(close_paren) = after_paren.find(')') {
                        let target_str = after_paren[..close_paren].trim();
                        // Ignore pure web URLs for file wiring, but strip anchors
                        let clean_target = target_str
                            .split('#')
                            .next()
                            .unwrap_or(target_str)
                            .split('?')
                            .next()
                            .unwrap_or(target_str)
                            .trim();

                        if !clean_target.is_empty()
                            && !clean_target.starts_with("http://")
                            && !clean_target.starts_with("https://")
                        {
                            let target_file_name = Path::new(clean_target)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(clean_target)
                                .to_string();
                            links.push(LinkItem {
                                label: if label.is_empty() { target_file_name } else { label.to_string() },
                                target: clean_target.to_string(),
                                line: line_num,
                                line_end: line_num,
                                source_code: line.to_string(),
                            });
                        }
                        i = label_end + 1 + close_paren + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_markdown_headings_and_links() {
        let content = r#"# Map Program Hub
Here is the overview.

## Slice specs (read these - not optional)
| Slice | Spec |
| **T-090.0** | ['t_090_0_map_program_hub.md'](t_090_0_map_program_hub.md) |
| **T-090.1** | [Basemap](t_090_1_aligned_basemap.md) |

```rust
fn example() {}
```
"#;
        let file = extract_markdown_file(
            Path::new("t_090_0_map_program_hub.md"),
            Path::new("t_090_0_map_program_hub.md"),
            content,
        );

        assert_eq!(file.structs.len(), 2, "Should find 2 headings");
        assert_eq!(file.structs[0].name, "Map Program Hub");
        assert_eq!(file.structs[0].derives[0], "H1");
        assert_eq!(file.structs[1].name, "Slice specs (read these - not optional)");
        assert_eq!(file.structs[1].derives[0], "H2");

        // Links
        let targets: Vec<&str> = file.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["t_090_0_map_program_hub.md", "t_090_1_aligned_basemap.md"]);
        assert_eq!(file.links[1].label, "Basemap");
        assert!(file.functions.iter().all(|f| f.name.starts_with("block:")), "links are not functions");

        // Code block
        let code_blocks: Vec<&FunctionItem> = file.functions.iter().filter(|f| f.name.starts_with("block:")).collect();
        assert_eq!(code_blocks.len(), 1);
        assert_eq!(code_blocks[0].name, "block:rust");
    }
}
