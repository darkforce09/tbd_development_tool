use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::treesitter::CodeLang;

/// Which extractor handles a file. Decided once per file by [`detect_language`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceLang {
    Rust,
    Markdown,
    /// Bohemia Interactive Enforce Script (Arma Reforger, DayZ).
    Enforce,
    /// Parsed with a tree-sitter grammar.
    Code(CodeLang),
    /// Data, config or unknown text: shown as a file card without members.
    #[default]
    Other,
}

/// Single source of truth for picking a file's extractor.
///
/// `.c` is shared by C and Enforce Script. A `.c` file is Enforce when it lives inside an Enforce
/// project (see [`in_enforce_project`]), sits in an engine script-module folder (see
/// [`in_engine_script_module`]), or its content can only be Enforce.
pub fn detect_language(path: &Path, content: &str) -> SourceLang {
    match extension(path).as_str() {
        "rs" => SourceLang::Rust,
        "md" | "markdown" => SourceLang::Markdown,
        "ens" | "es" | "enforce" => SourceLang::Enforce,
        "c" if in_engine_script_module(path) || in_enforce_project(path) || looks_like_enforce(content) => {
            SourceLang::Enforce
        }
        ext => CodeLang::from_extension(ext).map_or(SourceLang::Other, SourceLang::Code),
    }
}

/// Path-only variant for cards created before file content is read (skeleton graph).
pub fn detect_language_by_path(path: &Path) -> SourceLang {
    match extension(path).as_str() {
        "rs" => SourceLang::Rust,
        "md" | "markdown" => SourceLang::Markdown,
        "ens" | "es" | "enforce" => SourceLang::Enforce,
        "c" if in_engine_script_module(path) || in_enforce_project(path) => SourceLang::Enforce,
        ext => CodeLang::from_extension(ext).map_or(SourceLang::Other, SourceLang::Code),
    }
}

/// Engine script-module folders, matched as whole path components: Reforger `Scripts/Game`,
/// `Scripts/GameLib`, ... and DayZ `scripts/3_Game`, `4_World`, ... A plain `scripts/` folder does not count.
pub fn in_engine_script_module(path: &Path) -> bool {
    const REFORGER_MODULES: &[&str] = &["Game", "GameLib", "GameCode", "Core", "WorkbenchGame", "Workbench"];
    const DAYZ_MODULES: &[&str] = &["1_Core", "2_GameLib", "3_Game", "4_World", "5_Mission"];
    let parts: Vec<&str> = path.iter().filter_map(|c| c.to_str()).collect();
    parts.windows(2).any(|w| {
        (w[0] == "Scripts" && REFORGER_MODULES.contains(&w[1]))
            || (w[0].eq_ignore_ascii_case("scripts") && DAYZ_MODULES.contains(&w[1]))
    })
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

/// Content that cannot be C: Enforce-only keywords, or a class declaration.
pub fn looks_like_enforce(content: &str) -> bool {
    if content.contains("modded class") || content.contains("proto native") || content.contains("proto external") {
        return true;
    }
    const ENFORCE_ONLY: &[&str] = &["array<", "autoptr ", "notnull ", "typename "];
    content.lines().any(|line| {
        let line = line.trim_start();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with("/*") {
            return false;
        }
        if line.starts_with("override ") || line.starts_with("proto ") || line.starts_with("static proto ") {
            return true;
        }
        if ENFORCE_ONLY.iter().any(|kw| line.contains(kw)) {
            return true;
        }
        let rest = line
            .strip_prefix("modded ")
            .or_else(|| line.strip_prefix("sealed "))
            .unwrap_or(line);
        rest.strip_prefix("class ")
            .and_then(|r| r.chars().next())
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    })
}

/// True when the file or any ancestor directory contains an Enforce project marker:
/// a Reforger `*.gproj`, a DayZ `$PBOPREFIX$`, or a DayZ `config.cpp` declaring `CfgMods`.
pub fn in_enforce_project(path: &Path) -> bool {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, bool>>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);

    let mut visited = Vec::new();
    let mut result = false;
    for dir in path.ancestors().skip(1) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if let Some(&known) = memo.lock().unwrap().get(dir) {
            result = known;
            break;
        }
        visited.push(dir.to_path_buf());
        if dir_has_enforce_marker(dir) {
            result = true;
            break;
        }
    }

    // Every directory between the file and the decisive ancestor shares its answer.
    let mut memo = memo.lock().unwrap();
    for dir in visited {
        memo.insert(dir, result);
    }
    result
}

fn dir_has_enforce_marker(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".gproj") || name == "$PBOPREFIX$" {
            return true;
        }
        name.eq_ignore_ascii_case("config.cpp")
            && std::fs::read_to_string(entry.path()).is_ok_and(|c| c.contains("CfgMods"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_c_in_scripts_dir_is_not_enforce() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("scripts/build_helper.c");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let src = "#include <stdio.h>\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n";
        std::fs::write(&file, src).unwrap();
        assert_eq!(detect_language(&file, src), SourceLang::Code(CodeLang::C));
        assert_eq!(detect_language_by_path(&file), SourceLang::Code(CodeLang::C));
    }

    #[test]
    fn reforger_project_marks_c_as_enforce() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("addon.gproj"), "GameProject {}").unwrap();
        let file = dir.path().join("Scripts/Game/Thing.c");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "void Helper() {}").unwrap();
        assert_eq!(detect_language(&file, "void Helper() {}"), SourceLang::Enforce);
        assert_eq!(detect_language_by_path(&file), SourceLang::Enforce);
    }

    #[test]
    fn dayz_config_marks_c_as_enforce() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.cpp"), "class CfgMods { class MyMod {}; };").unwrap();
        let file = dir.path().join("scripts/4_World/thing.c");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        assert_eq!(detect_language_by_path(&file), SourceLang::Enforce);
    }

    #[test]
    fn enforce_content_outside_project() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("loose.c");
        assert_eq!(detect_language(&file, "class Foo : Managed\n{\n}\n"), SourceLang::Enforce);
        assert_eq!(detect_language(&file, "modded class SCR_Base {}"), SourceLang::Enforce);
        assert_eq!(detect_language(&file, "/* a class of problems */\nint x;\n"), SourceLang::Code(CodeLang::C));
    }

    #[test]
    fn engine_module_folders_and_enforce_only_syntax() {
        assert!(in_engine_script_module(Path::new("/x/Scripts/Game/Thing.c")));
        assert!(in_engine_script_module(Path::new("/x/mod/scripts/4_World/a/b.c")));
        assert!(!in_engine_script_module(Path::new("/x/scripts/game/build.c")));
        assert!(!in_engine_script_module(Path::new("/x/tools/Scripts/helpers/a.c")));
        let free_fn = "void Helper(notnull IEntity e)\n{\n}\n";
        assert_eq!(detect_language(Path::new("/tmp/a.c"), free_fn), SourceLang::Enforce);
        let c_code = "// uses array<int> in a comment\nstatic int add(int a, int b) { return a + b; }\n";
        assert_eq!(detect_language(Path::new("/tmp/b.c"), c_code), SourceLang::Code(CodeLang::C));
    }

    #[test]
    fn explicit_extensions() {
        let p = Path::new("a/b");
        assert_eq!(detect_language(&p.join("x.rs"), ""), SourceLang::Rust);
        assert_eq!(detect_language(&p.join("x.MD"), ""), SourceLang::Markdown);
        assert_eq!(detect_language(&p.join("x.enforce"), ""), SourceLang::Enforce);
        assert_eq!(detect_language(&p.join("x.py"), ""), SourceLang::Code(CodeLang::Python));
        assert_eq!(detect_language(&p.join("x.json"), ""), SourceLang::Other);
    }
}
