pub mod common;
pub mod files;
pub mod items;
pub mod members;
pub mod modules;

use crate::extractor::ExtractedProject;
use studio_graph::Graph;

pub use files::{build_files_graph, build_skeleton_files_graph, folder_cluster_id, materialize_folder};
pub use items::build_items_graph;
pub use modules::build_modules_graph;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewGranularity {
    #[default]
    FilesAndFolders,
    AllItems,
    PublicApi,
    Modules,
}

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

/// Builds a complete visual Graph from an extracted project.
pub fn build_project_graph(project: &ExtractedProject, granularity: ViewGranularity) -> (Graph, ProjectStats) {
    match granularity {
        ViewGranularity::FilesAndFolders => build_files_graph(project),
        ViewGranularity::AllItems => build_items_graph(project, false),
        ViewGranularity::PublicApi => build_items_graph(project, true),
        ViewGranularity::Modules => build_modules_graph(project),
    }
}
