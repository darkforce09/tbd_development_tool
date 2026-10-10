//! The pipeline: routes, senders, handlers, contracts and the flows through them, built only from deterministic
//! facts (docs/ROADMAP.md S7).
//!
//! [`build_pipeline`] works in this order, every step reproducible:
//!
//! 1. **Crates and routes.** The crate graph comes from [`Packages`] (cargo metadata, renames included; see
//!    [`crates::crate_graph`]); R1 indexes it ([`RustIndex`](studio_parser::analysis::rust_resolver::RustIndex)) and
//!    the axum route table is evaluated from the router code. One `Route` step per route entry (title
//!    `METHOD template`, at its `.route(` line), one `Handler` step per handler; a `Serves` link from route to
//!    handler, Proven (route table) when the handler resolves to one item, else Unresolved with the handler text as
//!    its label. A route under an `if` keeps its tier and carries the condition. Links found twice (one route
//!    evaluated from two roots or under two conditions) are one link: the stronger tier wins; at an equal tier an
//!    unconditional one wins, and different conditions join with " || " (sorted, deduplicated).
//! 2. **Programs and mounts.** Every binary's `fn main` (resolved in the crate root) is a `Main` step. The function
//!    holding each route's root (a router function or the serve site) `Mounts` that route, Proven.
//! 3. **Tags.** Only code files holding the bytes `@route` or `@contract` are read further; their owners are R1's
//!    functions for indexed Rust files and the language extractor's items otherwise. A route tag owned by a handler
//!    is a handler tag: it agrees when every (method × template) alternative unifies with an entry of that handler,
//!    and makes no link. Every other route tag makes its owner a sender (a `Sender` step, or its file at the tag
//!    line; an owner that is already a program or function keeps that kind and its detail notes it also sends); each
//!    alternative is matched against the entries with the same method (or `ANY`) whose template unifies: exactly
//!    one entry with a resolved handler is a Proven `Requests` link, n > 1 entries are n Possible-set links, and
//!    otherwise one Unresolved link goes to a `Route` step made for the tag itself. The sender's own contract tags
//!    are the link's label; `crossing` is set when the two steps' languages differ.
//! 4. **Commands.** clap subcommands and cargo aliases from [`Tools`] with a definition line are `Command` steps.
//! 5. **Calls.** From Handler, Main and Function steps, the calls R1 resolves to one workspace function become
//!    `Function` steps with Proven `Calls` links; calls written in `#[cfg]`-gated code (the statement, the calling
//!    function, its impl or trait, an enclosing inline module, or a whole file declared by a gated `#[cfg] mod x;`)
//!    are left out, as R1 marks them (`CallSite::cfg_gated`). No name matching.
//! 6. **Contracts.** A contract tag resolves to a JSON file and pointer among the project's `.json` files: a
//!    `Contract` step per (file, pointer), declared by the owner's step (an item owner's step, or every item step
//!    of the file for a file owner), Proven, Possible set or Unresolved (with the reason as the label). Contracts are
//!    data, so a `Declares` link never counts as a language crossing.
//! 7. **Flows.** One flow per entry point (each route-table route, each main, each command): layer 0 is the senders
//!    whose usable `Requests` links reach the entry (sorted), then the entry, always last; then BFS forward over
//!    Proven and Possible-set links, at most [`flows::MAX_LAYERS`] layers and [`flows::MAX_STEPS`] steps (the rest
//!    counted in `truncated`).
//!    A flow is in the first group that fits: Cross-language (a used link crosses), entry points with no resolved
//!    links (no used link), then Endpoints (one group per router file), Programs, Commands. Groups are ordered by
//!    kind then name, flows inside by (entry path, entry title, name), steps by (kind, path, line, title, …) and
//!    links by (from, to, kind); indices refer to the final order.

pub mod build;
pub mod crates;
pub mod flows;
pub mod model;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::hub::{JobContext, JobError, SourceEvent};
use crate::packages::Packages;
use crate::tools::Tools;

pub use build::Timings;
pub use model::{Flow, FlowGroup, FlowGroupKind, Link, LinkKind, Pipeline, PipelineStats, Step, StepKind};

/// The pipeline job: builds the [`Pipeline`] off the UI thread and sends it to the hub. `files` are the project
/// files (relative to the root or absolute), the same list the tools job gets.
pub fn pipeline_job(
    packages: Arc<Packages>,
    tools: Arc<Tools>,
    files: Vec<PathBuf>,
) -> impl FnOnce(&JobContext) -> Result<(), JobError> + Send {
    move |ctx: &JobContext| {
        let cancelled = || ctx.is_cancelled();
        let Some((pipeline, _)) = build_with(&ctx.root, &packages, &tools, &files, &cancelled) else {
            return Err(JobError::Cancelled);
        };
        if ctx.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        ctx.send(SourceEvent::Pipeline(Arc::new(pipeline)));
        Ok(())
    }
}

/// Builds the pipeline of the project at `root`.
pub fn build_pipeline(root: &Path, packages: &Packages, tools: &Tools, files: &[PathBuf]) -> Pipeline {
    build_with(root, packages, tools, files, &|| false).map(|(p, _)| p).unwrap_or_default()
}

/// What a build learned besides the pipeline, for reports.
#[derive(Debug, Clone, Default)]
pub struct BuildReport {
    pub timings: Timings,
    /// Every contract tag's Unresolved reason, verbatim, with how many tags have it (also tags no step owns).
    pub unresolved_contracts: Vec<(String, usize)>,
}

/// Builds the pipeline with the time each phase took; `None` when `cancelled` turned true between phases.
pub fn build_with(
    root: &Path,
    packages: &Packages,
    tools: &Tools,
    files: &[PathBuf],
    cancelled: &dyn Fn() -> bool,
) -> Option<(Pipeline, BuildReport)> {
    let started = Instant::now();
    let graph = crates::crate_graph(packages);
    let mut found = build::build(root, &graph, tools, files, cancelled)?;
    let mut unresolved_contracts: Vec<(String, usize)> =
        std::mem::take(&mut found.unresolved_contracts).into_iter().collect();
    unresolved_contracts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let report = BuildReport { timings: found.timings, unresolved_contracts };
    let mut pipeline = flows::assemble(found);
    pipeline.stats.elapsed_ms = started.elapsed().as_millis() as u64;
    Some((pipeline, report))
}
