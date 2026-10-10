//! What the districts around the map show. The host builds these views from what the project's
//! sources read (packages, tools, git, ...); the canvas lays them out and paints them in the
//! district's own units, so they stay crisp at every zoom.

pub mod files;
pub mod run;

use std::sync::Arc;

/// The views of the districts, each `None` until its source has reported.
#[derive(Debug, Clone, Default)]
pub struct DistrictViews {
    pub run: Option<Arc<run::RunView>>,
    pub files: Option<Arc<files::FilesView>>,
}
