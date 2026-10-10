//! Deterministic file classification: one table of rules ([`RULES`]), tried in order, the first
//! match decides. `docs/CLASSIFICATION.md` mirrors the table row for row (a test keeps them equal).
//!
//! Nothing is guessed from a bare folder name. Folder rules are tool conventions anchored to the
//! tool's own manifest beside the folder (Cargo's `tests/` next to a `Cargo.toml`) or to the
//! project root where the tool itself fixes the path (`.github/workflows/`). Heavy folders are
//! found by markers the tools write into them ([`HEAVY_MARKERS`]), never by name.

use std::collections::BTreeSet;
use std::path::Path;

use crate::tree::HeavyReason;

/// What a file is for, from the roadmap's region list (plus `vendored` for Linguist's override
/// and `other` so the table is total).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileClass {
    Code,
    Tests,
    Tooling,
    Docs,
    Schemas,
    Assets,
    Config,
    BuildOutput,
    Ci,
    Tickets,
    Vendored,
    Other,
}

impl FileClass {
    pub const ALL: [FileClass; 12] = [
        FileClass::Code,
        FileClass::Tests,
        FileClass::Tooling,
        FileClass::Docs,
        FileClass::Schemas,
        FileClass::Assets,
        FileClass::Config,
        FileClass::BuildOutput,
        FileClass::Ci,
        FileClass::Tickets,
        FileClass::Vendored,
        FileClass::Other,
    ];

    /// Short lowercase name, as the doc's table and the map cards write it.
    pub fn label(self) -> &'static str {
        match self {
            FileClass::Code => "code",
            FileClass::Tests => "tests",
            FileClass::Tooling => "tooling",
            FileClass::Docs => "docs",
            FileClass::Schemas => "schemas",
            FileClass::Assets => "assets",
            FileClass::Config => "config",
            FileClass::BuildOutput => "build output",
            FileClass::Ci => "CI",
            FileClass::Tickets => "tickets",
            FileClass::Vendored => "vendored",
            FileClass::Other => "other",
        }
    }
}

/// How a rule matches, in the order the table tries them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    /// A Linguist attribute git reports for the path (an input; the parser never asks git).
    GitAttribute,
    /// Git ignores the file or a folder above it.
    GitIgnored,
    /// The file name matches one of the space-separated name globs (`*` within a name).
    FileName,
    /// A tool's fixed layout: `/path` from the project root, or `{manifest}/path` from a folder
    /// that holds that manifest. `**` stands for one or more folders and the file.
    Convention,
    /// The file name ends with one of the extensions (written as name globs).
    Extension,
    /// Every file: the table is total.
    Fallback,
}

impl RuleKind {
    pub fn label(self) -> &'static str {
        match self {
            RuleKind::GitAttribute => "gitattributes",
            RuleKind::GitIgnored => "gitignored",
            RuleKind::FileName => "file name",
            RuleKind::Convention => "tool convention",
            RuleKind::Extension => "extension",
            RuleKind::Fallback => "fallback",
        }
    }
}

/// One row of the classification table.
#[derive(Debug, PartialEq, Eq)]
pub struct Rule {
    pub kind: RuleKind,
    pub pattern: &'static str,
    pub class: FileClass,
    /// Who defines the rule (a tool, a format, git), named in the doc with its source.
    pub basis: &'static str,
    pub note: &'static str,
}

impl Rule {
    /// Why a file has its class, for display: "tool convention: Cargo", "extension", ….
    pub fn why(&self) -> String {
        match self.kind {
            RuleKind::Convention => format!("tool convention: {}", self.basis),
            RuleKind::GitAttribute => format!("gitattributes: {}", self.pattern),
            kind => kind.label().to_string(),
        }
    }

    fn matches(&self, rel: &str, parts: &[&str], inputs: &ClassInputs) -> bool {
        let name = parts.last().copied().unwrap_or(rel);
        match self.kind {
            RuleKind::GitAttribute => inputs.attribute(self.pattern).contains(rel),
            RuleKind::GitIgnored => inputs.is_ignored(rel),
            RuleKind::FileName | RuleKind::Extension => self.pattern.split(' ').any(|p| name_glob(p, name)),
            RuleKind::Convention => self.pattern.split(' ').any(|p| convention(p, parts, inputs)),
            RuleKind::Fallback => true,
        }
    }
}

/// What classification needs besides the path: every file's path (for manifest lookups), what
/// git ignores, and the Linguist attribute sets. All paths are project-relative with `/`;
/// ignored folders end with `/`.
#[derive(Debug, Clone, Default)]
pub struct ClassInputs {
    pub files: BTreeSet<String>,
    pub ignored: BTreeSet<String>,
    pub generated: BTreeSet<String>,
    pub vendored: BTreeSet<String>,
    pub documentation: BTreeSet<String>,
}

impl ClassInputs {
    fn attribute(&self, name: &str) -> &BTreeSet<String> {
        match name {
            "linguist-generated" => &self.generated,
            "linguist-vendored" => &self.vendored,
            _ => &self.documentation,
        }
    }

    /// The file is ignored, or a folder above it is.
    pub fn is_ignored(&self, rel: &str) -> bool {
        self.ignored.contains(rel) || rel.match_indices('/').any(|(i, _)| self.ignored.contains(&rel[..=i]))
    }
}

/// The class of the file at `rel` (project-relative, `/`-separated) and the rule that decided it.
pub fn classify(rel: &str, inputs: &ClassInputs) -> (FileClass, &'static Rule) {
    let parts: Vec<&str> = rel.split('/').collect();
    let rule = RULES.iter().find(|r| r.matches(rel, &parts, inputs)).unwrap_or(&RULES[RULES.len() - 1]);
    (rule.class, rule)
}

/// `*` matches any run of characters within one name; ASCII case is ignored.
fn name_glob(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && p[pi].eq_ignore_ascii_case(&n[ni]) {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// Folder-by-folder glob: `**` is one or more names when last, zero or more elsewhere.
fn path_glob(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", [])) => !path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|i| path_glob(rest, &path[i..])),
        Some((p, rest)) => path.split_first().is_some_and(|(s, tail)| name_glob(p, s) && path_glob(rest, tail)),
    }
}

/// `/a/b` from the root, or `{manifest}/a/b` from any folder holding `manifest`.
/// `parts` is the file's path split at `/`.
fn convention(pattern: &str, parts: &[&str], inputs: &ClassInputs) -> bool {
    if let Some(rooted) = pattern.strip_prefix('/') {
        // Cheap reject before splitting the pattern: the first names must match.
        let first = rooted.split('/').next().unwrap_or("");
        if !name_glob(first, parts[0]) {
            return false;
        }
        let pattern: Vec<&str> = rooted.split('/').collect();
        return path_glob(&pattern, parts);
    }
    let Some((manifest, rest)) = pattern.strip_prefix('{').and_then(|p| p.split_once("}/")) else { return false };
    let first = rest.split('/').next().unwrap_or("");
    if !parts.iter().any(|s| name_glob(first, s)) {
        return false;
    }
    let pattern: Vec<&str> = rest.split('/').collect();
    (0..parts.len()).any(|k| {
        path_glob(&pattern, &parts[k..]) && {
            let beside = if k == 0 { manifest.to_string() } else { format!("{}/{manifest}", parts[..k].join("/")) };
            inputs.files.contains(&beside)
        }
    })
}

/// A file a tool writes into a folder it manages; the folder is heavy (listed, not descended).
#[derive(Debug, PartialEq, Eq)]
pub struct HeavyMarker {
    pub file: &'static str,
    pub reason: HeavyReason,
    pub tool: &'static str,
    pub source: &'static str,
}

/// The first line a `CACHEDIR.TAG` must start with (Cache Directory Tagging Specification).
pub const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// Markers checked in every folder the scan meets, in this order.
pub static HEAVY_MARKERS: &[HeavyMarker] = &[
    HeavyMarker {
        file: "CACHEDIR.TAG",
        reason: HeavyReason::BuildCache,
        tool: "Cache Directory Tagging (Cargo, pytest, mypy, ruff, …)",
        source: "https://bford.info/cachedir/",
    },
    HeavyMarker {
        file: "pyvenv.cfg",
        reason: HeavyReason::Dependencies,
        tool: "Python venv (PEP 405)",
        source: "https://peps.python.org/pep-0405/",
    },
    HeavyMarker {
        file: ".package-lock.json",
        reason: HeavyReason::Dependencies,
        tool: "npm 7+ hidden lockfile",
        source: "https://docs.npmjs.com/cli/configuring-npm/package-lock-json#hidden-lockfiles",
    },
    HeavyMarker {
        file: ".modules.yaml",
        reason: HeavyReason::Dependencies,
        tool: "pnpm",
        source: "https://github.com/pnpm/pnpm/tree/main/pkg-manager/modules-yaml",
    },
    HeavyMarker {
        file: ".yarn-integrity",
        reason: HeavyReason::Dependencies,
        tool: "Yarn 1 (classic)",
        source: "https://classic.yarnpkg.com/en/docs/cli/check",
    },
    HeavyMarker {
        file: ".yarn-state.yml",
        reason: HeavyReason::Dependencies,
        tool: "Yarn 2+ node-modules linker",
        source: "https://github.com/yarnpkg/berry/tree/master/packages/plugin-nm",
    },
];

/// The marker that makes the folder at `dir` heavy, if any: one stat per marker, and for
/// `CACHEDIR.TAG` a read of its first 43 bytes to check the signature.
pub fn heavy_marker(dir: &Path) -> Option<&'static HeavyMarker> {
    HEAVY_MARKERS.iter().find(|m| {
        let path = dir.join(m.file);
        path.is_file() && (m.file != "CACHEDIR.TAG" || has_cachedir_signature(&path))
    })
}

fn has_cachedir_signature(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; CACHEDIR_SIGNATURE.len()];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == CACHEDIR_SIGNATURE
}

/// The classification table, in precedence order: the first rule that matches decides.
pub static RULES: &[Rule] = &[
    Rule {
        kind: RuleKind::GitAttribute,
        pattern: "linguist-generated",
        class: FileClass::BuildOutput,
        basis: "Linguist",
        note: "marked generated in .gitattributes",
    },
    Rule {
        kind: RuleKind::GitAttribute,
        pattern: "linguist-vendored",
        class: FileClass::Vendored,
        basis: "Linguist",
        note: "marked vendored in .gitattributes",
    },
    Rule {
        kind: RuleKind::GitAttribute,
        pattern: "linguist-documentation",
        class: FileClass::Docs,
        basis: "Linguist",
        note: "marked documentation in .gitattributes",
    },
    Rule {
        kind: RuleKind::GitIgnored,
        pattern: ".gitignore",
        class: FileClass::BuildOutput,
        basis: "git",
        note: "git ignores it (any ignore source): not source",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "README README.* CHANGELOG CHANGELOG.* LICENSE LICENSE.* LICENCE LICENCE.* COPYING COPYING.* CONTRIBUTING CONTRIBUTING.* NOTICE NOTICE.* AUTHORS",
        class: FileClass::Docs,
        basis: "well-known names",
        note: "project documents",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "*_test.go",
        class: FileClass::Tests,
        basis: "Go",
        note: "the go command builds these only for go test",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "test_*.py *_test.py",
        class: FileClass::Tests,
        basis: "pytest",
        note: "pytest's default test file names",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "*.test.js *.test.jsx *.test.ts *.test.tsx *.test.mjs *.test.cjs *.spec.js *.spec.jsx *.spec.ts *.spec.tsx *.spec.mjs *.spec.cjs",
        class: FileClass::Tests,
        basis: "Jest, Vitest",
        note: "the runners' default test file names",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "*.schema.json openapi.json openapi.yaml openapi.yml swagger.json swagger.yaml swagger.yml",
        class: FileClass::Schemas,
        basis: "JSON Schema, OpenAPI",
        note: "schema and API contract files",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "Makefile GNUmakefile justfile .justfile Dockerfile Dockerfile.* Containerfile Taskfile.yml Taskfile.yaml",
        class: FileClass::Tooling,
        basis: "make, just, Docker, Task",
        note: "build and task runner files",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "Cargo.lock package-lock.json npm-shrinkwrap.json yarn.lock pnpm-lock.yaml poetry.lock uv.lock Pipfile.lock go.sum Gemfile.lock composer.lock flake.lock",
        class: FileClass::Config,
        basis: "package managers",
        note: "lockfiles",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: "Cargo.toml package.json pyproject.toml setup.cfg go.mod go.work Gemfile Pipfile requirements.txt requirements-*.txt CMakeLists.txt tsconfig.json tsconfig.*.json rust-toolchain rust-toolchain.toml",
        class: FileClass::Config,
        basis: "package managers",
        note: "manifests and toolchain files",
    },
    Rule {
        kind: RuleKind::FileName,
        pattern: ".gitignore .gitattributes .gitmodules .editorconfig .env .env.* .npmrc .nvmrc .prettierrc .prettierrc.* .eslintrc .eslintrc.* .clang-format rustfmt.toml .rustfmt.toml clippy.toml .clippy.toml",
        class: FileClass::Config,
        basis: "well-known names",
        note: "tool settings",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/.github/workflows/*.yml /.github/workflows/*.yaml",
        class: FileClass::Ci,
        basis: "GitHub Actions",
        note: "workflows live in .github/workflows at the repository root",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/.gitlab-ci.yml",
        class: FileClass::Ci,
        basis: "GitLab CI/CD",
        note: "the default pipeline file at the repository root",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/.circleci/config.yml",
        class: FileClass::Ci,
        basis: "CircleCI",
        note: "the pipeline file at the repository root",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/Jenkinsfile",
        class: FileClass::Ci,
        basis: "Jenkins",
        note: "the default Pipeline script path",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/azure-pipelines.yml",
        class: FileClass::Ci,
        basis: "Azure Pipelines",
        note: "the default pipeline file at the repository root",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "/.ai/tickets/T-*.toml",
        class: FileClass::Tickets,
        basis: "Studio",
        note: "the only folder Studio reads tickets from",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "{Cargo.toml}/build.rs",
        class: FileClass::Tooling,
        basis: "Cargo",
        note: "the package's build script",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "{Cargo.toml}/tests/**",
        class: FileClass::Tests,
        basis: "Cargo",
        note: "integration tests and their data",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "{Cargo.toml}/benches/**",
        class: FileClass::Tests,
        basis: "Cargo",
        note: "benchmarks",
    },
    Rule {
        kind: RuleKind::Convention,
        pattern: "{Cargo.toml}/examples/**",
        class: FileClass::Docs,
        basis: "Cargo",
        note: "example programs that show the package's use",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.rs *.py *.pyi *.js *.mjs *.cjs *.ts *.mts *.cts *.tsx *.jsx *.go *.c *.h *.cc *.cpp *.cxx *.hpp *.hh *.hxx *.cs *.java *.kt *.kts *.swift *.rb *.php *.lua *.dart *.scala *.zig *.vue *.svelte *.html *.htm *.css *.scss *.sass *.less *.sql *.wgsl *.glsl *.hlsl *.vert *.frag *.comp *.shader",
        class: FileClass::Code,
        basis: "languages",
        note: "source, markup, styles and shaders",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.sh *.bash *.zsh *.fish *.ps1 *.psm1 *.bat *.cmd *.mk *.cmake",
        class: FileClass::Tooling,
        basis: "shells, make, CMake",
        note: "scripts",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.md *.markdown *.mdx *.rst *.adoc *.txt *.org *.tex *.pdf",
        class: FileClass::Docs,
        basis: "document formats",
        note: "prose",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.proto *.graphql *.gql *.xsd *.avsc *.thrift *.capnp",
        class: FileClass::Schemas,
        basis: "schema languages",
        note: "contracts between programs",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.toml *.yaml *.yml *.json *.jsonc *.json5 *.ini *.cfg *.conf *.properties *.xml *.plist *.env *.csv *.tsv *.jsonl *.ndjson",
        class: FileClass::Config,
        basis: "data formats",
        note: "settings and data",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.png *.jpg *.jpeg *.gif *.webp *.bmp *.ico *.svg *.tga *.dds *.psd *.exr *.hdr *.tif *.tiff *.ttf *.otf *.woff *.woff2 *.wav *.ogg *.mp3 *.flac *.mp4 *.webm *.mov *.fbx *.gltf *.glb *.blend *.bvh",
        class: FileClass::Assets,
        basis: "media formats",
        note: "images, fonts, audio, video, models",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.et *.ent *.layer *.layout *.emat *.edds *.xob *.meta",
        class: FileClass::Assets,
        basis: "Enfusion",
        note: "engine resources: prefabs, worlds, UI layouts, materials, textures, models, resource metadata",
    },
    Rule {
        kind: RuleKind::Extension,
        pattern: "*.log *.o *.a *.lib *.so *.dylib *.dll *.exe *.pdb *.pyc *.pyo *.class *.rlib *.rmeta",
        class: FileClass::BuildOutput,
        basis: "compilers, loggers",
        note: "objects, binaries, bytecode, logs",
    },
    Rule {
        kind: RuleKind::Fallback,
        pattern: "*",
        class: FileClass::Other,
        basis: "none",
        note: "no rule above matched",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(files: &[&str]) -> ClassInputs {
        ClassInputs { files: files.iter().map(|f| f.to_string()).collect(), ..Default::default() }
    }

    fn class(rel: &str, inputs: &ClassInputs) -> FileClass {
        classify(rel, inputs).0
    }

    #[test]
    fn globs_match_within_one_name_ignoring_ascii_case() {
        assert!(name_glob("*.rs", "main.rs"));
        assert!(name_glob("*.rs", "MAIN.RS"));
        assert!(!name_glob("*.rs", "main.rsx"));
        assert!(name_glob("README.*", "readme.md"));
        assert!(!name_glob("README.*", "README"));
        assert!(name_glob("*_test.go", "a_b_test.go"));
        assert!(name_glob("test_*.py", "test_.py"));
        assert!(name_glob("*", ""));
        assert!(path_glob(&["tests", "**"], &["tests", "a", "b.rs"]));
        assert!(!path_glob(&["tests", "**"], &["tests"]), "a file named tests is not inside tests/");
    }

    #[test]
    fn a_bare_folder_name_never_decides_a_class() {
        let none = inputs(&["tests/it.rs", "benches/b.rs", "examples/e.rs", "build.rs", "pkg/tests/x.rs"]);
        for rel in ["tests/it.rs", "benches/b.rs", "examples/e.rs", "build.rs", "pkg/tests/x.rs"] {
            assert_eq!(classify(rel, &none).1.kind, RuleKind::Extension, "{rel}");
        }
        // A manifest elsewhere does not anchor the folder: it must sit beside it.
        let elsewhere = inputs(&["Cargo.toml", "pkg/tests/x.rs", "pkg/src/lib.rs"]);
        assert_eq!(class("pkg/tests/x.rs", &elsewhere), FileClass::Code);
        let beside = inputs(&["pkg/Cargo.toml", "pkg/tests/x.rs"]);
        assert_eq!(class("pkg/tests/x.rs", &beside), FileClass::Tests);
        assert_eq!(classify("pkg/tests/x.rs", &beside).1.why(), "tool convention: Cargo");
        // Root-anchored conventions need the exact root path.
        assert_eq!(class("sub/.github/workflows/ci.yml", &none), FileClass::Config);
        assert_eq!(class(".github/workflows/ci.yml", &none), FileClass::Ci);
    }

    #[test]
    fn earlier_rules_win() {
        let mut i = inputs(&["Cargo.toml", "tests/README.md", "tests/a.rs", "gen/out.rs"]);
        i.generated.insert("gen/out.rs".into());
        i.ignored.insert("tests/".into());
        assert_eq!(class("gen/out.rs", &i), FileClass::BuildOutput);
        assert_eq!(classify("gen/out.rs", &i).1.why(), "gitattributes: linguist-generated");
        assert_eq!(class("tests/a.rs", &i), FileClass::BuildOutput, "ignored folder above it");
        i.ignored.clear();
        assert_eq!(class("tests/README.md", &i), FileClass::Docs, "exact names before conventions");
        assert_eq!(class("tests/a.rs", &i), FileClass::Tests, "conventions before extensions");
    }

    #[test]
    fn the_table_is_total_and_ends_with_the_fallback() {
        assert_eq!(RULES.last().map(|r| r.kind), Some(RuleKind::Fallback));
        assert_eq!(RULES.iter().filter(|r| r.kind == RuleKind::Fallback).count(), 1);
        assert_eq!(class("no_extension_at_all", &ClassInputs::default()), FileClass::Other);
        assert_eq!(class("", &ClassInputs::default()), FileClass::Other);
        // Kinds appear in precedence order.
        let order = |k: RuleKind| match k {
            RuleKind::GitAttribute => 0,
            RuleKind::GitIgnored => 1,
            RuleKind::FileName => 2,
            RuleKind::Convention => 3,
            RuleKind::Extension => 4,
            RuleKind::Fallback => 5,
        };
        assert!(RULES.windows(2).all(|w| order(w[0].kind) <= order(w[1].kind)));
    }

    #[test]
    fn cachedir_tag_needs_its_signature() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CACHEDIR.TAG"), b"not a tag").unwrap();
        assert_eq!(heavy_marker(dir.path()), None);
        std::fs::write(dir.path().join("CACHEDIR.TAG"), [CACHEDIR_SIGNATURE, b"\n# a cache"].concat()).unwrap();
        assert_eq!(heavy_marker(dir.path()).map(|m| m.reason), Some(HeavyReason::BuildCache));
    }
}
