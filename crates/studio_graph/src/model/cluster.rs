use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use super::types::NodeId;

#[derive(Debug, Clone, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct GroupCluster {
    pub id: String,
    pub label: String,
    pub category: String,         // "Crate", "Directory", "Module", "Service", "Infrastructure"
    pub subtitle: Option<String>, // e.g. "Requests to the user service" or "5 files • 420 LOC"
    pub parent_id: Option<String>,
    pub child_cluster_ids: Vec<String>,
    pub color_index: usize,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub node_ids: Vec<NodeId>,
    pub is_collapsed: bool,
    pub depth: usize,
    /// Folder whose contents are not loaded yet (shown collapsed with totals).
    #[serde(default)]
    pub lazy: Option<LazyFolder>,
}

/// A folder listed with totals but not materialized (version control, gitignored, build caches,
/// dependencies). Expanding it loads its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct LazyFolder {
    pub abs_path: String,
    pub file_count: u64,
    pub dir_count: u64,
    pub total_bytes: u64,
    /// Why it starts collapsed, e.g. "gitignored".
    pub reason: String,
}

impl GroupCluster {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        category: impl Into<String>,
        color_index: usize,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: category.into(),
            subtitle: None,
            parent_id: None,
            child_cluster_ids: Vec::new(),
            color_index,
            position: [0.0, 0.0],
            size: [0.0, 0.0],
            node_ids: Vec::new(),
            is_collapsed: false,
            depth: 0,
            lazy: None,
        }
    }
}
