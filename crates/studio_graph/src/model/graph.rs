use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use super::cluster::GroupCluster;
use super::edge::Edge;
use super::node::Node;
use super::types::{DataType, EdgeId, NodeArchetype, NodeId, Port, PortDirection, PortId};

#[derive(Debug, Clone, Default, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct Graph {
    pub nodes: BTreeMap<NodeId, Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<GroupCluster>,
    next_id: u64,

    // Fast O(1) indices (ignored by serialization, rebuilt on load)
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub port_edges: HashMap<(NodeId, PortId), Vec<EdgeId>>,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub edge_indices: HashMap<EdgeId, usize>,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub node_degrees: HashMap<NodeId, u32>,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub isolated_nodes_count: usize,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub archetype_counts: [usize; 10],
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub collapsed_clusters_count: usize,
    #[serde(skip)]
    #[rkyv(with = rkyv::with::Skip)]
    pub collapsed_node_ids: HashSet<NodeId>,
}

impl Graph {
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            clusters: Vec::new(),
            next_id: 1,
            port_edges: HashMap::new(),
            edge_indices: HashMap::new(),
            node_degrees: HashMap::new(),
            isolated_nodes_count: 0,
            archetype_counts: [0; 10],
            collapsed_clusters_count: 0,
            collapsed_node_ids: HashSet::new(),
        }
    }

    pub fn rebuild_fast_indices(&mut self) {
        self.port_edges.clear();
        self.edge_indices.clear();
        self.node_degrees.clear();
        self.archetype_counts = [0; 10];
        self.rebuild_collapsed_cache();

        for (&id, node) in &self.nodes {
            self.node_degrees.insert(id, 0);
            let idx = node.archetype.index();
            if idx < 10 {
                self.archetype_counts[idx] += 1;
            }
        }

        for (idx, edge) in self.edges.iter().enumerate() {
            self.edge_indices.insert(edge.id, idx);
            self.port_edges.entry((edge.from_node, edge.from_port)).or_default().push(edge.id);
            self.port_edges.entry((edge.to_node, edge.to_port)).or_default().push(edge.id);
            *self.node_degrees.entry(edge.from_node).or_insert(0) += 1;
            *self.node_degrees.entry(edge.to_node).or_insert(0) += 1;
        }

        self.isolated_nodes_count = self.node_degrees.values().filter(|&&deg| deg == 0).count();
    }

    fn index_edge(&mut self, edge: &Edge) {
        self.port_edges.entry((edge.from_node, edge.from_port)).or_default().push(edge.id);
        self.port_edges.entry((edge.to_node, edge.to_port)).or_default().push(edge.id);

        let from_deg = self.node_degrees.entry(edge.from_node).or_insert(0);
        if *from_deg == 0 && self.isolated_nodes_count > 0 {
            self.isolated_nodes_count -= 1;
        }
        *from_deg += 1;

        let to_deg = self.node_degrees.entry(edge.to_node).or_insert(0);
        if *to_deg == 0 && self.isolated_nodes_count > 0 {
            self.isolated_nodes_count -= 1;
        }
        *to_deg += 1;
    }

    fn unindex_edge(&mut self, edge: &Edge) {
        if let Some(list) = self.port_edges.get_mut(&(edge.from_node, edge.from_port)) {
            list.retain(|&id| id != edge.id);
        }
        if let Some(list) = self.port_edges.get_mut(&(edge.to_node, edge.to_port)) {
            list.retain(|&id| id != edge.id);
        }

        if let Some(deg) = self.node_degrees.get_mut(&edge.from_node) {
            if *deg > 0 {
                *deg -= 1;
                if *deg == 0 {
                    self.isolated_nodes_count += 1;
                }
            }
        }
        if let Some(deg) = self.node_degrees.get_mut(&edge.to_node) {
            if *deg > 0 {
                *deg -= 1;
                if *deg == 0 {
                    self.isolated_nodes_count += 1;
                }
            }
        }
    }

    pub fn next_raw_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_node(
        &mut self,
        title: impl Into<String>,
        archetype: NodeArchetype,
        description: impl Into<String>,
        badge: Option<String>,
        input_defs: Vec<(String, DataType)>,
        output_defs: Vec<(String, DataType)>,
        position: [f32; 2],
    ) -> NodeId {
        let node_id = NodeId(self.next_raw_id());

        let mut inputs = Vec::new();
        for (name, dt) in input_defs {
            let port_id = PortId(self.next_raw_id());
            inputs.push(Port { id: port_id, name, data_type: dt, direction: PortDirection::Input });
        }

        let mut outputs = Vec::new();
        for (name, dt) in output_defs {
            let port_id = PortId(self.next_raw_id());
            outputs.push(Port { id: port_id, name, data_type: dt, direction: PortDirection::Output });
        }

        let title_str = title.into();
        let desc_str = description.into();
        let desc_height = if !desc_str.is_empty() { 24.0 } else { 0.0 };

        let max_ports = inputs.len().max(outputs.len()).max(1);
        let calculated_height = 34.0 + desc_height + (max_ports as f32 * 26.0) + 12.0;

        let title_chars = title_str.chars().count();
        let badge_chars = badge.as_ref().map(|b| b.chars().count()).unwrap_or(0);
        let header_need = (title_chars * 8) + (badge_chars * 7) + 85;

        let max_in_chars = inputs.iter().map(|p| p.name.chars().count()).max().unwrap_or(0);
        let max_out_chars = outputs.iter().map(|p| p.name.chars().count()).max().unwrap_or(0);
        let ports_need = if !inputs.is_empty() && !outputs.is_empty() {
            (max_in_chars + max_out_chars) * 7 + 45
        } else {
            max_in_chars.max(max_out_chars) * 7 + 35
        };

        let calculated_width = (header_need.max(ports_need) as f32).clamp(260.0, 360.0);
        let size = [calculated_width, calculated_height];
        let is_markdown_preview = badge.as_deref() == Some("MD");

        let node = Node {
            id: node_id,
            title: title_str,
            archetype,
            description: desc_str,
            badge,
            inputs,
            outputs,
            position,
            size,
            file_path: None,
            line_number: None,
            crate_name: None,
            module_path: None,
            doc_comment: None,
            source_code: None,
            group_id: None,
            is_code_expanded: false,
            is_markdown_preview,
            expanded_tab: 0,
            scroll_offset_y: 0.0,
            is_dropdown_expanded: false,
            show_member_wires: false,
            expanded_member_id: None,
            member_nodes: Vec::new(),
        };

        let arch_idx = archetype.index();
        if arch_idx < 10 {
            self.archetype_counts[arch_idx] += 1;
        }
        self.node_degrees.insert(node_id, 0);
        self.isolated_nodes_count += 1;
        self.nodes.insert(node_id, node);
        node_id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_node_metadata(
        &mut self,
        node_id: NodeId,
        file_path: Option<String>,
        line_number: Option<usize>,
        crate_name: Option<String>,
        module_path: Option<String>,
        doc_comment: Option<String>,
        source_code: Option<String>,
        group_id: Option<String>,
    ) {
        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.file_path = file_path;
            node.line_number = line_number;
            node.crate_name = crate_name;
            node.module_path = module_path;
            node.doc_comment = doc_comment;
            node.source_code = source_code;
            node.group_id = group_id;
        }
    }

    pub fn remove_node(&mut self, node_id: NodeId) {
        if let Some(removed_node) = self.nodes.remove(&node_id) {
            let arch_idx = removed_node.archetype.index();
            if arch_idx < 10 && self.archetype_counts[arch_idx] > 0 {
                self.archetype_counts[arch_idx] -= 1;
            }
            if let Some(deg) = self.node_degrees.remove(&node_id) {
                if deg == 0 && self.isolated_nodes_count > 0 {
                    self.isolated_nodes_count -= 1;
                }
            }

            let mut removed_edges = Vec::new();
            self.edges.retain(|e| {
                if e.from_node == node_id || e.to_node == node_id {
                    removed_edges.push(e.clone());
                    false
                } else {
                    true
                }
            });

            for e in removed_edges {
                self.unindex_edge(&e);
            }

            for c in &mut self.clusters {
                c.node_ids.retain(|&id| id != node_id);
            }
        }
    }

    pub fn connect(
        &mut self,
        from_node: NodeId,
        from_port: PortId,
        to_node: NodeId,
        to_port: PortId,
    ) -> Option<EdgeId> {
        self.connect_labeled(from_node, from_port, to_node, to_port, None, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn connect_labeled(
        &mut self,
        from_node: NodeId,
        from_port: PortId,
        to_node: NodeId,
        to_port: PortId,
        label: Option<String>,
        step_number: Option<usize>,
        source_line: Option<usize>,
    ) -> Option<EdgeId> {
        // Disconnect any existing edge feeding this exact input port (O(1) lookup via port_edges)
        if let Some(existing_edge_ids) = self.port_edges.get(&(to_node, to_port)).cloned() {
            for eid in existing_edge_ids {
                if let Some(pos) =
                    self.edges.iter().position(|e| e.id == eid && e.to_node == to_node && e.to_port == to_port)
                {
                    let edge = self.edges.remove(pos);
                    self.unindex_edge(&edge);
                }
            }
        }

        let edge_id = EdgeId(self.next_raw_id());
        let edge = Edge { id: edge_id, from_node, from_port, to_node, to_port, label, step_number, source_line };
        self.index_edge(&edge);
        self.edges.push(edge);
        Some(edge_id)
    }

    pub fn disconnect_edge(&mut self, edge_id: EdgeId) -> bool {
        if let Some(pos) = self.edges.iter().position(|e| e.id == edge_id) {
            let edge = self.edges.remove(pos);
            self.unindex_edge(&edge);
            true
        } else {
            false
        }
    }

    pub fn disconnect_port(&mut self, node_id: NodeId, port_id: PortId) -> Vec<EdgeId> {
        let mut removed = Vec::new();
        let mut removed_edges = Vec::new();
        self.edges.retain(|e| {
            let matches =
                (e.from_node == node_id && e.from_port == port_id) || (e.to_node == node_id && e.to_port == port_id);
            if matches {
                removed.push(e.id);
                removed_edges.push(e.clone());
                false
            } else {
                true
            }
        });
        for e in removed_edges {
            self.unindex_edge(&e);
        }
        removed
    }

    pub fn find_port(&self, node_id: NodeId, port_id: PortId) -> Option<&Port> {
        self.nodes.get(&node_id).and_then(|n| n.find_port(port_id))
    }

    #[inline]
    pub fn is_port_connected(&self, node_id: NodeId, port_id: PortId) -> bool {
        self.port_edges.get(&(node_id, port_id)).is_some_and(|edges| !edges.is_empty())
    }

    #[inline]
    pub fn get_port_edges(&self, node_id: NodeId, port_id: PortId) -> Vec<EdgeId> {
        self.port_edges.get(&(node_id, port_id)).cloned().unwrap_or_default()
    }

    #[inline]
    pub fn get_edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edge_indices.get(&id).and_then(|&idx| self.edges.get(idx))
    }

    #[inline]
    pub fn isolated_nodes_count(&self) -> usize {
        self.isolated_nodes_count
    }

    #[inline]
    pub fn archetype_count(&self, arch: NodeArchetype) -> usize {
        let idx = arch.index();
        if idx < 10 {
            self.archetype_counts[idx]
        } else {
            0
        }
    }

    /// Serializes the graph to aligned rkyv byte buffer.
    pub fn to_rkyv_bytes(&self) -> Result<Vec<u8>, rkyv::rancor::Error> {
        let aligned = rkyv::to_bytes::<rkyv::rancor::Error>(self)?;
        Ok(aligned.into_vec())
    }

    /// Deserializes a Graph from raw bytes produced by rkyv and rebuilds all fast indices.
    pub fn from_rkyv_bytes(bytes: &[u8]) -> Result<Self, rkyv::rancor::Error> {
        let archived = rkyv::access::<ArchivedGraph, rkyv::rancor::Error>(bytes)?;
        let mut graph: Graph = rkyv::deserialize::<Graph, rkyv::rancor::Error>(archived)?;
        graph.rebuild_fast_indices();
        Ok(graph)
    }
}
