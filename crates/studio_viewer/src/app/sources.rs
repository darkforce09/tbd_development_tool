//! The project's sources besides its code (packages, git, tools, ...): started when a project
//! opens, read off the UI thread, and put on the map when they arrive. Results are kept here
//! and put on the graph again whenever the graph is replaced.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use studio_graph::Graph;
use studio_sources::{
    disk_job, packages_job, settings_job, tools_job, GitDisk, Packages, Settings, SourceEvent, SourceHub, SourceKind,
    SourceState, Tools,
};

use super::StudioApp;

/// One project's sources.
pub struct ProjectSources {
    hub: SourceHub,
    pub status: BTreeMap<SourceKind, SourceState>,
    pub packages: Option<Arc<Packages>>,
    pub tools: Option<Arc<Tools>>,
    pub disk: Option<Arc<GitDisk>>,
    /// Bytes on disk of top-level and ignored entries, relative to the root.
    pub sizes: BTreeMap<PathBuf, u64>,
    pub settings: Option<Arc<Settings>>,
    /// Whether the jobs that need the parsed project have been started.
    started: bool,
}

impl ProjectSources {
    pub fn new(root: &Path, ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        Self {
            hub: SourceHub::new(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()), move || {
                ctx.request_repaint()
            }),
            status: BTreeMap::new(),
            packages: None,
            tools: None,
            disk: None,
            sizes: BTreeMap::new(),
            settings: None,
            started: false,
        }
    }
}

impl StudioApp {
    /// Starts the sources once the project's files are known, and takes in what arrived.
    /// Called every frame; costs nothing when nothing arrived.
    pub(crate) fn poll_sources(&mut self) {
        let Some(sources) = &mut self.sources else { return };
        if !sources.started && self.project_stats.is_some() && !self.is_loading {
            sources.started = true;
            let manifests = cargo_manifests(&self.graph);
            let json: Vec<PathBuf> = self
                .graph
                .nodes
                .values()
                .filter(|n| n.title.ends_with(".json"))
                .filter_map(|n| n.file_path.as_ref().map(PathBuf::from))
                .collect();
            sources.hub.spawn(SourceKind::Packages, packages_job(manifests.clone()));
            sources.hub.spawn(SourceKind::Disk, disk_job());
            sources.hub.spawn(SourceKind::Settings, settings_job(manifests, json));
        }
        let mut changed = false;
        for event in sources.hub.drain() {
            match event {
                SourceEvent::Status { source, state } => {
                    sources.status.insert(source, state);
                }
                SourceEvent::Packages(packages) => {
                    // Tools need the packages (their binaries), and the files on disk.
                    let files: Vec<PathBuf> =
                        self.graph.nodes.values().filter_map(|n| n.file_path.as_ref().map(PathBuf::from)).collect();
                    sources.hub.spawn(SourceKind::Tools, tools_job(packages.clone(), files));
                    sources.packages = Some(packages);
                    changed = true;
                }
                SourceEvent::Tools(tools) => {
                    sources.tools = Some(tools);
                    changed = true;
                }
                SourceEvent::GitDisk(disk) => {
                    sources.disk = Some(disk);
                    changed = true;
                }
                SourceEvent::FolderSize(path, bytes) => {
                    sources.sizes.insert(path, bytes);
                    changed = true;
                }
                // Read by nothing yet: the Changes district (stage S6) starts the history job.
                SourceEvent::GitHistory(_) => {}
                SourceEvent::Settings(settings) => {
                    sources.settings = Some(settings);
                    changed = true;
                }
            }
        }
        if changed {
            self.apply_sources();
        }
    }

    /// Puts what the sources read on the graph: run after they arrive and after the graph is
    /// replaced.
    pub(crate) fn apply_sources(&mut self) {
        let (Some(sources), Some(root)) = (&self.sources, &self.current_project_path) else { return };
        let files = super::districts::files_view(sources.disk.as_deref(), &sources.sizes, sources.settings.as_deref());
        self.canvas_state.districts.files = Some(Arc::new(files));
        if let Some(packages) = &sources.packages {
            mark_packages(&mut self.graph, root, packages);
            if let Some(tools) = &sources.tools {
                let view = super::districts::run_view(tools, packages, &self.graph);
                self.canvas_state.districts.run = Some(Arc::new(view));
            }
        }
    }
}

/// Every `Cargo.toml` in the project, as found on disk.
fn cargo_manifests(graph: &Graph) -> Vec<PathBuf> {
    graph
        .nodes
        .values()
        .filter(|n| n.title == "Cargo.toml")
        .filter_map(|n| n.file_path.as_ref().map(PathBuf::from))
        .collect()
}

/// Marks the folder of each package with what it is and builds, e.g. "crate · 2 binaries".
fn mark_packages(graph: &mut Graph, root: &Path, packages: &Packages) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    for package in &packages.packages {
        let folder = package.root.canonicalize().unwrap_or_else(|_| package.root.clone());
        let Ok(rel) = folder.strip_prefix(&root) else { continue };
        let id = studio_parser::folder_cluster_id(rel);
        let Some(cluster) = graph.clusters.iter_mut().find(|c| c.id == id) else { continue };
        let bins = package.targets.iter().filter(|t| t.kind == studio_sources::TargetKind::Bin).count();
        cluster.category = match bins {
            0 => format!("crate {}", package.name),
            1 => format!("crate {} · 1 binary", package.name),
            n => format!("crate {} · {n} binaries", package.name),
        };
    }
}
