//! Tools: the commands a project defines for itself, found by reading files, never by running
//! anything. Each records where it is defined, what it says it does, and the exact command line
//! that runs it.
//!
//! - Cargo binaries and examples, and `[alias]` entries in `.cargo/config.toml`.
//! - Command-line subcommands of Rust binaries that use clap's derive API, nested, with enums
//!   found in the package or in the packages it depends on.
//! - `scripts` of every `package.json`, in file order.
//! - Makefile targets and justfile recipes.
//! - GitHub Actions workflows and their jobs.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::packages::{Package, Packages, TargetKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolKind {
    Binary,
    Example,
    CargoAlias,
    Subcommand,
    NpmScript,
    MakeTarget,
    JustRecipe,
    Workflow,
    WorkflowJob,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Binary => "binary",
            ToolKind::Example => "example",
            ToolKind::CargoAlias => "cargo alias",
            ToolKind::Subcommand => "subcommand",
            ToolKind::NpmScript => "script",
            ToolKind::MakeTarget => "make target",
            ToolKind::JustRecipe => "just recipe",
            ToolKind::Workflow => "CI workflow",
            ToolKind::WorkflowJob => "CI job",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub kind: ToolKind,
    pub name: String,
    /// The exact command line, from the project root; empty for what only CI runs.
    pub invocation: String,
    pub description: Option<String>,
    /// Where it is defined (1-based line).
    pub file: PathBuf,
    pub line: usize,
    pub children: Vec<Tool>,
}

impl Tool {
    /// This tool and everything under it, depth first.
    pub fn walk(&self) -> Vec<&Tool> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.walk());
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    pub tools: Vec<Tool>,
}

impl Tools {
    pub fn all(&self) -> impl Iterator<Item = &Tool> {
        self.tools.iter().flat_map(Tool::walk)
    }

    pub fn count(&self, kind: ToolKind) -> usize {
        self.all().filter(|t| t.kind == kind).count()
    }
}

/// Finds every tool of the project at `root`, whose text files are `files`.
pub fn discover_tools(root: &Path, packages: &Packages, files: &[PathBuf]) -> Tools {
    let mut tools = Vec::new();
    let aliases = cargo_aliases(root);
    tools.extend(aliases.iter().map(|a| a.tool.clone()));
    let index = ClapIndex::build(packages);
    for package in &packages.packages {
        let bins: Vec<_> = package.targets.iter().filter(|t| t.kind == TargetKind::Bin).collect();
        for bin in &bins {
            let prefix = aliases
                .iter()
                .find(|a| a.runs_package.as_deref() == Some(package.name.as_str()) && bins.len() == 1)
                .map(|a| format!("cargo {}", a.tool.name))
                .unwrap_or_else(|| {
                    if bins.len() == 1 {
                        format!("cargo run -p {} --", package.name)
                    } else {
                        format!("cargo run -p {} --bin {} --", package.name, bin.name)
                    }
                });
            let mut tool = Tool {
                kind: ToolKind::Binary,
                name: bin.name.clone(),
                invocation: prefix.clone(),
                description: None,
                file: bin.src.clone(),
                line: 1,
                children: Vec::new(),
            };
            if let Some(parser) = index.parser_for(package, &bin.src) {
                tool.description = parser.about.clone();
                tool.file = parser.file.clone();
                tool.line = parser.line;
                if let Some(sub) = &parser.subcommand {
                    tool.children = index.subcommands(package, sub, &prefix, 0);
                }
            }
            tools.push(tool);
        }
        for example in package.targets.iter().filter(|t| t.kind == TargetKind::Example) {
            tools.push(Tool {
                kind: ToolKind::Example,
                name: example.name.clone(),
                invocation: format!("cargo run -p {} --example {}", package.name, example.name),
                description: None,
                file: example.src.clone(),
                line: 1,
                children: Vec::new(),
            });
        }
    }
    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let dir = file.parent().unwrap_or(root);
        let rel_dir = dir.strip_prefix(root).unwrap_or(Path::new(""));
        match name {
            "package.json" => tools.extend(npm_scripts(file, rel_dir)),
            "Makefile" | "makefile" | "GNUmakefile" => tools.extend(make_targets(file, rel_dir)),
            "justfile" | "Justfile" | ".justfile" => tools.extend(just_recipes(file, rel_dir)),
            _ if is_workflow(root, file) => tools.extend(workflow(file)),
            _ => {}
        }
    }
    Tools { tools }
}

/// The job that finds the tools for a [`crate::SourceHub`].
pub fn tools_job(
    packages: std::sync::Arc<Packages>,
    files: Vec<PathBuf>,
) -> impl FnOnce(&crate::JobContext) -> Result<(), crate::JobError> + Send {
    move |ctx| {
        let tools = discover_tools(&ctx.root, &packages, &files);
        ctx.send(crate::SourceEvent::Tools(std::sync::Arc::new(tools)));
        Ok(())
    }
}

fn is_workflow(root: &Path, file: &Path) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or_default();
    matches!(ext, "yml" | "yaml") && file.parent().is_some_and(|d| d == root.join(".github/workflows"))
}

/// `cd <dir> && ` for commands run from a folder other than the root.
fn in_dir(rel_dir: &Path, command: String) -> String {
    if rel_dir.as_os_str().is_empty() {
        command
    } else {
        format!("cd {} && {command}", rel_dir.to_string_lossy().replace('\\', "/"))
    }
}

/// The 1-based line of the first line containing `needle`.
fn line_of(text: &str, needle: &str) -> usize {
    text.lines().position(|l| l.contains(needle)).map_or(1, |i| i + 1)
}

struct Alias {
    tool: Tool,
    /// The package a `run --package <name> --` alias runs.
    runs_package: Option<String>,
}

/// `[alias]` entries of `.cargo/config.toml` (or `.cargo/config`).
fn cargo_aliases(root: &Path) -> Vec<Alias> {
    let Some((file, text)) = ["config.toml", "config"]
        .iter()
        .map(|n| root.join(".cargo").join(n))
        .find_map(|p| Some((p.clone(), std::fs::read_to_string(&p).ok()?)))
    else {
        return Vec::new();
    };
    let Ok(toml) = text.parse::<toml::Value>() else { return Vec::new() };
    let Some(table) = toml.get("alias").and_then(|a| a.as_table()) else { return Vec::new() };
    table
        .iter()
        .map(|(name, expansion)| {
            let words: Vec<String> = match expansion {
                toml::Value::String(s) => s.split_whitespace().map(str::to_string).collect(),
                toml::Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                _ => Vec::new(),
            };
            let runs_package = match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
                ["run", "--package" | "-p", package, "--"] => Some(package.to_string()),
                _ => None,
            };
            Alias {
                tool: Tool {
                    kind: ToolKind::CargoAlias,
                    name: name.clone(),
                    invocation: format!("cargo {name}"),
                    description: Some(format!("cargo {}", words.join(" "))),
                    file: file.clone(),
                    line: line_of(&text, name),
                    children: Vec::new(),
                },
                runs_package,
            }
        })
        .collect()
}

/// `scripts` of a package.json, in the order the file lists them.
fn npm_scripts(file: &Path, rel_dir: &Path) -> Vec<Tool> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else { return Vec::new() };
    let dir = file.parent().unwrap_or(Path::new("."));
    let runner = if dir.ancestors().any(|d| d.join("pnpm-lock.yaml").is_file()) {
        "pnpm run"
    } else if dir.ancestors().any(|d| d.join("yarn.lock").is_file()) {
        "yarn run"
    } else {
        "npm run"
    };
    scripts
        .iter()
        .map(|(name, command)| Tool {
            kind: ToolKind::NpmScript,
            name: name.clone(),
            invocation: in_dir(rel_dir, format!("{runner} {name}")),
            description: command.as_str().map(str::to_string),
            file: file.to_path_buf(),
            line: line_of(&text, &format!("\"{name}\"")),
            children: Vec::new(),
        })
        .collect()
}

/// A comment line directly above line `i` (0-based), without its `#`.
fn comment_above(lines: &[&str], i: usize) -> Option<String> {
    let above = lines.get(i.checked_sub(1)?)?.trim();
    let text = above.strip_prefix('#')?.trim_start_matches('#').trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Makefile targets: `name: ...` lines that are not variables, pattern rules or special targets.
/// A `## text` after the target, or a comment line above it, describes it.
fn make_targets(file: &Path, rel_dir: &Path) -> Vec<Tool> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        if rest.starts_with('=') || head.contains(['=', '%', '$', '(']) {
            continue;
        }
        let described = rest.split_once("##").map(|(_, d)| d.trim().to_string()).or_else(|| comment_above(&lines, i));
        for target in head.split_whitespace() {
            if seen.insert(target.to_string()) {
                out.push(Tool {
                    kind: ToolKind::MakeTarget,
                    name: target.to_string(),
                    invocation: in_dir(rel_dir, format!("make {target}")),
                    description: described.clone(),
                    file: file.to_path_buf(),
                    line: i + 1,
                    children: Vec::new(),
                });
            }
        }
    }
    out
}

/// justfile recipes: unindented `name args...:` lines that are not settings, aliases or
/// variables. A comment line above describes the recipe.
fn just_recipes(file: &Path, rel_dir: &Path) -> Vec<Tool> {
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with([' ', '\t', '#', '[']) || line.trim().is_empty() {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else { continue };
        if rest.starts_with('=') {
            continue;
        }
        let mut words = head.split_whitespace();
        let Some(first) = words.next() else { continue };
        if matches!(first, "set" | "alias" | "export" | "import" | "mod") {
            continue;
        }
        let name = first.trim_start_matches('@');
        if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            continue;
        }
        out.push(Tool {
            kind: ToolKind::JustRecipe,
            name: name.to_string(),
            invocation: in_dir(rel_dir, format!("just {name}")),
            description: comment_above(&lines, i),
            file: file.to_path_buf(),
            line: i + 1,
            children: Vec::new(),
        });
    }
    out
}

/// A GitHub Actions workflow and its jobs.
fn workflow(file: &Path) -> Option<Tool> {
    let text = std::fs::read_to_string(file).ok()?;
    let docs = yaml_rust2::YamlLoader::load_from_str(&text).ok()?;
    let doc = docs.first()?;
    let stem = file.file_stem()?.to_string_lossy().into_owned();
    let name = doc["name"].as_str().map(str::to_string).unwrap_or(stem);
    let triggers: Vec<String> = match &doc["on"] {
        yaml_rust2::Yaml::String(s) => vec![s.clone()],
        yaml_rust2::Yaml::Array(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        yaml_rust2::Yaml::Hash(h) => h.keys().filter_map(|k| k.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let jobs = doc["jobs"]
        .as_hash()
        .map(|jobs| {
            jobs.iter()
                .filter_map(|(key, job)| {
                    let key = key.as_str()?;
                    Some(Tool {
                        kind: ToolKind::WorkflowJob,
                        name: job["name"].as_str().unwrap_or(key).to_string(),
                        invocation: String::new(),
                        description: job["runs-on"].as_str().map(|r| format!("runs on {r}")),
                        file: file.to_path_buf(),
                        line: line_of(&text, &format!("{key}:")),
                        children: Vec::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Tool {
        kind: ToolKind::Workflow,
        name,
        invocation: String::new(),
        description: (!triggers.is_empty()).then(|| format!("on {}", triggers.join(", "))),
        file: file.to_path_buf(),
        line: 1,
        children: jobs,
    })
}

// ---- clap ---------------------------------------------------------------------------------------

/// A `#[derive(Parser)]` struct.
#[derive(Debug, Clone)]
struct ClapParser {
    /// The struct's name, for `Name::parse()` calls.
    ident: String,
    about: Option<String>,
    /// The type of its `#[command(subcommand)]` field.
    subcommand: Option<String>,
    file: PathBuf,
    line: usize,
}

/// A `#[derive(Subcommand)]` enum.
#[derive(Debug, Clone)]
struct ClapEnum {
    variants: Vec<ClapVariant>,
    file: PathBuf,
}

#[derive(Debug, Clone)]
struct ClapVariant {
    name: String,
    about: Option<String>,
    line: usize,
    /// The type holding its own subcommands: an enum, or an `Args` struct with a subcommand
    /// field.
    nested: Option<String>,
}

/// Every clap parser, subcommand enum and args struct, by package and type name.
#[derive(Default)]
struct ClapIndex {
    /// Package name → its parsers.
    parsers: HashMap<String, Vec<ClapParser>>,
    /// Package name → type name → enum.
    enums: HashMap<String, HashMap<String, ClapEnum>>,
    /// Package name → `Args` struct name → its subcommand field's type.
    args: HashMap<String, HashMap<String, String>>,
    /// Package name → packages it depends on, directly or not.
    closure: HashMap<String, Vec<String>>,
}

/// Most levels of nested subcommands followed.
const MAX_DEPTH: usize = 4;

impl ClapIndex {
    /// Reads the clap types of every package that builds a binary, and of everything they
    /// depend on.
    fn build(packages: &Packages) -> Self {
        let mut index = ClapIndex::default();
        let by_name: HashMap<&str, &Package> = packages.packages.iter().map(|p| (p.name.as_str(), p)).collect();
        let mut wanted = BTreeSet::new();
        for p in packages.packages.iter().filter(|p| p.targets.iter().any(|t| t.kind == TargetKind::Bin)) {
            let mut closure = Vec::new();
            let mut stack = vec![p.name.as_str()];
            let mut seen = BTreeSet::new();
            while let Some(name) = stack.pop() {
                if !seen.insert(name) {
                    continue;
                }
                closure.push(name.to_string());
                if let Some(dep) = by_name.get(name) {
                    stack.extend(dep.depends_on.iter().map(String::as_str));
                }
            }
            wanted.extend(closure.iter().cloned());
            index.closure.insert(p.name.clone(), closure);
        }
        for name in wanted {
            let Some(package) = by_name.get(name.as_str()) else { continue };
            index.read_package(package);
        }
        index
    }

    fn read_package(&mut self, package: &Package) {
        let walker = walkdir::WalkDir::new(&package.root).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && (n == "target" || n.starts_with('.') && e.depth() > 0 || n == "node_modules"))
        });
        for entry in walker.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            // Cheap check first: most files never mention clap.
            if !(text.contains("Subcommand") || text.contains("Parser") || text.contains("Args")) {
                continue;
            }
            let Ok(file) = syn::parse_file(&text) else { continue };
            self.read_items(&package.name, path, &file.items);
        }
    }

    fn read_items(&mut self, package: &str, path: &Path, items: &[syn::Item]) {
        for item in items {
            match item {
                syn::Item::Struct(s) if derives(&s.attrs, "Parser") => {
                    self.parsers.entry(package.to_string()).or_default().push(ClapParser {
                        ident: s.ident.to_string(),
                        about: command_about(&s.attrs).or_else(|| doc_line(&s.attrs)),
                        subcommand: subcommand_field(&s.fields),
                        file: path.to_path_buf(),
                        line: s.ident.span().start().line,
                    });
                    if let Some(sub) = subcommand_field(&s.fields) {
                        self.args.entry(package.to_string()).or_default().insert(s.ident.to_string(), sub);
                    }
                }
                syn::Item::Struct(s) if derives(&s.attrs, "Args") => {
                    if let Some(sub) = subcommand_field(&s.fields) {
                        self.args.entry(package.to_string()).or_default().insert(s.ident.to_string(), sub);
                    }
                }
                syn::Item::Enum(e) if derives(&e.attrs, "Subcommand") => {
                    let variants = e
                        .variants
                        .iter()
                        .filter(|v| !v.attrs.iter().any(|a| a.path().is_ident("command") && tokens(a).contains("skip")))
                        .map(|v| ClapVariant {
                            name: command_name(&v.attrs).unwrap_or_else(|| kebab_case(&v.ident.to_string())),
                            about: doc_line(&v.attrs),
                            line: v.ident.span().start().line,
                            nested: match &v.fields {
                                syn::Fields::Named(_) => subcommand_field(&v.fields),
                                syn::Fields::Unnamed(u) if u.unnamed.len() == 1 => type_name(&u.unnamed[0].ty),
                                _ => None,
                            },
                        })
                        .collect();
                    self.enums
                        .entry(package.to_string())
                        .or_default()
                        .insert(e.ident.to_string(), ClapEnum { variants, file: path.to_path_buf() });
                }
                syn::Item::Mod(m) => {
                    if let Some((_, items)) = &m.content {
                        self.read_items(package, path, items);
                    }
                }
                _ => {}
            }
        }
    }

    /// The parser a binary uses: one defined in the binary's own file, else the one it calls
    /// `::parse()` on, else for `src/main.rs` the package's only parser outside `src/bin`.
    fn parser_for(&self, package: &Package, bin_src: &Path) -> Option<&ClapParser> {
        let parsers = self.parsers.get(&package.name)?;
        if let Some(found) = parsers.iter().find(|p| p.file == bin_src) {
            return Some(found);
        }
        if let Ok(text) = std::fs::read_to_string(bin_src) {
            let called = |p: &&ClapParser| {
                text.contains(&format!("{}::parse", p.ident)) || text.contains(&format!("{}::try_parse", p.ident))
            };
            if let Some(found) = parsers.iter().find(called) {
                return Some(found);
            }
        }
        if bin_src.ends_with("src/main.rs") {
            let bin_dir = package.root.join("src/bin");
            let mut outside = parsers.iter().filter(|p| !p.file.starts_with(&bin_dir));
            if let (Some(only), None) = (outside.next(), outside.next()) {
                return Some(only);
            }
        }
        None
    }

    /// The enum named `ty` as `package` sees it: its own first, then the packages it depends on.
    fn find_enum(&self, package: &str, ty: &str) -> Option<&ClapEnum> {
        let closure = self.closure.get(package)?;
        closure.iter().find_map(|p| self.enums.get(p)?.get(ty))
    }

    fn find_args(&self, package: &str, ty: &str) -> Option<&String> {
        let closure = self.closure.get(package)?;
        closure.iter().find_map(|p| self.args.get(p)?.get(ty))
    }

    /// The subcommands of enum `ty`, nested, each with the full command line that runs it.
    fn subcommands(&self, package: &Package, ty: &str, prefix: &str, depth: usize) -> Vec<Tool> {
        let Some(found) = self.find_enum(&package.name, ty) else { return Vec::new() };
        found
            .variants
            .iter()
            .map(|v| {
                let invocation = format!("{prefix} {}", v.name);
                let nested = v.nested.as_deref().and_then(|t| {
                    if self.find_enum(&package.name, t).is_some() {
                        Some(t.to_string())
                    } else {
                        self.find_args(&package.name, t).cloned()
                    }
                });
                let children = match nested {
                    Some(t) if depth < MAX_DEPTH => self.subcommands(package, &t, &invocation, depth + 1),
                    _ => Vec::new(),
                };
                Tool {
                    kind: ToolKind::Subcommand,
                    name: v.name.clone(),
                    invocation,
                    description: v.about.clone(),
                    file: found.file.clone(),
                    line: v.line,
                    children,
                }
            })
            .collect()
    }
}

fn tokens(attr: &syn::Attribute) -> String {
    use quote::ToTokens;
    attr.meta.to_token_stream().to_string()
}

/// Whether `#[derive(..)]` lists `what` (by its last path segment).
fn derives(attrs: &[syn::Attribute], what: &str) -> bool {
    attrs.iter().filter(|a| a.path().is_ident("derive")).any(|a| {
        a.parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
            .is_ok_and(|paths| paths.iter().any(|p| p.segments.last().is_some_and(|s| s.ident == what)))
    })
}

/// A string argument of `#[command(..)]`: `name = "x"`.
fn command_arg(attrs: &[syn::Attribute], key: &str) -> Option<String> {
    attrs.iter().filter(|a| a.path().is_ident("command") || a.path().is_ident("clap")).find_map(|a| {
        let text = tokens(a);
        let at = text.find(&format!("{key} = \""))? + key.len() + 4;
        let end = text[at..].find('"')?;
        Some(text[at..at + end].to_string())
    })
}

fn command_name(attrs: &[syn::Attribute]) -> Option<String> {
    command_arg(attrs, "name")
}

fn command_about(attrs: &[syn::Attribute]) -> Option<String> {
    command_arg(attrs, "about")
}

/// The first line of the doc comment.
fn doc_line(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().filter(|a| a.path().is_ident("doc")).find_map(|a| match &a.meta {
        syn::Meta::NameValue(nv) => match &nv.value {
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => {
                let line = s.value().trim().to_string();
                (!line.is_empty()).then_some(line)
            }
            _ => None,
        },
        _ => None,
    })
}

/// The type name of the field marked `#[command(subcommand)]`.
fn subcommand_field(fields: &syn::Fields) -> Option<String> {
    fields.iter().find(|f| f.attrs.iter().any(|a| tokens(a).contains("subcommand"))).and_then(|f| type_name(&f.ty))
}

/// The last segment of a type path, through `Option<..>` and `Box<..>`.
fn type_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if matches!(last.ident.to_string().as_str(), "Option" | "Box") {
        if let syn::PathArguments::AngleBracketed(args) = &last.arguments {
            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                return type_name(inner);
            }
        }
    }
    Some(last.ident.to_string())
}

/// clap's default subcommand name: `RegistryGet` → `registry-get`, `HTTPServer` → `http-server`.
pub fn kebab_case(ident: &str) -> String {
    let chars: Vec<char> = ident.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev_lower = i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let prev_upper = i > 0 && chars[i - 1].is_uppercase();
            if i > 0 && (prev_lower || (prev_upper && next_lower)) {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else if c == '_' {
            out.push('-');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{CancelToken, Runner};

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn kebab_case_matches_clap() {
        assert_eq!(kebab_case("RegistryGet"), "registry-get");
        assert_eq!(kebab_case("Ai"), "ai");
        assert_eq!(kebab_case("HTTPServer"), "http-server");
        assert_eq!(kebab_case("Build2Go"), "build2-go");
    }

    /// An xtask whose subcommands come partly from another crate, a Makefile, a justfile, npm
    /// scripts and a workflow.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        write(r, "Cargo.toml", "[workspace]\nmembers = [\"xtask\", \"deploy\"]\nresolver = \"2\"\n");
        write(r, ".cargo/config.toml", "[alias]\nxtask = \"run --package xtask --\"\n");
        write(r, "deploy/Cargo.toml", "[package]\nname = \"deploy\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
        write(
            r,
            "deploy/src/lib.rs",
            "#[derive(clap::Subcommand)]\npub enum DeployCmd {\n    /// Ship the website\n    Website,\n    /// Ship the game server\n    GameServer { region: String },\n}\n",
        );
        write(
            r,
            "xtask/Cargo.toml",
            "[package]\nname = \"xtask\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ndeploy = { path = \"../deploy\" }\n",
        );
        write(r, "xtask/src/main.rs", "mod cli;\nfn main() { let _ = cli::Cli::parse(); }\n");
        write(
            r,
            "xtask/src/cli/mod.rs",
            r#"use clap::{Parser, Subcommand};
#[derive(Parser)]
#[command(name = "xtask", about = "Workspace tasks")]
pub struct Cli {
    #[command(subcommand)]
    cmd: TopCmd,
}
#[derive(Subcommand)]
enum TopCmd {
    /// Deploy things
    Deploy {
        #[command(subcommand)]
        cmd: deploy::DeployCmd,
    },
    /// Check the tree
    Verify(VerifyArgs),
    #[command(name = "registry-get")]
    RegistryGet { field: String },
}
#[derive(clap::Args)]
struct VerifyArgs {
    #[command(subcommand)]
    what: Option<VerifyCmd>,
}
#[derive(Subcommand)]
enum VerifyCmd { Contracts, Links }
"#,
        );
        write(r, "Makefile", ".PHONY: build\n# Build everything\nbuild: deps\n\tcargo build\ntest: ## Run the tests\n\tcargo test\nVERSION := 1\n%.o: %.c\n");
        write(
            r,
            "justfile",
            "set shell := [\"bash\"]\n# Serve the site\nserve port='8080':\n    echo\nalias s := serve\n",
        );
        write(
            r,
            "web/package.json",
            "{\"name\": \"web\", \"scripts\": {\"dev\": \"vite\", \"build\": \"vite build\", \"lint\": \"eslint .\"}}",
        );
        write(r, ".github/workflows/ci.yml", "name: CI\non: [push, pull_request]\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps: []\n  docs:\n    name: Build docs\n    runs-on: ubuntu-latest\n    steps: []\n");
        dir
    }

    fn files(root: &Path) -> Vec<PathBuf> {
        ["Makefile", "justfile", "web/package.json", ".github/workflows/ci.yml"].map(|f| root.join(f)).into()
    }

    #[test]
    fn every_kind_of_tool_is_found_with_its_exact_command() {
        let dir = fixture();
        let r = dir.path();
        let manifests: Vec<PathBuf> = ["Cargo.toml", "xtask/Cargo.toml", "deploy/Cargo.toml"].map(|m| r.join(m)).into();
        let packages = crate::packages::read_packages(&Runner::new(CancelToken::default()), &manifests);
        let tools = discover_tools(r, &packages, &files(r));
        let find = |name: &str| tools.all().find(|t| t.name == name).unwrap_or_else(|| panic!("no {name}")).clone();

        let xtask = find("xtask");
        let xtask = if xtask.kind == ToolKind::CargoAlias {
            tools.all().find(|t| t.kind == ToolKind::Binary).unwrap().clone()
        } else {
            xtask
        };
        assert_eq!(xtask.invocation, "cargo xtask", "the alias that runs it");
        assert_eq!(xtask.description.as_deref(), Some("Workspace tasks"));
        let names: Vec<&str> = xtask.children.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["deploy", "verify", "registry-get"]);
        let deploy = &xtask.children[0];
        assert_eq!(
            deploy.children.iter().map(|t| t.invocation.as_str()).collect::<Vec<_>>(),
            ["cargo xtask deploy website", "cargo xtask deploy game-server"],
            "found in the crate it depends on"
        );
        assert_eq!(deploy.children[0].description.as_deref(), Some("Ship the website"));
        assert_eq!(xtask.children[1].children.len(), 2, "through the Args struct");

        assert_eq!(find("build").invocation, "make build");
        assert_eq!(find("build").description.as_deref(), Some("Build everything"));
        assert_eq!(find("test").description.as_deref(), Some("Run the tests"));
        assert_eq!(tools.count(ToolKind::MakeTarget), 2, "no variables, pattern rules or .PHONY");
        assert_eq!(find("serve").invocation, "just serve");
        assert_eq!(tools.count(ToolKind::JustRecipe), 1);
        let scripts: Vec<String> =
            tools.all().filter(|t| t.kind == ToolKind::NpmScript).map(|t| t.name.clone()).collect();
        assert_eq!(scripts, ["dev", "build", "lint"], "in file order");
        assert_eq!(find("dev").invocation, "cd web && npm run dev");
        let ci = find("CI");
        assert_eq!(ci.description.as_deref(), Some("on push, pull_request"));
        assert_eq!(ci.children.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(), ["check", "Build docs"]);
    }
}
