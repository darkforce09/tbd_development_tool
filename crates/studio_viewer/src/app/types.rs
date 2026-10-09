use studio_graph::EdgeId;

/// High-level navigation modes inspired by CodeSee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StudioViewMode {
    #[default]
    CodebaseMaps,
    ArchitectureMap,
    FlowsAndTraces,
}

/// A flow trace variant for execution path comparison (CodeSee Flows).
#[derive(Debug, Clone)]
pub struct FlowVariant {
    pub name: String,
    pub coverage_pct: u8,
    pub summary: String,
    pub description: String,
    pub edge_ids: Vec<EdgeId>,
}

/// A flow definition representing an end-to-end trace.
#[derive(Debug, Clone)]
pub struct FlowDefinition {
    pub name: String,
    pub entry_point: String,
    pub description: String,
    pub originators: String,
    pub service_count: usize,
    pub api_call_count: usize,
    pub variants: Vec<FlowVariant>,
}
