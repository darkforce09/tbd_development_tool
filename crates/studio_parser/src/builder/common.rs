/// Path as a `/`-separated string, the form import paths and markdown links use on every OS.
pub fn slash_path(path: &std::path::Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod slash_path_tests {
    use super::slash_path;
    use std::path::PathBuf;

    #[test]
    fn joins_components_with_forward_slashes() {
        let p: PathBuf = ["scripts", "game", "engine.c"].iter().collect();
        assert_eq!(slash_path(&p), "scripts/game/engine.c");
    }
}
