//! Left-to-right dataflow layout of the folder tree (docs/VISUAL_LANGUAGE.md).
//!
//! Every folder is a container of cards and subfolders. A code wire between two cards is lifted
//! to the lowest folder that holds both; on the way out of each folder below that it passes an
//! output gate on the folder's right edge, and on the way in an input gate on the left edge.
//! Gates are keyed by the providing card, so one provider feeding several cards inside a folder
//! uses one gate. Items that depend on each other in a loop are grouped in a cycle box, which is a
//! container like a folder. Each container is laid out bottom-up: providers left of consumers
//! (longest-path layering), long wires get a reserved lane in every column they cross, crossings
//! are reduced by barycenter sweeps, and positions are aligned to neighbours under the order
//! constraint. Documentation and asset wires do not affect the layout.

pub mod scc;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::model::tree_layout::{COLLAPSED_FOLDER_SIZE, EMPTY_FOLDER_SIZE};
use crate::model::{Graph, NodeId};

const PAD_X: f32 = 22.0;
const PAD_TOP: f32 = 44.0;
const PAD_BOTTOM: f32 = 22.0;
/// Band at the top of a folder kept free for documentation wires.
const DOC_STRIP: f32 = 16.0;
const CYCLE_PAD_TOP: f32 = 26.0;
const CYCLE_PAD: f32 = 14.0;
/// Vertical gap between items in a column.
const ITEM_GAP: f32 = 18.0;
/// Height reserved for a wire passing through a column.
const LANE: f32 = 10.0;
/// Gap between columns before any wires are counted, and the extra width per wire track.
const GAP_BASE: f32 = 48.0;
const TRACK_PITCH: f32 = 8.0;
/// Width of a gate band with no wires, and the minimum spacing of gates along an edge.
const GATE_BAND: f32 = 24.0;
const GATE_PITCH: f32 = 14.0;
const SHELF_GAP: f32 = 30.0;
const ROOT_GAP: f32 = 80.0;
const ROOT_ORIGIN: [f32; 2] = [100.0, 140.0];
const SWEEPS: usize = 4;

/// Results of the dataflow layout that the canvas draws besides cards and folders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct FlowLayout {
    pub gates: Vec<Gate>,
    pub cycle_boxes: Vec<CycleBox>,
    /// One route per (provider, consumer) card pair joined by code wires.
    pub routes: Vec<WireRoute>,
}

/// The path of the code wires from one card to another: horizontal and vertical segments in
/// world coordinates, from the provider's output port to the consumer's input port. It leaves
/// and enters every folder through a gate and never crosses a card or folder in between. The
/// first two and last two points share a y, so the ends can be moved to a member's row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct WireRoute {
    pub provider: NodeId,
    pub consumer: NodeId,
    pub points: Vec<[f32; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum GateSide {
    /// Left edge: a provider outside feeds something inside.
    Input,
    /// Right edge: something inside feeds a consumer outside.
    Output,
}

/// A port on a folder or cycle box edge where a wire crosses the boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct Gate {
    /// Folder cluster id, or `cycle:<n>` for a cycle box.
    pub container: String,
    pub side: GateSide,
    /// The providing card whose wires pass through this gate.
    pub provider: NodeId,
    /// World position on the container edge.
    pub position: [f32; 2],
}

/// Items that depend on each other in a loop, drawn as one marked box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct CycleBox {
    pub id: String,
    pub position: [f32; 2],
    pub size: [f32; 2],
    /// Cards directly in the box (subfolders in the box are not listed).
    pub node_ids: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Item {
    Card(NodeId),
    Sub(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Folder(usize),
    Cycle,
}

struct Container {
    kind: Kind,
    parent: Option<usize>,
    items: Vec<Item>,
    collapsed: bool,
    /// Stable sort key (cluster id).
    key: String,
}

/// Where a lifted wire starts inside a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum From {
    /// An item's output: the card's output port, or the child container's output gate.
    Item(Item, NodeId),
    /// The container's own input gate.
    Gate(NodeId),
}

/// Where a lifted wire ends inside a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum To {
    Item(Item, NodeId),
    Gate(NodeId),
}

/// A laid-out container, relative to its own top-left corner.
#[derive(Debug, Clone, Default)]
struct Placed {
    size: [f32; 2],
    items: Vec<(Item, [f32; 2])>,
    in_gates: BTreeMap<NodeId, f32>,
    out_gates: BTreeMap<NodeId, f32>,
    /// Route of every wire inside the container, relative to its top-left corner.
    routes: BTreeMap<(From, To), Vec<[f32; 2]>>,
}

impl Graph {
    /// Lays out the folder tree left to right by code flow. See the module docs.
    pub fn layout_dataflow(&mut self) {
        let mut tree = Tree::build(self);
        let pairs = code_flow_pairs(self, &tree);
        tree.box_cycles(&pairs);
        let links = tree.lift(&pairs);
        let placed = tree.measure(self, &links);
        let (mut flow, origins) = tree.place(self, &placed);
        flow.routes = tree.compose_routes(&pairs, &placed, &origins);
        self.flow = Some(flow);
        self.rebuild_route_index();
        self.rebuild_collapsed_cache();
    }
}

/// Distinct (provider, consumer) card pairs joined by code-flow wires, in a stable order.
fn code_flow_pairs(graph: &Graph, tree: &Tree) -> Vec<(NodeId, NodeId)> {
    let pairs: BTreeSet<(NodeId, NodeId)> = graph
        .edges
        .iter()
        .filter(|e| e.kind.is_code_flow() && e.from_node != e.to_node)
        .filter(|e| tree.home.contains_key(&e.from_node) && tree.home.contains_key(&e.to_node))
        .map(|e| (e.from_node, e.to_node))
        .collect();
    pairs.into_iter().collect()
}

struct Tree {
    containers: Vec<Container>,
    /// Container that directly holds each card.
    home: HashMap<NodeId, usize>,
    roots: Vec<usize>,
}

impl Tree {
    fn build(graph: &Graph) -> Self {
        let index: HashMap<&str, usize> = graph.clusters.iter().enumerate().map(|(i, c)| (c.id.as_str(), i)).collect();
        let mut containers: Vec<Container> = graph
            .clusters
            .iter()
            .enumerate()
            .map(|(i, c)| Container {
                kind: Kind::Folder(i),
                parent: c.parent_id.as_deref().and_then(|p| index.get(p).copied()),
                items: Vec::new(),
                collapsed: c.is_collapsed,
                key: c.id.clone(),
            })
            .collect();
        let mut home = HashMap::new();
        for (i, c) in graph.clusters.iter().enumerate() {
            let mut cards: Vec<NodeId> = c.node_ids.iter().copied().filter(|id| graph.nodes.contains_key(id)).collect();
            cards.sort_by(|a, b| graph.nodes[a].title.cmp(&graph.nodes[b].title).then(a.cmp(b)));
            for &id in &cards {
                home.insert(id, i);
            }
            let mut subs: Vec<usize> =
                c.child_cluster_ids.iter().filter_map(|k| index.get(k.as_str()).copied()).collect();
            subs.sort_by(|&a, &b| graph.clusters[a].id.cmp(&graph.clusters[b].id));
            containers[i].items = cards.into_iter().map(Item::Card).chain(subs.into_iter().map(Item::Sub)).collect();
        }
        let roots = (0..containers.len()).filter(|&i| containers[i].parent.is_none()).collect();
        Tree { containers, home, roots }
    }

    /// Containers from the one holding `node` up to the root.
    fn chain(&self, node: NodeId) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut c = self.home.get(&node).copied();
        while let Some(i) = c {
            chain.push(i);
            c = self.containers[i].parent;
        }
        chain
    }

    /// The visible end of a wire: a card inside a collapsed folder is represented by the
    /// outermost collapsed folder around it. Returns the item and the chain of containers above it.
    fn visible_end(&self, node: NodeId) -> (Item, Vec<usize>) {
        let chain = self.chain(node);
        match chain.iter().rposition(|&c| self.containers[c].collapsed) {
            Some(i) => (Item::Sub(chain[i]), chain[i + 1..].to_vec()),
            None => (Item::Card(node), chain),
        }
    }

    /// The item directly inside `container` that holds the end `(item, chain)`.
    fn item_in(&self, item: Item, chain: &[usize], container: usize) -> Item {
        match chain.iter().position(|&c| c == container) {
            Some(0) | None => item,
            Some(i) => Item::Sub(chain[i - 1]),
        }
    }

    /// Groups items that depend on each other in a loop into cycle containers.
    fn box_cycles(&mut self, pairs: &[(NodeId, NodeId)]) {
        let mut sibling_edges: BTreeMap<usize, BTreeSet<(Item, Item)>> = BTreeMap::new();
        for &(u, v) in pairs {
            let (ui, uc) = self.visible_end(u);
            let (vi, vc) = self.visible_end(v);
            let Some(lca) = lowest_common(&uc, &vc) else { continue };
            let (a, b) = (self.item_in(ui, &uc, lca), self.item_in(vi, &vc, lca));
            if a != b {
                sibling_edges.entry(lca).or_default().insert((a, b));
            }
        }
        for (container, edges) in sibling_edges {
            if self.containers[container].collapsed {
                continue;
            }
            let items = self.containers[container].items.clone();
            let pos: HashMap<Item, usize> = items.iter().enumerate().map(|(i, &it)| (it, i)).collect();
            let mut adj = vec![Vec::new(); items.len()];
            for (a, b) in edges {
                if let (Some(&i), Some(&j)) = (pos.get(&a), pos.get(&b)) {
                    adj[i].push(j);
                }
            }
            let mut loops: Vec<Vec<usize>> =
                scc::strongly_connected(&adj).into_iter().filter(|c| c.len() > 1).collect();
            loops.sort();
            for members in loops {
                let id = self.containers.len();
                let member_items: Vec<Item> = members.iter().map(|&m| items[m]).collect();
                let key = format!("cycle:{}:{}", self.containers[container].key, id);
                for &it in &member_items {
                    match it {
                        Item::Card(n) => {
                            self.home.insert(n, id);
                        }
                        Item::Sub(s) => self.containers[s].parent = Some(id),
                    }
                }
                let parent_items = &mut self.containers[container].items;
                let first = parent_items.iter().position(|it| member_items.contains(it)).unwrap_or(0);
                parent_items.retain(|it| !member_items.contains(it));
                parent_items.insert(first.min(parent_items.len()), Item::Sub(id));
                self.containers.push(Container {
                    kind: Kind::Cycle,
                    parent: Some(container),
                    items: member_items,
                    collapsed: false,
                    key,
                });
            }
        }
    }

    /// The wire pieces a card pair is made of, in order from provider to consumer: out of each
    /// container below the lowest common one, across that one, then into each container down to
    /// the consumer.
    fn pieces(&self, u: NodeId, v: NodeId) -> Vec<(usize, From, To)> {
        let (ui, uc) = self.visible_end(u);
        let (vi, vc) = self.visible_end(v);
        let Some(lca) = lowest_common(&uc, &vc) else { return Vec::new() };
        let (a, b) = (self.item_in(ui, &uc, lca), self.item_in(vi, &vc, lca));
        if a == b {
            return Vec::new();
        }
        let mut pieces = Vec::new();
        for &c in uc.iter().take_while(|&&c| c != lca) {
            pieces.push((c, From::Item(self.item_in(ui, &uc, c), u), To::Gate(u)));
        }
        pieces.push((lca, From::Item(a, u), To::Item(b, u)));
        let inward: Vec<usize> = vc.iter().copied().take_while(|&c| c != lca).collect();
        for &c in inward.iter().rev() {
            pieces.push((c, From::Gate(u), To::Item(self.item_in(vi, &vc, c), u)));
        }
        pieces
    }

    /// Lifts every card pair into the containers it passes through. Returns, per container, the
    /// wires that run inside it.
    fn lift(&self, pairs: &[(NodeId, NodeId)]) -> Vec<BTreeSet<(From, To)>> {
        let mut links: Vec<BTreeSet<(From, To)>> = vec![BTreeSet::new(); self.containers.len()];
        for &(u, v) in pairs {
            for (c, from, to) in self.pieces(u, v) {
                links[c].insert((from, to));
            }
        }
        links
    }

    /// Joins each pair's pieces into one world-space route.
    fn compose_routes(&self, pairs: &[(NodeId, NodeId)], placed: &[Placed], origins: &[[f32; 2]]) -> Vec<WireRoute> {
        let mut routes = Vec::new();
        'pairs: for &(u, v) in pairs {
            let pieces = self.pieces(u, v);
            if pieces.is_empty() {
                continue;
            }
            let mut points: Vec<[f32; 2]> = Vec::new();
            for (c, from, to) in pieces {
                let Some(local) = placed[c].routes.get(&(from, to)) else { continue 'pairs };
                let o = origins[c];
                for p in local {
                    let w = [o[0] + p[0], o[1] + p[1]];
                    if points.last().is_none_or(|l| (l[0] - w[0]).abs() > 0.01 || (l[1] - w[1]).abs() > 0.01) {
                        points.push(w);
                    }
                }
            }
            routes.push(WireRoute { provider: u, consumer: v, points: simplify(points) });
        }
        routes
    }

    fn depth(&self, mut c: usize) -> usize {
        let mut d = 0;
        while let Some(p) = self.containers[c].parent {
            d += 1;
            c = p;
        }
        d
    }

    /// Lays out every container bottom-up.
    fn measure(&self, graph: &Graph, links: &[BTreeSet<(From, To)>]) -> Vec<Placed> {
        let mut order: Vec<usize> = (0..self.containers.len()).collect();
        order.sort_by_key(|&c| std::cmp::Reverse(self.depth(c)));
        let mut placed: Vec<Placed> = vec![Placed::default(); self.containers.len()];
        for c in order {
            placed[c] = self.measure_one(graph, c, &links[c], &placed);
        }
        placed
    }

    fn measure_one(&self, graph: &Graph, c: usize, links: &BTreeSet<(From, To)>, placed: &[Placed]) -> Placed {
        let container = &self.containers[c];
        if container.collapsed {
            // Minimised: every gate sits in the middle of its edge.
            let mid = COLLAPSED_FOLDER_SIZE[1] * 0.5;
            let mut p = Placed { size: COLLAPSED_FOLDER_SIZE, ..Default::default() };
            for (from, to) in links {
                if let From::Gate(k) = from {
                    p.in_gates.insert(*k, mid);
                }
                if let To::Gate(k) = to {
                    p.out_gates.insert(*k, mid);
                }
            }
            return p;
        }
        let is_cycle = container.kind == Kind::Cycle;
        let size_of = |it: Item| match it {
            Item::Card(n) => graph.nodes.get(&n).map_or([220.0, 42.0], |n| n.size),
            Item::Sub(s) => placed[s].size,
        };
        let port_y = |it: Item, key: NodeId, input: bool| -> f32 {
            match it {
                Item::Card(n) => graph.nodes.get(&n).map_or(21.0, card_port_offset),
                Item::Sub(s) => {
                    let gates = if input { &placed[s].in_gates } else { &placed[s].out_gates };
                    gates.get(&key).copied().unwrap_or(placed[s].size[1] * 0.5)
                }
            }
        };
        let mut lay = ColumnLayout::new(&container.items, links, &size_of, &port_y);
        lay.order();
        lay.coordinates();

        let (pad_top, pad_x, pad_bottom) =
            if is_cycle { (CYCLE_PAD_TOP, CYCLE_PAD, CYCLE_PAD) } else { (PAD_TOP + DOC_STRIP, PAD_X, PAD_BOTTOM) };
        let mut p = Placed::default();
        let content_left = pad_x + lay.in_band;
        let mut bottom = pad_top;
        let mut right = content_left;
        for node in &lay.nodes {
            let Some(it) = node.item else { continue };
            let pos = [content_left + lay.column_x[node.layer], pad_top + node.y];
            bottom = bottom.max(pos[1] + node.h);
            right = right.max(pos[0] + node.w);
            p.items.push((it, pos));
        }
        for (&k, &y) in &lay.in_gate_y {
            p.in_gates.insert(k, pad_top + y);
            bottom = bottom.max(pad_top + y + GATE_PITCH * 0.5);
        }
        for (&k, &y) in &lay.out_gate_y {
            p.out_gates.insert(k, pad_top + y);
            bottom = bottom.max(pad_top + y + GATE_PITCH * 0.5);
        }
        let has_columns = lay.nodes.iter().any(|n| n.item.is_some());
        if has_columns {
            right = right.max(content_left + lay.content_width);
        }
        // Wires that run backwards inside a cycle box loop below the cards, one row each.
        let back_top = bottom + ITEM_GAP;
        if lay.back_count > 0 {
            bottom = back_top + lay.back_count as f32 * LANE;
        }

        // Items with no code wires in this container: a shelf below the columns.
        let shelf: Vec<(Item, [f32; 2])> = lay.isolated.iter().map(|&it| (it, size_of(it))).collect();
        let shelf_top = if has_columns || !p.in_gates.is_empty() { bottom + SHELF_GAP } else { pad_top };
        let wrap = (right - pad_x).max(shelf.iter().map(|(_, s)| s[0] * s[1]).sum::<f32>().sqrt() * 1.4);
        let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
        for (it, s) in shelf {
            if x > 0.0 && x + s[0] > wrap {
                x = 0.0;
                y += row_h + ITEM_GAP;
                row_h = 0.0;
            }
            let pos = [pad_x + x, shelf_top + y];
            p.items.push((it, pos));
            right = right.max(pos[0] + s[0]);
            bottom = bottom.max(pos[1] + s[1]);
            x += s[0] + ITEM_GAP;
            row_h = row_h.max(s[1]);
        }

        let min = if is_cycle { [2.0 * pad_x, pad_top + pad_bottom] } else { EMPTY_FOLDER_SIZE };
        let width = right.max(content_left + lay.content_width + lay.out_band) + pad_x;
        p.size = [width.max(min[0]), (bottom + pad_bottom).max(min[1])];
        p.routes = lay.routes([content_left, pad_top], p.size[0], back_top);
        p
    }

    /// Assigns world positions top-down and records gates and cycle boxes. Also returns each
    /// container's world origin.
    fn place(&self, graph: &mut Graph, placed: &[Placed]) -> (FlowLayout, Vec<[f32; 2]>) {
        let mut flow = FlowLayout::default();
        let mut origins = vec![[0.0; 2]; self.containers.len()];
        let mut x = ROOT_ORIGIN[0];
        for &root in &self.roots {
            self.place_one(graph, placed, root, [x, ROOT_ORIGIN[1]], 0, &mut flow, &mut origins);
            x += placed[root].size[0] + ROOT_GAP;
        }
        (flow, origins)
    }

    #[allow(clippy::too_many_arguments)]
    fn place_one(
        &self,
        graph: &mut Graph,
        placed: &[Placed],
        c: usize,
        origin: [f32; 2],
        depth: usize,
        flow: &mut FlowLayout,
        origins: &mut [[f32; 2]],
    ) {
        origins[c] = origin;
        let p = &placed[c];
        let container = &self.containers[c];
        let id = match container.kind {
            Kind::Folder(i) => {
                let cluster = &mut graph.clusters[i];
                cluster.position = origin;
                cluster.size = p.size;
                cluster.depth = depth;
                cluster.id.clone()
            }
            Kind::Cycle => {
                let cards = container
                    .items
                    .iter()
                    .filter_map(|it| if let Item::Card(n) = it { Some(*n) } else { None })
                    .collect();
                flow.cycle_boxes.push(CycleBox {
                    id: container.key.clone(),
                    position: origin,
                    size: p.size,
                    node_ids: cards,
                });
                container.key.clone()
            }
        };
        for (&k, &y) in &p.in_gates {
            flow.gates.push(Gate {
                container: id.clone(),
                side: GateSide::Input,
                provider: k,
                position: [origin[0], origin[1] + y],
            });
        }
        for (&k, &y) in &p.out_gates {
            flow.gates.push(Gate {
                container: id.clone(),
                side: GateSide::Output,
                provider: k,
                position: [origin[0] + p.size[0], origin[1] + y],
            });
        }
        if container.collapsed {
            self.park(graph, c, origin, depth);
            return;
        }
        let child_depth = if container.kind == Kind::Cycle { depth } else { depth + 1 };
        for &(it, offset) in &p.items {
            let pos = [origin[0] + offset[0], origin[1] + offset[1]];
            match it {
                Item::Card(n) => {
                    if let Some(node) = graph.nodes.get_mut(&n) {
                        node.position = pos;
                    }
                }
                Item::Sub(s) => self.place_one(graph, placed, s, pos, child_depth, flow, origins),
            }
        }
    }

    /// Hidden contents of a collapsed folder sit on the folder card so they never overlap
    /// visible content.
    fn park(&self, graph: &mut Graph, c: usize, origin: [f32; 2], depth: usize) {
        for &it in &self.containers[c].items {
            match it {
                Item::Card(n) => {
                    if let Some(node) = graph.nodes.get_mut(&n) {
                        node.position = [origin[0] + PAD_X, origin[1]];
                    }
                }
                Item::Sub(s) => {
                    let child_depth = match self.containers[s].kind {
                        Kind::Folder(i) => {
                            graph.clusters[i].position = origin;
                            graph.clusters[i].depth = depth + 1;
                            depth + 1
                        }
                        Kind::Cycle => depth,
                    };
                    self.park(graph, s, origin, child_depth);
                }
            }
        }
    }
}

/// Y offset of a card's file-level ports from its top edge (see `port_world_position`).
fn card_port_offset(node: &crate::model::Node) -> f32 {
    if node.is_dropdown_expanded || node.is_code_expanded {
        19.0
    } else {
        node.size[1] * 0.5
    }
}

/// Lowest container on both chains (chains run from the inside out).
fn lowest_common(a: &[usize], b: &[usize]) -> Option<usize> {
    let in_b: BTreeSet<usize> = b.iter().copied().collect();
    a.iter().copied().find(|c| in_b.contains(c))
}

/// One column node: a real item or a lane for a wire passing through.
#[derive(Debug, Clone)]
struct ColNode {
    item: Option<Item>,
    layer: usize,
    w: f32,
    h: f32,
    y: f32,
    /// Port offsets from the top: where wires enter and leave.
    in_y: f32,
    out_y: f32,
}

/// Which wires share a vertical track in a gap: wires from one source (tag 0), the outgoing (1)
/// and incoming (2) ends of a backward wire.
type TrackKey = (Option<Item>, NodeId, u8);

/// The column nodes one wire passes, for turning into points once positions are known.
#[derive(Debug, Clone)]
struct LinkPath {
    from: From,
    to: To,
    nodes: Vec<usize>,
    /// Port offsets at the first node's output and the last node's input.
    from_y: f32,
    to_y: f32,
    /// Runs right to left inside a cycle box.
    back: bool,
}

/// A wire between two column nodes in adjacent columns, or between a gate and a column node.
#[derive(Debug, Clone, Copy)]
enum Seg {
    Node { from: usize, to: usize, from_y: f32, to_y: f32 },
    In { gate: NodeId, to: usize, to_y: f32 },
    Out { from: usize, from_y: f32, gate: NodeId },
}

struct ColumnLayout {
    nodes: Vec<ColNode>,
    segs: Vec<Seg>,
    layers: Vec<Vec<usize>>,
    isolated: Vec<Item>,
    /// Source key of each segment, for counting wire tracks per gap.
    seg_source: Vec<(Option<Item>, NodeId)>,
    in_gate_y: BTreeMap<NodeId, f32>,
    out_gate_y: BTreeMap<NodeId, f32>,
    column_x: Vec<f32>,
    content_width: f32,
    in_band: f32,
    out_band: f32,
    col_w: Vec<f32>,
    paths: Vec<LinkPath>,
    back_count: usize,
    /// x of each wire's vertical track, relative to the content's left edge. Index 0 is the input
    /// gate band, `l + 1` the gap right of column `l`; the last is the output gate band.
    tracks: Vec<BTreeMap<TrackKey, f32>>,
}

impl ColumnLayout {
    fn new(
        items: &[Item],
        links: &BTreeSet<(From, To)>,
        size_of: &dyn Fn(Item) -> [f32; 2],
        port_y: &dyn Fn(Item, NodeId, bool) -> f32,
    ) -> Self {
        let pos: HashMap<Item, usize> = items.iter().enumerate().map(|(i, &it)| (it, i)).collect();
        let n = items.len();
        let mut adj = vec![Vec::new(); n];
        let mut connected = vec![false; n];
        for (from, to) in links {
            if let From::Item(it, _) = from {
                if let Some(&i) = pos.get(it) {
                    connected[i] = true;
                }
            }
            if let To::Item(it, _) = to {
                if let Some(&i) = pos.get(it) {
                    connected[i] = true;
                }
            }
            if let (From::Item(a, _), To::Item(b, _)) = (from, to) {
                if let (Some(&i), Some(&j)) = (pos.get(a), pos.get(b)) {
                    if i != j {
                        adj[i].push(j);
                    }
                }
            }
        }
        for a in &mut adj {
            a.sort_unstable();
            a.dedup();
        }

        // Inside a cycle box some wires must point backwards; elsewhere the order is topological.
        let order = scc::acyclic_order(&adj);
        let rank: Vec<usize> = {
            let mut r = vec![0; n];
            for (i, &v) in order.iter().enumerate() {
                r[v] = i;
            }
            r
        };
        // Longest-path layering on the forward wires.
        let mut layer = vec![0usize; n];
        for &u in &order {
            for &v in &adj[u] {
                if rank[v] > rank[u] {
                    layer[v] = layer[v].max(layer[u] + 1);
                }
            }
        }
        // Pull sources towards their consumers.
        let mut has_pred = vec![false; n];
        for (u, succs) in adj.iter().enumerate() {
            for &v in succs {
                if rank[v] > rank[u] {
                    has_pred[v] = true;
                }
            }
        }
        for &u in order.iter().rev() {
            let succ_min = adj[u].iter().filter(|&&v| rank[v] > rank[u]).map(|&v| layer[v]).min();
            if let (false, Some(m)) = (has_pred[u], succ_min) {
                layer[u] = m.saturating_sub(1).max(layer[u]);
            }
        }

        let mut lay = ColumnLayout {
            nodes: Vec::new(),
            segs: Vec::new(),
            layers: Vec::new(),
            isolated: Vec::new(),
            seg_source: Vec::new(),
            in_gate_y: BTreeMap::new(),
            out_gate_y: BTreeMap::new(),
            column_x: Vec::new(),
            content_width: 0.0,
            in_band: 0.0,
            out_band: 0.0,
            col_w: Vec::new(),
            paths: Vec::new(),
            back_count: 0,
            tracks: Vec::new(),
        };
        let mut node_of = vec![usize::MAX; n];
        for i in 0..n {
            if !connected[i] {
                lay.isolated.push(items[i]);
                continue;
            }
            let s = size_of(items[i]);
            node_of[i] = lay.nodes.len();
            lay.nodes.push(ColNode {
                item: Some(items[i]),
                layer: layer[i],
                w: s[0],
                h: s[1],
                y: 0.0,
                in_y: 0.0,
                out_y: 0.0,
            });
        }
        let max_layer = lay.nodes.iter().map(|n| n.layer).max().unwrap_or(0);

        // Lanes are shared by every wire from the same source passing the same column.
        let mut lanes: HashMap<((Option<Item>, NodeId), usize), usize> = HashMap::new();
        let mut lane = |lay: &mut ColumnLayout, src: (Option<Item>, NodeId), layer: usize| -> usize {
            *lanes.entry((src, layer)).or_insert_with(|| {
                lay.nodes.push(ColNode {
                    item: None,
                    layer,
                    w: 0.0,
                    h: LANE,
                    y: 0.0,
                    in_y: LANE * 0.5,
                    out_y: LANE * 0.5,
                });
                lay.nodes.len() - 1
            })
        };
        // A chain of segments from `start` (a node, or the left gate band when `None`) through
        // lanes up to the node before `end_layer`; returns the last node and its port y.
        let mut push_chain = |lay: &mut ColumnLayout,
                              path: &mut Vec<usize>,
                              src: (Option<Item>, NodeId),
                              start: Option<(usize, f32)>,
                              from_layer: usize,
                              to_layer: usize|
         -> Option<(usize, f32)> {
            let mut prev = start;
            for l in from_layer..to_layer {
                let ln = lane(lay, src, l);
                path.push(ln);
                match prev {
                    Some((p, py)) => lay.segs.push(Seg::Node { from: p, to: ln, from_y: py, to_y: LANE * 0.5 }),
                    None => lay.segs.push(Seg::In { gate: src.1, to: ln, to_y: LANE * 0.5 }),
                }
                lay.seg_source.push(src);
                prev = Some((ln, LANE * 0.5));
            }
            prev
        };

        for (from, to) in links {
            match (*from, *to) {
                (From::Item(a, key), To::Item(b, _)) => {
                    let (Some(&i), Some(&j)) = (pos.get(&a), pos.get(&b)) else { continue };
                    let (u, v) = (node_of[i], node_of[j]);
                    if u == usize::MAX || v == usize::MAX || u == v {
                        continue;
                    }
                    let (fy, ty) = (port_y(a, key, false), port_y(b, key, true));
                    let (lu, lv) = (lay.nodes[u].layer, lay.nodes[v].layer);
                    let src = (Some(a), key);
                    let mut path = vec![u];
                    let back = lv <= lu;
                    if !back {
                        let last = push_chain(&mut lay, &mut path, src, Some((u, fy)), lu + 1, lv).unwrap_or((u, fy));
                        lay.segs.push(Seg::Node { from: last.0, to: v, from_y: last.1, to_y: ty });
                        lay.seg_source.push(src);
                    }
                    path.push(v);
                    lay.back_count += back as usize;
                    lay.paths.push(LinkPath { from: *from, to: *to, nodes: path, from_y: fy, to_y: ty, back });
                }
                (From::Gate(key), To::Item(b, _)) => {
                    let Some(&j) = pos.get(&b) else { continue };
                    let v = node_of[j];
                    if v == usize::MAX {
                        continue;
                    }
                    let ty = port_y(b, key, true);
                    let src = (None, key);
                    let lv = lay.nodes[v].layer;
                    let mut path = Vec::new();
                    match push_chain(&mut lay, &mut path, src, None, 0, lv) {
                        Some(last) => lay.segs.push(Seg::Node { from: last.0, to: v, from_y: last.1, to_y: ty }),
                        None => lay.segs.push(Seg::In { gate: key, to: v, to_y: ty }),
                    }
                    lay.seg_source.push(src);
                    lay.in_gate_y.insert(key, 0.0);
                    path.push(v);
                    let from_y = if path.len() > 1 { LANE * 0.5 } else { ty };
                    lay.paths.push(LinkPath { from: *from, to: *to, nodes: path, from_y, to_y: ty, back: false });
                }
                (From::Item(a, key), To::Gate(_)) => {
                    let Some(&i) = pos.get(&a) else { continue };
                    let u = node_of[i];
                    if u == usize::MAX {
                        continue;
                    }
                    let fy = port_y(a, key, false);
                    let src = (Some(a), key);
                    let lu = lay.nodes[u].layer;
                    let mut path = vec![u];
                    let last =
                        push_chain(&mut lay, &mut path, src, Some((u, fy)), lu + 1, max_layer + 1).unwrap_or((u, fy));
                    lay.segs.push(Seg::Out { from: last.0, from_y: last.1, gate: key });
                    lay.seg_source.push(src);
                    lay.out_gate_y.insert(key, 0.0);
                    let to_y = if path.len() > 1 { LANE * 0.5 } else { fy };
                    lay.paths.push(LinkPath { from: *from, to: *to, nodes: path, from_y: fy, to_y, back: false });
                }
                (From::Gate(_), To::Gate(_)) => {}
            }
        }

        lay.layers = vec![Vec::new(); max_layer + 1];
        for (i, node) in lay.nodes.iter().enumerate() {
            lay.layers[node.layer].push(i);
        }
        if lay.nodes.is_empty() {
            lay.layers.clear();
        }
        lay
    }

    /// Orders each column to reduce crossings: barycenter sweeps left-to-right and back.
    fn order(&mut self) {
        if self.layers.is_empty() {
            return;
        }
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        let mut in_gate_targets: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
        let mut gates_of: Vec<Vec<NodeId>> = vec![Vec::new(); self.nodes.len()];
        for seg in &self.segs {
            match *seg {
                Seg::Node { from, to, .. } => {
                    preds[to].push(from);
                    succs[from].push(to);
                }
                Seg::In { gate, to, .. } => {
                    in_gate_targets.entry(gate).or_default().push(to);
                    gates_of[to].push(gate);
                }
                Seg::Out { .. } => {}
            }
        }
        let mut pos = vec![0.0f32; self.nodes.len()];
        let refresh = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f32>| {
            for layer in layers {
                let len = layer.len().max(1) as f32;
                for (i, &n) in layer.iter().enumerate() {
                    pos[n] = (i as f32 + 0.5) / len;
                }
            }
        };
        refresh(&self.layers, &mut pos);
        for _ in 0..SWEEPS {
            // In-gates take the barycenter of what they feed; the first column then follows them.
            let gate_pos: BTreeMap<NodeId, f32> = in_gate_targets
                .iter()
                .map(|(&k, t)| (k, t.iter().map(|&n| pos[n]).sum::<f32>() / t.len() as f32))
                .collect();
            for l in 0..self.layers.len() {
                let mut keyed: Vec<(f32, usize, usize)> = self.layers[l]
                    .iter()
                    .enumerate()
                    .map(|(i, &n)| {
                        let mut ys: Vec<f32> = preds[n].iter().map(|&p| pos[p]).collect();
                        ys.extend(gates_of[n].iter().map(|k| gate_pos[k]));
                        let key = if ys.is_empty() { pos[n] } else { ys.iter().sum::<f32>() / ys.len() as f32 };
                        (key, i, n)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                self.layers[l] = keyed.into_iter().map(|(_, _, n)| n).collect();
                refresh(&self.layers, &mut pos);
            }
            for l in (0..self.layers.len()).rev() {
                let mut keyed: Vec<(f32, usize, usize)> = self.layers[l]
                    .iter()
                    .enumerate()
                    .map(|(i, &n)| {
                        let ys: Vec<f32> = succs[n].iter().map(|&s| pos[s]).collect();
                        let key = if ys.is_empty() { pos[n] } else { ys.iter().sum::<f32>() / ys.len() as f32 };
                        (key, i, n)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                self.layers[l] = keyed.into_iter().map(|(_, _, n)| n).collect();
                refresh(&self.layers, &mut pos);
            }
        }
    }

    /// Column x positions sized for the wires in each gap, then y positions aligned to
    /// neighbours without changing the order, then gate positions.
    fn coordinates(&mut self) {
        let layers = self.layers.len();
        if layers == 0 {
            return;
        }
        // Port offsets of each node from its segments.
        for seg in &self.segs {
            match *seg {
                Seg::Node { from, to, from_y, to_y } => {
                    self.nodes[from].out_y = from_y;
                    self.nodes[to].in_y = to_y;
                }
                Seg::In { to, to_y, .. } => self.nodes[to].in_y = to_y,
                Seg::Out { from, from_y, .. } => self.nodes[from].out_y = from_y,
            }
        }

        // Wire tracks per gap: index 0 is the input gate band, l + 1 the gap right of column l,
        // and the last the output gate band.
        let mut gap_keys: Vec<BTreeSet<TrackKey>> = vec![BTreeSet::new(); layers + 1];
        for (seg, &(item, key)) in self.segs.iter().zip(&self.seg_source) {
            let gap = match *seg {
                Seg::Node { from, .. } => self.nodes[from].layer + 1,
                Seg::In { .. } => 0,
                Seg::Out { .. } => layers,
            };
            gap_keys[gap].insert((item, key, 0));
        }
        for path in self.paths.iter().filter(|p| p.back) {
            let (From::Item(a, key), To::Item(b, _)) = (path.from, path.to) else { continue };
            let (u, v) = (path.nodes[0], path.nodes[1]);
            gap_keys[self.nodes[u].layer + 1].insert((Some(a), key, 1));
            gap_keys[self.nodes[v].layer].insert((Some(b), key, 2));
        }
        self.col_w = self.layers.iter().map(|l| l.iter().map(|&n| self.nodes[n].w).fold(0.0, f32::max)).collect();
        let band = |n: usize| if n == 0 { 0.0 } else { GATE_BAND + TRACK_PITCH * n as f32 };
        self.in_band = band(gap_keys[0].len());
        self.out_band = band(gap_keys[layers].len());
        let mut x = 0.0;
        let mut gap_left = vec![-self.in_band; layers + 1];
        self.column_x = Vec::with_capacity(layers);
        for l in 0..layers {
            self.column_x.push(x);
            x += self.col_w[l];
            gap_left[l + 1] = x;
            if l + 1 < layers {
                x += GAP_BASE + TRACK_PITCH * gap_keys[l + 1].len() as f32;
            }
        }
        self.content_width = x;

        // Initial stacking, then alternating passes towards the median of neighbouring ports.
        for layer in &self.layers {
            let mut y = 0.0;
            for &n in layer {
                self.nodes[n].y = y;
                y += self.nodes[n].h + gap_after(&self.nodes[n]);
            }
        }
        let mut in_nb: Vec<Vec<(usize, f32, f32)>> = vec![Vec::new(); self.nodes.len()];
        let mut out_nb: Vec<Vec<(usize, f32, f32)>> = vec![Vec::new(); self.nodes.len()];
        for seg in &self.segs {
            if let Seg::Node { from, to, from_y, to_y } = *seg {
                in_nb[to].push((from, from_y, to_y));
                out_nb[from].push((to, to_y, from_y));
            }
        }
        for pass in 0..SWEEPS * 2 {
            let forward = pass % 2 == 0;
            let order: Vec<usize> = if forward { (0..layers).collect() } else { (0..layers).rev().collect() };
            for l in order {
                let layer = self.layers[l].clone();
                let desired: Vec<f32> = layer
                    .iter()
                    .map(|&n| {
                        let nb = if forward { &in_nb[n] } else { &out_nb[n] };
                        let mut ys: Vec<f32> =
                            nb.iter().map(|&(m, m_port, own_port)| self.nodes[m].y + m_port - own_port).collect();
                        if ys.is_empty() {
                            return self.nodes[n].y;
                        }
                        ys.sort_by(f32::total_cmp);
                        ys[ys.len() / 2]
                    })
                    .collect();
                let heights: Vec<(f32, f32)> =
                    layer.iter().map(|&n| (self.nodes[n].h, gap_after(&self.nodes[n]))).collect();
                for (&n, y) in layer.iter().zip(ordered_fit(&desired, &heights)) {
                    self.nodes[n].y = y;
                }
            }
        }

        // Gates sit level with what they feed (input) or what feeds them (output).
        let mut in_targets: BTreeMap<NodeId, Vec<f32>> = BTreeMap::new();
        let mut out_sources_y: BTreeMap<NodeId, Vec<f32>> = BTreeMap::new();
        for seg in &self.segs {
            match *seg {
                Seg::In { gate, to, to_y } => in_targets.entry(gate).or_default().push(self.nodes[to].y + to_y),
                Seg::Out { from, from_y, gate } => {
                    out_sources_y.entry(gate).or_default().push(self.nodes[from].y + from_y)
                }
                Seg::Node { .. } => {}
            }
        }
        self.in_gate_y = place_gates(&in_targets);
        self.out_gate_y = place_gates(&out_sources_y);

        // Each gap's tracks, ordered by the height their wires start at so they cross less.
        let mut key_y: BTreeMap<(usize, TrackKey), f32> = BTreeMap::new();
        for (seg, &(item, key)) in self.segs.iter().zip(&self.seg_source) {
            let (gap, y) = match *seg {
                Seg::Node { from, from_y, .. } => (self.nodes[from].layer + 1, self.nodes[from].y + from_y),
                Seg::In { gate, .. } => (0, self.in_gate_y.get(&gate).copied().unwrap_or(0.0)),
                Seg::Out { from, from_y, .. } => (layers, self.nodes[from].y + from_y),
            };
            key_y.entry((gap, (item, key, 0))).or_insert(y);
        }
        for path in self.paths.iter().filter(|p| p.back) {
            let (From::Item(a, key), To::Item(b, _)) = (path.from, path.to) else { continue };
            let (u, v) = (path.nodes[0], path.nodes[1]);
            key_y.insert((self.nodes[u].layer + 1, (Some(a), key, 1)), self.nodes[u].y + path.from_y);
            key_y.insert((self.nodes[v].layer, (Some(b), key, 2)), self.nodes[v].y + path.to_y);
        }
        self.tracks = vec![BTreeMap::new(); layers + 1];
        for (gap, keys) in gap_keys.iter().enumerate() {
            let mut ordered: Vec<(f32, TrackKey)> =
                keys.iter().map(|k| (key_y.get(&(gap, *k)).copied().unwrap_or(0.0), *k)).collect();
            ordered.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let margin = if gap == 0 || gap == layers { GATE_BAND * 0.5 } else { GAP_BASE * 0.5 };
            for (t, (_, k)) in ordered.into_iter().enumerate() {
                self.tracks[gap].insert(k, gap_left[gap] + margin + t as f32 * TRACK_PITCH);
            }
        }

        // Shift everything so the topmost node or gate starts at 0.
        let top = self
            .nodes
            .iter()
            .map(|n| n.y)
            .chain(self.in_gate_y.values().map(|y| y - GATE_PITCH * 0.5))
            .chain(self.out_gate_y.values().map(|y| y - GATE_PITCH * 0.5))
            .fold(f32::INFINITY, f32::min);
        if top.is_finite() {
            for n in &mut self.nodes {
                n.y -= top;
            }
            for y in self.in_gate_y.values_mut().chain(self.out_gate_y.values_mut()) {
                *y -= top;
            }
        }
    }
}

impl ColumnLayout {
    /// Points of every wire in the container, relative to its top-left corner. `origin` is where
    /// column 0 and y = 0 of the content sit; `back_top` is the top of the rows for backward wires.
    fn routes(&self, origin: [f32; 2], width: f32, back_top: f32) -> BTreeMap<(From, To), Vec<[f32; 2]>> {
        let [cl, ct] = origin;
        let layers = self.layers.len();
        let left = |n: usize| cl + self.column_x[self.nodes[n].layer];
        let right = |n: usize| {
            let node = &self.nodes[n];
            left(n) + if node.item.is_some() { node.w } else { self.col_w[node.layer] }
        };
        let y_of = |n: usize, offset: f32| ct + self.nodes[n].y + offset;
        let track = |gap: usize, key: TrackKey| cl + self.tracks[gap].get(&key).copied().unwrap_or(0.0);

        let mut out = BTreeMap::new();
        let mut back_row = 0;
        for path in &self.paths {
            let (src_item, key) = match path.from {
                From::Item(a, k) => (Some(a), k),
                From::Gate(k) => (None, k),
            };
            let points = if path.back {
                let To::Item(b, _) = path.to else { continue };
                let (u, v) = (path.nodes[0], path.nodes[1]);
                let (yu, yv) = (y_of(u, path.from_y), y_of(v, path.to_y));
                let row = back_top + (back_row as f32 + 0.5) * LANE;
                back_row += 1;
                let t1 = track(self.nodes[u].layer + 1, (src_item, key, 1));
                let t2 = track(self.nodes[v].layer, (Some(b), key, 2));
                vec![[right(u), yu], [t1, yu], [t1, row], [t2, row], [t2, yv], [left(v), yv]]
            } else {
                let from_gate = matches!(path.from, From::Gate(_));
                let mut y = match path.from {
                    From::Gate(g) => ct + self.in_gate_y.get(&g).copied().unwrap_or(0.0),
                    From::Item(..) => y_of(path.nodes[0], path.from_y),
                };
                let mut points = vec![[if from_gate { 0.0 } else { right(path.nodes[0]) }, y]];
                let last = path.nodes.len() - 1;
                for (i, &n) in path.nodes.iter().enumerate() {
                    if i == 0 && !from_gate {
                        continue;
                    }
                    let is_lane = self.nodes[n].item.is_none();
                    let ny =
                        if is_lane { y_of(n, LANE * 0.5) } else { y_of(n, if i == last { path.to_y } else { 0.0 }) };
                    // The gap left of column l has index l.
                    let tx = track(self.nodes[n].layer, (src_item, key, 0));
                    points.extend([[tx, y], [tx, ny], [left(n), ny]]);
                    if is_lane {
                        points.push([right(n), ny]);
                    }
                    y = ny;
                }
                if let To::Gate(g) = path.to {
                    let tx = track(layers, (src_item, key, 0));
                    let gy = ct + self.out_gate_y.get(&g).copied().unwrap_or(0.0);
                    points.extend([[tx, y], [tx, gy], [width, gy]]);
                }
                points
            };
            out.insert((path.from, path.to), points);
        }
        out
    }
}

/// Drops points that lie on a straight line between their neighbours (a wire passing straight
/// through several columns) and zero-length steps.
fn simplify(points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    let same = |a: f32, b: f32| (a - b).abs() < 0.01;
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|l| same(l[0], p[0]) && same(l[1], p[1])) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if (same(a[0], b[0]) && same(b[0], p[0])) || (same(a[1], b[1]) && same(b[1], p[1])) {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

fn gap_after(node: &ColNode) -> f32 {
    if node.item.is_some() {
        ITEM_GAP
    } else {
        2.0
    }
}

/// Positions as close as possible to `desired` (least squares) that keep the given order and
/// leave each item's height plus gap free below it. Pool-adjacent-violators on shifted targets.
fn ordered_fit(desired: &[f32], heights: &[(f32, f32)]) -> Vec<f32> {
    let mut offset = Vec::with_capacity(desired.len());
    let mut acc = 0.0;
    for &(h, gap) in heights {
        offset.push(acc);
        acc += h + gap;
    }
    // Blocks of (sum, count); each block's value is its mean.
    let mut blocks: Vec<(f32, usize)> = Vec::new();
    for (d, o) in desired.iter().zip(&offset) {
        blocks.push((d - o, 1));
        while blocks.len() > 1 {
            let (s2, c2) = blocks[blocks.len() - 1];
            let (s1, c1) = blocks[blocks.len() - 2];
            if s1 / c1 as f32 <= s2 / c2 as f32 {
                break;
            }
            blocks.pop();
            *blocks.last_mut().unwrap() = (s1 + s2, c1 + c2);
        }
    }
    let mut out = Vec::with_capacity(desired.len());
    for (sum, count) in blocks {
        for _ in 0..count {
            out.push(sum / count as f32);
        }
    }
    out.iter().zip(&offset).map(|(z, o)| z + o).collect()
}

/// Each gate at the median y of its wires, spread to at least `GATE_PITCH` apart in that order.
fn place_gates(wires: &BTreeMap<NodeId, Vec<f32>>) -> BTreeMap<NodeId, f32> {
    let mut wanted: Vec<(f32, NodeId)> = wires
        .iter()
        .map(|(&k, ys)| {
            let mut ys = ys.clone();
            ys.sort_by(f32::total_cmp);
            (ys[ys.len() / 2], k)
        })
        .collect();
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let desired: Vec<f32> = wanted.iter().map(|w| w.0 - GATE_PITCH * 0.5).collect();
    let heights = vec![(GATE_PITCH, 0.0); wanted.len()];
    ordered_fit(&desired, &heights).into_iter().zip(wanted).map(|(y, (_, k))| (k, y + GATE_PITCH * 0.5)).collect()
}

#[cfg(test)]
mod tests;
