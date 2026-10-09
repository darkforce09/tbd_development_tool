use std::collections::HashMap;

use super::graph::Graph;
use super::types::NodeId;

impl Graph {
    /// Recalculates bounding boxes for all group clusters based on contained nodes and nested child clusters.
    /// Uses bottom-up traversal (deepest clusters first) to ensure parent containers enclose their children.
    pub fn update_cluster_bounds(&mut self) {
        if self.clusters.is_empty() {
            return;
        }

        // 1. Compute cluster depths based on parent chain
        let id_to_parent: HashMap<String, Option<String>> =
            self.clusters.iter().map(|c| (c.id.clone(), c.parent_id.clone())).collect();

        for cluster in &mut self.clusters {
            let mut depth = 0;
            let mut curr = cluster.parent_id.clone();
            while let Some(ref pid) = curr {
                depth += 1;
                curr = id_to_parent.get(pid).cloned().flatten();
            }
            cluster.depth = depth;
        }

        // 2. Sort processing order: deepest clusters first
        let mut indices: Vec<usize> = (0..self.clusters.len()).collect();
        indices.sort_by(|&a, &b| self.clusters[b].depth.cmp(&self.clusters[a].depth));

        // Map cluster ID to index for O(1) child lookups
        let id_to_idx: HashMap<String, usize> =
            self.clusters.iter().enumerate().map(|(i, c)| (c.id.clone(), i)).collect();

        // 3. Process clusters bottom-up
        for &idx in &indices {
            if self.clusters[idx].is_collapsed {
                self.clusters[idx].size = [220.0, 38.0];
                continue;
            }

            let mut min_x = f32::MAX;
            let mut min_y = f32::MAX;
            let mut max_x = f32::MIN;
            let mut max_y = f32::MIN;
            let mut has_content = false;

            // Bounding box of direct contained nodes
            for id in &self.clusters[idx].node_ids {
                if let Some(node) = self.nodes.get(id) {
                    has_content = true;
                    min_x = min_x.min(node.position[0]);
                    min_y = min_y.min(node.position[1]);
                    max_x = max_x.max(node.position[0] + node.size[0]);
                    max_y = max_y.max(node.position[1] + node.size[1]);
                }
            }

            // Bounding box of child clusters (already computed because we go bottom-up!)
            let child_ids = self.clusters[idx].child_cluster_ids.clone();
            for cid in &child_ids {
                if let Some(&child_idx) = id_to_idx.get(cid) {
                    let child = &self.clusters[child_idx];
                    if child.size[0] > 0.0 && child.size[1] > 0.0 {
                        has_content = true;
                        min_x = min_x.min(child.position[0]);
                        min_y = min_y.min(child.position[1]);
                        max_x = max_x.max(child.position[0] + child.size[0]);
                        max_y = max_y.max(child.position[1] + child.size[1]);
                    }
                }
            }

            if has_content {
                let padding_x = 22.0;
                let padding_top = 44.0; // Space for folder header tab
                let padding_bottom = 22.0;

                self.clusters[idx].position = [min_x - padding_x, min_y - padding_top];
                self.clusters[idx].size =
                    [(max_x - min_x) + padding_x * 2.0, (max_y - min_y) + padding_top + padding_bottom];
            }
        }
    }

    /// Rebuilds the fast lookup cache for collapsed nodes in O(C + N) time.
    pub fn rebuild_collapsed_cache(&mut self) {
        self.collapsed_clusters_count = self.clusters.iter().filter(|c| c.is_collapsed).count();
        self.collapsed_node_ids.clear();
        self.hidden_cluster_ids.clear();
        if self.collapsed_clusters_count == 0 {
            return;
        }

        let mut collapsed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &self.clusters {
            if c.is_collapsed {
                collapsed_ids.insert(c.id.clone());
            }
        }

        // Propagate collapsed status down to child clusters
        let mut changed = true;
        while changed {
            changed = false;
            for c in &self.clusters {
                if let Some(ref pid) = c.parent_id {
                    if collapsed_ids.contains(pid) && collapsed_ids.insert(c.id.clone()) {
                        changed = true;
                    }
                }
            }
        }

        for c in &self.clusters {
            if collapsed_ids.contains(&c.id) {
                for &nid in &c.node_ids {
                    self.collapsed_node_ids.insert(nid);
                }
                // Descendants of a collapsed folder are hidden; the collapsed folder itself is drawn.
                if c.parent_id.as_ref().is_some_and(|p| collapsed_ids.contains(p)) {
                    self.hidden_cluster_ids.insert(c.id.clone());
                }
            }
        }
    }

    /// Toggles collapse state for a cluster and updates all cluster bounds.
    pub fn toggle_cluster_collapse(&mut self, cluster_id: &str) -> bool {
        let mut found = false;
        for cluster in &mut self.clusters {
            if cluster.id == cluster_id {
                cluster.is_collapsed = !cluster.is_collapsed;
                found = true;
                break;
            }
        }
        if found {
            self.rebuild_collapsed_cache();
            self.relayout();
        }
        found
    }

    pub fn is_cluster_collapsed(&self, cluster_id: &str) -> bool {
        self.clusters.iter().find(|c| c.id == cluster_id).map(|c| c.is_collapsed).unwrap_or(false)
    }

    /// O(1) check if a node is inside a collapsed cluster or any of its collapsed ancestors.
    #[inline]
    pub fn is_node_in_collapsed_cluster(&self, node_id: NodeId) -> bool {
        if self.collapsed_clusters_count == 0 {
            return false;
        }
        self.collapsed_node_ids.contains(&node_id)
    }
}
