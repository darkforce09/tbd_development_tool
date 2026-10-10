use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use studio_graph::FolderDetail;
use studio_parser::{load_folder_contents, materialize_folder, FolderContents};

use super::StudioApp;

/// A collapsed folder being loaded on a background thread.
pub struct FolderLoad {
    pub cluster_id: String,
    pub label: String,
    /// Detail level to show the folder at once it is loaded.
    pub detail: FolderDetail,
    rx: Receiver<FolderContents>,
}

impl StudioApp {
    /// Starts loading a minimised (lazy) folder's contents; it switches to `detail` when the load
    /// finishes.
    pub(crate) fn start_folder_load(&mut self, cluster_id: String, detail: FolderDetail) {
        if self.folder_loads.iter().any(|l| l.cluster_id == cluster_id) {
            return;
        }
        let Some(cluster) = self.graph.clusters.iter_mut().find(|c| c.id == cluster_id) else { return };
        let Some(lazy) = cluster.lazy.clone() else { return };

        let files = lazy.totals.map(|t| format!(" {} files", t.file_count)).unwrap_or_default();
        cluster.subtitle = Some(format!("loading{files}…"));
        let label = cluster.label.clone();
        let (tx, rx) = channel();
        let path = PathBuf::from(&lazy.abs_path);
        let prefix = cluster_id.strip_prefix("dir:").unwrap_or_default().to_string();
        std::thread::spawn(move || {
            let _ = tx.send(load_folder_contents(&path, &prefix));
        });
        self.canvas_state.status_message = Some(format!("Loading {label}{files}…"));
        self.folder_loads.push(FolderLoad { cluster_id, label, detail, rx });
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
            let Some(contents) = result else {
                self.canvas_state.status_message = Some(format!("Loading {} failed", load.label));
                self.palette_folder_failed(&load.cluster_id);
                continue;
            };
            match materialize_folder(&mut self.graph, &load.cluster_id, &contents.tree, &contents.parsed) {
                Ok(added) => {
                    if load.detail != FolderDetail::Open {
                        self.graph.set_folder_detail(&load.cluster_id, load.detail);
                    }
                    self.canvas_state.mark_scene_dirty();
                    self.search_index.push(contents.files);
                    self.search_index.push(contents.symbols);
                    self.canvas_state.status_message = Some(format!("Loaded {}: {} files", load.label, added));
                    self.palette_folder_loaded(&load.cluster_id);
                }
                // The project was reloaded or switched while this folder was loading.
                Err(e) => {
                    self.canvas_state.status_message = Some(format!("Skipped loading {}: {e}", load.label));
                    self.palette_folder_failed(&load.cluster_id);
                }
            }
        }
    }
}
