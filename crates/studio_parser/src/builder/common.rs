use std::collections::HashMap;
use studio_graph::{NodeId, PortId};

#[allow(dead_code)]
pub struct NodeLookup {
    pub node_id: NodeId,
    pub exec_in_port: Option<PortId>,
    pub call_out_port: Option<PortId>,
    pub self_out_port: Option<PortId>,
    pub type_inputs: HashMap<String, PortId>,
    pub type_outputs: HashMap<String, PortId>,
}

pub fn truncate_str(s: &str, max_len: usize) -> &str {
    match s.char_indices().nth(max_len) {
        None => s,
        Some((idx, _)) => &s[..idx],
    }
}

pub fn clean_type_key(s: &str) -> String {
    let clean = s
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim();
    // If it's a path like `crate::model::NodeId`, take the last segment
    clean.split("::").last().unwrap_or(clean).to_string()
}
