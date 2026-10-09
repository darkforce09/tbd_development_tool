pub mod connection;
pub mod layout;
pub mod mock;
pub mod model;

pub use connection::{can_connect, ConnectionError};
pub use layout::{
    ContainerStats, CycleBox, FlowLayout, Gate, GateKind, GateSide, Hub, VisibleEnd, WireBundle, WireRoute,
    DOC_PORT_OFFSET_Y, HUB_MIN_CONSUMERS,
};
pub use mock::create_showcase_graph;
pub use model::*;
