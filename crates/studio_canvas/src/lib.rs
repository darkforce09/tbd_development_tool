pub mod camera;
pub mod desk;
pub mod districts;
pub mod gpu;
pub mod grid;
pub mod interaction;
pub mod scene;
pub mod spatial;
pub mod transform;
pub mod view;
pub mod wire;
pub mod world;

pub use camera::{CameraTarget, Stop};
pub use desk::{content_for_text, CardContent, DeskRequest, DeskState};
pub use districts::changes::{
    BranchView, ChangesAction, ChangesLayout, ChangesSelection, ChangesView, CommitBead, FileChangeView, SessionChip,
    ShippedView, TicketLinkView, TicketView, TicketsColumnView, WorktreeRowView,
};
pub use districts::files::{FilesView, IgnoredView, SettingRow, SettingsCardView};
pub use districts::pipeline::{
    PipelineAction, PipelineGroup, PipelineLane, PipelineLink, PipelineStep, PipelineView, StepSlot,
};
pub use districts::run::{PackageBrick, RunSection, RunView, ToolTile, WorkflowRow};
pub use districts::DistrictViews;
pub use gpu::{CanvasFrame, CanvasGpu, CanvasLayer, CanvasPaint, SceneUniforms};
pub use grid::paint_infinite_grid;
pub use interaction::{HoverState, InteractionMode};
pub use scene::CanvasScene;
pub use spatial::SpatialHashGrid;
pub use transform::{screen_to_world, world_to_screen, CanvasTransform};
pub use view::shortcuts;
pub use view::{
    archetype_color, calculate_file_node_size, data_type_color, edge_kind_color, CanvasAction, CanvasFrameStats,
    CanvasState, CanvasView,
};
pub use wire::{compute_bezier_control_points, distance_to_bezier, paint_bezier_wire};
pub use world::DistrictExtents;
pub use world::WorldLayout;
