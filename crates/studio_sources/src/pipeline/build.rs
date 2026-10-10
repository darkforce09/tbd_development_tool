//! Steps and links: the route table, the tags, the programs and commands, and the calls between them. See the
//! module doc of [`crate::pipeline`] for the rules.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;
use studio_graph::{Basis, EvidenceTier, Provenance};
use studio_parser::analysis::axum::{evaluate_routes, RouteEntry, RouteTable};
use studio_parser::analysis::contracts::{pointer_line, resolve_contract, ContractFiles, ContractResolution};
use studio_parser::analysis::crate_graph::{normalize, CrateGraph, TargetKind};
use studio_parser::analysis::route_template::RouteTemplate;
use studio_parser::analysis::rust_resolver::{ItemKind, ItemRef, Resolution, RustIndex};
use studio_parser::analysis::tags::{owner_spans, scan_tags, ContractTag, OwnerSpan, RouteTag, TagOwner};
use studio_parser::extractor::{detect_language, detect_language_by_path, extract_source, SourceLang};

use super::model::{Link, LinkKind, Step, StepKind};
use crate::tools::{ToolKind, Tools};

/// What the builder found before ordering: steps and links in discovery order, and the counts for the stats.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub steps: Vec<Step>,
    pub links: Vec<Link>,
    /// Step indices of the entry points: route table routes, mains, commands.
    pub entries: Vec<usize>,
    pub routes: usize,
    pub route_tags: usize,
    pub contract_tags: usize,
    pub handler_tags_agree: usize,
    pub handler_tags_disagree: usize,
    pub timings: Timings,
    /// Every contract tag's Unresolved reason (verbatim) with how many tags have it.
    pub unresolved_contracts: BTreeMap<String, usize>,
}

/// Milliseconds per phase, for the report.
#[derive(Debug, Default, Clone, Copy)]
pub struct Timings {
    pub index_ms: u64,
    pub routes_ms: u64,
    pub tags_ms: u64,
    pub calls_ms: u64,
    pub contracts_ms: u64,
}

/// How a step is known, so that one item is one step.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    /// A function or method: (relative file, line of its name).
    Item(PathBuf, usize),
    /// A route of the route table: (title, `.route(` file, line).
    Route(String, PathBuf, usize),
    /// A handler that does not resolve: (text, `.route(` file, line).
    LooseHandler(String, PathBuf, usize),
    /// A route only a tag declares: (title, tag file, line).
    TagRoute(String, PathBuf, usize),
    /// A contract: (relative file, pointer).
    Contract(PathBuf, Option<String>),
    /// A contract tag that resolves to no file: (tag file, line, text).
    LooseContract(PathBuf, usize, String),
    /// A sender owned by its whole file: (file, tag line).
    FileSender(PathBuf, usize),
    Command(PathBuf, usize, String),
}

struct Builder<'a> {
    root: &'a Path,
    /// The root as given and canonicalized, to make paths relative.
    roots: Vec<PathBuf>,
    steps: Vec<Step>,
    keys: HashMap<Key, usize>,
    links: Vec<Link>,
    link_keys: HashMap<(usize, usize, LinkKind), usize>,
    /// Steps of Rust functions: (absolute file, line, function name).
    functions: HashMap<usize, (PathBuf, usize, String)>,
}

/// The rank of a step kind when one item plays several parts: the highest wins. A code role (handler, program,
/// function) always outranks Sender: a route tag adds `Requests` links from a step, it never changes what it is.
fn rank(kind: StepKind) -> u8 {
    match kind {
        StepKind::Handler => 4,
        StepKind::Main => 3,
        StepKind::Function => 2,
        StepKind::Sender => 1,
        _ => 0,
    }
}

/// The note a step's detail gets when it plays another part and also sends requests.
const ALSO_SENDER: &str = " · also a sender";

/// Unresolved, from `basis`.
fn unresolved(basis: Basis) -> Provenance {
    Provenance { tier: EvidenceTier::Unresolved, basis, candidates: 0 }
}

/// The display name of a language.
pub(crate) fn language_name(lang: SourceLang, path: &Path) -> String {
    match lang {
        SourceLang::Rust => "Rust".into(),
        SourceLang::Enforce => "Enforce".into(),
        SourceLang::Markdown => "Markdown".into(),
        SourceLang::Code(l) => l.name().into(),
        SourceLang::Other => match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
            "json" => "JSON".into(),
            "toml" => "TOML".into(),
            "yml" | "yaml" => "YAML".into(),
            _ => "Text".into(),
        },
    }
}

fn file_line(path: &Path, line: usize) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    format!("{name}:{line}")
}

impl<'a> Builder<'a> {
    fn new(root: &'a Path) -> Self {
        let mut roots = vec![normalize(root)];
        if let Ok(c) = root.canonicalize() {
            if c != roots[0] {
                roots.push(c);
            }
        }
        Builder {
            root,
            roots,
            steps: Vec::new(),
            keys: HashMap::new(),
            links: Vec::new(),
            link_keys: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    fn rel(&self, path: &Path) -> PathBuf {
        let path = normalize(path);
        for r in &self.roots {
            if let Ok(rel) = path.strip_prefix(r) {
                return rel.to_path_buf();
            }
        }
        path
    }

    fn abs(&self, path: &Path) -> PathBuf {
        normalize(&self.root.join(path))
    }

    /// The step for `key`, created from `make` when new. An item step takes the highest-ranked kind it is seen as.
    fn step(&mut self, key: Key, make: impl FnOnce() -> Step) -> usize {
        if let Some(&i) = self.keys.get(&key) {
            return i;
        }
        let i = self.steps.len();
        self.steps.push(make());
        self.keys.insert(key, i);
        i
    }

    /// The step of a Rust item, as `kind` (upgraded when the item already has a lower-ranked step).
    fn item_step(&mut self, item: &ItemRef, kind: StepKind, title: Option<&str>) -> usize {
        let path = self.rel(&item.file);
        let key = Key::Item(path.clone(), item.line);
        let i = self.step(key, || Step {
            kind,
            title: title.unwrap_or(&item.name).to_string(),
            detail: file_line(&path, item.line),
            language: "Rust".into(),
            path,
            line: item.line,
            line_end: Some(item.line_end),
        });
        self.functions.entry(i).or_insert_with(|| (normalize(&item.file), item.line, item.name.clone()));
        if rank(kind) > rank(self.steps[i].kind) {
            self.steps[i].kind = kind;
            if let Some(t) = title {
                self.steps[i].title = t.to_string();
            }
        }
        i
    }

    /// Adds a link, or merges it into the same (from, to, kind) link: the stronger tier wins, evidence is united.
    /// At an equal tier the conditions merge: unconditional wins, different conditions join with " || ".
    fn link(&mut self, link: Link) -> usize {
        let key = (link.from, link.to, link.kind);
        if let Some(&i) = self.link_keys.get(&key) {
            let old = &mut self.links[i];
            let new_rank = (link.provenance.tier, link.provenance.candidates);
            let old_rank = (old.provenance.tier, old.provenance.candidates);
            if new_rank < old_rank {
                old.provenance = link.provenance;
                old.label = link.label;
                old.conditional = link.conditional;
            } else if new_rank == old_rank {
                old.conditional = either(old.conditional.take(), link.conditional);
            }
            for e in link.evidence {
                if !old.evidence.contains(&e) {
                    old.evidence.push(e);
                }
            }
            old.evidence.sort();
            return i;
        }
        let i = self.links.len();
        self.links.push(link);
        self.link_keys.insert(key, i);
        i
    }

    fn crossing(&self, from: usize, to: usize) -> Option<(String, String)> {
        let (a, b) = (&self.steps[from].language, &self.steps[to].language);
        (a != b).then(|| (a.clone(), b.clone()))
    }
}

/// The condition under which either of two conditional things holds: none when either is unconditional, else
/// the alternatives of both, sorted and deduplicated, joined with " || ".
fn either(a: Option<String>, b: Option<String>) -> Option<String> {
    let (a, b) = (a?, b?);
    let mut parts: Vec<&str> = a.split(" || ").chain(b.split(" || ")).collect();
    parts.sort_unstable();
    parts.dedup();
    Some(parts.join(" || "))
}

/// A file's text, from disk.
fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Builds every step and link. Returns `None` when `cancelled` says stop.
pub(crate) fn build(
    root: &Path,
    graph: &CrateGraph,
    tools: &Tools,
    files: &[PathBuf],
    cancelled: &dyn Fn() -> bool,
) -> Option<Found> {
    let mut found = Found::default();
    let mut b = Builder::new(root);
    let files: BTreeSet<PathBuf> = files.iter().map(|f| b.abs(f)).collect();

    // 1. The Rust index and the route table.
    let started = std::time::Instant::now();
    let index = RustIndex::build(graph, &read);
    found.timings.index_ms = started.elapsed().as_millis() as u64;
    if cancelled() {
        return None;
    }
    let started = std::time::Instant::now();
    let table = evaluate_routes(&index, graph, &read);
    found.timings.routes_ms = started.elapsed().as_millis() as u64;
    found.routes = table.entries.len();
    let route_steps = add_routes(&mut b, &table);
    found.entries.extend(route_steps.iter().copied());

    // 2. Programs: every binary's `fn main`.
    for node in graph.crates.iter().filter(|c| c.kind == TargetKind::Bin) {
        let root_file = normalize(&node.root_file);
        let Some(module) = index.module_of_file(&root_file) else { continue };
        if let Resolution::Item(item) = index.resolve(module, &["main"]) {
            if item.kind == ItemKind::Fn && item.file == root_file {
                let i = b.item_step(&item, StepKind::Main, Some(&node.name));
                if !found.entries.contains(&i) {
                    found.entries.push(i);
                }
            }
        }
    }

    // 3. Mounts: the root Router function (or the function holding the serve site) mounts each route it evaluated.
    let mut fns_cache: BTreeMap<PathBuf, Vec<ItemRef>> = BTreeMap::new();
    for (entry, &route) in table.entries.iter().zip(&route_steps) {
        let (file, line) = &entry.root;
        let fns = fns_cache.entry(file.clone()).or_insert_with(|| index.functions_in(file));
        let Some(holder) =
            fns.iter().filter(|f| f.line <= *line && *line <= f.line_end).min_by_key(|f| f.line_end - f.line).cloned()
        else {
            continue;
        };
        let from = b.item_step(&holder, StepKind::Function, None);
        let evidence = vec![(b.rel(file), *line), (b.rel(&entry.defined_at.0), entry.defined_at.1)];
        b.link(Link {
            from,
            to: route,
            kind: LinkKind::Mounts,
            provenance: Provenance::proven(Basis::RouteTable),
            conditional: entry.conditional.clone(),
            crossing: None,
            label: String::new(),
            evidence,
        });
    }
    if cancelled() {
        return None;
    }

    // 4. Tags: only files holding the bytes `@route` or `@contract` are extracted.
    let started = std::time::Instant::now();
    let tagged = scan_project_tags(&index, &files);
    let routes_tags: Vec<&RouteTag> = tagged.iter().flat_map(|t| &t.routes).collect();
    let contract_tags: Vec<&ContractTag> = tagged.iter().flat_map(|t| &t.contracts).collect();
    found.route_tags = routes_tags.len();
    found.contract_tags = contract_tags.len();
    let langs: BTreeMap<PathBuf, String> = tagged.iter().map(|t| (t.file.clone(), t.language.clone())).collect();
    add_route_tags(&mut b, &table, &route_steps, &routes_tags, &contract_tags, &langs, &mut found);
    found.timings.tags_ms = started.elapsed().as_millis() as u64;
    if cancelled() {
        return None;
    }

    // 5. Commands the project defines in code or cargo configuration.
    for tool in tools.tools.iter().flat_map(|t| t.walk()) {
        if !matches!(tool.kind, ToolKind::Subcommand | ToolKind::CargoAlias) || tool.line == 0 {
            continue;
        }
        if tool.file.as_os_str().is_empty() {
            continue;
        }
        let path = b.rel(&b.abs(&tool.file));
        let title = if tool.invocation.is_empty() { tool.name.clone() } else { tool.invocation.clone() };
        let language = language_name(detect_language_by_path(&path), &path);
        let key = Key::Command(path.clone(), tool.line, title.clone());
        let i = b.step(key, || Step {
            kind: StepKind::Command,
            title,
            detail: file_line(&path, tool.line),
            language,
            path,
            line: tool.line,
            line_end: None,
        });
        if !found.entries.contains(&i) {
            found.entries.push(i);
        }
    }

    // 6. Calls from handlers, programs and the functions they reach, through R1.
    let started = std::time::Instant::now();
    add_calls(&mut b, &index);
    found.timings.calls_ms = started.elapsed().as_millis() as u64;
    if cancelled() {
        return None;
    }

    // 7. Contracts: declared by the steps that own the tags.
    let started = std::time::Instant::now();
    let json: Vec<PathBuf> =
        files.iter().filter(|f| f.extension().is_some_and(|e| e == "json")).map(|f| b.rel(f)).collect();
    found.unresolved_contracts = add_contracts(&mut b, &contract_tags, &ContractFiles::new(json));
    found.timings.contracts_ms = started.elapsed().as_millis() as u64;

    // A step that is a handler, program or function and also requests a route says so in its detail.
    let requesters: BTreeSet<usize> = b.links.iter().filter(|l| l.kind == LinkKind::Requests).map(|l| l.from).collect();
    for i in requesters {
        let step = &mut b.steps[i];
        if step.kind != StepKind::Sender && !step.detail.ends_with(ALSO_SENDER) {
            step.detail.push_str(ALSO_SENDER);
        }
    }

    found.steps = b.steps;
    found.links = b.links;
    Some(found)
}

/// One Route step per route table entry and one Handler step per handler, with the Serves links.
fn add_routes(b: &mut Builder, table: &RouteTable) -> Vec<usize> {
    let mut out = Vec::with_capacity(table.entries.len());
    for entry in &table.entries {
        let title = format!("{} {}", entry.method, entry.template);
        let path = b.rel(&entry.defined_at.0);
        let line = entry.defined_at.1;
        let key = Key::Route(title.clone(), path.clone(), line);
        let route = b.step(key, || Step {
            kind: StepKind::Route,
            title,
            detail: file_line(&path, line),
            language: "Rust".into(),
            path: path.clone(),
            line,
            line_end: None,
        });
        out.push(route);
        let evidence = vec![(path.clone(), line)];
        let (handler, provenance, label) = match &entry.handler {
            Resolution::Item(item) => {
                (b.item_step(item, StepKind::Handler, None), Provenance::proven(Basis::RouteTable), String::new())
            }
            other => {
                let text = entry.handler_text.clone();
                let key = Key::LooseHandler(text.clone(), path.clone(), line);
                let detail = format!("not resolved: {}", reason(other));
                let i = b.step(key, || Step {
                    kind: StepKind::Handler,
                    title: text.clone(),
                    detail,
                    language: "Rust".into(),
                    path: path.clone(),
                    line,
                    line_end: None,
                });
                (i, unresolved(Basis::RouteTable), text)
            }
        };
        b.link(Link {
            from: route,
            to: handler,
            kind: LinkKind::Serves,
            provenance,
            conditional: entry.conditional.clone(),
            crossing: None,
            label,
            evidence,
        });
    }
    out
}

fn reason(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Item(_) => "resolved".into(),
        Resolution::External(path) => format!("outside the workspace ({path})"),
        Resolution::Ambiguous(c) => format!("{} candidates", c.len()),
        Resolution::Unresolved(r) => format!("{r:?}").to_lowercase(),
    }
}

/// The tags of one file.
struct Tagged {
    file: PathBuf,
    language: String,
    routes: Vec<RouteTag>,
    contracts: Vec<ContractTag>,
}

/// Scans the code files that contain `@route` or `@contract`. Owners: R1's functions for Rust files in the index,
/// the language extractor's items otherwise. Sorted by file.
fn scan_project_tags(index: &RustIndex, files: &BTreeSet<PathBuf>) -> Vec<Tagged> {
    let files: Vec<&PathBuf> = files.iter().collect();
    let mut out: Vec<Tagged> = files
        .par_iter()
        .filter(|file| !matches!(detect_language_by_path(file), SourceLang::Markdown | SourceLang::Other))
        .filter_map(|file| {
            let bytes = std::fs::read(file).ok()?;
            if !has_tag_bytes(&bytes) {
                return None;
            }
            let text = String::from_utf8(bytes).ok()?;
            let lang = detect_language(file, &text);
            if matches!(lang, SourceLang::Markdown | SourceLang::Other) {
                return None;
            }
            let owners: Vec<OwnerSpan> = if lang == SourceLang::Rust && index.module_of_file(file).is_some() {
                index
                    .functions_in(file)
                    .into_iter()
                    .map(|f| OwnerSpan { name: f.name, line: f.line, line_end: f.line_end })
                    .collect()
            } else {
                owner_spans(&extract_source(file, file, &text))
            };
            let tags = scan_tags(file, &text, lang, &owners);
            if tags.routes.is_empty() && tags.contracts.is_empty() {
                return None;
            }
            Some(Tagged {
                file: (*file).clone(),
                language: language_name(lang, file),
                routes: tags.routes,
                contracts: tags.contracts,
            })
        })
        .collect();
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// The cheap prefilter: the bytes `@route` or `@contract` appear somewhere.
fn has_tag_bytes(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .any(|(i, &c)| c == b'@' && (bytes[i + 1..].starts_with(b"route") || bytes[i + 1..].starts_with(b"contract")))
}

/// Every (method, template) alternative of a route tag.
fn alternatives(tag: &RouteTag) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in &tag.methods {
        for t in &tag.templates {
            out.push((m.to_uppercase(), t.clone()));
        }
    }
    out
}

fn method_matches(tag: &str, entry: &str) -> bool {
    tag == entry || entry == "ANY" || tag == "ANY"
}

fn owner_line(owner: &TagOwner) -> Option<usize> {
    match owner {
        TagOwner::Item(span) => Some(span.line),
        TagOwner::File => None,
    }
}

/// Handler tags (counted) and senders (steps with Requests links).
fn add_route_tags(
    b: &mut Builder,
    table: &RouteTable,
    route_steps: &[usize],
    tags: &[&RouteTag],
    contracts: &[&ContractTag],
    langs: &BTreeMap<PathBuf, String>,
    found: &mut Found,
) {
    // Handlers by (file, line), with their entries.
    let mut handlers: BTreeMap<(PathBuf, usize), Vec<&RouteEntry>> = BTreeMap::new();
    for entry in &table.entries {
        if let Resolution::Item(item) = &entry.handler {
            handlers.entry((normalize(&item.file), item.line)).or_default().push(entry);
        }
    }
    let templates: Vec<RouteTemplate> = table.entries.iter().map(|e| RouteTemplate::parse(&e.template)).collect();

    for tag in tags {
        let file = normalize(&tag.file);
        if let Some(entries) = owner_line(&tag.owner).and_then(|l| handlers.get(&(file.clone(), l))) {
            let agrees = alternatives(tag).iter().all(|(m, t)| {
                let t = RouteTemplate::parse(t);
                entries.iter().any(|e| method_matches(m, &e.method) && t.unifies(&RouteTemplate::parse(&e.template)))
            });
            if agrees {
                found.handler_tags_agree += 1;
            } else {
                found.handler_tags_disagree += 1;
            }
            continue;
        }

        // A sender: the owner item (keeping the kind it already has, e.g. a program), or the file at the tag's line.
        let path = b.rel(&file);
        let language = langs.get(&file).cloned().unwrap_or_else(|| "Text".into());
        let sender = match &tag.owner {
            TagOwner::Item(span) => {
                let key = Key::Item(path.clone(), span.line);
                b.step(key, || Step {
                    kind: StepKind::Sender,
                    title: span.name.clone(),
                    detail: file_line(&path, span.line),
                    language: language.clone(),
                    path: path.clone(),
                    line: span.line,
                    line_end: Some(span.line_end),
                })
            }
            TagOwner::File => {
                let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                b.step(Key::FileSender(path.clone(), tag.line), || Step {
                    kind: StepKind::Sender,
                    title,
                    detail: file_line(&path, tag.line),
                    language: language.clone(),
                    path: path.clone(),
                    line: tag.line,
                    line_end: None,
                })
            }
        };
        // The contracts the same owner declares, as the link's label.
        let label = contracts
            .iter()
            .filter(|c| normalize(&c.file) == file && owner_line(&c.owner) == owner_line(&tag.owner))
            .map(|c| contract_text(c))
            .collect::<Vec<_>>()
            .join(", ");

        for (method, template) in alternatives(tag) {
            let t = RouteTemplate::parse(&template);
            let mut matches: Vec<usize> = Vec::new();
            for (ei, entry) in table.entries.iter().enumerate() {
                if method_matches(&method, &entry.method) && t.unifies(&templates[ei]) {
                    matches.push(ei);
                }
            }
            // Candidates are Route steps: entries of one route (evaluated from two roots, or under two conditions)
            // all link to it, so the link's condition is that of every entry.
            let n = matches.iter().map(|&ei| route_steps[ei]).collect::<BTreeSet<_>>().len();
            let tag_evidence = (path.clone(), tag.line);
            let resolved = n == 1 && matches.iter().all(|&ei| matches!(table.entries[ei].handler, Resolution::Item(_)));
            if resolved || n > 1 {
                for ei in matches {
                    let entry = &table.entries[ei];
                    let to = route_steps[ei];
                    let provenance = if n == 1 {
                        Provenance::proven(Basis::RouteTable)
                    } else {
                        Provenance::possible(Basis::RouteTable, n.min(u16::MAX as usize) as u16)
                    };
                    let evidence = vec![tag_evidence.clone(), (b.rel(&entry.defined_at.0), entry.defined_at.1)];
                    let crossing = b.crossing(sender, to);
                    b.link(Link {
                        from: sender,
                        to,
                        kind: LinkKind::Requests,
                        provenance,
                        conditional: entry.conditional.clone(),
                        crossing,
                        label: label.clone(),
                        evidence,
                    });
                }
                continue;
            }
            // No route, or one whose handler does not resolve: a Route step for the tag itself.
            let title = format!("{method} {template}");
            let detail = if matches.is_empty() {
                "declared by tag; no route in the route table".to_string()
            } else {
                "declared by tag; the route's handler does not resolve".to_string()
            };
            let key = Key::TagRoute(title.clone(), path.clone(), tag.line);
            let to = b.step(key, || Step {
                kind: StepKind::Route,
                title,
                detail,
                language: language.clone(),
                path: path.clone(),
                line: tag.line,
                line_end: None,
            });
            let crossing = b.crossing(sender, to);
            b.link(Link {
                from: sender,
                to,
                kind: LinkKind::Requests,
                provenance: unresolved(Basis::Tag),
                conditional: None,
                crossing,
                label: label.clone(),
                evidence: vec![tag_evidence],
            });
        }
    }
}

fn contract_text(tag: &ContractTag) -> String {
    match &tag.pointer {
        Some(p) => format!("{}#{p}", tag.file_ref),
        None => tag.file_ref.clone(),
    }
}

/// The calls of every function, by (caller line, caller name): (call line, callee) for calls R1 resolves to a
/// workspace function, outside cfg-gated code.
type FileCalls = BTreeMap<(usize, String), Vec<(usize, ItemRef)>>;

fn file_calls(index: &RustIndex, file: &Path) -> FileCalls {
    let mut out: FileCalls = BTreeMap::new();
    for (site, resolution) in index.resolved_calls(file) {
        let Some(caller) = &site.caller else { continue };
        let Resolution::Item(callee) = resolution else { continue };
        // Gated: the statement, the fn, its `impl` or trait, or a module on the way up (`#[cfg] mod x;` included).
        if callee.kind != ItemKind::Fn || site.cfg_gated {
            continue;
        }
        out.entry((caller.line, caller.name.clone())).or_default().push((site.line, callee));
    }
    out
}

/// Follows calls from Handler, Main and Function steps, wave by wave (each wave's files read in parallel).
fn add_calls(b: &mut Builder, index: &RustIndex) {
    let cache: Mutex<BTreeMap<PathBuf, FileCalls>> = Mutex::new(BTreeMap::new());
    let expandable = |kind: StepKind| matches!(kind, StepKind::Handler | StepKind::Main | StepKind::Function);
    let mut done: BTreeSet<usize> = BTreeSet::new();
    loop {
        let mut wave: Vec<usize> =
            b.functions.keys().copied().filter(|i| !done.contains(i) && expandable(b.steps[*i].kind)).collect();
        if wave.is_empty() {
            break;
        }
        wave.sort();
        let needed: BTreeSet<PathBuf> = wave.iter().map(|i| b.functions[i].0.clone()).collect();
        let missing: Vec<PathBuf> = {
            let c = cache.lock().unwrap();
            needed.into_iter().filter(|f| !c.contains_key(f)).collect()
        };
        let read: Vec<(PathBuf, FileCalls)> = missing.par_iter().map(|f| (f.clone(), file_calls(index, f))).collect();
        cache.lock().unwrap().extend(read);
        let cache = cache.lock().unwrap();
        for i in wave {
            done.insert(i);
            let (file, line, name) = b.functions[&i].clone();
            let Some(calls) = cache.get(&file) else { continue };
            let Some(sites) = calls.get(&(line, name)) else { continue };
            for (call_line, callee) in sites.clone() {
                let to = b.item_step(&callee, StepKind::Function, None);
                let evidence = vec![(b.steps[i].path.clone(), call_line)];
                b.link(Link {
                    from: i,
                    to,
                    kind: LinkKind::Calls,
                    provenance: Provenance::proven(Basis::PathResolution),
                    conditional: None,
                    crossing: None,
                    label: String::new(),
                    evidence,
                });
            }
        }
    }
}

/// A contract a tag declares: (contract step, provenance, label, evidence).
type Declared = (usize, Provenance, String, Vec<(PathBuf, usize)>);

/// Contract steps and the Declares links from the steps that own the tags.
/// Returns every contract tag's Unresolved reason (owned by a step or not) with its count.
fn add_contracts(b: &mut Builder, tags: &[&ContractTag], files: &ContractFiles) -> BTreeMap<String, usize> {
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let texts: Mutex<BTreeMap<PathBuf, Option<String>>> = Mutex::new(BTreeMap::new());
    let root = b.root.to_path_buf();
    let read_rel = |p: &Path| -> Option<String> {
        let mut t = texts.lock().unwrap();
        t.entry(p.to_path_buf()).or_insert_with(|| read(&root.join(p))).clone()
    };
    // Owner steps: item steps by (file, line), and every item step of a file.
    let mut by_item: BTreeMap<(PathBuf, usize), usize> = BTreeMap::new();
    let mut by_file: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (key, &i) in &b.keys {
        if let Key::Item(path, line) = key {
            by_item.insert((path.clone(), *line), i);
        }
        if matches!(key, Key::Item(..) | Key::FileSender(..)) {
            by_file.entry(b.steps[i].path.clone()).or_default().push(i);
        }
    }
    for list in by_file.values_mut() {
        list.sort();
    }

    for tag in tags {
        let file = b.rel(&tag.file);
        let owners: Vec<usize> = match &tag.owner {
            TagOwner::Item(span) => by_item.get(&(file.clone(), span.line)).copied().into_iter().collect(),
            TagOwner::File => by_file.get(&file).cloned().unwrap_or_default(),
        };
        let resolution = resolve_contract(tag, files, &read_rel);
        if let ContractResolution::Unresolved(why) = &resolution {
            *reasons.entry(why.clone()).or_default() += 1;
        }
        if owners.is_empty() {
            continue;
        }
        let text = contract_text(tag);
        let tag_evidence = (file.clone(), tag.line);
        let partial = if tag.partial { "partial".to_string() } else { String::new() };
        let mut targets: Vec<Declared> = Vec::new();
        match resolution {
            ContractResolution::Proven { file: target, pointer, line } => {
                let i = contract_step(b, &target, pointer.as_deref(), line);
                targets.push((
                    i,
                    Provenance::proven(Basis::JsonPointer),
                    partial.clone(),
                    vec![tag_evidence.clone(), (target, line)],
                ));
            }
            ContractResolution::Possible(candidates) => {
                let n = candidates.len().min(u16::MAX as usize) as u16;
                for target in candidates {
                    let line = match (&tag.pointer, read_rel(&target)) {
                        (Some(p), Some(t)) if !p.is_empty() => pointer_line(&t, p),
                        _ => 1,
                    };
                    let i = contract_step(b, &target, tag.pointer.as_deref(), line);
                    targets.push((
                        i,
                        Provenance::possible(Basis::JsonPointer, n),
                        partial.clone(),
                        vec![tag_evidence.clone(), (target, line)],
                    ));
                }
            }
            ContractResolution::Unresolved(why) => {
                let title = contract_title(tag.pointer.as_deref(), Path::new(&tag.file_ref));
                let key = Key::LooseContract(file.clone(), tag.line, text.clone());
                let detail = text.clone();
                let i = b.step(key, || Step {
                    kind: StepKind::Contract,
                    title,
                    detail,
                    language: "JSON".into(),
                    path: file.clone(),
                    line: tag.line,
                    line_end: None,
                });
                targets.push((i, unresolved(Basis::JsonPointer), why, vec![tag_evidence.clone()]));
            }
        }
        for from in owners {
            for (to, provenance, label, evidence) in &targets {
                b.link(Link {
                    from,
                    to: *to,
                    kind: LinkKind::Declares,
                    provenance: *provenance,
                    conditional: None,
                    crossing: None,
                    label: label.clone(),
                    evidence: evidence.clone(),
                });
            }
        }
    }
    reasons
}

/// A pointer's last token, else the file name.
fn contract_title(pointer: Option<&str>, file: &Path) -> String {
    match pointer.and_then(|p| p.rsplit('/').next()).filter(|t| !t.is_empty()) {
        Some(token) => token.replace("~1", "/").replace("~0", "~"),
        None => file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
    }
}

fn contract_step(b: &mut Builder, file: &Path, pointer: Option<&str>, line: usize) -> usize {
    let key = Key::Contract(file.to_path_buf(), pointer.map(str::to_string));
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let detail = match pointer {
        Some(p) => format!("{name}#{p}"),
        None => name,
    };
    b.step(key, || Step {
        kind: StepKind::Contract,
        title: contract_title(pointer, file),
        detail,
        language: "JSON".into(),
        path: file.to_path_buf(),
        line,
        line_end: None,
    })
}
