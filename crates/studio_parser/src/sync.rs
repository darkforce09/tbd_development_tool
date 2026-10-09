use std::collections::HashMap;
use std::path::Path;
use quote::ToTokens;
use studio_graph::{FileMemberNode, Graph, NodeArchetype};

/// Saves edited code to disk and dynamically hot-reloads node metadata in the active Graph.
/// Supports Rust, Markdown, Enforce Script, and universal polyglot code files.
pub fn save_and_reparse(path: &Path, new_content: &str, graph: &mut Graph) -> Result<usize, String> {
    // 1. Write updated content to disk
    std::fs::write(path, new_content)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let path_str = path.to_string_lossy().to_string();
    let mut updated_count = 0;

    let mut item_sources: HashMap<String, String> = HashMap::new();
    let mut new_member_nodes: Vec<FileMemberNode> = Vec::new();

    if ext == "rs" {
        if let Ok(syn_file) = syn::parse_file(new_content) {
            for item in &syn_file.items {
                match item {
                    syn::Item::Fn(f) => {
                        let name = format!("fn {}", f.sig.ident);
                        let src = f.to_token_stream().to_string();
                        item_sources.insert(name.clone(), src.clone());
                        item_sources.insert(f.sig.ident.to_string(), src.clone());
                        new_member_nodes.push(FileMemberNode::new(
                            format!("fn:{}", f.sig.ident),
                            f.sig.ident.to_string(),
                            NodeArchetype::Function,
                            "",
                            format!("fn {}", f.sig.ident),
                            1,
                            src,
                            None,
                        ));
                    }
                    syn::Item::Struct(s) => {
                        let name = format!("struct {}", s.ident);
                        let src = s.to_token_stream().to_string();
                        item_sources.insert(name.clone(), src.clone());
                        item_sources.insert(s.ident.to_string(), src.clone());
                        new_member_nodes.push(FileMemberNode::new(
                            format!("struct:{}", s.ident),
                            s.ident.to_string(),
                            NodeArchetype::Struct,
                            "",
                            format!("struct {}", s.ident),
                            1,
                            src,
                            None,
                        ));
                    }
                    syn::Item::Enum(e) => {
                        let name = format!("enum {}", e.ident);
                        let src = e.to_token_stream().to_string();
                        item_sources.insert(name.clone(), src.clone());
                        item_sources.insert(e.ident.to_string(), src.clone());
                        new_member_nodes.push(FileMemberNode::new(
                            format!("enum:{}", e.ident),
                            e.ident.to_string(),
                            NodeArchetype::Enum,
                            "",
                            format!("enum {}", e.ident),
                            1,
                            src,
                            None,
                        ));
                    }
                    syn::Item::Trait(t) => {
                        let name = format!("trait {}", t.ident);
                        let src = t.to_token_stream().to_string();
                        item_sources.insert(name.clone(), src.clone());
                        item_sources.insert(t.ident.to_string(), src.clone());
                        new_member_nodes.push(FileMemberNode::new(
                            format!("trait:{}", t.ident),
                            t.ident.to_string(),
                            NodeArchetype::Trait,
                            "",
                            format!("trait {}", t.ident),
                            1,
                            src,
                            None,
                        ));
                    }
                    _ => {}
                }
            }
        }
    } else if ext == "md" || ext == "markdown" {
        let extracted = crate::extractor::markdown::extract_markdown_file(path, path, new_content);
        for s in &extracted.structs {
            let tag = s.derives.first().cloned().unwrap_or_else(|| "H1".to_string());
            new_member_nodes.push(FileMemberNode::new(
                format!("heading:{}", s.name),
                &s.name,
                NodeArchetype::Module,
                tag,
                &s.name,
                s.line,
                &s.source_code,
                None,
            ));
        }
        for f in &extracted.functions {
            new_member_nodes.push(FileMemberNode::new(
                format!("link:{}", f.name),
                &f.name,
                NodeArchetype::Function,
                "LNK",
                &f.name,
                f.line,
                &f.source_code,
                None,
            ));
        }
    } else if crate::extractor::enforce::is_enforce_script(path, new_content) {
        let extracted = crate::extractor::enforce::extract_enforce_script_file(path, path, new_content);
        for s in &extracted.structs {
            let is_modded = s.derives.contains(&"modded".to_string());
            let vis = if is_modded { "MOD" } else { "CLS" };
            new_member_nodes.push(FileMemberNode::new(
                format!("class:{}", s.name),
                &s.name,
                NodeArchetype::Struct,
                vis,
                format!("class {}", s.name),
                s.line,
                &s.source_code,
                None,
            ));
            item_sources.insert(s.name.clone(), s.source_code.clone());
        }
        for imp in &extracted.impls {
            for m in &imp.methods {
                new_member_nodes.push(FileMemberNode::new(
                    format!("method:{}::{}", imp.target_type, m.name),
                    format!("{}::{}", imp.target_type, m.name),
                    NodeArchetype::Function,
                    "FN",
                    &m.name,
                    m.line,
                    &m.source_code,
                    None,
                ));
                item_sources.insert(m.name.clone(), m.source_code.clone());
            }
        }
    } else {
        let extracted = crate::extractor::universal::extract_universal_file(path, path, new_content);
        for s in &extracted.structs {
            new_member_nodes.push(FileMemberNode::new(
                format!("struct:{}", s.name),
                &s.name,
                NodeArchetype::Struct,
                "CLS",
                &s.name,
                s.line,
                &s.source_code,
                None,
            ));
            item_sources.insert(s.name.clone(), s.source_code.clone());
        }
        for f in &extracted.functions {
            new_member_nodes.push(FileMemberNode::new(
                format!("fn:{}", f.name),
                &f.name,
                NodeArchetype::Function,
                "FN",
                &f.name,
                f.line,
                &f.source_code,
                None,
            ));
            item_sources.insert(f.name.clone(), f.source_code.clone());
        }
    }

    // 3. Update existing graph nodes belonging to this file in-place
    for node in graph.nodes.values_mut() {
        let is_matching_file = node
            .file_path
            .as_ref()
            .map(|p| p == &path_str || path_str.ends_with(p) || p.ends_with(&path_str))
            .unwrap_or(false);

        if is_matching_file {
            if node.archetype == NodeArchetype::File {
                node.source_code = Some(new_content.to_string());
                if !new_member_nodes.is_empty() {
                    node.member_nodes = new_member_nodes.clone();
                }
                updated_count += 1;
            } else if let Some(new_src) = item_sources.get(&node.title) {
                node.source_code = Some(new_src.clone());
                updated_count += 1;
            } else if node.source_code.is_some() {
                node.source_code = Some(new_content.to_string());
                updated_count += 1;
            }
        }
    }

    Ok(updated_count)
}
