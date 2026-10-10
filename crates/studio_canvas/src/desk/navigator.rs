//! The Desk's navigator: Finder-style columns from the top of the project down to a file. The
//! first column lists the project's parts; each pick opens the next column. A folder holding only
//! one folder is folded into it (`src/main/java`), so deep layouts take fewer clicks.

use studio_graph::{Graph, GroupCluster, NodeId};

/// What is picked in the columns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Navigator {
    /// The folder (cluster id) picked in each column, left to right.
    pub path: Vec<String>,
    /// The file picked last.
    pub file: Option<NodeId>,
}

/// One column: the contents of a folder.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigatorColumn {
    pub folder: String,
    pub title: String,
    pub rows: Vec<NavigatorRow>,
}

/// A row of a column.
#[derive(Debug, Clone, PartialEq)]
pub enum NavigatorRow {
    Folder {
        /// The deepest folder of a folded chain: picking the row shows its contents.
        id: String,
        label: String,
        /// Folders and files directly inside.
        items: usize,
        /// Not loaded yet; the number of files, once counted.
        unloaded: Option<Option<u64>>,
        picked: bool,
    },
    File {
        id: NodeId,
        label: String,
        badge: Option<String>,
        picked: bool,
    },
}

fn cluster<'g>(graph: &'g Graph, id: &str) -> Option<&'g GroupCluster> {
    graph.clusters.iter().find(|c| c.id == id)
}

impl Navigator {
    /// The columns to show: the project's top level, then one per picked folder.
    pub fn columns(&self, graph: &Graph) -> Vec<NavigatorColumn> {
        let Some(root) = graph.clusters.iter().find(|c| c.parent_id.is_none()) else { return Vec::new() };
        let mut columns = vec![self.column(graph, root, self.path.first())];
        for (i, id) in self.path.iter().enumerate() {
            let Some(folder) = cluster(graph, id) else { break };
            if folder.lazy.is_some() {
                break;
            }
            columns.push(self.column(graph, folder, self.path.get(i + 1)));
        }
        columns
    }

    fn column(&self, graph: &Graph, folder: &GroupCluster, picked: Option<&String>) -> NavigatorColumn {
        let mut folders: Vec<NavigatorRow> = folder
            .child_cluster_ids
            .iter()
            .filter_map(|id| cluster(graph, id))
            .map(|child| {
                let (deepest, label) = fold_chain(graph, child);
                NavigatorRow::Folder {
                    id: deepest.id.clone(),
                    label,
                    items: deepest.child_cluster_ids.len() + deepest.node_ids.len(),
                    unloaded: deepest.lazy.as_ref().map(|l| l.totals.map(|t| t.file_count)),
                    picked: picked == Some(&deepest.id),
                }
            })
            .collect();
        folders.sort_by_key(|row| match row {
            NavigatorRow::Folder { label, .. } => label.to_lowercase(),
            NavigatorRow::File { .. } => String::new(),
        });
        let mut files: Vec<NavigatorRow> = folder
            .node_ids
            .iter()
            .filter_map(|id| graph.nodes.get(id))
            .map(|node| NavigatorRow::File {
                id: node.id,
                label: node.title.clone(),
                badge: node.badge.clone(),
                picked: self.file == Some(node.id),
            })
            .collect();
        files.sort_by_key(|row| match row {
            NavigatorRow::File { label, .. } => label.to_lowercase(),
            NavigatorRow::Folder { .. } => String::new(),
        });
        folders.extend(files);
        NavigatorColumn { folder: folder.id.clone(), title: folder.label.clone(), rows: folders }
    }

    /// Picks a folder in column `column`: the columns after it close and its contents open.
    pub fn pick_folder(&mut self, column: usize, id: String) {
        self.path.truncate(column);
        self.path.push(id);
    }

    /// Shows `node`'s folder and picks the file, for a file opened from the map.
    pub fn reveal(&mut self, graph: &Graph, node: NodeId) {
        let Some(folder) = graph.nodes.get(&node).and_then(|n| n.group_id.clone()) else { return };
        // The chain from the top, without the project root (the first column).
        let mut chain = graph.cluster_path(&folder);
        chain.retain(|id| cluster(graph, id).is_some_and(|c| c.parent_id.is_some()));
        // Folded folders are skipped: only the deepest of each chain is a pick.
        self.path = chain.into_iter().filter(|id| cluster(graph, id).is_some_and(|c| !folds_into_child(c))).collect();
        self.file = Some(node);
    }
}

/// Whether a folder holds exactly one folder and nothing else, so it folds into it.
fn folds_into_child(c: &GroupCluster) -> bool {
    c.lazy.is_none() && c.node_ids.is_empty() && c.child_cluster_ids.len() == 1
}

/// Follows folders that hold only one folder, joining their names.
fn fold_chain<'g>(graph: &'g Graph, start: &'g GroupCluster) -> (&'g GroupCluster, String) {
    let mut at = start;
    let mut label = start.label.clone();
    while folds_into_child(at) {
        let Some(child) = cluster(graph, &at.child_cluster_ids[0]) else { break };
        label.push('/');
        label.push_str(&child.label);
        at = child;
    }
    (at, label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_graph::{GroupCluster, NodeArchetype};

    /// root/{crates/{api/src/{main.rs}, web/{lib.rs}}, README.md, node_modules (not loaded)}
    fn graph() -> Graph {
        let mut g = Graph::new();
        let folder = |id: &str, parent: Option<&str>, kids: &[&str]| {
            let mut c = GroupCluster::new(id, id.rsplit('/').next().unwrap(), "Folder", 0);
            c.parent_id = parent.map(str::to_string);
            c.child_cluster_ids = kids.iter().map(|k| k.to_string()).collect();
            c
        };
        let mut nm = folder("root/node_modules", Some("root"), &[]);
        nm.lazy = Some(studio_graph::LazyFolder {
            abs_path: "/p/node_modules".into(),
            totals: None,
            reason: "dependencies".into(),
        });
        g.clusters = vec![
            folder("root", None, &["root/crates", "root/node_modules"]),
            folder("root/crates", Some("root"), &["root/crates/api", "root/crates/web"]),
            folder("root/crates/api", Some("root/crates"), &["root/crates/api/src"]),
            folder("root/crates/api/src", Some("root/crates/api"), &[]),
            folder("root/crates/web", Some("root/crates"), &[]),
            nm,
        ];
        for (name, folder) in [("main.rs", 3), ("lib.rs", 4), ("README.md", 0)] {
            let id = g.add_node(name, NodeArchetype::File, "", None, vec![], vec![], [0.0, 0.0]);
            g.nodes.get_mut(&id).unwrap().group_id = Some(g.clusters[folder].id.clone());
            g.clusters[folder].node_ids.push(id);
        }
        g
    }

    fn labels(column: &NavigatorColumn) -> Vec<String> {
        column
            .rows
            .iter()
            .map(|r| match r {
                NavigatorRow::Folder { label, unloaded: Some(_), .. } => format!("{label} (load)"),
                NavigatorRow::Folder { label, .. } => format!("{label}/"),
                NavigatorRow::File { label, .. } => label.clone(),
            })
            .collect()
    }

    #[test]
    fn columns_open_one_pick_at_a_time_folders_first() {
        let g = graph();
        let mut nav = Navigator::default();
        let columns = nav.columns(&g);
        assert_eq!(columns.len(), 1);
        assert_eq!(labels(&columns[0]), ["crates/", "node_modules (load)", "README.md"]);

        nav.pick_folder(0, "root/crates".into());
        let columns = nav.columns(&g);
        assert_eq!(labels(&columns[1]), ["api/src/", "web/"], "api holds only src: one row");

        nav.pick_folder(1, "root/crates/api/src".into());
        assert_eq!(labels(&nav.columns(&g)[2]), ["main.rs"], "three clicks to a file two folders down");

        nav.pick_folder(0, "root/node_modules".into());
        assert_eq!(nav.columns(&g).len(), 1, "an unloaded folder opens no column until it is loaded");
    }

    #[test]
    fn a_file_opened_from_the_map_is_shown_in_its_column() {
        let g = graph();
        let main = g.nodes.values().find(|n| n.title == "main.rs").unwrap().id;
        let mut nav = Navigator::default();
        nav.reveal(&g, main);
        assert_eq!(nav.path, ["root/crates", "root/crates/api/src"]);
        let columns = nav.columns(&g);
        assert!(matches!(&columns[2].rows[0], NavigatorRow::File { picked: true, .. }));
    }
}
