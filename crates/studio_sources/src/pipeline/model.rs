//! The Pipeline district's data: steps, the links between them, and the flows built from them.

use std::path::PathBuf;
use studio_graph::Provenance;

/// Everything the Pipeline district shows. Indices are positions in the vectors; all vectors are in their final order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pipeline {
    pub steps: Vec<Step>,
    pub links: Vec<Link>,
    pub flows: Vec<Flow>,
    pub groups: Vec<FlowGroup>,
    pub stats: PipelineStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StepKind {
    Main,
    Command,
    Sender,
    Route,
    Handler,
    Function,
    Contract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub kind: StepKind,
    pub title: String,  // e.g. "SendReport", "POST /api/v1/fleet-executor/commands/{commandId}/result"
    pub detail: String, // one line: e.g. "fleet_executor.rs:81", or a contract pointer
    pub language: String, // "Rust", "Enforce", "JSON", … from SourceLang
    pub path: PathBuf,  // project-relative
    pub line: usize,    // 1-based
    pub line_end: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkKind {
    Calls,
    Requests,
    Serves,
    Declares,
    Mounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    pub kind: LinkKind,
    pub provenance: Provenance,
    pub conditional: Option<String>,
    pub crossing: Option<(String, String)>, // (from language, to language) when they differ
    pub label: String, // chip text without the tier, e.g. a contract "fleet-command.schema.json#/definitions/ExecutionStart"
    pub evidence: Vec<(PathBuf, usize)>, // the lines that prove it (tag line, .route line, …)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    pub name: String,            // named after the entry point
    pub entry: usize,            // step index
    pub group: usize,            // FlowGroup index
    pub layers: Vec<Vec<usize>>, // step indices per BFS layer, layer 0 = [upstream senders…, entry]
    pub links: Vec<usize>,       // link indices used by this flow
    pub truncated: usize,        // steps beyond the data cap (0 normally)
    pub crosses_languages: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlowGroupKind {
    CrossLanguage,
    Endpoints,
    Mains,
    Commands,
    NoLinks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowGroup {
    pub kind: FlowGroupKind,
    pub name: String, // e.g. "Cross-language", "crates/api/api_missions/src/routes.rs", "Programs", "Commands"
    pub flows: Vec<usize>,
    pub expanded_by_default: bool, // by rule: CrossLanguage true, others false
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineStats {
    pub routes: usize,
    pub route_tags: usize,
    pub contract_tags: usize,
    pub entry_points: usize,
    pub flows: usize,
    pub steps: usize,
    pub links_by_tier: [usize; 4], // Proven, PossibleSet, Observed, Unresolved
    pub crossings: usize,
    pub handler_tags_agree: usize,
    pub handler_tags_disagree: usize,
    pub elapsed_ms: u64,
}
