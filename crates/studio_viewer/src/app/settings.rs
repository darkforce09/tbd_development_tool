use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use studio_parser::ViewGranularity;

/// Settings remembered between runs (eframe persistence).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedSettings {
    pub last_project: Option<PathBuf>,
    pub granularity: Option<String>,
    pub left_sidebar_open: Option<bool>,
    pub right_inspector_open: Option<bool>,
}

pub fn granularity_key(g: ViewGranularity) -> &'static str {
    match g {
        ViewGranularity::FilesAndFolders => "files",
        ViewGranularity::AllItems => "items",
        ViewGranularity::PublicApi => "public_api",
        ViewGranularity::Modules => "modules",
    }
}

pub fn granularity_from_key(key: &str) -> Option<ViewGranularity> {
    Some(match key {
        "files" => ViewGranularity::FilesAndFolders,
        "items" => ViewGranularity::AllItems,
        "public_api" => ViewGranularity::PublicApi,
        "modules" => ViewGranularity::Modules,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granularity_keys_round_trip() {
        for g in [ViewGranularity::FilesAndFolders, ViewGranularity::AllItems, ViewGranularity::PublicApi, ViewGranularity::Modules] {
            assert_eq!(granularity_from_key(granularity_key(g)), Some(g));
        }
        assert_eq!(granularity_from_key("bogus"), None);
    }
}
