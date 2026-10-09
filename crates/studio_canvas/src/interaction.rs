use egui::Pos2;
use studio_graph::{EdgeId, NodeId, PortId};

/// Tracks the active user interaction mode on the canvas.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum InteractionMode {
    #[default]
    Idle,
    /// Mouse pressed down on a node panel; waiting for hold-and-drag threshold (>= 4px) or release (single click).
    PendingNodeDrag {
        node_id: NodeId,
        press_start_screen: Pos2,
        press_start_world: Pos2,
        is_shift: bool,
    },
    DraggingNodes {
        node_ids: Vec<NodeId>,
        drag_start_world: Pos2,
        initial_positions: Vec<[f32; 2]>,
    },
    Connecting {
        from_node: NodeId,
        from_port: PortId,
        is_output: bool,
        start_screen_pos: Pos2,
        current_screen_pos: Pos2,
        snapped_target: Option<(NodeId, PortId)>,
    },
    Panning {
        start_pointer: Pos2,
        has_panned: bool,
    },
}

/// Tracks elements currently hovered by the mouse pointer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoverState {
    pub hovered_node: Option<NodeId>,
    pub hovered_port: Option<(NodeId, PortId)>,
    pub hovered_edge: Option<EdgeId>,
}
