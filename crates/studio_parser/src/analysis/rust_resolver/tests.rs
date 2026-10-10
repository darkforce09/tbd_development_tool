use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::analysis::crate_graph::{CrateNode, TargetKind};

/// An index over in-memory files: one lib crate `app` at `/w/app/src/lib.rs` plus `dep` at `/w/dep/src/lib.rs`.
fn index(files: &[(&str, &str)]) -> RustIndex {
    let map: BTreeMap<PathBuf, String> = files.iter().map(|(p, t)| (PathBuf::from(p), t.to_string())).collect();
    let node = |name: &str, deps: Vec<(String, usize)>| CrateNode {
        package: name.to_string(),
        name: name.to_string(),
        kind: TargetKind::Lib,
        root_file: PathBuf::from(format!("/w/{name}/src/lib.rs")),
        manifest: PathBuf::from(format!("/w/{name}/Cargo.toml")),
        deps,
        external_deps: vec![("ext".to_string(), "ext".to_string())],
        edition: 2021,
        gated_deps: Vec::new(),
    };
    let graph = CrateGraph { crates: vec![node("app", vec![("dep".to_string(), 1)]), node("dep", Vec::new())] };
    let read = |p: &Path| map.get(p).cloned();
    RustIndex::build(&graph, &read)
}

fn resolve(idx: &RustIndex, file: &str, path: &str) -> String {
    let scope = idx.module_of_file(Path::new(file)).expect("module");
    let segs: Vec<&str> = path.split("::").collect();
    match idx.resolve(scope, &segs) {
        Resolution::Item(i) => format!("{}:{}", i.file.display(), i.line),
        other => format!("{other:?}"),
    }
}

#[test]
fn globs_respect_visibility_and_cycles_end() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod a { fn hidden() {} pub fn shown() {} }\nmod b { use super::a::*; use super::c::*; }\n\
             mod c { use super::b::*; }\nfn parent_private() {}\nmod child { use super::*; }\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "b::shown"), "/w/app/src/lib.rs:1");
    // A private item of a sibling is not imported by its glob.
    assert_eq!(resolve(&idx, lib, "b::hidden"), "Unresolved(NotFound)");
    // `b` and `c` import each other's globs: the cycle provides nothing new and ends.
    assert_eq!(resolve(&idx, lib, "c::nothing"), "Unresolved(NotFound)");
    assert_eq!(resolve(&idx, lib, "c::shown"), "Unresolved(NotFound)");
    // A child's `use super::*` sees the parent's private items.
    assert_eq!(resolve(&idx, lib, "child::parent_private"), "/w/app/src/lib.rs:4");
}

#[test]
fn cfg_modules_test_files_and_twin_module_files() {
    let idx = index(&[
        ("/w/app/src/lib.rs", "#[cfg(unix)]\nmod gated;\nmod tested;\nmod twin;\npub use dep::f as g;\n"),
        ("/w/app/src/gated.rs", "pub fn f() {}"),
        ("/w/app/src/tested.rs", "#![cfg(test)]\npub fn t() {}"),
        ("/w/app/src/twin.rs", "pub fn x() {}"),
        ("/w/app/src/twin/mod.rs", "pub fn x() {}"),
        ("/w/dep/src/lib.rs", "pub fn f() {}"),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "gated::f"), "Unresolved(Cfg)");
    assert_eq!(resolve(&idx, lib, "tested::t"), "Unresolved(NotFound)");
    assert_eq!(resolve(&idx, lib, "twin::x"), "Unresolved(Other(\"both twin.rs and twin/mod.rs exist\"))");
    assert_eq!(resolve(&idx, lib, "g"), "/w/dep/src/lib.rs:1");
    assert_eq!(idx.module_of_file(Path::new("/w/app/src/tested.rs")), None);
}

#[test]
fn item_macros_and_external_globs_stay_honest() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod m { thread_local! { static X: u8 = 0; } pub fn real() {} }\nmod e { use ext::prelude::*; }\n\
             mod two { use ext::a::*; use ext::b::*; }\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "m::real"), "/w/app/src/lib.rs:1");
    // A macro invocation at item level may define any name.
    assert_eq!(resolve(&idx, lib, "m::X"), "Unresolved(Macro)");
    assert_eq!(resolve(&idx, lib, "e::Thing"), "External(\"ext::prelude::Thing\")");
    assert_eq!(resolve(&idx, lib, "two::Thing"), "Unresolved(Glob)");
}

#[test]
fn call_sites_skip_tests_and_keep_locals_unresolved() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub fn a() {}\npub fn run<T>(a: u8) {\n    #[cfg(test)]\n    b();\n    a();\n    T::new();\n    \
             println!(\"{}\", a());\n    crate::a();\n}\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let calls: Vec<String> = idx
        .resolved_calls(Path::new("/w/app/src/lib.rs"))
        .into_iter()
        .map(|(site, res)| format!("{} {} {:?}", site.line, site.path.join("::"), res))
        .collect();
    assert_eq!(calls.len(), 3, "{calls:#?}");
    assert!(calls[0].starts_with("5 a Unresolved(Other(\"local binding or block item\"))"), "{calls:#?}");
    assert!(calls[1].starts_with("6 T::new Unresolved(Other("), "{calls:#?}");
    assert!(calls[2].starts_with("8 crate::a Item("), "{calls:#?}");
}

fn calls(idx: &RustIndex, file: &str) -> Vec<String> {
    idx.resolved_calls(Path::new(file))
        .into_iter()
        .map(|(site, res)| match res {
            Resolution::Item(i) => format!("{} {} -> {}:{}", site.line, site.path.join("::"), i.file.display(), i.line),
            other => format!("{} {} -> {other:?}", site.line, site.path.join("::")),
        })
        .collect()
}

#[test]
fn calls_inside_a_module_declared_in_a_body_are_not_read_from_the_outer_module() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub fn h() {}\npub fn run() {\n    mod inner {\n        pub fn g() {\n            self::h();\n            \
             crate::h();\n            Vec::new();\n        }\n        fn h() {}\n    }\n    inner::g();\n}\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let local = "Unresolved(Other(\"local binding or block item\"))";
    assert_eq!(
        calls(&idx, "/w/app/src/lib.rs"),
        vec![
            format!("5 self::h -> {local}"),
            "6 crate::h -> /w/app/src/lib.rs:1".to_string(),
            format!("7 Vec::new -> {local}"),
            format!("11 inner::g -> {local}"),
        ]
    );
}

#[test]
fn a_glob_whose_base_is_unknown_makes_its_names_uncertain() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod a { pub fn f() {} }\nmod user { use super::a::*; use super::missing::*; }\n\
             mod fine { use super::a::*; use ext::*; }\nmod x_home { pub mod x { pub fn g() {} } }\n\
             mod via { use super::x_home::*; use x::*; }\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "user::f"), "Unresolved(Glob)");
    // An external glob next to a workspace one: the workspace item (two different providers do not compile).
    assert_eq!(resolve(&idx, lib, "fine::f"), "/w/app/src/lib.rs:1");
    // While `x` itself is looked up, the glob `x::*` cannot provide it (a cycle, not an unknown base): `x` comes
    // from the other glob, and `x::*` then provides `g`.
    assert_eq!(resolve(&idx, lib, "via::g"), "/w/app/src/lib.rs:4");
}

#[test]
fn extern_block_items_and_body_macros_shadow_outer_names() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod a { pub fn ffi() {} }\nmod b {\n    use super::a::*;\n    extern \"C\" {\n        pub fn ffi();\n    }\n}\n\
             pub fn helper() {}\npub fn run() {\n    make_fn!(pub helper);\n    helper();\n}\n\
             pub fn plain() {\n    println!(\"x\");\n    log::info!(\"helper {}\", self::helper::X, x.helper);\n    helper();\n}\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "b::ffi"), "/w/app/src/lib.rs:5");
    assert_eq!(
        calls(&idx, lib),
        vec!["11 helper -> Unresolved(Macro)".to_string(), "16 helper -> /w/app/src/lib.rs:8".to_string()]
    );
}

#[test]
fn gated_variants_and_trait_items_are_cfg() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub enum E {\n    #[cfg(unix)]\n    A,\n    B,\n}\npub trait T {\n    #[cfg(unix)]\n    fn m();\n    fn n();\n}\n\
             mod g { pub use super::E::*; }\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "E::A"), "Unresolved(Cfg)");
    assert_eq!(resolve(&idx, lib, "E::B"), "/w/app/src/lib.rs:1");
    assert_eq!(resolve(&idx, lib, "g::A"), "Unresolved(Cfg)");
    assert_eq!(resolve(&idx, lib, "g::B"), "/w/app/src/lib.rs:1");
    assert_eq!(resolve(&idx, lib, "T::m"), "Unresolved(Cfg)");
    assert_eq!(resolve(&idx, lib, "T::n"), "/w/app/src/lib.rs:6");
}

#[test]
fn self_in_a_blanket_impl_is_not_a_path() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub struct T;\nimpl T {\n    pub fn new() -> T {\n        T\n    }\n}\npub trait Make {\n    fn make() -> Self;\n}\n\
             impl<T: Default> Make for T {\n    fn make() -> Self {\n        Self::new()\n    }\n}\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    assert_eq!(calls(&idx, "/w/app/src/lib.rs"), vec!["12 Self::new -> Unresolved(Method)".to_string()]);
}

#[test]
fn a_file_that_does_not_parse_is_unknown_not_empty() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod broken;\nmod good { pub fn f() {} }\nmod user { use super::broken::*; use super::good::*; }\n",
        ),
        ("/w/app/src/broken.rs", "fn ( {"),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert!(
        resolve(&idx, lib, "user::f").starts_with("Unresolved(Other(\"Syntax error"),
        "{}",
        resolve(&idx, lib, "user::f")
    );
    assert!(resolve(&idx, lib, "broken::anything").starts_with("Unresolved(Other(\"Syntax error"));
}

#[test]
fn item_macros_do_not_fall_back_to_a_workspace_crate() {
    let idx = index(&[
        ("/w/app/src/lib.rs", "mod m;\n"),
        (
            "/w/app/src/m.rs",
            "thread_local! { static X: u8 = 0; }\nuse dep::f as g;\npub fn y() {\n    dep::f();\n    std::mem::drop(1);\n    \
             g();\n}\n",
        ),
        ("/w/dep/src/lib.rs", "pub fn f() {}"),
    ]);
    assert_eq!(
        calls(&idx, "/w/app/src/m.rs"),
        vec![
            "4 dep::f -> Unresolved(Macro)".to_string(),
            "5 std::mem::drop -> External(\"std::mem::drop\")".to_string(),
            // In a `use` path the crate is exact: a macro-made `dep` would be ambiguous there (E0659).
            "6 g -> /w/dep/src/lib.rs:1".to_string(),
        ]
    );
}

#[test]
fn restricted_visibility_through_a_glob_is_uncertain() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub mod a {\n    pub mod inner {\n        pub(in crate::a) fn r() {}\n        pub fn p() {}\n    }\n}\n\
             mod b { use super::a::inner::*; }\n",
        ),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let lib = "/w/app/src/lib.rs";
    assert_eq!(resolve(&idx, lib, "b::r"), "Unresolved(Glob)");
    assert_eq!(resolve(&idx, lib, "b::p"), "/w/app/src/lib.rs:4");
}

#[test]
fn root_extern_crate_aliases_join_the_extern_prelude_and_bad_module_ids_do_not_panic() {
    let idx = index(&[
        ("/w/app/src/lib.rs", "extern crate dep as d;\nmod m { pub fn y() { d::f(); } }\n"),
        ("/w/dep/src/lib.rs", "pub fn f() {}"),
    ]);
    assert_eq!(calls(&idx, "/w/app/src/lib.rs"), vec!["2 d::f -> /w/dep/src/lib.rs:1".to_string()]);
    assert_eq!(
        idx.resolve(ModuleId(9_999), &["x"]),
        Resolution::Unresolved(UnresolvedReason::Other("unknown module".into()))
    );
}

#[test]
fn diamond_glob_graphs_resolve_quickly() {
    // Level i has modules a_i and b_i, each globbing both modules of level i - 1: 2^depth routes to the bottom.
    let depth = 24;
    let mut text = String::from("pub mod a_0 { pub fn bottom() {} }\npub mod b_0 {}\n");
    for i in 1..=depth {
        for side in ["a", "b"] {
            text.push_str(&format!(
                "pub mod {side}_{i} {{ pub use super::a_{0}::*; pub use super::b_{0}::*; }}\n",
                i - 1
            ));
        }
    }
    let idx = index(&[("/w/app/src/lib.rs", text.as_str()), ("/w/dep/src/lib.rs", "")]);
    let lib = "/w/app/src/lib.rs";
    let started = std::time::Instant::now();
    assert_eq!(resolve(&idx, lib, &format!("a_{depth}::bottom")), "/w/app/src/lib.rs:1");
    assert_eq!(resolve(&idx, lib, &format!("a_{depth}::missing")), "Unresolved(NotFound)");
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

#[test]
fn call_sites_carry_their_cfg_gate() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "pub fn a() {}\npub fn run() {\n    a();\n    #[cfg(unix)]\n    a();\n}\n#[cfg(unix)]\npub fn gated() {\n    a();\n}\n\
             pub struct S;\n#[cfg(unix)]\nimpl S {\n    pub fn m() {\n        a();\n    }\n}\n#[cfg(unix)]\nmod g {\n    \
             pub fn inner() {\n        crate::a();\n    }\n}\nmod sub;\n",
        ),
        ("/w/app/src/sub.rs", "#![cfg(windows)]\npub fn f() {\n    crate::a();\n}\n"),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let gates = |file: &str| -> Vec<(usize, bool)> {
        idx.resolved_calls(Path::new(file)).iter().map(|(site, _)| (site.line, site.cfg_gated)).collect()
    };
    assert_eq!(gates("/w/app/src/lib.rs"), vec![(3, false), (5, true), (9, true), (15, true), (21, true)]);
    assert_eq!(gates("/w/app/src/sub.rs"), vec![(3, true)]);
}

#[test]
fn use_leaves_resolve_in_both_name_spaces() {
    let idx = index(&[
        (
            "/w/app/src/lib.rs",
            "mod both;\nmod same {\n    pub mod x {}\n    pub fn x() {}\n}\npub struct Named {\n    pub a: u8,\n}\n\
             pub use both::y;\npub use same::x as renamed;\nuse self::Named;\nuse crate::same::*;\n#[cfg(unix)]\nuse dep::f;\n\
             use ::dep::{self as d};\n#[cfg(test)]\nuse dep::t;\n",
        ),
        ("/w/app/src/both.rs", "pub mod y;\npub fn y() {}\n"),
        ("/w/app/src/both/y.rs", ""),
        ("/w/dep/src/lib.rs", "pub fn f() {}"),
    ]);
    let got: Vec<String> = idx
        .resolved_uses(Path::new("/w/app/src/lib.rs"))
        .into_iter()
        .map(|(leaf, res)| {
            let res = match res {
                Resolution::Item(i) => format!("{}:{}", i.file.display(), i.line),
                Resolution::Ambiguous(list) => {
                    list.iter().map(|i| format!("{}:{}", i.file.display(), i.line)).collect::<Vec<_>>().join(" | ")
                }
                other => format!("{other:?}"),
            };
            format!(
                "{} {} {:?} glob={} gated={} -> {res}",
                leaf.line,
                leaf.path.join("::"),
                leaf.alias,
                leaf.is_glob,
                leaf.cfg_gated
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            // `mod y;` and `fn y` are both in both.rs: the module's line.
            "9 both::y None glob=false gated=false -> /w/app/src/both.rs:1".to_string(),
            // An inline `mod x {}` and `fn x` in lib.rs: same file, the module.
            "10 same::x Some(\"renamed\") glob=false gated=false -> /w/app/src/lib.rs:3".to_string(),
            "11 self::Named None glob=false gated=false -> /w/app/src/lib.rs:6".to_string(),
            "12 crate::same None glob=true gated=false -> /w/app/src/lib.rs:2".to_string(),
            // The leaf is flagged; its path still resolves exactly (as a call in gated code does).
            "14 dep::f None glob=false gated=true -> /w/dep/src/lib.rs:1".to_string(),
            "15 dep Some(\"d\") glob=false gated=false -> /w/dep/src/lib.rs:1".to_string(),
        ]
    );
}

#[test]
fn use_leaves_naming_items_in_two_files_are_ambiguous_and_module_files_are_their_own() {
    let idx = index(&[
        ("/w/app/src/lib.rs", "mod a;\nmod b {\n    pub use super::a::*;\n    pub fn k() {}\n}\npub use b::k;\n"),
        ("/w/app/src/a.rs", "#[allow(non_camel_case_types)]\npub struct k {}\n"),
        ("/w/dep/src/lib.rs", ""),
    ]);
    let uses = idx.resolved_uses(Path::new("/w/app/src/lib.rs"));
    let Resolution::Ambiguous(list) = &uses[0].1 else { panic!("{uses:?}") };
    let files: Vec<String> = list.iter().map(|i| format!("{}:{}", i.file.display(), i.line)).collect();
    assert_eq!(files, vec!["/w/app/src/a.rs:2".to_string(), "/w/app/src/lib.rs:4".to_string()]);

    let root = idx.module_of_file(Path::new("/w/app/src/lib.rs")).unwrap();
    let a = idx.module_of_file(Path::new("/w/app/src/a.rs")).unwrap();
    assert_eq!(idx.module_file(root), Some(PathBuf::from("/w/app/src/lib.rs")));
    assert_eq!(idx.module_file(a), Some(PathBuf::from("/w/app/src/a.rs")));
    // The inline `mod b`: the file that contains it.
    let Resolution::Item(b) = idx.resolve(root, &["b"]) else { panic!() };
    let b_module = idx.modules.iter().position(|m| m.decl_item.is_some_and(|d| idx.items[d].r == b)).unwrap();
    assert_eq!(idx.module_file(ModuleId(b_module as u32)), Some(PathBuf::from("/w/app/src/lib.rs")));
    assert_eq!(idx.module_file(ModuleId(9_999)), None);
}
