use std::path::Path;
use quote::ToTokens;
use rayon::prelude::*;
use syn::spanned::Spanned;
use syn::visit::Visit;
use crate::project::RustProject;

use super::helpers::{clean_tokens, extract_derives, extract_docs, extract_source_lines, extract_vis, CallVisitor};
use super::types::{
    EnumItem, ExtractedCrate, ExtractedFile, ExtractedProject, FieldInfo, FunctionItem, ImplItem,
    ParamInfo, StructItem, TraitItem, UseItem,
};

/// Extracts AST information from all source files in a scanned Rust project using parallel CPU cores.
pub fn extract_project(project: &RustProject) -> ExtractedProject {
    let extracted_crates: Vec<ExtractedCrate> = project
        .crates
        .par_iter()
        .map(|krate| {
            let extracted_files: Vec<ExtractedFile> = krate
                .source_files
                .par_iter()
                .map(|file_path| {
                    let rel_path = file_path
                        .strip_prefix(&krate.root_path)
                        .unwrap_or(file_path)
                        .to_path_buf();
                    extract_file(file_path, &rel_path)
                })
                .collect();

            ExtractedCrate {
                name: krate.name.clone(),
                root_path: krate.root_path.clone(),
                files: extracted_files,
            }
        })
        .collect();

    ExtractedProject {
        name: project.name.clone(),
        root_path: project.root_path.clone(),
        crates: extracted_crates,
    }
}

pub fn extract_file(file_path: &Path, rel_path: &Path) -> ExtractedFile {
    let module_name = file_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mod".to_string());

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
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
                parse_error: Some(format!("Failed to read file: {}", e)),
            };
        }
    };

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "md" || ext == "markdown" {
        return super::markdown::extract_markdown_file(file_path, rel_path, &content);
    }

    if super::enforce::is_enforce_script(file_path, &content) {
        return super::enforce::extract_enforce_script_file(file_path, rel_path, &content);
    }

    if ext != "rs" {
        return super::universal::extract_universal_file(file_path, rel_path, &content);
    }

    let syn_file = match syn::parse_file(&content) {
        Ok(sf) => sf,
        Err(e) => {
            let mut fallback = super::universal::extract_universal_file(file_path, rel_path, &content);
            fallback.parse_error = Some(format!("Syntax error: {}", e));
            return fallback;
        }
    };

    let mut functions = Vec::new();
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut traits = Vec::new();
    let mut impls = Vec::new();
    let mut uses = Vec::new();

    let content_lines: Vec<&str> = content.lines().collect();

    for item in syn_file.items {
        match item {
            syn::Item::Fn(fn_item) => {
                functions.push(parse_fn(&fn_item, false, &content_lines));
            }
            syn::Item::Struct(s_item) => {
                structs.push(parse_struct(&s_item, &content_lines));
            }
            syn::Item::Enum(e_item) => {
                enums.push(parse_enum(&e_item, &content_lines));
            }
            syn::Item::Trait(t_item) => {
                traits.push(parse_trait(&t_item, &content_lines));
            }
            syn::Item::Impl(i_item) => {
                impls.push(parse_impl(&i_item, &content_lines));
            }
            syn::Item::Use(u_item) => {
                uses.push(parse_use(&u_item));
            }
            _ => {}
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
        impls,
        uses,
        parse_error: None,
    }
}

pub fn parse_fn(fn_item: &syn::ItemFn, is_method: bool, content_lines: &[&str]) -> FunctionItem {
    let name = fn_item.sig.ident.to_string();
    let visibility = extract_vis(&fn_item.vis);
    let is_async = fn_item.sig.asyncness.is_some();
    let docs = extract_docs(&fn_item.attrs);
    let line = fn_item.sig.ident.span().start().line;

    let mut self_param = None;
    let mut inputs = Vec::new();

    for arg in &fn_item.sig.inputs {
        match arg {
            syn::FnArg::Receiver(r) => {
                let s = r.to_token_stream().to_string();
                self_param = Some(clean_tokens(&s));
            }
            syn::FnArg::Typed(pat_type) => {
                let pat_str = clean_tokens(&pat_type.pat.to_token_stream().to_string());
                let ty_str = clean_tokens(&pat_type.ty.to_token_stream().to_string());
                inputs.push(ParamInfo {
                    name: pat_str,
                    type_str: ty_str,
                });
            }
        }
    }

    let output = match &fn_item.sig.output {
        syn::ReturnType::Default => None,
        syn::ReturnType::Type(_, ty) => Some(clean_tokens(&ty.to_token_stream().to_string())),
    };

    let mut visitor = CallVisitor { calls: Vec::new() };
    visitor.visit_block(&fn_item.block);

    let source_code = extract_source_lines(content_lines, line)
        .unwrap_or_else(|| clean_tokens(&fn_item.to_token_stream().to_string()));

    FunctionItem {
        name,
        visibility,
        is_async,
        is_method,
        self_param,
        inputs,
        output,
        calls: visitor.calls,
        docs,
        line,
        source_code,
    }
}

pub fn parse_struct(s_item: &syn::ItemStruct, content_lines: &[&str]) -> StructItem {
    let name = s_item.ident.to_string();
    let visibility = extract_vis(&s_item.vis);
    let docs = extract_docs(&s_item.attrs);
    let derives = extract_derives(&s_item.attrs);
    let line = s_item.ident.span().start().line;
    let source_code = extract_source_lines(content_lines, line)
        .unwrap_or_else(|| clean_tokens(&s_item.to_token_stream().to_string()));

    let mut fields = Vec::new();
    match &s_item.fields {
        syn::Fields::Named(named) => {
            for f in &named.named {
                let f_name = f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default();
                let f_ty = clean_tokens(&f.ty.to_token_stream().to_string());
                let f_vis = extract_vis(&f.vis);
                fields.push(FieldInfo {
                    name: f_name,
                    type_str: f_ty,
                    visibility: f_vis,
                });
            }
        }
        syn::Fields::Unnamed(unnamed) => {
            for (idx, f) in unnamed.unnamed.iter().enumerate() {
                let f_name = format!("{}", idx);
                let f_ty = clean_tokens(&f.ty.to_token_stream().to_string());
                let f_vis = extract_vis(&f.vis);
                fields.push(FieldInfo {
                    name: f_name,
                    type_str: f_ty,
                    visibility: f_vis,
                });
            }
        }
        syn::Fields::Unit => {}
    }

    StructItem {
        name,
        visibility,
        fields,
        derives,
        docs,
        line,
        source_code,
    }
}

pub fn parse_enum(e_item: &syn::ItemEnum, content_lines: &[&str]) -> EnumItem {
    let name = e_item.ident.to_string();
    let visibility = extract_vis(&e_item.vis);
    let docs = extract_docs(&e_item.attrs);
    let line = e_item.ident.span().start().line;
    let source_code = extract_source_lines(content_lines, line)
        .unwrap_or_else(|| clean_tokens(&e_item.to_token_stream().to_string()));

    let variants = e_item
        .variants
        .iter()
        .map(|v| v.ident.to_string())
        .collect();

    EnumItem {
        name,
        visibility,
        variants,
        docs,
        line,
        source_code,
    }
}

pub fn parse_trait(t_item: &syn::ItemTrait, content_lines: &[&str]) -> TraitItem {
    let name = t_item.ident.to_string();
    let visibility = extract_vis(&t_item.vis);
    let docs = extract_docs(&t_item.attrs);
    let line = t_item.ident.span().start().line;
    let source_code = extract_source_lines(content_lines, line)
        .unwrap_or_else(|| clean_tokens(&t_item.to_token_stream().to_string()));

    let mut methods = Vec::new();
    for item in &t_item.items {
        if let syn::TraitItem::Fn(fn_item) = item {
            methods.push(fn_item.sig.ident.to_string());
        }
    }

    TraitItem {
        name,
        visibility,
        methods,
        docs,
        line,
        source_code,
    }
}

pub fn parse_impl(i_item: &syn::ItemImpl, content_lines: &[&str]) -> ImplItem {
    let target_type = clean_tokens(&i_item.self_ty.to_token_stream().to_string());
    let trait_name = i_item
        .trait_
        .as_ref()
        .map(|(_, path, _)| clean_tokens(&path.to_token_stream().to_string()));
    let line = i_item.self_ty.span().start().line;

    let mut methods = Vec::new();
    for item in &i_item.items {
        if let syn::ImplItem::Fn(fn_item) = item {
            let name = fn_item.sig.ident.to_string();
            let visibility = extract_vis(&fn_item.vis);
            let is_async = fn_item.sig.asyncness.is_some();
            let docs = extract_docs(&fn_item.attrs);
            let fn_line = fn_item.sig.ident.span().start().line;
            let fn_source = extract_source_lines(content_lines, fn_line)
                .unwrap_or_else(|| clean_tokens(&fn_item.to_token_stream().to_string()));

            let mut self_param = None;
            let mut inputs = Vec::new();

            for arg in &fn_item.sig.inputs {
                match arg {
                    syn::FnArg::Receiver(r) => {
                        self_param = Some(clean_tokens(&r.to_token_stream().to_string()));
                    }
                    syn::FnArg::Typed(pat_type) => {
                        inputs.push(ParamInfo {
                            name: clean_tokens(&pat_type.pat.to_token_stream().to_string()),
                            type_str: clean_tokens(&pat_type.ty.to_token_stream().to_string()),
                        });
                    }
                }
            }

            let output = match &fn_item.sig.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, ty) => Some(clean_tokens(&ty.to_token_stream().to_string())),
            };

            let mut visitor = CallVisitor { calls: Vec::new() };
            visitor.visit_block(&fn_item.block);

            methods.push(FunctionItem {
                name,
                visibility,
                is_async,
                is_method: self_param.is_some(),
                self_param,
                inputs,
                output,
                calls: visitor.calls,
                docs,
                line: fn_line,
                source_code: fn_source,
            });
        }
    }

    ImplItem {
        target_type,
        trait_name,
        methods,
        line,
    }
}

pub fn parse_use(u_item: &syn::ItemUse) -> UseItem {
    let path = clean_tokens(&u_item.tree.to_token_stream().to_string());
    UseItem {
        path,
        items: Vec::new(),
    }
}
