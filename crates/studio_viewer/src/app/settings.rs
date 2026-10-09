use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Settings remembered between runs (eframe persistence). Panels are not remembered: the app
/// always opens on the bare canvas.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedSettings {
    pub last_project: Option<PathBuf>,
}
