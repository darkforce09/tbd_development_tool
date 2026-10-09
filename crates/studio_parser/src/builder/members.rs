use crate::extractor::{CodeLang, ExtractedFile, SourceLang};
use std::collections::HashMap;
use std::path::Path;
use studio_graph::{DataType, FileMemberNode, Graph, NodeArchetype, NodeId, Port, PortDirection, PortId};

/// Lowercased file extension used for per-language labels.
pub fn file_ext(file: &ExtractedFile) -> String {
    file.relative_path
        .extension()
        .or_else(|| file.file_path.extension())
        .and_then(|e| e.to_str())
        .unwrap_or("rs")
        .to_lowercase()
}

/// Builds the inline member list (functions, types, methods, headings, links) shown inside a file card,
/// sorted by line. Ports are not assigned; see [`attach_member_ports`].
/// Keyword shown before function names in member signatures (`def`, `func`, `fn`, ...).
fn fn_prefix(file: &ExtractedFile, is_async: bool) -> String {
    let keyword = match file.language {
        SourceLang::Code(lang) => lang.fn_keyword(),
        SourceLang::Enforce => "",
        _ => "fn",
    };
    match (is_async, keyword) {
        (true, "") => "async ".to_string(),
        (true, k) => format!("async {k} "),
        (false, "") => String::new(),
        (false, k) => format!("{k} "),
    }
}

/// Languages whose struct-like items are classes (shown as `class X : Base`).
fn has_classes(file: &ExtractedFile) -> bool {
    match file.language {
        SourceLang::Enforce => true,
        SourceLang::Code(lang) => !matches!(lang, CodeLang::C | CodeLang::Go | CodeLang::Zig),
        _ => false,
    }
}

pub fn build_member_nodes(file: &ExtractedFile) -> Vec<FileMemberNode> {
    let is_enforce = file.language == SourceLang::Enforce;
    let is_class_lang = has_classes(file);
    let mut member_nodes = Vec::new();

    for s in &file.structs {
        let is_heading = s.derives.first().map(|d| d.starts_with('H')).unwrap_or(false);
        let is_modded = s.derives.contains(&"modded".to_string());
        let (archetype, vis, sig) = if is_heading {
            let lvl = s.derives.first().cloned().unwrap_or_else(|| "H1".to_string());
            (NodeArchetype::Module, lvl, s.name.clone())
        } else if is_modded {
            (NodeArchetype::Struct, "MOD".to_string(), format!("modded class {}", s.name))
        } else if is_class_lang {
            let base = s.derives.first().map(|b| format!(" : {}", b)).unwrap_or_default();
            (NodeArchetype::Struct, "CLS".to_string(), format!("class {}{}", s.name, base))
        } else {
            let vis = if s.visibility.is_public() { "pub" } else { "" };
            (NodeArchetype::Struct, vis.to_string(), format!("struct {} ({} fields)", s.name, s.fields.len()))
        };

        member_nodes.push(FileMemberNode::new(
            format!("struct:{}", s.name),
            &s.name,
            archetype,
            vis,
            sig,
            s.line,
            &s.source_code,
            if s.docs.is_empty() { None } else { Some(s.docs.clone()) },
        ));
    }

    for e in &file.enums {
        let vis = if e.visibility.is_public() { "pub" } else { "" };
        member_nodes.push(FileMemberNode::new(
            format!("enum:{}", e.name),
            &e.name,
            NodeArchetype::Enum,
            vis,
            format!("enum {} ({} variants)", e.name, e.variants.len()),
            e.line,
            &e.source_code,
            if e.docs.is_empty() { None } else { Some(e.docs.clone()) },
        ));
    }

    for t in &file.traits {
        let vis = if t.visibility.is_public() { "pub" } else { "" };
        let kind_label = match file.language {
            SourceLang::Code(lang) => lang.interface_label(),
            _ => "trait",
        };
        member_nodes.push(FileMemberNode::new(
            format!("trait:{}", t.name),
            &t.name,
            NodeArchetype::Trait,
            vis,
            format!("{} {} ({} methods)", kind_label, t.name, t.methods.len()),
            t.line,
            &t.source_code,
            if t.docs.is_empty() { None } else { Some(t.docs.clone()) },
        ));
    }

    for f in &file.functions {
        let is_link = f.docs.starts_with("Markdown link");
        let is_code_block = f.name.starts_with("block:");
        let (archetype, vis, sig) = if is_link {
            (NodeArchetype::Function, "LNK".to_string(), format!("[{}]", f.name))
        } else if is_code_block {
            (NodeArchetype::Module, "CODE".to_string(), f.name.clone())
        } else {
            let vis = if f.visibility.is_public() { "pub" } else { "" };
            let params_sig =
                f.inputs.iter().map(|p| format!("{}: {}", p.name, p.type_str)).collect::<Vec<_>>().join(", ");
            let ret_sig = f.output.as_ref().map(|o| format!(" -> {}", o)).unwrap_or_default();
            let prefix = fn_prefix(file, f.is_async);
            (NodeArchetype::Function, vis.to_string(), format!("{}{}({}){}", prefix, f.name, params_sig, ret_sig))
        };

        member_nodes.push(FileMemberNode::new(
            format!("fn:{}", f.name),
            &f.name,
            archetype,
            vis,
            sig,
            f.line,
            &f.source_code,
            if f.docs.is_empty() { None } else { Some(f.docs.clone()) },
        ));
    }

    for imp in &file.impls {
        for m in &imp.methods {
            let vis = if is_enforce {
                "FN"
            } else if m.visibility.is_public() {
                "pub"
            } else {
                ""
            };
            let params_sig =
                m.inputs.iter().map(|p| format!("{}: {}", p.name, p.type_str)).collect::<Vec<_>>().join(", ");
            let ret_sig = m.output.as_ref().map(|o| format!(" -> {}", o)).unwrap_or_default();
            let prefix = fn_prefix(file, m.is_async);
            member_nodes.push(FileMemberNode::new(
                format!("method:{}::{}", imp.target_type, m.name),
                format!("{}::{}", imp.target_type, m.name),
                NodeArchetype::Function,
                vis,
                format!("{}{}({}){}", prefix, m.name, params_sig, ret_sig),
                m.line,
                &m.source_code,
                if m.docs.is_empty() { None } else { Some(m.docs.clone()) },
            ));
        }
    }

    member_nodes.sort_by_key(|m| m.line_number);
    member_nodes
}

/// Calls made by each callable member, keyed by member id.
pub fn member_calls(file: &ExtractedFile) -> Vec<(String, &[String])> {
    let mut out: Vec<(String, &[String])> =
        file.functions.iter().map(|f| (format!("fn:{}", f.name), f.calls.as_slice())).collect();
    for imp in &file.impls {
        for m in &imp.methods {
            out.push((format!("method:{}::{}", imp.target_type, m.name), m.calls.as_slice()));
        }
    }
    out
}

/// Gives every member without ports a fresh in/out port pair on the owning file node.
/// Members that already carry port ids (reused across a re-parse) are left untouched.
pub fn attach_member_ports(graph: &mut Graph, node_id: NodeId, members: &mut [FileMemberNode]) {
    for m in members.iter_mut().filter(|m| m.in_port_id.is_none() || m.out_port_id.is_none()) {
        let in_pid = PortId(graph.next_raw_id());
        let out_pid = PortId(graph.next_raw_id());
        m.in_port_id = Some(in_pid);
        m.out_port_id = Some(out_pid);

        if let Some(n) = graph.nodes.get_mut(&node_id) {
            n.inputs.push(Port {
                id: in_pid,
                name: format!("{}:in", m.name),
                data_type: DataType::RustFlow,
                direction: PortDirection::Input,
            });
            n.outputs.push(Port {
                id: out_pid,
                name: format!("{}:out", m.name),
                data_type: DataType::RustFlow,
                direction: PortDirection::Output,
            });
        }
    }
}

/// Name → port lookups used to wire member-level call/link edges between file cards.
#[derive(Default)]
pub struct MemberPortIndex {
    pub member_in_ports: HashMap<String, (NodeId, PortId)>,
    pub file_to_node: HashMap<String, NodeId>,
    pub file_to_in_port: HashMap<NodeId, PortId>,
}

impl MemberPortIndex {
    /// Rebuilds the index from an existing Files graph (same keys the builder registers).
    pub fn from_graph(graph: &Graph) -> Self {
        let mut index = Self::default();
        for (&node_id, node) in graph.nodes.iter().filter(|(_, n)| n.archetype == NodeArchetype::File) {
            if let Some(in_port) = node.inputs.first() {
                index.file_to_in_port.insert(node_id, in_port.id);
            }
            index.file_to_node.insert(node.title.clone(), node_id);
            if let Some(stem) = Path::new(&node.title).file_stem().and_then(|s| s.to_str()) {
                index.file_to_node.insert(stem.to_string(), node_id);
            }
            for m in &node.member_nodes {
                if let Some(in_pid) = m.in_port_id {
                    index.member_in_ports.insert(m.name.clone(), (node_id, in_pid));
                    index.member_in_ports.insert(m.id.clone(), (node_id, in_pid));
                }
            }
        }
        index
    }

    /// Resolves a call/link target name to a member in-port, falling back to a file's in-port.
    pub fn resolve(&self, target_name: &str) -> Option<(NodeId, PortId)> {
        let clean = target_name.trim_start_matches("./");
        let short_name = target_name.split([':', '.', '>', '-']).rfind(|s| !s.is_empty()).unwrap_or(target_name);

        self.member_in_ports
            .get(target_name)
            .or_else(|| self.member_in_ports.get(clean))
            .or_else(|| self.member_in_ports.get(short_name))
            .copied()
            .or_else(|| {
                // A markdown link or call that names a file directly wires to that file's in-port.
                self.file_to_node
                    .get(target_name)
                    .or_else(|| self.file_to_node.get(clean))
                    .or_else(|| self.file_to_node.get(short_name))
                    .and_then(|tid| self.file_to_in_port.get(tid).map(|&pid| (*tid, pid)))
            })
    }
}
