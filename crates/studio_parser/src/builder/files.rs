use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use studio_graph::{
    human_bytes, Basis, DataType, EdgeKind, FileContent, FolderTotals, Graph, GroupCluster, LazyFolder, NodeArchetype,
    NodeId, PortId, Provenance,
};

use super::common::{node_rel_path, resolve_link, slash_path};
use super::members::{attach_member_ports, build_member_nodes, link_member_id, member_calls, MemberPortIndex};
use super::rust_paths::{self, FileRefs, Outcome, RustPathStats, Target};
use super::ProjectStats;
use crate::analysis::crate_graph::{DepKind, PathDep};
use crate::extractor::{detect_language_by_path, ExtractedFile, ExtractedProject, SourceLang, UseItem};
use crate::tree::{DirKind, FileKind, ProjectTree};

const CARD_SIZE: [f32; 2] = [220.0, 42.0];

/// Builds the Files view: one nested cluster per folder (empty folders included) and one card per
/// file, laid out as a folder tree. Heavy folders start collapsed and unloaded. Rust imports and
/// calls that R1 resolves uniquely are Proven wires to R1's target (see [`rust_paths`]).
pub fn build_files_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats = ProjectStats {
        project_name: project.name.clone(),
        crate_count: project.crates.iter().filter(|c| c.is_package()).count(),
        ..Default::default()
    };

    let mut parsed: HashMap<PathBuf, ParsedFile> = HashMap::new();
    for krate in &project.crates {
        for file in &krate.files {
            stats.function_count += file.functions.len() + file.impls.iter().map(|i| i.methods.len()).sum::<usize>();
            stats.type_count += file.structs.len() + file.enums.len() + file.traits.len();
            parsed.insert(file.file_path.clone(), ParsedFile { file, crate_name: &krate.name });
        }
    }

    // R1 reads the module trees while the cards are added (one thread); Rust cards queue their
    // references once R1's answers are in.
    let mut wiring = Wiring { defer_rust: true, ..Default::default() };
    let (r1, ()) = rayon::join(
        || rust_paths::analyze(project),
        || {
            insert_folder_tree(
                &mut graph,
                &project.tree,
                Path::new(""),
                TreeRoot::New(&project.name),
                &parsed,
                &mut wiring,
            )
        },
    );
    stats.r1 = r1.stats.clone();
    wiring.queue_deferred_rust(&graph, &parsed, &r1);
    let manifest_wires = wiring.manifest_wires(&r1.path_deps);
    wiring.connect(&mut graph, &mut stats.r1);
    stats.r1.manifest_edges = connect_manifest_wires(&mut graph, &manifest_wires);
    let proven = Provenance::proven(Basis::PathResolution);
    stats.r1.proven_edges = graph.edges.iter().filter(|e| e.provenance == proven).count();
    summarize_folders(&mut graph, &project.tree);
    stats.file_count = project.tree.files.len();

    graph.layout_folder_tree();
    graph.rebuild_fast_indices();
    stats.node_count = graph.nodes.len();
    stats.wire_count = graph.edges.len();
    (graph, stats)
}

/// Builds the instant first frame from the scanned tree alone (no parsing): same folders, cards and
/// layout as the full Files view, without members or wires.
pub fn build_skeleton_files_graph(project: &crate::project::RustProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut wiring = Wiring::default();
    insert_folder_tree(
        &mut graph,
        &project.tree,
        Path::new(""),
        TreeRoot::New(&project.name),
        &HashMap::new(),
        &mut wiring,
    );
    graph.layout_folder_tree();
    graph.rebuild_fast_indices();
    let stats = ProjectStats {
        project_name: project.name.clone(),
        crate_count: project.crates.iter().filter(|c| c.is_package()).count(),
        file_count: project.tree.files.len(),
        node_count: graph.nodes.len(),
        ..Default::default()
    };
    (graph, stats)
}

/// Loads a collapsed (lazy) folder into an existing Files graph: its subfolders and files are
/// added under the folder's cluster, parsed files get members and call wires, and the tree is laid
/// out again. `subtree` is a scan of the folder itself; `parsed` holds extractions of its text files.
pub fn materialize_folder(
    graph: &mut Graph,
    cluster_id: &str,
    subtree: &ProjectTree,
    parsed: &[ExtractedFile],
) -> Result<usize, String> {
    let rel_prefix = cluster_id
        .strip_prefix("dir:")
        .map(|p| if p == "." { PathBuf::new() } else { p.split('/').collect::<PathBuf>() })
        .ok_or_else(|| format!("{cluster_id} is not a folder"))?;
    if !graph.clusters.iter().any(|c| c.id == cluster_id) {
        return Err(format!("folder {cluster_id} is not in the graph"));
    }

    // Files loaded on demand are outside every module tree R1 read: their wires are name matches.
    let lookup: HashMap<PathBuf, ParsedFile> =
        parsed.iter().map(|f| (f.file_path.clone(), ParsedFile { file: f, crate_name: "" })).collect();
    let mut wiring = Wiring::seeded_from(graph);
    let before = graph.nodes.len();
    insert_folder_tree(graph, subtree, &rel_prefix, TreeRoot::Existing(cluster_id), &lookup, &mut wiring);
    wiring.connect(graph, &mut RustPathStats::default());
    graph.layout_folder_tree();
    graph.rebuild_fast_indices();
    Ok(graph.nodes.len() - before)
}

struct ParsedFile<'a> {
    file: &'a ExtractedFile,
    crate_name: &'a str,
}

enum TreeRoot<'a> {
    /// Create a root cluster with this label (the project).
    New(&'a str),
    /// Fill an existing collapsed folder cluster.
    Existing(&'a str),
}

pub fn folder_cluster_id(rel: &Path) -> String {
    let path = slash_path(rel);
    format!("dir:{}", if path.is_empty() { "." } else { &path })
}

fn insert_folder_tree(
    graph: &mut Graph,
    tree: &ProjectTree,
    rel_prefix: &Path,
    root: TreeRoot,
    parsed: &HashMap<PathBuf, ParsedFile>,
    wiring: &mut Wiring,
) {
    graph.tree_layout = true;
    graph.flow_layout = true;
    let mut cluster_ids: Vec<String> = Vec::with_capacity(tree.dirs.len());
    let mut colors: Vec<usize> = Vec::with_capacity(tree.dirs.len());
    let mut direct_files = vec![0usize; tree.dirs.len()];
    for f in &tree.files {
        direct_files[f.dir] += 1;
    }

    for (i, dir) in tree.dirs.iter().enumerate() {
        let id = folder_cluster_id(&rel_prefix.join(&dir.rel));
        let label = match (i, &root) {
            (0, TreeRoot::New(name)) => name.to_string(),
            _ => dir
                .rel
                .file_name()
                .map_or_else(|| rel_prefix.display().to_string(), |n| n.to_string_lossy().to_string()),
        };
        let parent_color = dir.parent.map(|p| colors[p]);
        let color = match dir.parent {
            Some(0) => tree.dirs[..i].iter().filter(|d| d.parent == Some(0)).count(),
            _ => parent_color.unwrap_or(0),
        };
        colors.push(color);

        let (subtitle, lazy) = match &dir.kind {
            DirKind::Heavy(info) => (
                heavy_subtitle(info.reason.label(), info.totals),
                Some(LazyFolder {
                    abs_path: tree.root.join(&dir.rel).to_string_lossy().to_string(),
                    totals: info.totals,
                    reason: info.reason.label().to_string(),
                }),
            ),
            DirKind::Unreadable(err) => (format!("unreadable: {err}"), None),
            DirKind::Normal => match direct_files[i] {
                0 if !tree.dirs.iter().any(|d| d.parent == Some(i)) => ("empty".to_string(), None),
                n => (format!("{n} files"), None),
            },
        };

        match (i, &root) {
            (0, TreeRoot::Existing(existing)) => {
                if let Some(c) = graph.clusters.iter_mut().find(|c| c.id == *existing) {
                    c.lazy = None;
                    c.detail = studio_graph::FolderDetail::Open;
                    c.subtitle = Some(subtitle);
                }
                cluster_ids.push(existing.to_string());
            }
            _ => {
                let mut cluster = GroupCluster::new(&id, label, "Folder", color);
                cluster.subtitle = Some(subtitle);
                // A project opens at its top level: the root open, every folder in it closed
                // (node view), unloaded folders minimised. A folder loaded on demand opens, its
                // subfolders closed.
                cluster.detail = if lazy.is_some() {
                    studio_graph::FolderDetail::Minimised
                } else if i == 0 {
                    studio_graph::FolderDetail::Open
                } else {
                    studio_graph::FolderDetail::NodeView
                };
                cluster.lazy = lazy;
                cluster.parent_id = dir.parent.map(|p| cluster_ids[p].clone());
                if let Some(p) = dir.parent {
                    let parent_id = cluster_ids[p].clone();
                    if let Some(parent) = graph.clusters.iter_mut().find(|c| c.id == parent_id) {
                        parent.child_cluster_ids.push(id.clone());
                    }
                }
                graph.clusters.push(cluster);
                cluster_ids.push(id);
            }
        }
    }

    let cluster_index: HashMap<String, usize> =
        graph.clusters.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
    for f in &tree.files {
        let abs = tree.root.join(&f.rel);
        let rel_in_project = rel_prefix.join(&f.rel);
        let node_id =
            add_file_card(graph, &abs, &rel_in_project, f.size, &f.kind, parsed.get(&abs), &cluster_ids[f.dir], wiring);
        if let Some(&ci) = cluster_index.get(&cluster_ids[f.dir]) {
            graph.clusters[ci].node_ids.push(node_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_file_card(
    graph: &mut Graph,
    abs: &Path,
    rel_in_project: &Path,
    size: u64,
    kind: &FileKind,
    parsed: Option<&ParsedFile>,
    cluster_id: &str,
    wiring: &mut Wiring,
) -> NodeId {
    let file_name = abs.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".to_string());
    let content = match kind {
        FileKind::Text => FileContent::Code,
        FileKind::Binary => FileContent::Binary,
        FileKind::Image => FileContent::Image,
        FileKind::TooLarge => FileContent::TooLarge,
        FileKind::Symlink(target) => FileContent::Symlink(target.to_string_lossy().to_string()),
        FileKind::Unreadable(err) => FileContent::Unreadable(err.clone()),
    };

    let ext = abs.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let language = parsed.map(|p| p.file.language).unwrap_or_else(|| detect_language_by_path(abs));
    let badge = match language {
        SourceLang::Markdown => "MD".to_string(),
        SourceLang::Enforce => "ENS".to_string(),
        _ if matches!(content, FileContent::Symlink(_)) => "LINK".to_string(),
        _ if ext.is_empty() => "FILE".to_string(),
        _ => ext,
    };
    let desc = match (parsed, content.label()) {
        (_, Some(reason)) => format!("{reason} · {}", human_bytes(size)),
        (Some(p), None) => describe_parsed(p.file),
        (None, None) => human_bytes(size),
    };

    let node_id = graph.add_node(
        &file_name,
        NodeArchetype::File,
        desc,
        Some(badge),
        vec![("in".to_string(), DataType::RustFlow), ("docs".to_string(), DataType::Documentation)],
        vec![("out".to_string(), DataType::RustFlow)],
        [0.0, 0.0],
    );

    let (crate_name, module_path, docs) = match parsed {
        Some(p) => {
            let docs = p
                .file
                .functions
                .iter()
                .find(|f| !f.docs.is_empty())
                .map(|f| f.docs.clone())
                .or_else(|| p.file.structs.iter().find(|s| !s.docs.is_empty()).map(|s| s.docs.clone()));
            let module = if p.crate_name.is_empty() {
                p.file.module_name.clone()
            } else {
                format!("{}::{}", p.crate_name, p.file.module_name)
            };
            (Some(p.crate_name.to_string()).filter(|c| !c.is_empty()), Some(module), docs)
        }
        None => (None, None, None),
    };
    graph.set_node_metadata(
        node_id,
        Some(abs.to_string_lossy().to_string()),
        Some(1),
        crate_name,
        module_path,
        docs,
        None,
        Some(cluster_id.to_string()),
    );
    if let Some(n) = graph.nodes.get_mut(&node_id) {
        n.size = CARD_SIZE;
        n.content = content;
        n.size_bytes = Some(size);
        n.test_count = parsed.map_or(0, |p| p.file.tests as u32);
    }

    wiring.register_file_names(graph, node_id, abs, rel_in_project, &file_name);
    if let Some(p) = parsed {
        wiring.register_parsed(graph, node_id, rel_in_project, p.file, p.crate_name);
    }
    node_id
}

fn describe_parsed(file: &ExtractedFile) -> String {
    match file.language {
        SourceLang::Markdown => format!("{} headings, {} links", file.structs.len(), file.functions.len()),
        SourceLang::Enforce => {
            let methods = file.impls.iter().map(|i| i.methods.len()).sum::<usize>() + file.functions.len();
            format!("{} classes, {} methods", file.structs.len(), methods)
        }
        _ => {
            let total = file.functions.len() + file.structs.len() + file.enums.len() + file.traits.len();
            format!("{} items ({} fns, {} types)", total, file.functions.len(), file.structs.len() + file.enums.len())
        }
    }
}

/// Puts what each folder holds on its subtitle (files and tests inside it, at any depth, and its
/// largest file classes, with what `tree` knows git ignores) and the first sentence of its README
/// as what it is for. Folders not loaded or unreadable keep theirs.
pub fn summarize_folders(graph: &mut Graph, tree: &ProjectTree) {
    let index: HashMap<String, usize> = graph.clusters.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
    let count = graph.clusters.len();
    let mut files = vec![0usize; count];
    let mut tests = vec![0u64; count];
    let mut classes = folder_classes(graph, tree);
    for (i, c) in graph.clusters.iter().enumerate() {
        files[i] = c.node_ids.len();
        tests[i] = c.node_ids.iter().filter_map(|id| graph.nodes.get(id)).map(|n| n.test_count as u64).sum();
    }
    // A folder comes after its parent, so walking backwards adds each folder to its parent once
    // its own total is complete.
    for i in (0..count).rev() {
        if let Some(&p) = graph.clusters[i].parent_id.as_ref().and_then(|p| index.get(p)) {
            files[p] += files[i];
            tests[p] += tests[i];
            let child = std::mem::take(&mut classes[i]);
            classes[p].add(&child);
            classes[i] = child;
        }
    }
    for i in 0..count {
        let readme = graph.clusters[i]
            .node_ids
            .iter()
            .filter_map(|id| graph.nodes.get(id))
            .find(|n| {
                matches!(n.title.to_lowercase().as_str(), "readme.md" | "readme.markdown" | "readme" | "readme.txt")
            })
            .and_then(|n| n.file_path.clone());
        let about = readme.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|text| first_sentence(&text));
        let c = &mut graph.clusters[i];
        if c.lazy.is_some() || c.subtitle.as_deref().is_some_and(|s| s.starts_with("unreadable")) || files[i] == 0 {
            continue;
        }
        let mut parts = vec![if files[i] == 1 {
            "1 file".to_string()
        } else {
            format!("{} files", group_thousands(files[i] as u64))
        }];
        match tests[i] {
            0 => {}
            1 => parts.push("1 test".to_string()),
            n => parts.push(format!("{} tests", group_thousands(n))),
        }
        parts.push(classes[i].top(3));
        let mut subtitle = parts.join(" · ");
        if let Some(about) = &about {
            subtitle = format!("{subtitle} — {about}");
        }
        c.subtitle = Some(subtitle);
        c.about = about;
    }
}

/// Files per class in one folder, and the tool conventions that decided any of them.
#[derive(Default)]
struct ClassCounts {
    counts: [u64; crate::classify::FileClass::ALL.len()],
    conventions: [std::collections::BTreeSet<&'static str>; crate::classify::FileClass::ALL.len()],
}

impl ClassCounts {
    fn add(&mut self, other: &ClassCounts) {
        for (i, n) in other.counts.iter().enumerate() {
            self.counts[i] += n;
            self.conventions[i].extend(other.conventions[i].iter().copied());
        }
    }

    /// The `n` largest classes, largest first (ties in table order): "12 code, 3 test files
    /// (tool convention: Cargo), 1 docs".
    fn top(&self, n: usize) -> String {
        use crate::classify::FileClass;
        let mut order: Vec<usize> = (0..FileClass::ALL.len()).filter(|&i| self.counts[i] > 0).collect();
        order.sort_by(|&a, &b| self.counts[b].cmp(&self.counts[a]).then(a.cmp(&b)));
        let shown: Vec<String> = order
            .into_iter()
            .take(n)
            .map(|i| {
                let class = FileClass::ALL[i];
                let label = if class == FileClass::Tests { "test files" } else { class.label() };
                let mut text = format!("{} {label}", group_thousands(self.counts[i]));
                if !self.conventions[i].is_empty() {
                    let tools: Vec<&str> = self.conventions[i].iter().copied().collect();
                    text.push_str(&format!(" (tool convention: {})", tools.join(", ")));
                }
                text
            })
            .collect();
        shown.join(", ")
    }
}

/// Each folder's own files by class, from the classification table with the scan's inputs (every
/// path and what git ignores; Linguist overrides are not known here).
fn folder_classes(graph: &Graph, tree: &ProjectTree) -> Vec<ClassCounts> {
    use crate::classify::{classify, FileClass, RuleKind};
    use rayon::prelude::*;
    let inputs = crate::tree::class_inputs(tree);
    let class_index = |class: FileClass| FileClass::ALL.iter().position(|&c| c == class).unwrap_or(0);
    graph
        .clusters
        .par_iter()
        .map(|c| {
            let mut counts = ClassCounts::default();
            for rel in c.node_ids.iter().filter_map(|id| graph.nodes.get(id)).filter_map(node_rel_path) {
                let (class, rule) = classify(&rel, &inputs);
                counts.counts[class_index(class)] += 1;
                if rule.kind == RuleKind::Convention {
                    counts.conventions[class_index(class)].insert(rule.basis);
                }
            }
            counts
        })
        .collect()
}

/// The first sentence of a document's first paragraph of text (badges, headings and HTML
/// skipped), at most 200 characters.
pub fn first_sentence(markdown: &str) -> Option<String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut text = String::new();
    let (mut in_paragraph, mut in_image) = (false, false);
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Paragraph) => {
                in_paragraph = true;
                text.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                in_paragraph = false;
                if !text.trim().is_empty() {
                    break;
                }
            }
            Event::Start(Tag::Image { .. }) => in_image = true,
            Event::End(TagEnd::Image) => in_image = false,
            Event::Text(t) | Event::Code(t) if in_paragraph && !in_image => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak if in_paragraph => text.push(' '),
            _ => {}
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    // Up to the first full stop, question or exclamation mark followed by a space.
    let end = text
        .char_indices()
        .find(|&(i, c)| matches!(c, '.' | '!' | '?') && text[i + c.len_utf8()..].starts_with(' '))
        .map_or(text.len(), |(i, c)| i + c.len_utf8());
    let sentence = &text[..end];
    Some(if sentence.chars().count() > 200 {
        format!("{}…", sentence.chars().take(199).collect::<String>())
    } else {
        sentence.to_string()
    })
}

/// Subtitle of a folder listed but not loaded: its totals once counted, and why it is collapsed.
fn heavy_subtitle(reason: &str, totals: Option<FolderTotals>) -> String {
    match totals {
        Some(t) => format!("{} files · {} · {reason}", group_thousands(t.file_count), human_bytes(t.total_bytes)),
        None => reason.to_string(),
    }
}

/// What each heavy folder in `tree` holds, by cluster id, once [`crate::tree::measure_heavy_dirs`]
/// has counted it.
pub fn heavy_folder_totals(tree: &ProjectTree) -> Vec<(String, FolderTotals)> {
    tree.heavy_dirs().filter_map(|(_, dir, info)| Some((folder_cluster_id(&dir.rel), info.totals?))).collect()
}

/// Puts counted totals on folders that are still collapsed placeholders.
pub fn apply_folder_totals(graph: &mut Graph, totals: &[(String, FolderTotals)]) {
    let by_id: HashMap<&str, FolderTotals> = totals.iter().map(|(id, t)| (id.as_str(), *t)).collect();
    for cluster in &mut graph.clusters {
        let Some(&t) = by_id.get(cluster.id.as_str()) else { continue };
        if let Some(lazy) = &mut cluster.lazy {
            lazy.totals = Some(t);
            cluster.subtitle = Some(heavy_subtitle(&lazy.reason, Some(t)));
        }
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Ports of a file card: code comes in on the left, documentation in at the top left, and what
/// the file provides goes out on the right.
#[derive(Debug, Clone, Copy)]
struct FilePorts {
    input: PortId,
    output: PortId,
    doc: Option<PortId>,
}

/// Name → node/port tables collected while cards are added, then resolved into wires. Every wire
/// runs from the provider's output to the consumer's input (docs/VISUAL_LANGUAGE.md).
#[derive(Default)]
struct Wiring {
    file_ports: HashMap<NodeId, FilePorts>,
    /// Project-relative `/` path of every file card, for exact documentation links.
    path_to_node: HashMap<String, NodeId>,
    module_to_file: HashMap<String, NodeId>,
    symbol_to_file: HashMap<String, NodeId>,
    members: MemberPortIndex,
    /// (consumer file, name it uses, kind).
    pending_file_deps: Vec<(NodeId, String, EdgeKind)>,
    /// (consumer file, consumer member in-port, name it calls).
    pending_member_calls: Vec<(NodeId, PortId, String)>,
    /// (documentation file, link member out-port, resolved project-relative target).
    pending_links: Vec<(NodeId, Option<PortId>, String)>,
    /// Absolute path of every file card, for R1's targets.
    abs_to_node: HashMap<PathBuf, NodeId>,
    /// Rust references R1 answered, resolved into wires (or back into name matches) by [`Wiring::connect`].
    pending_r1: Vec<R1Ref>,
    /// Rust cards wait for R1's answers before queuing their uses and calls (a full project build).
    defer_rust: bool,
    /// (card, file path) of the Rust cards waiting.
    deferred_rust: Vec<(NodeId, PathBuf)>,
}

/// One Rust reference R1 answered.
enum R1Ref {
    /// A `use` leaf, with the texts today's name match looks up for it.
    Use { consumer: NodeId, last: String, joined: String, outcome: Outcome },
    /// A call site. `member` is the calling member's (input, output) ports when it has a row; `matched` says the
    /// extractor recorded the same call, so a name match was looked up for it before R1.
    Call { consumer: NodeId, member: Option<(PortId, PortId)>, name: String, outcome: Outcome, matched: bool },
}

/// Proven wires from R1: file pairs (provider, consumer, kind) and member pairs (provider, provider out-port,
/// consumer, consumer in-port).
#[derive(Default)]
struct ProvenWires {
    files: Vec<(NodeId, NodeId, EdgeKind)>,
    members: Vec<(NodeId, PortId, NodeId, PortId)>,
}

impl Wiring {
    /// Tables for an existing graph, so newly loaded files can wire to what is already there.
    fn seeded_from(graph: &Graph) -> Self {
        let mut w = Wiring { members: MemberPortIndex::from_graph(graph), ..Default::default() };
        for (&id, node) in graph.nodes.iter().filter(|(_, n)| n.archetype == NodeArchetype::File) {
            if let (Some(input), Some(output)) = (node.inputs.first(), node.outputs.first()) {
                w.file_ports.insert(id, FilePorts { input: input.id, output: output.id, doc: node.doc_port() });
            }
            if let Some(rel) = node_rel_path(node) {
                w.path_to_node.insert(rel, id);
            }
            if let Some(module) = &node.module_path {
                w.module_to_file.insert(module.clone(), id);
                if let Some(short) = module.rsplit("::").next() {
                    w.module_to_file.entry(short.to_string()).or_insert(id);
                }
            }
            for m in node.member_nodes.iter().filter(|m| m.archetype != NodeArchetype::Link) {
                w.symbol_to_file.entry(m.name.clone()).or_insert(id);
            }
        }
        w
    }

    fn register_file_names(&mut self, graph: &Graph, node_id: NodeId, abs: &Path, rel: &Path, file_name: &str) {
        self.abs_to_node.insert(abs.to_path_buf(), node_id);
        let node = &graph.nodes[&node_id];
        if let (Some(input), Some(output)) = (node.inputs.first(), node.outputs.first()) {
            self.file_ports.insert(node_id, FilePorts { input: input.id, output: output.id, doc: node.doc_port() });
        }
        self.path_to_node.insert(slash_path(rel), node_id);
        if let Some(output) = node.outputs.first() {
            self.members.file_out_port.insert(node_id, output.id);
        }
        self.members.file_to_node.insert(slash_path(rel), node_id);
        self.members.file_to_node.insert(file_name.to_string(), node_id);
        if let Some(stem) = rel.file_stem().and_then(|s| s.to_str()) {
            self.members.file_to_node.insert(stem.to_string(), node_id);
        }
    }

    /// Adds member rows/ports for a parsed file and queues what it uses, calls and links to (a Rust
    /// card of a full build queues its uses and calls later, see [`Wiring::queue_deferred_rust`]).
    fn register_parsed(
        &mut self,
        graph: &mut Graph,
        node_id: NodeId,
        rel: &Path,
        file: &ExtractedFile,
        crate_name: &str,
    ) {
        let mut member_nodes = build_member_nodes(file);
        attach_member_ports(graph, node_id, &mut member_nodes);
        self.members.register_members(node_id, &member_nodes);
        let out_port = |id: &str| member_nodes.iter().find(|m| m.id == id).and_then(|m| m.out_port_id);

        self.module_to_file.insert(file.module_name.clone(), node_id);
        if !crate_name.is_empty() {
            self.module_to_file.insert(format!("{}::{}", crate_name, file.module_name), node_id);
        }

        if self.defer_rust && file.language == SourceLang::Rust {
            self.deferred_rust.push((node_id, file.file_path.clone()));
        } else {
            self.queue_code_refs(node_id, file, None, &member_nodes);
        }
        for f in &file.functions {
            self.symbol_to_file.insert(f.name.clone(), node_id);
        }
        for imp in &file.impls {
            for m in &imp.methods {
                self.symbol_to_file.insert(format!("{}::{}", imp.target_type, m.name), node_id);
            }
        }
        for name in file
            .structs
            .iter()
            .map(|s| &s.name)
            .chain(file.enums.iter().map(|e| &e.name))
            .chain(file.traits.iter().map(|t| &t.name))
        {
            self.symbol_to_file.insert(name.clone(), node_id);
        }

        let from = slash_path(rel);
        for link in &file.links {
            if let Some(target) = resolve_link(&from, &link.target) {
                self.pending_links.push((node_id, out_port(&link_member_id(link)), target));
            }
        }

        if let Some(n) = graph.nodes.get_mut(&node_id) {
            n.member_nodes = member_nodes;
        }
    }

    /// Queues the Rust cards that waited for R1, now that its answers are in.
    fn queue_deferred_rust(
        &mut self,
        graph: &Graph,
        parsed: &HashMap<PathBuf, ParsedFile>,
        r1: &rust_paths::RustPaths,
    ) {
        for (node_id, path) in std::mem::take(&mut self.deferred_rust) {
            let (Some(p), Some(node)) = (parsed.get(&path), graph.nodes.get(&node_id)) else { continue };
            self.queue_code_refs(node_id, p.file, r1.file(&path), &node.member_nodes);
        }
    }

    /// Queues what a file's members call and what it uses. A Rust file R1 answered for queues its
    /// `use` leaves and call sites with R1's outcome instead of bare names.
    fn queue_code_refs(
        &mut self,
        node_id: NodeId,
        file: &ExtractedFile,
        refs: Option<&FileRefs>,
        member_nodes: &[studio_graph::FileMemberNode],
    ) {
        let in_port = |id: &str| member_nodes.iter().find(|m| m.id == id).and_then(|m| m.in_port_id);
        let out_port = |id: &str| member_nodes.iter().find(|m| m.id == id).and_then(|m| m.out_port_id);
        // R1's use leaves pair with the extractor's file-level `use` declarations (both in source order; R1
        // skips test-only ones); if they ever disagree, the file keeps today's name matching.
        let refs = refs
            .filter(|_| file.language == SourceLang::Rust)
            .and_then(|r| r.read_uses(&file.uses).map(|read| (r, read)));
        match refs {
            Some((refs, _)) => self.queue_rust_calls(node_id, file, refs, &in_port, &out_port),
            None => {
                for (member_id, calls) in member_calls(file) {
                    let consumer_in = in_port(&member_id);
                    for call in calls {
                        self.queue_call_name(node_id, consumer_in, call);
                    }
                }
            }
        }
        match refs {
            Some((refs, read)) => {
                self.pending_r1.extend(refs.uses.iter().map(|leaf| R1Ref::Use {
                    consumer: node_id,
                    last: leaf.last.clone(),
                    joined: leaf.joined.clone(),
                    outcome: leaf.outcome.clone(),
                }));
                // Test code: R1 does not read it.
                for (u, _) in file.uses.iter().zip(read).filter(|(_, read)| !read) {
                    self.queue_use_names(node_id, u);
                }
            }
            None => {
                for u in &file.uses {
                    self.queue_use_names(node_id, u);
                }
            }
        }
    }

    /// Today's name lookups for one `use` declaration: its last segment, its whole text and its items.
    fn queue_use_names(&mut self, node_id: NodeId, u: &UseItem) {
        let last_segment =
            u.path.split([':', '/', '\\', '.']).rfind(|s| !s.is_empty()).unwrap_or("").trim().to_string();
        if !last_segment.is_empty() {
            self.pending_file_deps.push((node_id, last_segment, EdgeKind::Import));
        }
        if !u.path.is_empty() {
            self.pending_file_deps.push((node_id, u.path.clone(), EdgeKind::Import));
        }
        for item in &u.items {
            if !item.is_empty() && item != "import" && item != "#include" {
                self.pending_file_deps.push((node_id, item.clone(), EdgeKind::Import));
            }
        }
    }

    /// Today's name lookups for one call: the file that defines the name, and the member that does.
    fn queue_call_name(&mut self, node_id: NodeId, consumer_in: Option<PortId>, name: &str) {
        self.pending_file_deps.push((node_id, name.to_string(), EdgeKind::Call));
        if let Some(input) = consumer_in {
            self.pending_member_calls.push((node_id, input, name.to_string()));
        }
    }

    /// Queues R1's call sites of a file, each paired with the extractor's record of the same call (same
    /// caller, same name) when there is one. Calls the extractor saw that R1 does not report (test code)
    /// keep today's name lookups.
    fn queue_rust_calls(
        &mut self,
        node_id: NodeId,
        file: &ExtractedFile,
        refs: &FileRefs,
        in_port: &dyn Fn(&str) -> Option<PortId>,
        out_port: &dyn Fn(&str) -> Option<PortId>,
    ) {
        // (member id, bare name, line of the name, calls not yet paired).
        let mut callers: Vec<(String, &str, usize, Vec<Option<&String>>)> = file
            .functions
            .iter()
            .map(|f| (format!("fn:{}", f.name), f.name.as_str(), f.line, f.calls.iter().map(Some).collect()))
            .collect();
        for imp in &file.impls {
            for m in &imp.methods {
                let id = format!("method:{}::{}", imp.target_type, m.name);
                callers.push((id, m.name.as_str(), m.line, m.calls.iter().map(Some).collect()));
            }
        }
        for call in &refs.calls {
            let caller = callers.iter_mut().find(|c| c.1 == call.caller && c.2 == call.caller_line);
            let (member, matched) = match caller {
                Some((id, _, _, names)) => {
                    let ports = in_port(id).zip(out_port(id));
                    let slot = names.iter_mut().find(|n| n.is_some_and(|n| *n == call.name));
                    (ports, slot.map(Option::take).is_some())
                }
                None => (None, false),
            };
            self.pending_r1.push(R1Ref::Call {
                consumer: node_id,
                member,
                name: call.name.clone(),
                outcome: call.outcome.clone(),
                matched,
            });
        }
        for (id, _, _, names) in &callers {
            let consumer_in = in_port(id);
            for name in names.iter().flatten() {
                self.queue_call_name(node_id, consumer_in, name);
            }
        }
    }

    /// Resolves queued names into file-to-file, member-to-member and documentation wires, and fills
    /// R1's counters.
    fn connect(mut self, graph: &mut Graph, counts: &mut RustPathStats) {
        let proven = self.expand_r1(graph, counts);
        self.connect_files(graph, &proven.files);
        self.connect_members(graph, &proven.members);
        self.connect_links(graph);
    }

    /// Turns R1's references into Proven wires, or back into today's name lookups when R1 could not
    /// decide, and counts each against what the name match finds for it.
    fn expand_r1(&mut self, graph: &Graph, counts: &mut RustPathStats) -> ProvenWires {
        let mut proven = ProvenWires::default();
        for r in std::mem::take(&mut self.pending_r1) {
            match r {
                R1Ref::Use { consumer, last, joined, outcome } => {
                    let old = self.resolve_file(&last).or_else(|| self.resolve_file(&joined));
                    let target = match &outcome {
                        Outcome::Proven(t) => self.abs_to_node.get(&t.file).copied(),
                        _ => None,
                    };
                    match (&outcome, target) {
                        (Outcome::Proven(_), Some(target)) => {
                            count_proven(counts, old.map(|o| o == target));
                            proven.files.push((target, consumer, EdgeKind::Import));
                            continue;
                        }
                        (Outcome::External, _) => {
                            count_external(counts, old.is_some());
                            continue;
                        }
                        (Outcome::Gated, _) => counts.cfg_gated += 1,
                        // Undecided, or a target without a card (inside a folder not loaded).
                        _ => counts.unresolved += 1,
                    }
                    self.pending_file_deps.push((consumer, last, EdgeKind::Import));
                    self.pending_file_deps.push((consumer, joined, EdgeKind::Import));
                }
                R1Ref::Call { consumer, member, name, outcome, matched } => {
                    let old_member = if matched { self.members.resolve_provider(&name) } else { None };
                    let old_file = if matched { self.resolve_file(&name) } else { None };
                    let target = match &outcome {
                        Outcome::Proven(t) => self.abs_to_node.get(&t.file).map(|&node| (node, t)),
                        _ => None,
                    };
                    match (&outcome, target) {
                        (Outcome::Proven(_), Some((node, t))) => {
                            let Some(port) =
                                member_out(graph, node, t).or_else(|| self.file_ports.get(&node).map(|p| p.output))
                            else {
                                continue;
                            };
                            let same = match (old_member, old_file) {
                                (Some(old), _) => Some(old == (node, port)),
                                (None, Some(old)) => Some(old == node),
                                (None, None) => None,
                            };
                            count_proven(counts, same);
                            proven.files.push((node, consumer, EdgeKind::Call));
                            if let Some((input, _)) = member {
                                proven.members.push((node, port, consumer, input));
                            }
                            continue;
                        }
                        (Outcome::External, _) => {
                            count_external(counts, old_member.is_some() || old_file.is_some());
                            continue;
                        }
                        (Outcome::Gated, _) => counts.cfg_gated += 1,
                        _ => counts.unresolved += 1,
                    }
                    if matched {
                        self.queue_call_name(consumer, member.map(|m| m.0), &name);
                    }
                }
            }
        }
        proven
    }

    /// One wire per (provider, consumer) file pair. An import outranks a call: if the consumer
    /// imports the provider anywhere, the file-level wire is an import. The wire is Proven when any
    /// reference behind it resolved exactly through R1.
    fn connect_files(&self, graph: &mut Graph, proven: &[(NodeId, NodeId, EdgeKind)]) {
        let mut pairs: Vec<(NodeId, NodeId, EdgeKind, bool)> = Vec::new();
        let mut pair_index: HashMap<(NodeId, NodeId), usize> = HashMap::new();
        let named = self.pending_file_deps.iter().filter_map(|(consumer, target_name, kind)| {
            Some((self.resolve_file(target_name)?, *consumer, *kind, false))
        });
        let exact = proven.iter().map(|&(provider, consumer, kind)| (provider, consumer, kind, true));
        for (provider, consumer, kind, is_proven) in named.chain(exact) {
            if provider == consumer {
                continue;
            }
            match pair_index.get(&(provider, consumer)) {
                Some(&i) => {
                    if kind == EdgeKind::Import {
                        pairs[i].2 = EdgeKind::Import;
                    }
                    pairs[i].3 |= is_proven;
                }
                None => {
                    pair_index.insert((provider, consumer), pairs.len());
                    pairs.push((provider, consumer, kind, is_proven));
                }
            }
        }
        for (provider, consumer, kind, is_proven) in pairs {
            if let (Some(p), Some(c)) = (self.file_ports.get(&provider), self.file_ports.get(&consumer)) {
                let id = graph.connect_kind(provider, p.output, consumer, c.input, kind);
                if let (Some(id), true) = (id, is_proven) {
                    graph.set_provenance(id, Provenance::proven(Basis::PathResolution));
                }
            }
        }
    }

    /// One wire per (provider member, consumer member), Proven when any call behind it resolved
    /// exactly through R1.
    fn connect_members(&self, graph: &mut Graph, proven: &[(NodeId, PortId, NodeId, PortId)]) {
        let output_of: HashMap<PortId, PortId> =
            self.members.member_ports.values().map(|m| (m.input, m.output)).collect();
        let mut wires: Vec<(NodeId, PortId, NodeId, PortId, bool)> = Vec::new();
        let mut index: HashMap<(PortId, PortId), usize> = HashMap::new();
        let named = self.pending_member_calls.iter().filter_map(|&(consumer, consumer_in, ref target_name)| {
            let (provider, provider_out) = self.members.resolve_provider(target_name)?;
            Some((provider, provider_out, consumer, consumer_in, false))
        });
        let exact = proven.iter().map(|&(p, p_out, c, c_in)| (p, p_out, c, c_in, true));
        for (provider, provider_out, consumer, consumer_in, is_proven) in named.chain(exact) {
            // A recursive function would wire to itself.
            if output_of.get(&consumer_in) == Some(&provider_out) {
                continue;
            }
            match index.get(&(provider_out, consumer_in)) {
                Some(&i) => wires[i].4 |= is_proven,
                None => {
                    index.insert((provider_out, consumer_in), wires.len());
                    wires.push((provider, provider_out, consumer, consumer_in, is_proven));
                }
            }
        }
        for (provider, provider_out, consumer, consumer_in, is_proven) in wires {
            let id = graph.connect_kind(provider, provider_out, consumer, consumer_in, EdgeKind::Call);
            if let (Some(id), true) = (id, is_proven) {
                graph.set_provenance(id, Provenance::proven(Basis::PathResolution));
            }
        }
    }

    /// Documentation wires run from the documentation file (and from the link's row) into the
    /// target's documentation port.
    fn connect_links(&self, graph: &mut Graph) {
        let mut file_pairs: HashSet<(NodeId, NodeId)> = HashSet::new();
        for &(doc_file, link_out, ref target) in &self.pending_links {
            let Some(&target_node) = self.path_to_node.get(target) else { continue };
            if target_node == doc_file {
                continue;
            }
            let (Some(doc_ports), Some(target_ports)) =
                (self.file_ports.get(&doc_file), self.file_ports.get(&target_node))
            else {
                continue;
            };
            let Some(doc_in) = target_ports.doc else { continue };
            // The link names a file that exists: the wire is a fact.
            let proven = Provenance::proven(Basis::DocLink);
            if file_pairs.insert((doc_file, target_node)) {
                if let Some(id) =
                    graph.connect_kind(doc_file, doc_ports.output, target_node, doc_in, EdgeKind::Documentation)
                {
                    graph.set_provenance(id, proven);
                }
            }
            if let Some(out) = link_out {
                if let Some(id) = graph.connect_kind(doc_file, out, target_node, doc_in, EdgeKind::Documentation) {
                    graph.set_provenance(id, proven);
                }
            }
        }
    }

    /// One wire per pair of packages joined by a normal or build path dependency: (dependency's manifest card,
    /// its out-port, dependent's manifest card, its in-port, line of the first entry declaring it). Dev
    /// dependencies make no wire: a wire could not tell that only the package's tests use the other.
    fn manifest_wires(&self, deps: &[PathDep]) -> Vec<ManifestWire> {
        let mut seen: HashSet<(NodeId, NodeId)> = HashSet::new();
        let mut out = Vec::new();
        // `deps` is sorted by (manifest, target, kind, line): the first entry of a pair is its strongest.
        for d in deps.iter().filter(|d| d.kind != DepKind::Dev) {
            let (Some(&provider), Some(&consumer)) =
                (self.abs_to_node.get(&d.target), self.abs_to_node.get(&d.manifest))
            else {
                continue;
            };
            let (Some(p), Some(c)) = (self.file_ports.get(&provider), self.file_ports.get(&consumer)) else { continue };
            if seen.insert((provider, consumer)) {
                out.push((provider, p.output, consumer, c.input, d.line));
            }
        }
        out
    }

    fn resolve_file(&self, target_name: &str) -> Option<NodeId> {
        let clean = target_name
            .trim_start_matches("./")
            .trim_end_matches(".js")
            .trim_end_matches(".ts")
            .trim_end_matches(".tsx")
            .trim_end_matches(".jsx")
            .trim_end_matches(".py");
        let stem = Path::new(&clean).file_stem().and_then(|s| s.to_str()).unwrap_or(clean);
        let files = &self.members.file_to_node;
        self.module_to_file
            .get(target_name)
            .or_else(|| self.symbol_to_file.get(target_name))
            .or_else(|| files.get(target_name))
            .or_else(|| files.get(clean))
            .or_else(|| files.get(stem))
            .or_else(|| self.symbol_to_file.get(stem))
            .or_else(|| self.module_to_file.get(stem))
            .copied()
    }
}

/// A manifest wire: (provider card, out-port, consumer card, in-port, line in the consumer's manifest).
type ManifestWire = (NodeId, PortId, NodeId, PortId, Option<usize>);

/// Adds the manifest wires, Proven by the manifest; the wire's `source_line` is the dependency's line in the
/// dependent's manifest. Returns how many were added.
fn connect_manifest_wires(graph: &mut Graph, wires: &[ManifestWire]) -> usize {
    let mut added = 0;
    for &(provider, out, consumer, input, line) in wires {
        if let Some(id) = graph.connect_labeled(provider, out, consumer, input, EdgeKind::Depends, None, None, line) {
            graph.set_provenance(id, Provenance::proven(Basis::Manifest));
            added += 1;
        }
    }
    added
}

/// The out-port of the member row R1's target names (same line, same bare name), if it has one.
fn member_out(graph: &Graph, node: NodeId, target: &Target) -> Option<PortId> {
    if target.line == 0 {
        return None;
    }
    let bare = |name: &str| name.rsplit("::").next().unwrap_or(name).to_string();
    let want = bare(&target.name);
    graph
        .nodes
        .get(&node)?
        .member_nodes
        .iter()
        .find(|m| m.archetype != NodeArchetype::Link && m.line_number == target.line && bare(&m.name) == want)?
        .out_port_id
}

/// Counts a reference R1 resolved to a workspace target: `same` compares it with the name match's target
/// (`None` when the name match found nothing).
fn count_proven(counts: &mut RustPathStats, same: Option<bool>) {
    match same {
        Some(true) => counts.upgraded += 1,
        Some(false) => counts.retargeted += 1,
        None => counts.added += 1,
    }
}

/// Counts a reference R1 resolved outside the workspace sources.
fn count_external(counts: &mut RustPathStats, had_name_match: bool) {
    if had_name_match {
        counts.dropped += 1;
    } else {
        counts.external += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R1 runs inside the build, before the one layout of the complete graph: the map is laid out from
    /// which wires exist, never from their tiers. The same build with every wire downgraded to
    /// Unresolved before layout puts every card and folder in the same place.
    #[test]
    fn layout_does_not_depend_on_wire_tiers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r1_graph");
        let scanned = crate::project::scan_project(&root).unwrap();
        let project = crate::extractor::extract_project(&scanned);
        let (graph, stats) = build_files_graph(&project);
        assert!(stats.r1.proven_edges > 0, "the fixture has Proven wires");

        // build_files_graph, with the tiers dropped between wiring and layout.
        let r1 = rust_paths::analyze(&project);
        let mut parsed: HashMap<PathBuf, ParsedFile> = HashMap::new();
        for krate in &project.crates {
            for file in &krate.files {
                parsed.insert(file.file_path.clone(), ParsedFile { file, crate_name: &krate.name });
            }
        }
        let mut flat = Graph::new();
        let mut wiring = Wiring { defer_rust: true, ..Default::default() };
        insert_folder_tree(&mut flat, &project.tree, Path::new(""), TreeRoot::New(&project.name), &parsed, &mut wiring);
        wiring.queue_deferred_rust(&flat, &parsed, &r1);
        let manifest_wires = wiring.manifest_wires(&r1.path_deps);
        wiring.connect(&mut flat, &mut RustPathStats::default());
        connect_manifest_wires(&mut flat, &manifest_wires);
        let ids: Vec<_> = flat.edges.iter().map(|e| e.id).collect();
        for id in ids {
            flat.set_provenance(id, Provenance::default());
        }
        summarize_folders(&mut flat, &project.tree);
        flat.layout_folder_tree();

        assert_eq!(flat.edges.len(), graph.edges.len());
        assert!(flat.edges.iter().all(|e| e.provenance.tier == studio_graph::EvidenceTier::Unresolved));
        let positions = |g: &Graph| {
            let mut nodes: Vec<_> = g.nodes.values().map(|n| (n.id, n.position, n.size)).collect();
            nodes.sort_by_key(|n| n.0);
            let clusters: Vec<_> = g.clusters.iter().map(|c| (c.id.clone(), c.position, c.size)).collect();
            (nodes, clusters)
        };
        assert_eq!(positions(&flat), positions(&graph));
    }
}
