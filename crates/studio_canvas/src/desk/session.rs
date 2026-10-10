//! The files of the agent session chosen in the Changes district, found on the map: the host
//! sets the files when the session changes, and they are matched to cards once per change of
//! session or of the map (never per frame).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use studio_graph::{Graph, NodeId};

/// The chosen session's files and the cards they are on the map.
#[derive(Debug, Clone, Default)]
pub struct SessionLights {
    /// The session (id) the files are for.
    pub session: Option<String>,
    /// Its files, spelled as the map spells file paths.
    paths: Arc<Vec<String>>,
    /// Bumped whenever the files change.
    generation: u64,
    /// The (scene revision, generation) the cards were last found for.
    resolved_for: Option<(u64, u64)>,
    /// Each file on the map to its card, for one scene revision; built only while lit.
    index: Option<(u64, Arc<HashMap<String, NodeId>>)>,
}

impl SessionLights {
    /// Sets the chosen session and its files (none: nothing is lit).
    pub fn set(&mut self, session: Option<String>, paths: Vec<String>) {
        self.session = session;
        self.paths = Arc::new(paths);
        self.generation += 1;
    }

    /// The files lit.
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// The cards to light, when the files or the map (`revision`) changed since the last call;
    /// `None` when nothing changed.
    pub fn refresh(&mut self, graph: &Graph, revision: u64) -> Option<BTreeSet<NodeId>> {
        let key = (revision, self.generation);
        if self.resolved_for == Some(key) {
            return None;
        }
        self.resolved_for = Some(key);
        if self.paths.is_empty() {
            return Some(BTreeSet::new());
        }
        let index = match &self.index {
            Some((r, index)) if *r == revision => index.clone(),
            _ => {
                let index = Arc::new(path_index(graph));
                self.index = Some((revision, index.clone()));
                index
            }
        };
        Some(resolve(&index, &self.paths))
    }
}

/// Every card with a file, by its path.
pub fn path_index(graph: &Graph) -> HashMap<String, NodeId> {
    graph.nodes.values().filter_map(|n| n.file_path.as_ref().map(|p| (p.clone(), n.id))).collect()
}

/// The cards of `paths` that are on the map.
pub fn resolve(index: &HashMap<String, NodeId>, paths: &[String]) -> BTreeSet<NodeId> {
    paths.iter().filter_map(|p| index.get(p).copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_graph::NodeArchetype;

    /// Cards 1, 2, ... for `files`, in order.
    fn graph(files: &[&str]) -> Graph {
        let mut g = Graph::new();
        for f in files {
            let id = g.add_node(f.rsplit('/').next().unwrap(), NodeArchetype::File, "", None, vec![], vec![], [0.0; 2]);
            g.nodes.get_mut(&id).unwrap().file_path = Some(f.to_string());
        }
        g
    }

    #[test]
    fn touched_files_light_their_cards_once_per_change() {
        let g = graph(&["/p/src/a.rs", "/p/src/b.rs", "/p/README.md"]);
        let mut lights = SessionLights::default();
        let ids = |g: &Graph, files: &[&str]| -> BTreeSet<NodeId> { files.iter().map(|f| path_index(g)[*f]).collect() };
        assert_eq!(lights.refresh(&g, 1), Some(BTreeSet::new()), "nothing chosen: nothing lit");
        assert_eq!(lights.refresh(&g, 1), None, "unchanged");

        lights.set(Some("s1".into()), vec!["/p/src/b.rs".into(), "/p/README.md".into(), "/p/gone.rs".into()]);
        assert_eq!(lights.refresh(&g, 1), Some(ids(&g, &["/p/src/b.rs", "/p/README.md"])), "files not on the map skip");
        assert_eq!(lights.refresh(&g, 1), None);

        // A folder loaded: the map changed, the cards are found again.
        let g2 = graph(&["/p/src/a.rs", "/p/src/b.rs", "/p/README.md", "/p/gone.rs"]);
        assert_eq!(lights.refresh(&g2, 2).map(|s| s.len()), Some(3));

        lights.set(None, Vec::new());
        assert_eq!(lights.refresh(&g2, 2), Some(BTreeSet::new()), "cleared");
    }
}
