use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use studio_graph::{EdgeId, Graph, NodeArchetype, NodeId, PortId};

use crate::builder::members::{attach_member_ports, build_member_nodes, member_calls, MemberPortIndex};
use crate::extractor::{extract_source, ExtractedFile};

/// Outcome of a save + re-parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveReport {
    /// Graph nodes whose content was refreshed.
    pub updated_nodes: usize,
}

/// Saves edited code to disk atomically and refreshes the file cards that belong to that file, using
/// the same extraction and member-building code as a full project load.
pub fn save_and_reparse(path: &Path, new_content: &str, graph: &mut Graph) -> Result<SaveReport, String> {
    crate::edit::atomic_write(path, new_content.as_bytes())
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;

    let extracted = extract_source(path, path, new_content);
    let mut report = SaveReport::default();

    for node_id in nodes_for_path(graph, path) {
        if graph.nodes[&node_id].archetype == NodeArchetype::File {
            refresh_file_node(graph, node_id, &extracted, new_content);
            report.updated_nodes += 1;
        }
    }

    graph.rebuild_fast_indices();
    Ok(report)
}

/// Graph nodes whose `file_path` refers to the same file as `path`.
fn nodes_for_path(graph: &Graph, path: &Path) -> Vec<NodeId> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let file_name = path.file_name();
    let mut canonical_cache: HashMap<&str, PathBuf> = HashMap::new();

    graph
        .nodes
        .iter()
        .filter(|(_, n)| {
            let Some(p) = n.file_path.as_deref() else { return false };
            if Path::new(p) == path {
                return true;
            }
            if Path::new(p).file_name() != file_name {
                return false;
            }
            let resolved = canonical_cache
                .entry(p)
                .or_insert_with(|| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p)));
            *resolved == canonical
        })
        .map(|(&id, _)| id)
        .collect()
}

/// Rebuilds a file card's members, keeping port ids for members that still exist so their edges
/// survive, and re-wires the card's outgoing member call edges.
fn refresh_file_node(graph: &mut Graph, node_id: NodeId, extracted: &ExtractedFile, content: &str) {
    let mut members = build_member_nodes(extracted);
    if members.is_empty() && extracted.parse_error.is_some() {
        // Mid-edit syntax error: keep the last good members and edges.
        if let Some(node) = graph.nodes.get_mut(&node_id) {
            if node.source_code.is_some() {
                node.source_code = Some(content.to_string());
            }
        }
        return;
    }
    let old_ports: HashMap<String, (Option<PortId>, Option<PortId>)> =
        graph.nodes[&node_id].member_nodes.iter().map(|m| (m.id.clone(), (m.in_port_id, m.out_port_id))).collect();

    for m in &mut members {
        if let Some(&(in_pid, out_pid)) = old_ports.get(&m.id) {
            m.in_port_id = in_pid;
            m.out_port_id = out_pid;
        }
    }

    let kept: HashSet<&str> = members.iter().map(|m| m.id.as_str()).collect();
    let vanished: Vec<PortId> = old_ports
        .iter()
        .filter(|(id, _)| !kept.contains(id.as_str()))
        .flat_map(|(_, &(i, o))| [i, o])
        .flatten()
        .collect();
    for pid in &vanished {
        graph.disconnect_port(node_id, *pid);
    }
    if let Some(node) = graph.nodes.get_mut(&node_id) {
        node.inputs.retain(|p| !vanished.contains(&p.id));
        node.outputs.retain(|p| !vanished.contains(&p.id));
    }

    attach_member_ports(graph, node_id, &mut members);

    let out_port_by_member: HashMap<String, PortId> =
        members.iter().filter_map(|m| Some((m.id.clone(), m.out_port_id?))).collect();
    if let Some(node) = graph.nodes.get_mut(&node_id) {
        if node.source_code.is_some() {
            node.source_code = Some(content.to_string());
        }
        node.member_nodes = members;
    }

    rewire_member_calls(graph, node_id, extracted, &out_port_by_member);
}

/// Re-wires the card's outgoing member call edges from the new extraction, in the same order and
/// with the same input-port semantics as the Files builder, so a save matches a fresh load.
fn rewire_member_calls(
    graph: &mut Graph,
    node_id: NodeId,
    extracted: &ExtractedFile,
    out_port_by_member: &HashMap<String, PortId>,
) {
    let member_out_ports: HashSet<PortId> = out_port_by_member.values().copied().collect();
    let stale: Vec<EdgeId> = graph
        .edges
        .iter()
        .filter(|e| e.from_node == node_id && member_out_ports.contains(&e.from_port))
        .map(|e| e.id)
        .collect();
    for edge_id in stale {
        graph.disconnect_edge(edge_id);
    }

    let index = MemberPortIndex::from_graph(graph);
    let mut step = graph.edges.iter().filter_map(|e| e.step_number).max().unwrap_or(0) + 1;
    let mut connected: HashSet<(PortId, NodeId, PortId)> = HashSet::new();
    for (member_id, calls) in member_calls(extracted) {
        let Some(&out_port) = out_port_by_member.get(&member_id) else { continue };
        for call in calls {
            let Some((target_node, in_port)) = index.resolve(call) else { continue };
            if (target_node != node_id || in_port != out_port) && connected.insert((out_port, target_node, in_port)) {
                graph.connect_labeled(
                    node_id,
                    out_port,
                    target_node,
                    in_port,
                    Some("call".to_string()),
                    Some(step),
                    None,
                );
                step += 1;
            }
        }
    }
}
