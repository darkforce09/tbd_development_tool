use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use super::types::{FileMemberNode, NodeArchetype, NodeId, Port, PortDirection, PortId};

#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct Node {
    pub id: NodeId,
    pub title: String,
    pub archetype: NodeArchetype,
    pub description: String,
    pub badge: Option<String>,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
    pub position: [f32; 2],
    pub size: [f32; 2],

    // Rich metadata for CodeSee, Haystack & Code Canvas
    pub file_path: Option<String>,
    pub line_number: Option<usize>,
    pub crate_name: Option<String>,
    pub module_path: Option<String>,
    pub doc_comment: Option<String>,
    pub source_code: Option<String>,
    pub group_id: Option<String>,
    pub is_code_expanded: bool,
    pub is_markdown_preview: bool,
    pub expanded_tab: usize,
    pub scroll_offset_y: f32,

    // File Node Dropdown Expansion & Members
    pub is_dropdown_expanded: bool,
    pub show_member_wires: bool,
    pub expanded_member_id: Option<String>,
    pub member_nodes: Vec<FileMemberNode>,
}

impl Node {
    pub fn find_port(&self, port_id: PortId) -> Option<&Port> {
        self.inputs
            .iter()
            .chain(self.outputs.iter())
            .find(|p| p.id == port_id)
    }

    pub fn port_index(&self, port_id: PortId) -> Option<(usize, PortDirection)> {
        if let Some(idx) = self.inputs.iter().position(|p| p.id == port_id) {
            return Some((idx, PortDirection::Input));
        }
        if let Some(idx) = self.outputs.iter().position(|p| p.id == port_id) {
            return Some((idx, PortDirection::Output));
        }
        None
    }
}
