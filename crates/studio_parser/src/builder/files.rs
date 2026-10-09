use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use studio_graph::{
    human_bytes, DataType, EdgeKind, FileContent, Graph, GroupCluster, LazyFolder, NodeArchetype, NodeId, PortId,
};

use super::common::{node_rel_path, resolve_link, slash_path};
use super::members::{attach_member_ports, build_member_nodes, link_member_id, member_calls, MemberPortIndex};
use super::ProjectStats;
use crate::extractor::{detect_language_by_path, ExtractedFile, ExtractedProject, SourceLang};
use crate::tree::{DirKind, FileKind, ProjectTree};

const CARD_SIZE: [f32; 2] = [220.0, 42.0];

/// Builds the Files view: one nested cluster per folder (empty folders included) and one card per
/// file, laid out as a folder tree. Heavy folders start collapsed and unloaded.
pub fn build_files_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    let mut graph = Graph::new();
    let mut stats =
        ProjectStats { project_name: project.name.clone(), crate_count: project.crates.len(), ..Default::default() };

    let mut parsed: HashMap<PathBuf, ParsedFile> = HashMap::new();
    for krate in &project.crates {
        for file in &krate.files {
            stats.function_count += file.functions.len() + file.impls.iter().map(|i| i.methods.len()).sum::<usize>();
            stats.type_count += file.structs.len() + file.enums.len() + file.traits.len();
            parsed.insert(file.file_path.clone(), ParsedFile { file, crate_name: &krate.name });
        }
    }

    let mut wiring = Wiring::default();
    insert_folder_tree(&mut graph, &project.tree, Path::new(""), TreeRoot::New(&project.name), &parsed, &mut wiring);
    wiring.connect(&mut graph);
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
        crate_count: project.crates.len(),
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

    let lookup: HashMap<PathBuf, ParsedFile> =
        parsed.iter().map(|f| (f.file_path.clone(), ParsedFile { file: f, crate_name: "" })).collect();
    let mut wiring = Wiring::seeded_from(graph);
    let before = graph.nodes.len();
    insert_folder_tree(graph, subtree, &rel_prefix, TreeRoot::Existing(cluster_id), &lookup, &mut wiring);
    wiring.connect(graph);
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
                format!(
                    "{} files · {} · {}",
                    group_thousands(info.file_count),
                    human_bytes(info.total_bytes),
                    info.reason.label()
                ),
                Some(LazyFolder {
                    abs_path: tree.root.join(&dir.rel).to_string_lossy().to_string(),
                    file_count: info.file_count,
                    dir_count: info.dir_count,
                    total_bytes: info.total_bytes,
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
                    c.is_collapsed = false;
                    c.subtitle = Some(subtitle);
                }
                cluster_ids.push(existing.to_string());
            }
            _ => {
                let mut cluster = GroupCluster::new(&id, label, "Folder", color);
                cluster.subtitle = Some(subtitle);
                cluster.is_collapsed = lazy.is_some();
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
    }

    wiring.register_file_names(graph, node_id, rel_in_project, &file_name);
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

    fn register_file_names(&mut self, graph: &Graph, node_id: NodeId, rel: &Path, file_name: &str) {
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

    /// Adds member rows/ports for a parsed file and queues what it uses, calls and links to.
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
        let in_port = |id: &str| member_nodes.iter().find(|m| m.id == id).and_then(|m| m.in_port_id);
        let out_port = |id: &str| member_nodes.iter().find(|m| m.id == id).and_then(|m| m.out_port_id);

        self.module_to_file.insert(file.module_name.clone(), node_id);
        if !crate_name.is_empty() {
            self.module_to_file.insert(format!("{}::{}", crate_name, file.module_name), node_id);
        }

        for (member_id, calls) in member_calls(file) {
            let consumer_in = in_port(&member_id);
            for call in calls {
                self.pending_file_deps.push((node_id, call.clone(), EdgeKind::Call));
                if let Some(input) = consumer_in {
                    self.pending_member_calls.push((node_id, input, call.clone()));
                }
            }
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

        for u in &file.uses {
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

    /// Resolves queued names into file-to-file, member-to-member and documentation wires.
    fn connect(self, graph: &mut Graph) {
        self.connect_files(graph);
        self.connect_members(graph);
        self.connect_links(graph);
    }

    /// One wire per (provider, consumer) file pair. An import outranks a call: if the consumer
    /// imports the provider anywhere, the file-level wire is an import.
    fn connect_files(&self, graph: &mut Graph) {
        let mut pairs: Vec<(NodeId, NodeId, EdgeKind)> = Vec::new();
        let mut pair_index: HashMap<(NodeId, NodeId), usize> = HashMap::new();
        for (consumer, target_name, kind) in &self.pending_file_deps {
            let Some(provider) = self.resolve_file(target_name) else { continue };
            if provider == *consumer {
                continue;
            }
            match pair_index.get(&(provider, *consumer)) {
                Some(&i) if *kind == EdgeKind::Import => pairs[i].2 = EdgeKind::Import,
                Some(_) => {}
                None => {
                    pair_index.insert((provider, *consumer), pairs.len());
                    pairs.push((provider, *consumer, *kind));
                }
            }
        }
        for (provider, consumer, kind) in pairs {
            if let (Some(p), Some(c)) = (self.file_ports.get(&provider), self.file_ports.get(&consumer)) {
                graph.connect_kind(provider, p.output, consumer, c.input, kind);
            }
        }
    }

    fn connect_members(&self, graph: &mut Graph) {
        let output_of: HashMap<PortId, PortId> =
            self.members.member_ports.values().map(|m| (m.input, m.output)).collect();
        let mut connected: HashSet<(PortId, PortId)> = HashSet::new();
        for &(consumer, consumer_in, ref target_name) in &self.pending_member_calls {
            let Some((provider, provider_out)) = self.members.resolve_provider(target_name) else { continue };
            // A recursive function would wire to itself.
            let self_loop = output_of.get(&consumer_in) == Some(&provider_out);
            if !self_loop && connected.insert((provider_out, consumer_in)) {
                graph.connect_kind(provider, provider_out, consumer, consumer_in, EdgeKind::Call);
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
            if file_pairs.insert((doc_file, target_node)) {
                graph.connect_kind(doc_file, doc_ports.output, target_node, doc_in, EdgeKind::Documentation);
            }
            if let Some(out) = link_out {
                graph.connect_kind(doc_file, out, target_node, doc_in, EdgeKind::Documentation);
            }
        }
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
