use syn::visit::Visit;
use super::types::ItemVisibility;

pub struct CallVisitor {
    pub calls: Vec<String>,
}

impl<'ast> Visit<'ast> for CallVisitor {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(ref expr_path) = *node.func {
            if let Some(ident) = expr_path.path.segments.last() {
                self.calls.push(ident.ident.to_string());
            }
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.calls.push(node.method.to_string());
        syn::visit::visit_expr_method_call(self, node);
    }
}

pub fn extract_docs(attrs: &[syn::Attribute]) -> String {
    let mut docs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let syn::Meta::NameValue(meta_nv) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta_nv.value {
                    if let syn::Lit::Str(lit_str) = &expr_lit.lit {
                        let text = lit_str.value().trim().to_string();
                        if !text.is_empty() {
                            docs.push(text);
                        }
                    }
                }
            }
        }
    }
    docs.join(" ")
}

pub fn extract_vis(vis: &syn::Visibility) -> ItemVisibility {
    match vis {
        syn::Visibility::Public(_) => ItemVisibility::Public,
        syn::Visibility::Restricted(r) => {
            if r.path.is_ident("crate") {
                ItemVisibility::Crate
            } else {
                ItemVisibility::Private
            }
        }
        syn::Visibility::Inherited => ItemVisibility::Private,
    }
}

pub fn extract_derives(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut derives = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("derive") {
            if let syn::Meta::List(meta_list) = &attr.meta {
                let s = meta_list.tokens.to_string();
                for part in s.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        derives.push(trimmed.to_string());
                    }
                }
            }
        }
    }
    derives
}

pub fn clean_tokens(tokens: &str) -> String {
    tokens
        .replace(" & ", "&")
        .replace("& ", "&")
        .replace(" ,", ",")
        .replace(" :", ":")
        .replace(" <", "<")
        .replace(" >", ">")
        .replace(" (", "(")
        .replace(" )", ")")
        .trim()
        .to_string()
}

/// Slices the authentic multi-line source code of an AST item from file content lines.
pub fn extract_source_lines(content_lines: &[&str], start_line_1_based: usize) -> Option<String> {
    if content_lines.is_empty() || start_line_1_based == 0 || start_line_1_based > content_lines.len() {
        return None;
    }

    let mut start_idx = start_line_1_based - 1;
    // Walk backward to include doc comments or attributes right above this item
    while start_idx > 0 {
        let prev = content_lines[start_idx - 1].trim();
        if prev.starts_with("///") || prev.starts_with("#[") || prev.starts_with("//!") {
            start_idx -= 1;
        } else {
            break;
        }
    }

    let mut brace_depth: i32 = 0;
    let mut found_open_brace = false;
    let mut collected = Vec::new();

    for (offset, &line) in content_lines[start_idx..].iter().enumerate() {
        collected.push(line);
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
        if offset >= 300 {
            break;
        }
    }

    if !collected.is_empty() {
        Some(collected.join("\n"))
    } else {
        None
    }
}
