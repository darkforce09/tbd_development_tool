use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use studio_parser::extractor::ExtractedFile;
use studio_parser::tree::ProjectTree;
use studio_parser::{load_folder_contents, materialize_folder, SymbolSearchIndex};

use super::StudioApp;

/// A collapsed folder being loaded on a background thread.
pub struct FolderLoad {
    pub cluster_id: String,
    pub label: String,
    rx: Receiver<(ProjectTree, Vec<ExtractedFile>)>,
}

impl StudioApp {
    /// Starts loading a collapsed (lazy) folder's contents; it expands when the load finishes.
    pub(crate) fn start_folder_load(&mut self, cluster_id: String) {
        if self.folder_loads.iter().any(|l| l.cluster_id == cluster_id) {
            return;
        }
        let Some(cluster) = self.graph.clusters.iter_mut().find(|c| c.id == cluster_id) else { return };
        let Some(lazy) = cluster.lazy.clone() else { return };

        cluster.subtitle = Some(format!("loading {} files…", lazy.file_count));
        let label = cluster.label.clone();
        let (tx, rx) = channel();
        let path = PathBuf::from(&lazy.abs_path);
        std::thread::spawn(move || {
            let _ = tx.send(load_folder_contents(&path));
        });
        self.canvas_state.status_message = Some(format!("Loading {label} ({} files)…", lazy.file_count));
        self.folder_loads.push(FolderLoad { cluster_id, label, rx });
    }

    /// Merges finished folder loads into the graph. Called every frame.
    pub(crate) fn poll_folder_loads(&mut self) {
        let mut finished = Vec::new();
        for (i, load) in self.folder_loads.iter().enumerate() {
            match load.rx.try_recv() {
                Ok(result) => finished.push((i, Some(result))),
                Err(TryRecvError::Disconnected) => finished.push((i, None)),
                Err(TryRecvError::Empty) => {}
            }
        }
        for (i, result) in finished.into_iter().rev() {
            let load = self.folder_loads.remove(i);
            let Some((tree, parsed)) = result else {
                self.canvas_state.status_message = Some(format!("Loading {} failed", load.label));
                continue;
            };
            match materialize_folder(&mut self.graph, &load.cluster_id, &tree, &parsed) {
                Ok(added) => {
                    self.canvas_state.mark_scene_dirty();
                    self.search_index = SymbolSearchIndex::build(&self.graph);
                    self.canvas_state.status_message = Some(format!("Loaded {}: {} files", load.label, added));
                }
                // The project was reloaded or switched while this folder was loading.
                Err(e) => self.canvas_state.status_message = Some(format!("Skipped loading {}: {e}", load.label)),
            }
        }
    }
}
