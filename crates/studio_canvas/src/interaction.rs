use studio_graph::{EdgeId, NodeId, PortId};

/// What the pointer is doing on the canvas. Nothing the pointer does changes the map's data:
/// cards stay where the layout puts them and wires come from the code (docs/ROADMAP.md S1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[default]
    Idle,
    /// The view is being dragged, with any button.
    Panning,
}

/// Tracks elements currently hovered by the mouse pointer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoverState {
    pub hovered_node: Option<NodeId>,
    pub hovered_port: Option<(NodeId, PortId)>,
    pub hovered_edge: Option<EdgeId>,
}
