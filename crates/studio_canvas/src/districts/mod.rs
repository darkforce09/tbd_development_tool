//! What the districts around the map show. The host builds these views from what the project's
//! sources read (packages, tools, git, ...); the canvas lays them out and paints them in the
//! district's own units, so they stay crisp at every zoom.

pub mod changes;
pub mod files;
pub mod pipeline;
pub mod run;

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::camera::Stop;

/// The views of the districts, each `None` until its source has reported.
#[derive(Debug, Clone, Default)]
pub struct DistrictViews {
    pub run: Option<Arc<run::RunView>>,
    pub files: Option<Arc<files::FilesView>>,
    pub changes: Option<Arc<changes::ChangesView>>,
    pub pipeline: Option<Arc<pipeline::PipelineView>>,
    /// Why a district without a view has nothing to show, when its source said why ("No Cargo
    /// packages here", "The pipeline could not be read: …"); else it is still reading.
    pub notes: BTreeMap<Stop, String>,
}
