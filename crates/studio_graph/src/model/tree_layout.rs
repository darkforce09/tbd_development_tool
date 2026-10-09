//! Nested folder-tree layout for the Files view.
//!
//! Every cluster is a folder; its file cards sit in a grid at the top and its subfolders are
//! shelf-packed below. Sizes are measured bottom-up, then positions are assigned top-down, so a
//! parent always encloses its children and siblings never overlap. Collapsed and empty folders
//! get a fixed, visible size.

use std::collections::HashMap;

use super::graph::Graph;

const PAD_X: f32 = 22.0;
const PAD_TOP: f32 = 44.0;
const PAD_BOTTOM: f32 = 22.0;
const CARD_GAP_X: f32 = 14.0;
const CARD_GAP_Y: f32 = 10.0;
const FOLDER_GAP: f32 = 30.0;
const ROOT_GAP: f32 = 80.0;
/// Size of a collapsed folder card.
pub const COLLAPSED_FOLDER_SIZE: [f32; 2] = [240.0, 44.0];
/// Minimum size of an expanded folder with nothing in it.
pub const EMPTY_FOLDER_SIZE: [f32; 2] = [240.0, PAD_TOP + PAD_BOTTOM];
const ROOT_ORIGIN: [f32; 2] = [100.0, 140.0];

struct Measured {
    size: [f32; 2],
    /// Offsets of file cards relative to the folder origin, in `node_ids` order.
    node_offsets: Vec<[f32; 2]>,
    /// (child cluster index, offset relative to the folder origin)
    child_offsets: Vec<(usize, [f32; 2])>,
}

impl Graph {
    /// Lays out the folder tree. No-op for graphs not built as a folder tree.
    pub fn layout_folder_tree(&mut self) {
        if !self.tree_layout || self.clusters.is_empty() {
            return;
        }
        if self.flow_layout {
            self.layout_dataflow();
            return;
        }
        self.flow = None;
        let index: HashMap<String, usize> = self.clusters.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();
        let children: Vec<Vec<usize>> = self
            .clusters
            .iter()
            .map(|c| c.child_cluster_ids.iter().filter_map(|id| index.get(id).copied()).collect())
            .collect();

        let mut measured: HashMap<usize, Measured> = HashMap::new();
        let roots: Vec<usize> = (0..self.clusters.len())
            .filter(|&i| self.clusters[i].parent_id.as_ref().is_none_or(|p| !index.contains_key(p)))
            .collect();
        for &root in &roots {
            self.measure(root, &children, &mut measured);
        }

        let mut x = ROOT_ORIGIN[0];
        for &root in &roots {
            self.place(root, [x, ROOT_ORIGIN[1]], 0, &measured);
            x += measured[&root].size[0] + ROOT_GAP;
        }
        self.rebuild_collapsed_cache();
    }

    /// Re-runs whichever layout the graph uses after a size or collapse change.
    pub fn relayout(&mut self) {
        if self.tree_layout {
            self.layout_folder_tree();
        } else {
            self.update_cluster_bounds();
        }
    }

    fn measure(&self, idx: usize, children: &[Vec<usize>], out: &mut HashMap<usize, Measured>) -> [f32; 2] {
        for &child in &children[idx] {
            self.measure(child, children, out);
        }
        let cluster = &self.clusters[idx];
        if cluster.is_collapsed {
            out.insert(
                idx,
                Measured { size: COLLAPSED_FOLDER_SIZE, node_offsets: Vec::new(), child_offsets: Vec::new() },
            );
            return COLLAPSED_FOLDER_SIZE;
        }

        // File cards: a grid of roughly square overall proportions; each row is as tall as its tallest card.
        let sizes: Vec<[f32; 2]> =
            cluster.node_ids.iter().map(|id| self.nodes.get(id).map_or([220.0, 42.0], |n| n.size)).collect();
        let cols = if sizes.is_empty() { 0 } else { ((sizes.len() as f32 * 0.19).sqrt().ceil() as usize).clamp(1, 16) };
        let cell_w = sizes.iter().map(|s| s[0]).fold(0.0, f32::max);
        let mut node_offsets = Vec::with_capacity(sizes.len());
        let mut files_h = 0.0;
        for row in sizes.chunks(cols.max(1)) {
            let row_h = row.iter().map(|s| s[1]).fold(0.0, f32::max);
            for c in 0..row.len() {
                node_offsets.push([PAD_X + c as f32 * (cell_w + CARD_GAP_X), PAD_TOP + files_h]);
            }
            files_h += row_h + CARD_GAP_Y;
        }
        if files_h > 0.0 {
            files_h -= CARD_GAP_Y;
        }
        let files_w = if cols == 0 { 0.0 } else { cols as f32 * (cell_w + CARD_GAP_X) - CARD_GAP_X };

        // Subfolders: shelf-pack under the files, wrapping at a width that keeps the folder roughly square.
        let kids = &children[idx];
        let child_sizes: Vec<[f32; 2]> = kids.iter().map(|k| out[k].size).collect();
        let area: f32 = child_sizes.iter().map(|s| s[0] * s[1]).sum();
        let widest = child_sizes.iter().map(|s| s[0]).fold(0.0, f32::max);
        let wrap_w = files_w.max(widest).max(area.sqrt() * 1.4);
        let top = PAD_TOP + if files_h > 0.0 { files_h + FOLDER_GAP } else { 0.0 };
        let (mut x, mut y, mut row_h, mut used_w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut child_offsets = Vec::with_capacity(kids.len());
        for (&kid, size) in kids.iter().zip(&child_sizes) {
            if x > 0.0 && x + size[0] > wrap_w {
                x = 0.0;
                y += row_h + FOLDER_GAP;
                row_h = 0.0;
            }
            child_offsets.push((kid, [PAD_X + x, top + y]));
            x += size[0] + FOLDER_GAP;
            used_w = used_w.max(x - FOLDER_GAP);
            row_h = row_h.max(size[1]);
        }
        let children_h = if kids.is_empty() { 0.0 } else { y + row_h };

        let content_w = files_w.max(used_w);
        let content_bottom = if kids.is_empty() { PAD_TOP + files_h } else { top + children_h };
        let size = [
            (content_w + 2.0 * PAD_X).max(EMPTY_FOLDER_SIZE[0]),
            (content_bottom + PAD_BOTTOM).max(EMPTY_FOLDER_SIZE[1]),
        ];
        out.insert(idx, Measured { size, node_offsets, child_offsets });
        size
    }

    fn place(&mut self, idx: usize, origin: [f32; 2], depth: usize, measured: &HashMap<usize, Measured>) {
        let m = &measured[&idx];
        let collapsed = self.clusters[idx].is_collapsed;
        {
            let cluster = &mut self.clusters[idx];
            cluster.position = origin;
            cluster.size = m.size;
            cluster.depth = depth;
        }
        let node_ids = self.clusters[idx].node_ids.clone();
        for (i, id) in node_ids.iter().enumerate() {
            if let Some(node) = self.nodes.get_mut(id) {
                // Hidden cards park on the collapsed folder so they never overlap visible content.
                let offset = if collapsed { [PAD_X, 0.0] } else { m.node_offsets[i] };
                node.position = [origin[0] + offset[0], origin[1] + offset[1]];
            }
        }
        if collapsed {
            // Park the whole hidden subtree on the collapsed card too.
            let kids = self.clusters[idx].child_cluster_ids.clone();
            for kid in kids {
                if let Some(k) = self.clusters.iter().position(|c| c.id == kid) {
                    self.park_subtree(k, origin, depth + 1);
                }
            }
            return;
        }
        for &(kid, offset) in &m.child_offsets {
            self.place(kid, [origin[0] + offset[0], origin[1] + offset[1]], depth + 1, measured);
        }
    }

    fn park_subtree(&mut self, idx: usize, origin: [f32; 2], depth: usize) {
        self.clusters[idx].position = origin;
        self.clusters[idx].depth = depth;
        let node_ids = self.clusters[idx].node_ids.clone();
        for id in node_ids {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.position = [origin[0] + PAD_X, origin[1]];
            }
        }
        let kids = self.clusters[idx].child_cluster_ids.clone();
        for kid in kids {
            if let Some(k) = self.clusters.iter().position(|c| c.id == kid) {
                self.park_subtree(k, origin, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DataType, GroupCluster, NodeArchetype};

    /// root/{a.rs, b.rs, sub/{c.rs, deep/{d.rs}}, empty/, only_dirs/{x/, y/}}
    fn tree_graph() -> Graph {
        let mut g = Graph::new();
        g.tree_layout = true;
        let file = |g: &mut Graph, name: &str| {
            g.add_node(name, NodeArchetype::File, "", None, vec![("in".into(), DataType::RustFlow)], vec![], [0.0, 0.0])
        };
        let a = file(&mut g, "a.rs");
        let b = file(&mut g, "b.rs");
        let c = file(&mut g, "c.rs");
        let d = file(&mut g, "d.rs");
        for n in g.nodes.values_mut() {
            n.size = [220.0, 42.0];
        }
        let folder = |id: &str, parent: Option<&str>, nodes: Vec<crate::NodeId>, kids: &[&str]| {
            let mut c = GroupCluster::new(id, id.rsplit('/').next().unwrap(), "Folder", 0);
            c.parent_id = parent.map(str::to_string);
            c.node_ids = nodes;
            c.child_cluster_ids = kids.iter().map(|k| k.to_string()).collect();
            c
        };
        g.clusters = vec![
            folder("root", None, vec![a, b], &["root/sub", "root/empty", "root/only_dirs"]),
            folder("root/sub", Some("root"), vec![c], &["root/sub/deep"]),
            folder("root/sub/deep", Some("root/sub"), vec![d], &[]),
            folder("root/empty", Some("root"), vec![], &[]),
            folder("root/only_dirs", Some("root"), vec![], &["root/only_dirs/x", "root/only_dirs/y"]),
            folder("root/only_dirs/x", Some("root/only_dirs"), vec![], &[]),
            folder("root/only_dirs/y", Some("root/only_dirs"), vec![], &[]),
        ];
        g
    }

    fn rect_of(g: &Graph, id: &str) -> [f32; 4] {
        let c = g.clusters.iter().find(|c| c.id == id).unwrap();
        [c.position[0], c.position[1], c.position[0] + c.size[0], c.position[1] + c.size[1]]
    }

    fn inside(inner: [f32; 4], outer: [f32; 4]) -> bool {
        inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
    }

    fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
    }

    fn assert_well_formed(g: &Graph) {
        for c in &g.clusters {
            assert!(c.size[0] > 1.0 && c.size[1] > 1.0, "{} has no visible size", c.id);
            if g.hidden_cluster_ids.contains(&c.id) {
                continue;
            }
            let r = rect_of(g, &c.id);
            if let Some(p) = &c.parent_id {
                assert!(inside(r, rect_of(g, p)), "{} escapes {}", c.id, p);
            }
            for id in &c.node_ids {
                let n = &g.nodes[id];
                let nr = [n.position[0], n.position[1], n.position[0] + n.size[0], n.position[1] + n.size[1]];
                if !g.is_node_in_collapsed_cluster(*id) {
                    assert!(inside(nr, r), "card escapes {}", c.id);
                }
            }
            let siblings: Vec<[f32; 4]> = c.child_cluster_ids.iter().map(|k| rect_of(g, k)).collect();
            for i in 0..siblings.len() {
                for j in i + 1..siblings.len() {
                    assert!(!overlaps(siblings[i], siblings[j]), "children of {} overlap", c.id);
                }
            }
        }
    }

    #[test]
    fn nested_folders_enclose_children_without_overlap() {
        let mut g = tree_graph();
        g.layout_folder_tree();
        assert_well_formed(&g);
        let depth = |id: &str| g.clusters.iter().find(|c| c.id == id).unwrap().depth;
        assert_eq!(depth("root/sub/deep"), 2);
        assert!(rect_of(&g, "root/empty")[2] - rect_of(&g, "root/empty")[0] >= EMPTY_FOLDER_SIZE[0]);
    }

    #[test]
    fn collapsing_hides_subtree_and_relayout_stays_clean() {
        let mut g = tree_graph();
        g.layout_folder_tree();
        g.toggle_cluster_collapse("root/sub");
        assert!(g.hidden_cluster_ids.contains("root/sub/deep"));
        assert!(!g.hidden_cluster_ids.contains("root/sub"), "the collapsed folder itself stays visible");
        let sub = rect_of(&g, "root/sub");
        assert_eq!([sub[2] - sub[0], sub[3] - sub[1]], COLLAPSED_FOLDER_SIZE);
        assert_well_formed(&g);

        g.toggle_cluster_collapse("root/sub");
        assert!(g.hidden_cluster_ids.is_empty());
        assert_well_formed(&g);
    }

    #[test]
    fn taller_cards_get_taller_rows() {
        let mut g = tree_graph();
        let first = g.clusters[0].node_ids[0];
        g.nodes.get_mut(&first).unwrap().size = [220.0, 300.0];
        g.layout_folder_tree();
        assert_well_formed(&g);
    }
}
