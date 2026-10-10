pub mod common;
pub mod files;
pub mod members;
pub mod rust_paths;

use crate::extractor::ExtractedProject;
use studio_graph::Graph;

pub use files::{
    apply_folder_totals, build_files_graph, build_skeleton_files_graph, first_sentence, folder_cluster_id,
    heavy_folder_totals, materialize_folder, summarize_folders,
};

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
    /// R1 on the Code map: which Rust imports and calls became Proven, and what it cost.
    pub r1: rust_paths::RustPathStats,
}

/// Builds the Files & Folders graph for an extracted project.
/// Rust files in a crate's module tree are resolved by R1 first (see [`rust_paths`]).
pub fn build_project_graph(project: &ExtractedProject) -> (Graph, ProjectStats) {
    build_files_graph(project)
}
