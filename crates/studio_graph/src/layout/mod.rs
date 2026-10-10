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

pub mod docs;
pub mod scc;

pub use docs::DOC_PORT_OFFSET_Y;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize, Serialize};

use crate::model::tree_layout::{COLLAPSED_FOLDER_SIZE, EMPTY_FOLDER_SIZE};
use crate::model::{EdgeKind, Graph, NodeId};

const PAD_X: f32 = 22.0;
const PAD_TOP: f32 = 44.0;
const PAD_BOTTOM: f32 = 22.0;
/// Band at the top of a folder kept free for documentation wires; it grows with the number of
/// documentation tracks crossing the folder.
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
/// Width of a folder in node view.
const NODE_VIEW_WIDTH: f32 = 280.0;
const SWEEPS: usize = 4;
/// Hub rule (docs/VISUAL_LANGUAGE.md): an item feeding at least this many of its siblings, and
/// at least half of the connected ones, shows a badge instead of its wires.
pub const HUB_MIN_CONSUMERS: usize = 8;
/// Gate keys at or above this stand for a closed folder (`FOLDER_KEY_BASE + container`) rather
/// than a card: outside a closed folder, all wires from its cards share its gates and lanes.
const FOLDER_KEY_BASE: u64 = 1 << 62;

fn folder_key(container: usize) -> NodeId {
    NodeId(FOLDER_KEY_BASE + container as u64)
}

fn key_folder(key: NodeId) -> Option<usize> {
    (key.0 >= FOLDER_KEY_BASE).then(|| (key.0 - FOLDER_KEY_BASE) as usize)
}

/// Results of the dataflow layout that the canvas draws besides cards and folders.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct FlowLayout {
    pub gates: Vec<Gate>,
    pub cycle_boxes: Vec<CycleBox>,
    /// One route per (provider, consumer) card pair joined by code wires.
    pub routes: Vec<WireRoute>,
    /// One route per (documentation file, documented card) pair.
    #[serde(default)]
    pub doc_routes: Vec<WireRoute>,
    /// Code wires grouped by what is visible at each end (a card, or the closed folder around
    /// it): one drawn wire per bundle, as thick as the number of card pairs it stands for.
    #[serde(default)]
    pub bundles: Vec<WireBundle>,
    /// Items whose wires are shown as a "used by N" badge instead (the hub rule).
    #[serde(default)]
    pub hubs: Vec<Hub>,
}

/// One end of a bundle as it is seen: a card, or the outermost closed folder hiding it.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum VisibleEnd {
    Card(NodeId),
    /// Folder cluster id.
    Folder(String),
}

/// Code wires between the same two visible ends. Every route in it has the same path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct WireBundle {
    pub from: VisibleEnd,
    pub to: VisibleEnd,
    /// Card pairs the bundle stands for.
    pub pairs: u32,
    /// Wires per kind, indexed by `EdgeKind as usize`.
    pub kinds: [u32; EdgeKind::COUNT],
    /// Index of the route drawn for the whole bundle.
    pub route: u32,
}

impl WireBundle {
    /// The kind the bundle carries most (ties go to the earlier kind), for its colour.
    pub fn main_kind(&self) -> crate::model::EdgeKind {
        let kinds = EdgeKind::ALL;
        let best = (0..kinds.len()).max_by_key(|&i| (self.kinds[kinds[i] as usize], std::cmp::Reverse(i))).unwrap_or(0);
        kinds[best]
    }
}

/// An item that feeds at least half of its connected siblings (and at least
/// [`HUB_MIN_CONSUMERS`]): its wires are not laid out or drawn, it shows "used by N".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct Hub {
    /// Folder cluster id or cycle box id the item sits in.
    pub container: String,
    pub item: VisibleEnd,
    /// Siblings it feeds.
    pub used_by: u32,
    /// The card pairs whose wires it hides.
    pub pairs: Vec<(NodeId, NodeId)>,
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
    /// Index into [`FlowLayout::bundles`]; `u32::MAX` for documentation routes.
    #[serde(default)]
    pub bundle: u32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum GateSide {
    /// Left edge: a provider outside feeds something inside.
    Input,
    /// Right edge: something inside feeds a consumer outside.
    Output,
}

/// Which wires a gate carries.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    Serialize,
    Deserialize,
    Archive,
    RkyvSerialize,
    RkyvDeserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub enum GateKind {
    /// Code wires: on the container's sides, level with what they feed.
    #[default]
    Code,
    /// Documentation wires: on the container's sides, in the documentation strip at the top.
    Documentation,
}

/// A port on a folder or cycle box edge where a wire crosses the boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Archive, RkyvSerialize, RkyvDeserialize)]
#[rkyv(derive(Debug))]
pub struct Gate {
    /// Folder cluster id, or `cycle:<n>` for a cycle box.
    pub container: String,
    pub side: GateSide,
    #[serde(default)]
    pub kind: GateKind,
    /// Where the wires through this gate come from: a card, or a closed folder standing in for
    /// all its cards.
    pub source: VisibleEnd,
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

#[derive(Clone)]
struct Container {
    kind: Kind,
    parent: Option<usize>,
    items: Vec<Item>,
    /// Contents hidden: minimised or node view.
    collapsed: bool,
    /// Node view: every gate listed as a pin (L5 level 2).
    node_view: bool,
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

impl From {
    /// The providing card the wire belongs to.
    fn key(&self) -> NodeId {
        match *self {
            From::Item(_, k) | From::Gate(k) => k,
        }
    }
}

/// What the layout made of one container, for diagnosing its size and shape (bench_scale
/// `--layout-report`). Not saved.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContainerStats {
    /// Folder cluster id or cycle box id.
    pub id: String,
    pub depth: usize,
    pub size: [f32; 2],
    /// Items in columns (wired) and on the shelf (unwired).
    pub column_items: usize,
    pub shelf_items: usize,
    pub shelf_rows: usize,
    pub layers: usize,
    /// Most items (cards, subfolders, wire lanes excluded) in one column.
    pub max_column_items: usize,
    /// Most wire lanes in one column.
    pub max_column_lanes: usize,
    /// Height of the tallest column, including lanes.
    pub max_column_height: f32,
    /// Sum of column widths and of the gaps between them.
    pub columns_width: f32,
    pub gaps_width: f32,
    /// Most vertical wire tracks in one gap between columns.
    pub max_gap_tracks: usize,
    pub in_gates: usize,
    pub out_gates: usize,
    pub doc_gates: usize,
    /// Widths of the input and output gate bands.
    pub in_band: f32,
    pub out_band: f32,
    pub doc_strip: f32,
}

/// Providers whose wires cross a container's edges, by kind and side.
#[derive(Debug, Clone, Default)]
struct GateKeys {
    code_in: BTreeSet<NodeId>,
    code_out: BTreeSet<NodeId>,
    doc_in: BTreeSet<NodeId>,
    doc_out: BTreeSet<NodeId>,
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
    /// Documentation gates (y on the left and right edges) and routes.
    doc_in: BTreeMap<NodeId, f32>,
    doc_out: BTreeMap<NodeId, f32>,
    doc_routes: BTreeMap<(From, To), Vec<[f32; 2]>>,
    stats: ContainerStats,
}

impl Graph {
    /// Lays out the folder tree left to right by code flow. See the module docs.
    pub fn layout_dataflow(&mut self) {
        let mut tree = Tree::build(self);
        let all_pairs = code_flow_pairs(self, &tree);
        let doc_pairs = documentation_pairs(self, &tree);
        tree.box_cycles(&all_pairs);
        let hubs = tree.find_hubs(&all_pairs);
        let pairs: Vec<(NodeId, NodeId)> =
            all_pairs.iter().copied().filter(|p| !hubs.hidden_pairs.contains(p)).collect();
        let links = tree.lift(&pairs);
        let doc_links = tree.lift(&doc_pairs);
        let outer = outer_gate_keys(tree.containers.len(), &links, &doc_links);
        let n = tree.containers.len();
        // Opening or closing a folder lays everything out again; keep each unchanged container's
        // columns and order from the last layout, so nothing else moves around.
        let carried: Vec<Option<Frozen>> = match self.layout_cache.take() {
            Some(old) => {
                let by_key: HashMap<&str, usize> =
                    old.tree.containers.iter().enumerate().map(|(i, c)| (c.key.as_str(), i)).collect();
                tree.containers
                    .iter()
                    .map(|c| {
                        let o = *by_key.get(c.key.as_str())?;
                        (old.tree.containers[o].items == c.items).then(|| old.frozen[o].clone())
                    })
                    .collect()
            }
            None => vec![None; n],
        };
        let mut cache = LayoutCache {
            fingerprint: fingerprint(self),
            tree,
            pairs,
            doc_pairs,
            links,
            doc_links,
            outer,
            hubs,
            carried,
            pair_kinds: pair_kinds(self),
            frozen: vec![Frozen::default(); n],
            placed: vec![Placed::default(); n],
        };
        cache.measure(self, &vec![true; n], false);
        self.apply_layout(cache);
    }

    /// Lays the graph out again after cards changed size or folders changed detail level,
    /// keeping every container's columns and order: only the containers holding `cards` and the
    /// folders whose level changed, and the folders around them, are measured again. Returns
    /// false, changing nothing, when there is no layout to adjust or the graph's structure
    /// changed since; then a full layout is needed.
    pub fn relayout_geometry(&mut self, cards: &[NodeId]) -> bool {
        let Some(mut cache) = self.layout_cache.take() else { return false };
        if cache.fingerprint != fingerprint(self) {
            // The full layout that follows carries the columns and order over from it.
            self.layout_cache = Some(cache);
            return false;
        }
        let tree = &mut cache.tree;
        let mut dirty = vec![false; tree.containers.len()];
        let mark = |dirty: &mut Vec<bool>, tree: &Tree, mut c: Option<usize>| {
            while let Some(i) = c {
                dirty[i] = true;
                c = tree.containers[i].parent;
            }
        };
        for i in 0..tree.containers.len() {
            let Kind::Folder(f) = tree.containers[i].kind else { continue };
            let Some(cluster) = self.clusters.get(f) else { return false };
            let (collapsed, node_view) =
                (cluster.is_collapsed(), cluster.detail == crate::model::FolderDetail::NodeView);
            if tree.containers[i].collapsed != collapsed || tree.containers[i].node_view != node_view {
                tree.containers[i].collapsed = collapsed;
                tree.containers[i].node_view = node_view;
                mark(&mut dirty, tree, Some(i));
            }
        }
        for card in cards {
            mark(&mut dirty, tree, tree.home.get(card).copied());
        }
        cache.measure(self, &dirty, true);
        self.apply_layout(*cache);
        true
    }

    /// Whether a layout is kept that a geometry-only relayout can adjust.
    pub fn has_layout_cache(&self) -> bool {
        self.layout_cache.is_some()
    }

    /// Places the measured containers, composes the routes and keeps the cache.
    fn apply_layout(&mut self, cache: LayoutCache) {
        let (mut flow, origins) = cache.tree.place(self, &cache.placed);
        let hidden = cache.tree.hidden();
        flow.routes = cache.tree.compose_routes(&cache.pairs, &cache.placed, &origins, &hidden, |p| &p.routes);
        flow.doc_routes =
            cache.tree.compose_routes(&cache.doc_pairs, &cache.placed, &origins, &hidden, |p| &p.doc_routes);
        for r in &mut flow.doc_routes {
            r.bundle = u32::MAX;
        }
        flow.bundles = cache.tree.bundle(self, &mut flow.routes, &cache.pair_kinds);
        flow.hubs = cache.hubs.public.clone();
        self.layout_stats = cache
            .placed
            .iter()
            .enumerate()
            .map(|(c, p)| ContainerStats {
                id: cache.tree.containers[c].key.clone(),
                depth: cache.tree.depth(c),
                size: p.size,
                ..p.stats.clone()
            })
            .collect();
        self.flow = Some(flow);
        self.rebuild_route_index();
        self.rebuild_collapsed_cache();
        self.layout_cache = Some(Box::new(cache));
    }
}

/// What a geometry-only relayout needs from the last full layout: the folder tree with its cycle
/// boxes, the lifted wires, and each container's columns, order and measurements. Not saved.
#[derive(Clone, Default)]
pub(crate) struct LayoutCache {
    /// Cards, wires and folders the layout was made for.
    fingerprint: u64,
    tree: Tree,
    pairs: Vec<(NodeId, NodeId)>,
    doc_pairs: Vec<(NodeId, NodeId)>,
    links: Vec<BTreeSet<(From, To)>>,
    doc_links: Vec<BTreeSet<(From, To)>>,
    outer: Vec<GateKeys>,
    hubs: Hubs,
    /// Columns and order from the previous layout, for containers whose items did not change.
    carried: Vec<Option<Frozen>>,
    /// Wires per kind for every code card pair.
    pair_kinds: HashMap<(NodeId, NodeId), [u32; EdgeKind::COUNT]>,
    frozen: Vec<Frozen>,
    placed: Vec<Placed>,
}

impl std::fmt::Debug for LayoutCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LayoutCache {{ {} containers }}", self.tree.containers.len())
    }
}

impl LayoutCache {
    /// Measures the containers marked in `which`, children before parents. With `reuse`, each
    /// keeps the columns and order it had.
    fn measure(&mut self, graph: &Graph, which: &[bool], reuse: bool) {
        let mut order: Vec<usize> = (0..self.tree.containers.len()).filter(|&c| which[c]).collect();
        order.sort_by_key(|&c| std::cmp::Reverse(self.tree.depth(c)));
        for c in order {
            let frozen = reuse.then(|| &self.frozen[c]);
            let carried = self.carried[c].as_ref();
            let (placed, frozen) = self.tree.measure_one(
                graph,
                c,
                &self.links[c],
                &self.hubs.order_only[c],
                &self.doc_links[c],
                &self.outer[c],
                &self.placed,
                frozen,
                carried,
            );
            self.placed[c] = placed;
            self.frozen[c] = frozen;
        }
    }
}

/// Changes when cards, wires or folders are added or removed, or a folder opens or closes:
/// which folders are closed decides which gates their wires share.
fn fingerprint(graph: &Graph) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    graph.nodes.len().hash(&mut h);
    graph.clusters.len().hash(&mut h);
    for e in &graph.edges {
        (e.id, e.from_node, e.to_node, e.kind as u8).hash(&mut h);
    }
    for c in &graph.clusters {
        c.is_collapsed().hash(&mut h);
    }
    h.finish()
}

/// Wires per kind for every (provider, consumer) card pair.
fn pair_kinds(graph: &Graph) -> HashMap<(NodeId, NodeId), [u32; EdgeKind::COUNT]> {
    let mut kinds: HashMap<(NodeId, NodeId), [u32; EdgeKind::COUNT]> = HashMap::new();
    for e in graph.edges.iter().filter(|e| e.kind.is_code_flow()) {
        kinds.entry((e.from_node, e.to_node)).or_default()[e.kind as usize] += 1;
    }
    kinds
}

/// Hubs found by the hub rule, and what they change in the layout.
#[derive(Clone, Default)]
struct Hubs {
    public: Vec<Hub>,
    /// Card pairs whose wires a hub hides: not lifted, laid out or routed.
    hidden_pairs: BTreeSet<(NodeId, NodeId)>,
    /// Per container, the hidden sibling wires that still order the columns, so a hub stays left
    /// of what it feeds.
    order_only: Vec<BTreeSet<(Item, Item)>>,
}

/// The gates each container needs, seen from outside: every provider its parent wires into or out
/// of it. A folder whose contents are hidden has no links of its own to tell.
fn outer_gate_keys(n: usize, links: &[BTreeSet<(From, To)>], doc_links: &[BTreeSet<(From, To)>]) -> Vec<GateKeys> {
    let mut outer = vec![GateKeys::default(); n];
    for (code, all) in [(true, links), (false, doc_links)] {
        for (from, to) in all.iter().flatten() {
            if let From::Item(Item::Sub(s), k) = from {
                let keys = &mut outer[*s];
                if code { &mut keys.code_out } else { &mut keys.doc_out }.insert(*k);
            }
            if let To::Item(Item::Sub(s), k) = to {
                let keys = &mut outer[*s];
                if code { &mut keys.code_in } else { &mut keys.doc_in }.insert(*k);
            }
        }
    }
    outer
}

/// A container's columns, kept between layouts so a relayout never reorders: the layer of each
/// item and the column order found by the crossing reduction.
#[derive(Debug, Clone, Default)]
struct Frozen {
    item_layers: Vec<usize>,
    order: Vec<Vec<usize>>,
    nodes: usize,
    /// The items of each column in order, without lanes.
    item_order: Vec<Vec<Item>>,
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

/// Distinct (documentation file, documented card) pairs, in a stable order. They are routed but
/// never change the layout.
fn documentation_pairs(graph: &Graph, tree: &Tree) -> Vec<(NodeId, NodeId)> {
    let pairs: BTreeSet<(NodeId, NodeId)> = graph
        .edges
        .iter()
        .filter(|e| e.kind == crate::model::EdgeKind::Documentation && e.from_node != e.to_node)
        .filter(|e| tree.home.contains_key(&e.from_node) && tree.home.contains_key(&e.to_node))
        .map(|e| (e.from_node, e.to_node))
        .collect();
    pairs.into_iter().collect()
}

#[derive(Clone, Default)]
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
                collapsed: c.is_collapsed(),
                node_view: c.detail == crate::model::FolderDetail::NodeView,
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

    /// Containers that are hidden: inside a folder whose contents are hidden, or that folder's
    /// own inside. Wires are lifted and laid out through them anyway, so showing them again
    /// changes nothing else.
    fn hidden(&self) -> Vec<bool> {
        let mut hidden = vec![false; self.containers.len()];
        for (c, flag) in hidden.iter_mut().enumerate() {
            let mut at = Some(c);
            while let Some(i) = at {
                if self.containers[i].collapsed {
                    *flag = true;
                    break;
                }
                at = self.containers[i].parent;
            }
        }
        hidden
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
            let (ui, uc) = (Item::Card(u), self.chain(u));
            let (vi, vc) = (Item::Card(v), self.chain(v));
            let Some(lca) = lowest_common(&uc, &vc) else { continue };
            let (a, b) = (self.item_in(ui, &uc, lca), self.item_in(vi, &vc, lca));
            if a != b {
                sibling_edges.entry(lca).or_default().insert((a, b));
            }
        }
        for (container, edges) in sibling_edges {
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
                    node_view: false,
                    key,
                });
            }
        }
    }

    /// The wire pieces a card pair is made of, in order from provider to consumer: out of each
    /// container below the lowest common one, across that one, then into each container down to
    /// the consumer.
    ///
    /// Outside a closed folder its cards' wires are keyed by the folder, so they share one gate
    /// and one lane per container; inside it (hidden, laid out ahead for when it opens) they keep
    /// their card keys.
    fn pieces(&self, u: NodeId, v: NodeId) -> Vec<(usize, From, To)> {
        let (ui, uc) = (Item::Card(u), self.chain(u));
        let (vi, vc) = (Item::Card(v), self.chain(v));
        let Some(lca) = lowest_common(&uc, &vc) else { return Vec::new() };
        let (a, b) = (self.item_in(ui, &uc, lca), self.item_in(vi, &vc, lca));
        if a == b {
            return Vec::new();
        }
        // The key of `u` in the container at chain position `p`: the outermost closed folder
        // below it, if any.
        let key_at =
            |p: usize| uc[..p].iter().rposition(|&c| self.containers[c].collapsed).map_or(u, |i| folder_key(uc[i]));
        let lca_pos = uc.iter().position(|&c| c == lca).unwrap_or(uc.len());
        let mut pieces = Vec::new();
        for (p, &c) in uc.iter().enumerate().take_while(|&(_, &c)| c != lca) {
            let k = key_at(p);
            pieces.push((c, From::Item(self.item_in(ui, &uc, c), k), To::Gate(k)));
        }
        let k = key_at(lca_pos);
        pieces.push((lca, From::Item(a, k), To::Item(b, k)));
        let inward: Vec<usize> = vc.iter().copied().take_while(|&c| c != lca).collect();
        for &c in inward.iter().rev() {
            pieces.push((c, From::Gate(k), To::Item(self.item_in(vi, &vc, c), k)));
        }
        pieces
    }

    /// How a card is seen: itself, or the outermost closed folder hiding it.
    fn visible(&self, card: NodeId) -> Result<NodeId, usize> {
        let chain = self.chain(card);
        match chain.iter().rposition(|&c| self.containers[c].collapsed) {
            Some(i) => Err(chain[i]),
            None => Ok(card),
        }
    }

    fn container_id(&self, graph: &Graph, c: usize) -> String {
        match self.containers[c].kind {
            Kind::Folder(i) => graph.clusters[i].id.clone(),
            Kind::Cycle => self.containers[c].key.clone(),
        }
    }

    /// Applies the hub rule in every container: an item feeding at least half of the container's
    /// connected items, and at least [`HUB_MIN_CONSUMERS`], is a hub.
    fn find_hubs(&self, pairs: &[(NodeId, NodeId)]) -> Hubs {
        // Per container: each item's consumers, every connected item, and the pairs per item.
        let mut feeds: BTreeMap<usize, BTreeMap<Item, BTreeSet<Item>>> = BTreeMap::new();
        let mut connected: BTreeMap<usize, BTreeSet<Item>> = BTreeMap::new();
        // (container, item) → the card pairs it feeds, with the consuming item.
        type PairsOf = BTreeMap<(usize, Item), Vec<(NodeId, NodeId, Item)>>;
        let mut pairs_of: PairsOf = BTreeMap::new();
        for &(u, v) in pairs {
            let (uc, vc) = (self.chain(u), self.chain(v));
            let Some(lca) = lowest_common(&uc, &vc) else { continue };
            let (a, b) = (self.item_in(Item::Card(u), &uc, lca), self.item_in(Item::Card(v), &vc, lca));
            if a == b {
                continue;
            }
            feeds.entry(lca).or_default().entry(a).or_default().insert(b);
            connected.entry(lca).or_default().extend([a, b]);
            pairs_of.entry((lca, a)).or_default().push((u, v, b));
        }
        let mut hubs = Hubs { order_only: vec![BTreeSet::new(); self.containers.len()], ..Default::default() };
        for (c, items) in feeds {
            let others = connected[&c].len().saturating_sub(1);
            for (a, consumers) in items {
                if consumers.len() < HUB_MIN_CONSUMERS || consumers.len() * 2 < others {
                    continue;
                }
                let hidden = &pairs_of[&(c, a)];
                for &(u, v, b) in hidden {
                    hubs.hidden_pairs.insert((u, v));
                    hubs.order_only[c].insert((a, b));
                }
                // A container's key is its folder's cluster id, or the cycle box id.
                hubs.public.push(Hub {
                    container: self.containers[c].key.clone(),
                    item: match a {
                        Item::Card(n) => VisibleEnd::Card(n),
                        Item::Sub(s) => VisibleEnd::Folder(self.containers[s].key.clone()),
                    },
                    used_by: consumers.len() as u32,
                    pairs: hidden.iter().map(|&(u, v, _)| (u, v)).collect(),
                });
            }
        }
        hubs
    }

    /// Groups the code routes by what is visible at each end and records each route's bundle.
    fn bundle(
        &self,
        graph: &Graph,
        routes: &mut [WireRoute],
        pair_kinds: &HashMap<(NodeId, NodeId), [u32; EdgeKind::COUNT]>,
    ) -> Vec<WireBundle> {
        let end = |card: NodeId| match self.visible(card) {
            Ok(n) => VisibleEnd::Card(n),
            Err(c) => VisibleEnd::Folder(self.container_id(graph, c)),
        };
        let mut index: HashMap<(VisibleEnd, VisibleEnd), u32> = HashMap::new();
        let mut bundles: Vec<WireBundle> = Vec::new();
        for (i, r) in routes.iter_mut().enumerate() {
            let key = (end(r.provider), end(r.consumer));
            let b = *index.entry(key.clone()).or_insert_with(|| {
                bundles.push(WireBundle {
                    from: key.0,
                    to: key.1,
                    pairs: 0,
                    kinds: [0; EdgeKind::COUNT],
                    route: i as u32,
                });
                (bundles.len() - 1) as u32
            });
            let bundle = &mut bundles[b as usize];
            bundle.pairs += 1;
            if let Some(k) = pair_kinds.get(&(r.provider, r.consumer)) {
                for (total, n) in bundle.kinds.iter_mut().zip(k) {
                    *total += n;
                }
            }
            r.bundle = b;
        }
        bundles
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

    /// Joins each pair's pieces into one world-space route, using the routes `of` each container.
    fn compose_routes(
        &self,
        pairs: &[(NodeId, NodeId)],
        placed: &[Placed],
        origins: &[[f32; 2]],
        hidden: &[bool],
        of: impl Fn(&Placed) -> &BTreeMap<(From, To), Vec<[f32; 2]>> + Sync,
    ) -> Vec<WireRoute>
    where
        Self: Sync,
    {
        use rayon::prelude::*;
        let compose = |&(u, v): &(NodeId, NodeId)| -> Option<WireRoute> {
            // Pieces inside a hidden folder are skipped: the route starts or ends at its gate.
            let pieces: Vec<(usize, From, To)> = self.pieces(u, v).into_iter().filter(|p| !hidden[p.0]).collect();
            if pieces.is_empty() {
                return None;
            }
            let mut points: Vec<[f32; 2]> = Vec::new();
            for (c, from, to) in pieces {
                let local = of(&placed[c]).get(&(from, to))?;
                let o = origins[c];
                for p in local {
                    let w = [o[0] + p[0], o[1] + p[1]];
                    if points.last().is_none_or(|l| (l[0] - w[0]).abs() > 0.01 || (l[1] - w[1]).abs() > 0.01) {
                        points.push(w);
                    }
                }
            }
            Some(WireRoute { provider: u, consumer: v, points: simplify(points), bundle: 0 })
        };
        // Pairs are independent; the result keeps their order.
        pairs.par_iter().filter_map(compose).collect()
    }

    fn depth(&self, mut c: usize) -> usize {
        let mut d = 0;
        while let Some(p) = self.containers[c].parent {
            d += 1;
            c = p;
        }
        d
    }

    #[allow(clippy::too_many_arguments)]
    fn measure_one(
        &self,
        graph: &Graph,
        c: usize,
        links: &BTreeSet<(From, To)>,
        order_only: &BTreeSet<(Item, Item)>,
        doc_links: &BTreeSet<(From, To)>,
        outer: &GateKeys,
        placed: &[Placed],
        frozen: Option<&Frozen>,
        carried: Option<&Frozen>,
    ) -> (Placed, Frozen) {
        let container = &self.containers[c];
        let is_cycle = container.kind == Kind::Cycle;
        let doc_providers: BTreeSet<NodeId> = doc_links.iter().map(|(from, _)| from.key()).collect();
        let strip_top = if is_cycle { CYCLE_PAD_TOP } else { PAD_TOP };
        let strip = docs::strip_height(doc_providers.len(), if is_cycle { 0.0 } else { DOC_STRIP });
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
        // Columns and order: kept from the last layout when asked, found afresh otherwise.
        let mut lay = ColumnLayout::new(
            &container.items,
            links,
            order_only,
            &size_of,
            &port_y,
            is_cycle,
            frozen.or(carried).map(|f| &f.item_layers[..]),
        );
        match frozen {
            Some(f) if f.nodes == lay.nodes.len() && f.order.iter().map(Vec::len).sum::<usize>() == lay.nodes.len() => {
                lay.layers = f.order.clone();
            }
            _ => {
                lay.order();
                // A full layout after a folder opened or closed: the wire lanes may differ, but
                // the items keep the order they had.
                if let Some(prev) = carried {
                    lay.keep_item_order(&prev.item_order);
                }
            }
        }
        let item_order = lay.layers.iter().map(|l| l.iter().filter_map(|&n| lay.nodes[n].item).collect()).collect();
        let kept = Frozen {
            item_layers: lay.item_layer.clone(),
            order: lay.layers.clone(),
            nodes: lay.nodes.len(),
            item_order,
        };

        if container.node_view {
            let name = |k: &NodeId| match key_folder(*k) {
                Some(f) => match self.containers[f].kind {
                    Kind::Folder(i) => graph.clusters[i].label.clone(),
                    Kind::Cycle => "loop".to_string(),
                },
                None => graph.nodes.get(k).map_or_else(String::new, |n| n.title.clone()),
            };
            return (node_view(outer, &name), kept);
        }
        if container.collapsed {
            // Minimised: one input and one output gate. Every code wire meets in the middle of its
            // edge, every documentation wire level with a card's documentation port.
            let mid = COLLAPSED_FOLDER_SIZE[1] * 0.5;
            let at = |keys: &BTreeSet<NodeId>, y: f32| keys.iter().map(|&k| (k, y)).collect();
            let p = Placed {
                size: COLLAPSED_FOLDER_SIZE,
                in_gates: at(&outer.code_in, mid),
                out_gates: at(&outer.code_out, mid),
                doc_in: at(&outer.doc_in, DOC_PORT_OFFSET_Y),
                doc_out: at(&outer.doc_out, DOC_PORT_OFFSET_Y),
                ..Default::default()
            };
            return (p, kept);
        }
        lay.coordinates();

        let (pad_top, pad_x, pad_bottom) =
            if is_cycle { (CYCLE_PAD_TOP + strip, CYCLE_PAD, CYCLE_PAD) } else { (PAD_TOP + strip, PAD_X, PAD_BOTTOM) };
        let mut p = Placed::default();
        let content_left = pad_x + lay.in_band;
        let mut bottom = pad_top;
        let mut right = content_left;
        for node in &lay.nodes {
            // Lanes count too: a wire passing below the lowest item must stay inside.
            let pos = [content_left + lay.column_x[node.layer], pad_top + node.y];
            bottom = bottom.max(pos[1] + node.h);
            let Some(it) = node.item else { continue };
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
        let content_above_shelf = has_columns || !p.in_gates.is_empty() || !p.out_gates.is_empty();
        let shelf_top = if content_above_shelf { bottom + SHELF_GAP } else { pad_top };
        let wrap = (right - pad_x).max(shelf.iter().map(|(_, s)| s[0] * s[1]).sum::<f32>().sqrt() * 1.4);
        let (mut x, mut y, mut row_h) = (0.0f32, 0.0f32, 0.0f32);
        let mut rows: Vec<docs::ShelfRow> = Vec::new();
        for (it, s) in shelf {
            if x > 0.0 && x + s[0] > wrap {
                x = 0.0;
                y += row_h + ITEM_GAP;
                row_h = 0.0;
            }
            let pos = [pad_x + x, shelf_top + y];
            if x == 0.0 {
                rows.push(docs::ShelfRow { top: pos[1], items: Vec::new() });
            }
            if let Some(row) = rows.last_mut() {
                row.items.push((it, pos[0], pos[0] + s[0]));
            }
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
        p.stats = lay.stats();
        p.stats.shelf_items = lay.isolated.len();
        p.stats.shelf_rows = rows.len();
        p.stats.in_gates = p.in_gates.len();
        p.stats.out_gates = p.out_gates.len();
        p.stats.doc_gates =
            doc_links.iter().filter(|(f, t)| matches!(f, From::Gate(_)) || matches!(t, To::Gate(_))).count();
        p.stats.doc_strip = strip;

        // Documentation: gates on the provider's strip track, routes through the free channels.
        let tracks = docs::strip_tracks(&doc_providers, strip_top, strip);
        for (from, to) in doc_links {
            if let From::Gate(k) = from {
                p.doc_in.insert(*k, tracks[k]);
            }
            if let To::Gate(k) = to {
                p.doc_out.insert(*k, tracks[k]);
            }
        }
        if !doc_links.is_empty() {
            let channels = docs::Channels {
                width: p.size[0],
                tracks: &tracks,
                columns: (0..lay.layers.len())
                    .map(|l| {
                        let left = content_left + lay.column_x[l];
                        (left, left + lay.col_w[l])
                    })
                    .collect(),
                column_of: lay.nodes.iter().filter_map(|n| n.item.map(|it| (it, n.layer))).collect(),
                rows,
                shelf_top,
                content_above_shelf,
                margin: pad_x,
                rects: p.items.iter().map(|&(it, pos)| (it, (pos, size_of(it)))).collect(),
            };
            let exit_y = |it: Item, k: NodeId| match it {
                Item::Card(n) => graph.nodes.get(&n).map_or(21.0, card_port_offset),
                Item::Sub(s) => placed[s].doc_out.get(&k).copied().unwrap_or(DOC_PORT_OFFSET_Y),
            };
            let entry_y = |it: Item, k: NodeId| match it {
                Item::Card(_) => DOC_PORT_OFFSET_Y,
                Item::Sub(s) => placed[s].doc_in.get(&k).copied().unwrap_or(DOC_PORT_OFFSET_Y),
            };
            p.doc_routes = channels.route(doc_links, &exit_y, &entry_y);
        }
        (p, kept)
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
        let sides = [
            (&p.in_gates, GateSide::Input, GateKind::Code),
            (&p.out_gates, GateSide::Output, GateKind::Code),
            (&p.doc_in, GateSide::Input, GateKind::Documentation),
            (&p.doc_out, GateSide::Output, GateKind::Documentation),
        ];
        for (gates, side, kind) in sides {
            let x = if side == GateSide::Input { origin[0] } else { origin[0] + p.size[0] };
            for (&k, &y) in gates {
                let source = match key_folder(k) {
                    Some(f) => VisibleEnd::Folder(self.container_id(graph, f)),
                    None => VisibleEnd::Card(k),
                };
                flow.gates.push(Gate { container: id.clone(), side, kind, source, position: [x, origin[1] + y] });
            }
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

/// A folder in node view (L5 level 2): every gate listed like the pins of a node, documentation
/// gates first, each side in alphabetical order of the file whose wires it carries. The inside is
/// hidden.
fn node_view(outer: &GateKeys, name: &dyn Fn(&NodeId) -> String) -> Placed {
    let sorted = |keys: &BTreeSet<NodeId>| {
        let mut keys: Vec<NodeId> = keys.iter().copied().collect();
        keys.sort_by_cached_key(|k| (name(k), *k));
        keys
    };
    let (doc_in, code_in) = (sorted(&outer.doc_in), sorted(&outer.code_in));
    let (doc_out, code_out) = (sorted(&outer.doc_out), sorted(&outer.code_out));
    let rows = (doc_in.len() + code_in.len()).max(doc_out.len() + code_out.len());
    let row_y = |i: usize| PAD_TOP + (i as f32 + 0.5) * GATE_PITCH;
    let mut p = Placed {
        size: [NODE_VIEW_WIDTH, (PAD_TOP + rows as f32 * GATE_PITCH + PAD_BOTTOM).max(COLLAPSED_FOLDER_SIZE[1])],
        ..Default::default()
    };
    for (i, k) in doc_in.iter().enumerate() {
        p.doc_in.insert(*k, row_y(i));
    }
    for (i, k) in code_in.iter().enumerate() {
        p.in_gates.insert(*k, row_y(doc_in.len() + i));
    }
    for (i, k) in doc_out.iter().enumerate() {
        p.doc_out.insert(*k, row_y(i));
    }
    for (i, k) in code_out.iter().enumerate() {
        p.out_gates.insert(*k, row_y(doc_out.len() + i));
    }
    p
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
    /// Layer of each item (by index in the container), as decided before the lanes were added.
    item_layer: Vec<usize>,
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
    /// Row below the columns for each backward path (by index into `paths`).
    back_row: Vec<usize>,
    /// x of each wire's vertical track, relative to the content's left edge. Index 0 is the input
    /// gate band, `l + 1` the gap right of column `l`; the last is the output gate band.
    tracks: Vec<BTreeMap<TrackKey, f32>>,
}

impl ColumnLayout {
    #[allow(clippy::too_many_arguments)]
    fn new(
        items: &[Item],
        links: &BTreeSet<(From, To)>,
        order_only: &BTreeSet<(Item, Item)>,
        size_of: &dyn Fn(Item) -> [f32; 2],
        port_y: &dyn Fn(Item, NodeId, bool) -> f32,
        is_cycle: bool,
        frozen_layers: Option<&[usize]>,
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
        // Wires hidden behind a hub badge still order the columns.
        for (a, b) in order_only {
            if let (Some(&i), Some(&j)) = (pos.get(a), pos.get(b)) {
                if i != j {
                    adj[i].push(j);
                    connected[i] = true;
                    connected[j] = true;
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

        // Shape the columns towards TARGET_ASPECT. Inside a cycle box the order cannot hold
        // anyway, so the items are packed into columns in their acyclic order; elsewhere a column
        // much taller than the rest is split into side-by-side columns (L1 still holds: items in
        // one layer never wire to each other).
        let sizes: Vec<[f32; 2]> = items.iter().map(|&it| size_of(it)).collect();
        let layer = if let Some(kept) = frozen_layers.filter(|k| k.len() == n) {
            kept.to_vec()
        } else if is_cycle {
            let seq: Vec<usize> = order.iter().copied().filter(|&i| connected[i]).collect();
            let mut packed = vec![0usize; n];
            for (&i, column) in seq.iter().zip(pack_columns(&seq, &sizes)) {
                packed[i] = column;
            }
            packed
        } else {
            wrap_tall_layers(&layer, &connected, &sizes)
        };

        let mut lay = ColumnLayout {
            item_layer: layer.clone(),
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
            back_row: Vec::new(),
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

        // The vertical span each wire covers in each gap. Wires whose spans do not overlap share a
        // track, so a gap is only as wide as the most wires passing any one height.
        let mut spans: BTreeMap<(usize, TrackKey), (f32, f32)> = BTreeMap::new();
        let mut cover = |gap: usize, key: TrackKey, a: f32, b: f32| {
            let span = spans.entry((gap, key)).or_insert((f32::INFINITY, f32::NEG_INFINITY));
            *span = (span.0.min(a.min(b)), span.1.max(a.max(b)));
        };
        for (seg, &(item, key)) in self.segs.iter().zip(&self.seg_source) {
            let gate_y = |gates: &BTreeMap<NodeId, f32>, g: &NodeId| gates.get(g).copied().unwrap_or(0.0);
            let (gap, a, b) = match *seg {
                Seg::Node { from, to, from_y, to_y } => {
                    (self.nodes[from].layer + 1, self.nodes[from].y + from_y, self.nodes[to].y + to_y)
                }
                Seg::In { gate, to, to_y } => (0, gate_y(&self.in_gate_y, &gate), self.nodes[to].y + to_y),
                Seg::Out { from, from_y, gate } => {
                    (layers, self.nodes[from].y + from_y, gate_y(&self.out_gate_y, &gate))
                }
            };
            cover(gap, (item, key, 0), a, b);
        }
        // The ends of a backward wire run down to the rows below the columns.
        for path in self.paths.iter().filter(|p| p.back) {
            let (From::Item(a, key), To::Item(b, _)) = (path.from, path.to) else { continue };
            let (u, v) = (path.nodes[0], path.nodes[1]);
            cover(self.nodes[u].layer + 1, (Some(a), key, 1), self.nodes[u].y + path.from_y, f32::INFINITY);
            cover(self.nodes[v].layer, (Some(b), key, 2), self.nodes[v].y + path.to_y, f32::INFINITY);
        }
        let mut slot: BTreeMap<(usize, TrackKey), usize> = BTreeMap::new();
        let mut used = vec![0usize; layers + 1];
        for (gap, keys) in gap_keys.iter().enumerate() {
            let intervals: Vec<(f32, f32, TrackKey)> = keys
                .iter()
                .map(|k| {
                    let (lo, hi) = spans.get(&(gap, *k)).copied().unwrap_or((0.0, 0.0));
                    (lo, hi, *k)
                })
                .collect();
            let (slots, count) = share_tracks(&intervals);
            used[gap] = count;
            for ((_, _, k), t) in intervals.iter().zip(slots) {
                slot.insert((gap, *k), t);
            }
        }

        // Column x positions, sized for the tracks in each gap.
        self.col_w = self.layers.iter().map(|l| l.iter().map(|&n| self.nodes[n].w).fold(0.0, f32::max)).collect();
        let band = |n: usize| if n == 0 { 0.0 } else { GATE_BAND + TRACK_PITCH * n as f32 };
        self.in_band = band(used[0]);
        self.out_band = band(used[layers]);
        let mut x = 0.0;
        let mut gap_left = vec![-self.in_band; layers + 1];
        self.column_x = Vec::with_capacity(layers);
        for l in 0..layers {
            self.column_x.push(x);
            x += self.col_w[l];
            gap_left[l + 1] = x;
            if l + 1 < layers {
                x += GAP_BASE + TRACK_PITCH * used[l + 1] as f32;
            }
        }
        self.content_width = x;
        self.tracks = vec![BTreeMap::new(); layers + 1];
        for (&(gap, k), &t) in &slot {
            let margin = if gap == 0 || gap == layers { GATE_BAND * 0.5 } else { GAP_BASE * 0.5 };
            self.tracks[gap].insert(k, gap_left[gap] + margin + t as f32 * TRACK_PITCH);
        }

        // Rows for backward wires: wires whose horizontal runs do not overlap share a row.
        let back: Vec<(usize, f32, f32)> = self
            .paths
            .iter()
            .enumerate()
            .filter(|(_, p)| p.back)
            .filter_map(|(i, p)| {
                let (From::Item(a, key), To::Item(b, _)) = (p.from, p.to) else { return None };
                let (u, v) = (p.nodes[0], p.nodes[1]);
                let t1 = self.tracks[self.nodes[u].layer + 1].get(&(Some(a), key, 1)).copied()?;
                let t2 = self.tracks[self.nodes[v].layer].get(&(Some(b), key, 2)).copied()?;
                Some((i, t1.min(t2), t1.max(t2)))
            })
            .collect();
        let intervals: Vec<(f32, f32, usize)> = back.iter().map(|&(i, lo, hi)| (lo, hi, i)).collect();
        let (rows, count) = share_tracks(&intervals);
        self.back_row = vec![0; self.paths.len()];
        for (&(i, _, _), row) in back.iter().zip(rows) {
            self.back_row[i] = row;
        }
        self.back_count = count;

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
    /// Puts each column's items back in the given order, leaving the wire lanes where they are.
    /// Items not in `order` keep their place after the ones that are.
    fn keep_item_order(&mut self, order: &[Vec<Item>]) {
        for (layer, wanted) in self.layers.iter_mut().zip(order) {
            let rank = |it: &Item| wanted.iter().position(|w| w == it).unwrap_or(usize::MAX);
            let slots: Vec<usize> = (0..layer.len()).filter(|&i| self.nodes[layer[i]].item.is_some()).collect();
            let mut items: Vec<usize> = slots.iter().map(|&i| layer[i]).collect();
            items.sort_by_key(|&n| rank(&self.nodes[n].item.unwrap()));
            for (slot, node) in slots.into_iter().zip(items) {
                layer[slot] = node;
            }
        }
    }

    /// Shape of the columns, for the layout report.
    fn stats(&self) -> ContainerStats {
        let layers = self.layers.len();
        let count = |l: &Vec<usize>, items: bool| l.iter().filter(|&&n| self.nodes[n].item.is_some() == items).count();
        let height = |l: &Vec<usize>| l.iter().map(|&n| self.nodes[n].h + gap_after(&self.nodes[n])).sum::<f32>();
        ContainerStats {
            column_items: self.nodes.iter().filter(|n| n.item.is_some()).count(),
            layers,
            max_column_items: self.layers.iter().map(|l| count(l, true)).max().unwrap_or(0),
            max_column_lanes: self.layers.iter().map(|l| count(l, false)).max().unwrap_or(0),
            max_column_height: self.layers.iter().map(height).fold(0.0, f32::max),
            columns_width: self.col_w.iter().sum(),
            gaps_width: self.content_width - self.col_w.iter().sum::<f32>(),
            max_gap_tracks: self
                .tracks
                .iter()
                .skip(1)
                .take(layers.saturating_sub(1))
                .map(|t| t.len())
                .max()
                .unwrap_or(0),
            in_band: self.in_band,
            out_band: self.out_band,
            ..Default::default()
        }
    }

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
        for (index, path) in self.paths.iter().enumerate() {
            let (src_item, key) = match path.from {
                From::Item(a, k) => (Some(a), k),
                From::Gate(k) => (None, k),
            };
            let points = if path.back {
                let To::Item(b, _) = path.to else { continue };
                let (u, v) = (path.nodes[0], path.nodes[1]);
                let (yu, yv) = (y_of(u, path.from_y), y_of(v, path.to_y));
                let row = back_top + (self.back_row.get(index).copied().unwrap_or(0) as f32 + 0.5) * LANE;
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

/// Width over height the layout aims for when it has a choice: inside cycle boxes and when
/// splitting very tall columns.
const TARGET_ASPECT: f32 = 1.6;
/// A column is split when it is this many times taller than the container's target height.
const TALL_COLUMN: f32 = 1.5;

/// Height a column of these items takes when stacked.
fn stack_height(items: impl Iterator<Item = [f32; 2]>) -> f32 {
    items.map(|s| s[1] + ITEM_GAP).sum()
}

/// Column of each item of `seq` (in that order) when the sequence is cut into consecutive
/// columns, choosing the number of columns that brings the block closest to TARGET_ASPECT.
fn pack_columns(seq: &[usize], sizes: &[[f32; 2]]) -> Vec<usize> {
    let total = stack_height(seq.iter().map(|&i| sizes[i]));
    let mut best: Option<(f32, Vec<usize>)> = None;
    for k in 1..=seq.len().max(1) {
        let limit = total / k as f32;
        let (mut columns, mut column, mut height) = (Vec::with_capacity(seq.len()), 0, 0.0f32);
        let (mut widths, mut tallest) = (vec![0.0f32], 0.0f32);
        for &i in seq {
            let h = sizes[i][1] + ITEM_GAP;
            if height > 0.0 && height + h > limit + 0.5 {
                column += 1;
                height = 0.0;
                widths.push(0.0);
            }
            height += h;
            tallest = tallest.max(height);
            widths[column] = widths[column].max(sizes[i][0]);
            columns.push(column);
        }
        let width = widths.iter().sum::<f32>() + GAP_BASE * column as f32;
        let score = (width / tallest.max(1.0) / TARGET_ASPECT).ln().abs();
        if best.as_ref().is_none_or(|(b, _)| score < *b - 1e-4) {
            best = Some((score, columns));
        }
        if column + 1 < k {
            // More columns were asked for than the items fill: later k change nothing.
            break;
        }
    }
    best.map_or_else(Vec::new, |(_, c)| c)
}

/// Splits layers whose stack is far taller than the container's target height into consecutive
/// side-by-side layers, shifting later layers right. Items keep their order within a layer.
fn wrap_tall_layers(layer: &[usize], connected: &[bool], sizes: &[[f32; 2]]) -> Vec<usize> {
    let layers = layer.iter().zip(connected).filter(|(_, &c)| c).map(|(&l, _)| l + 1).max().unwrap_or(0);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for (i, &l) in layer.iter().enumerate() {
        if connected[i] {
            members[l].push(i);
        }
    }
    let area: f32 = members.iter().flatten().map(|&i| (sizes[i][0] + GAP_BASE) * (sizes[i][1] + ITEM_GAP)).sum();
    let tallest_item = members.iter().flatten().map(|&i| sizes[i][1] + ITEM_GAP).fold(0.0, f32::max);
    let target = (area / TARGET_ASPECT).sqrt().max(tallest_item);
    let mut out = layer.to_vec();
    let mut shift = 0;
    for column in &members {
        let height = stack_height(column.iter().map(|&i| sizes[i]));
        let parts = if height > target * TALL_COLUMN { (height / target).ceil() as usize } else { 1 };
        let limit = height / parts as f32;
        let (mut part, mut h) = (0, 0.0f32);
        for &i in column {
            let ih = sizes[i][1] + ITEM_GAP;
            if h > 0.0 && h + ih > limit + 0.5 && part + 1 < parts {
                part += 1;
                h = 0.0;
            }
            h += ih;
            out[i] = layer[i] + shift + part;
        }
        shift += part;
    }
    out
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

/// Assigns each interval `(lo, hi, key)` the lowest track free over its span, with one track
/// pitch of clearance, visiting intervals by start. Returns each interval's track (in the input
/// order) and the number of tracks used, which is the most intervals overlapping at any point.
fn share_tracks<K: Ord + Copy>(intervals: &[(f32, f32, K)]) -> (Vec<usize>, usize) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut order: Vec<usize> = (0..intervals.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&intervals[a], &intervals[b]);
        x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)).then(x.2.cmp(&y.2))
    });
    // Tracks in use, by the end of their last span (plus clearance), and free tracks by index.
    let mut busy: BinaryHeap<Reverse<(Ord32, usize)>> = BinaryHeap::new();
    let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    let mut count = 0;
    let mut slots = vec![0; intervals.len()];
    for i in order {
        let (lo, hi, _) = intervals[i];
        while let Some(Reverse((Ord32(end), t))) = busy.peek().copied() {
            if end > lo {
                break;
            }
            busy.pop();
            free.push(Reverse(t));
        }
        let t = match free.pop() {
            Some(Reverse(t)) => t,
            None => {
                count += 1;
                count - 1
            }
        };
        busy.push(Reverse((Ord32(hi + TRACK_PITCH), t)));
        slots[i] = t;
    }
    (slots, count)
}

/// An f32 ordered by `total_cmp`, for heaps.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ord32(f32);

impl Eq for Ord32 {}

impl PartialOrd for Ord32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ord32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
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
