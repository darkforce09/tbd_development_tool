use egui::{Pos2, Rect};
use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use studio_graph::{Graph, NodeId};

/// Fast 2D Uniform Spatial Hash Grid for O(1) cursor hit-testing and AVX-friendly frustum culling
#[derive(Debug, Clone)]
pub struct SpatialHashGrid {
    pub cell_size: f32,
    cells: FxHashMap<(i32, i32), Vec<NodeId>>,
    node_bounds: FxHashMap<NodeId, [f32; 4]>, // [min_x, min_y, max_x, max_y]
    world_bounds: Option<[f32; 4]>,
    all_nodes: Vec<NodeId>,
    query_epoch: Cell<u32>,
    last_seen_node: RefCell<Vec<u32>>,
}

impl Default for SpatialHashGrid {
    fn default() -> Self {
        Self::new(768.0)
    }
}

impl SpatialHashGrid {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size,
            cells: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            world_bounds: None,
            all_nodes: Vec::new(),
            query_epoch: Cell::new(1),
            last_seen_node: RefCell::new(Vec::new()),
        }
    }

    #[inline]
    fn to_cell_coords(&self, x: f32, y: f32) -> (i32, i32) {
        ((x / self.cell_size).floor() as i32, (y / self.cell_size).floor() as i32)
    }

    pub fn clear(&mut self) {
        self.cells.clear();
        self.node_bounds.clear();
        self.world_bounds = None;
        self.all_nodes.clear();
    }

    #[inline]
    fn next_epoch(&self) -> u32 {
        let current = self.query_epoch.get();
        if current == u32::MAX {
            self.last_seen_node.borrow_mut().fill(0);
            self.query_epoch.set(1);
            1
        } else {
            let next = current + 1;
            self.query_epoch.set(next);
            next
        }
    }

    /// Rebuilds the spatial index directly from a Graph
    pub fn build_from_graph(&mut self, graph: &Graph) {
        self.clear();
        self.all_nodes.reserve(graph.nodes.len());

        for (&id, node) in &graph.nodes {
            let bounds =
                [node.position[0], node.position[1], node.position[0] + node.size[0], node.position[1] + node.size[1]];
            self.insert(id, bounds);
        }
    }

    pub fn insert(&mut self, node_id: NodeId, bounds: [f32; 4]) {
        self.node_bounds.insert(node_id, bounds);
        self.all_nodes.push(node_id);

        match &mut self.world_bounds {
            Some(wb) => {
                wb[0] = wb[0].min(bounds[0]);
                wb[1] = wb[1].min(bounds[1]);
                wb[2] = wb[2].max(bounds[2]);
                wb[3] = wb[3].max(bounds[3]);
            }
            None => {
                self.world_bounds = Some(bounds);
            }
        }

        let (min_ix, min_iy) = self.to_cell_coords(bounds[0], bounds[1]);
        let (max_ix, max_iy) = self.to_cell_coords(bounds[2], bounds[3]);

        for ix in min_ix..=max_ix {
            for iy in min_iy..=max_iy {
                self.cells.entry((ix, iy)).or_default().push(node_id);
            }
        }
    }

    pub fn remove(&mut self, node_id: NodeId) {
        if let Some(bounds) = self.node_bounds.remove(&node_id) {
            let (min_ix, min_iy) = self.to_cell_coords(bounds[0], bounds[1]);
            let (max_ix, max_iy) = self.to_cell_coords(bounds[2], bounds[3]);

            for ix in min_ix..=max_ix {
                for iy in min_iy..=max_iy {
                    if let Some(cell) = self.cells.get_mut(&(ix, iy)) {
                        cell.retain(|&id| id != node_id);
                    }
                }
            }
            self.all_nodes.retain(|&id| id != node_id);
        }
    }

    pub fn update(&mut self, node_id: NodeId, new_bounds: [f32; 4]) {
        self.remove(node_id);
        self.insert(node_id, new_bounds);
    }

    /// Queries all nodes intersecting the given world-space rectangle (viewport frustum).
    /// Uses world-bounds clamping and single-instruction epoch deduplication for ultra-high FPS.
    pub fn query_rect(&self, query_rect: Rect) -> Vec<NodeId> {
        let wb = match self.world_bounds {
            Some(b) => b,
            None => return Vec::new(),
        };

        let q_min_x = query_rect.min.x;
        let q_min_y = query_rect.min.y;
        let q_max_x = query_rect.max.x;
        let q_max_y = query_rect.max.y;

        // If the query rect completely contains the entire world, all nodes are visible
        if q_min_x <= wb[0] && q_min_y <= wb[1] && q_max_x >= wb[2] && q_max_y >= wb[3] {
            return self.all_nodes.clone();
        }

        // Clamp query bounds to world bounds to eliminate empty margins
        let clamped_min_x = q_min_x.max(wb[0]);
        let clamped_min_y = q_min_y.max(wb[1]);
        let clamped_max_x = q_max_x.min(wb[2]);
        let clamped_max_y = q_max_y.min(wb[3]);

        if clamped_min_x > clamped_max_x || clamped_min_y > clamped_max_y {
            return Vec::new();
        }

        let (min_ix, min_iy) = self.to_cell_coords(clamped_min_x, clamped_min_y);
        let (max_ix, max_iy) = self.to_cell_coords(clamped_max_x, clamped_max_y);

        let epoch = self.next_epoch();
        let mut last_seen = self.last_seen_node.borrow_mut();
        let mut visible = Vec::with_capacity(1024);

        let cell_count = ((max_ix - min_ix + 1) as u64) * ((max_iy - min_iy + 1) as u64);

        if cell_count > self.cells.len() as u64 {
            // Sparse iteration: iterate occupied cells directly
            for (&(ix, iy), node_ids) in &self.cells {
                if ix >= min_ix && ix <= max_ix && iy >= min_iy && iy <= max_iy {
                    let is_interior = ix > min_ix && ix < max_ix && iy > min_iy && iy < max_iy;
                    for &id in node_ids {
                        let idx = id.0 as usize;
                        if idx >= last_seen.len() {
                            last_seen.resize(idx + 1024, 0);
                        }
                        if last_seen[idx] == epoch {
                            continue;
                        }
                        last_seen[idx] = epoch;

                        if is_interior {
                            visible.push(id);
                        } else if let Some(&[b_min_x, b_min_y, b_max_x, b_max_y]) = self.node_bounds.get(&id) {
                            if b_min_x <= q_max_x && b_max_x >= q_min_x && b_min_y <= q_max_y && b_max_y >= q_min_y {
                                visible.push(id);
                            }
                        }
                    }
                }
            }
        } else {
            // Dense iteration: direct cell coordinates
            for ix in min_ix..=max_ix {
                let is_interior_x = ix > min_ix && ix < max_ix;
                for iy in min_iy..=max_iy {
                    let is_interior = is_interior_x && iy > min_iy && iy < max_iy;
                    if let Some(node_ids) = self.cells.get(&(ix, iy)) {
                        for &id in node_ids {
                            let idx = id.0 as usize;
                            if idx >= last_seen.len() {
                                last_seen.resize(idx + 1024, 0);
                            }
                            if last_seen[idx] == epoch {
                                continue;
                            }
                            last_seen[idx] = epoch;

                            if is_interior {
                                visible.push(id);
                            } else if let Some(&[b_min_x, b_min_y, b_max_x, b_max_y]) = self.node_bounds.get(&id) {
                                if b_min_x <= q_max_x && b_max_x >= q_min_x && b_min_y <= q_max_y && b_max_y >= q_min_y
                                {
                                    visible.push(id);
                                }
                            }
                        }
                    }
                }
            }
        }

        visible
    }

    /// O(1) query for all nodes containing the given world position (cursor position)
    pub fn query_point(&self, point: Pos2) -> Vec<NodeId> {
        let cell = self.to_cell_coords(point.x, point.y);
        let mut matching = Vec::new();

        if let Some(node_ids) = self.cells.get(&cell) {
            for &id in node_ids {
                if let Some(&[min_x, min_y, max_x, max_y]) = self.node_bounds.get(&id) {
                    if point.x >= min_x && point.x <= max_x && point.y >= min_y && point.y <= max_y {
                        matching.push(id);
                    }
                }
            }
        }

        matching
    }

    #[inline]
    pub fn get_bounds(&self, node_id: NodeId) -> Option<[f32; 4]> {
        self.node_bounds.get(&node_id).copied()
    }

    #[inline]
    pub fn world_bounds(&self) -> Option<[f32; 4]> {
        self.world_bounds
    }
}
