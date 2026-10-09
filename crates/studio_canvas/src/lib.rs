pub mod gpu;
pub mod grid;
pub mod interaction;
pub mod scene;
pub mod spatial;
pub mod transform;
pub mod view;
pub mod wire;

pub use gpu::{CanvasFrame, CanvasGpu, CanvasLayer, CanvasPaint, SceneUniforms};
pub use grid::paint_infinite_grid;
pub use interaction::{HoverState, InteractionMode};
pub use scene::CanvasScene;
pub use spatial::SpatialHashGrid;
pub use transform::{screen_to_world, world_to_screen, CanvasTransform};
pub use view::{
    archetype_color, calculate_file_node_size, data_type_color, edge_kind_color, CanvasAction, CanvasFrameStats,
    CanvasState, CanvasView,
};
pub use wire::{compute_bezier_control_points, distance_to_bezier, paint_bezier_wire};
