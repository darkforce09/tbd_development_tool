pub mod common;
pub mod files;
pub mod members;

use crate::extractor::ExtractedProject;
use studio_graph::Graph;

pub use files::{build_files_graph, build_skeleton_files_graph, folder_cluster_id, materialize_folder};

#[derive(Debug, Clone, Default, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(derive(Debug))]
pub struct ProjectStats {
    pub project_name: String,
    pub crate_count: usize,
    pub file_count: usize,
    pub node_count: usize,
    pub wire_count: usize,
    pub function_count: usize,
    pub type_count: usize,
}

/// Builds the Files & Folders graph for an extracted project.
pub fn build_project_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    build_files_graph(project)
}
