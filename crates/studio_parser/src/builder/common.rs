/// Path as a `/`-separated string, the form import paths and markdown links use on every OS.
pub fn slash_path(path: &std::path::Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Project-relative `/` path of a file card, from its folder cluster id (`dir:<folder>`) and title.
pub fn node_rel_path(node: &studio_graph::Node) -> Option<String> {
    let folder = node.group_id.as_deref()?.strip_prefix("dir:")?;
    Some(if folder == "." { node.title.clone() } else { format!("{folder}/{}", node.title) })
}

/// Resolves a link written in the file at `from` (project-relative) to a project-relative path.
/// Targets starting with `/` are relative to the project root. Returns `None` for links that
/// leave the project.
pub fn resolve_link(from: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = match target.strip_prefix('/') {
        Some(_) => Vec::new(),
        None => from.split('/').filter(|s| !s.is_empty()).collect(),
    };
    if target.strip_prefix('/').is_none() {
        parts.pop(); // the linking file itself
    }
    for segment in target.split(['/', '\\']) {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod slash_path_tests {
    use super::{resolve_link, slash_path};
    use std::path::PathBuf;

    #[test]
    fn links_resolve_relative_to_the_linking_file() {
        assert_eq!(resolve_link("docs/guide.md", "api.md").as_deref(), Some("docs/api.md"));
        assert_eq!(resolve_link("docs/guide.md", "../src/lib.rs").as_deref(), Some("src/lib.rs"));
        assert_eq!(resolve_link("docs/guide.md", "./img/a.png").as_deref(), Some("docs/img/a.png"));
        assert_eq!(resolve_link("docs/guide.md", "/README.md").as_deref(), Some("README.md"));
        assert_eq!(resolve_link("README.md", "crates/x/src/lib.rs").as_deref(), Some("crates/x/src/lib.rs"));
        assert_eq!(resolve_link("README.md", "../outside.md"), None);
    }

    #[test]
    fn joins_components_with_forward_slashes() {
        let p: PathBuf = ["scripts", "game", "engine.c"].iter().collect();
        assert_eq!(slash_path(&p), "scripts/game/engine.c");
    }
}
