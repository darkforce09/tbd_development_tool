# File classification

Studio gives every file one class, from one table, the same way on every project. The code is
`crates/studio_parser/src/classify.rs` (`RULES`, `HEAVY_MARKERS`); this page mirrors it row for row,
and a test (`crates/studio_parser/tests/classification.rs`) fails if the two drift apart.

## Classes

The regions of the roadmap's Phase 2, plus two:

| Class | What it holds |
|---|---|
| code | source, markup, styles, shaders |
| tests | test files, test data and benchmarks |
| tooling | build scripts, task runners, shell scripts |
| docs | prose, project documents, examples |
| schemas | schemas and contracts between programs |
| assets | images, fonts, audio, video, models |
| config | manifests, lockfiles, settings |
| build output | build output, caches and logs; anything git ignores |
| CI | pipeline definitions |
| tickets | Studio tickets |
| vendored | files `.gitattributes` marks `linguist-vendored` (added: Linguist's override needs a class) |
| other | no rule matched (added: the table is total) |

## Rules

The first row that matches decides; nothing later is consulted. `classify` returns the row, so the
interface can say why a file has its class ("tool convention: Cargo", "extension", …).

Patterns are space-separated alternatives:

- **gitattributes**: the Linguist attribute git reports for the path. Git is not asked by the parser:
  the attribute sets are inputs, filled by the viewer from git's answers.
- **gitignored**: git ignores the file or a folder above it (`.gitignore`, `.git/info/exclude`, the
  global excludes file).
- **file name** and **extension**: globs on the file name only; `*` matches within the name; ASCII case
  is ignored.
- **tool convention**: a path a tool itself fixes. `/a/b` starts at the project root; `{Cargo.toml}/tests/**`
  starts at a folder that holds `Cargo.toml` (the manifest and the folder side by side). `**` is one or
  more folders and the file. A folder name alone never matches.

Precedence: overrides from `.gitattributes`, then gitignore, then exact file names, then tool
conventions, then extensions, then the fallback. Exact names come before conventions so that a
`README.md` inside Cargo's `tests/` is still docs; conventions come before extensions, or `tests/it.rs`
would be code by its extension and the convention could never apply.

| # | Kind | Pattern | Class | Basis | Note |
|---|---|---|---|---|---|
| 1 | gitattributes | `linguist-generated` | build output | Linguist | marked generated in .gitattributes |
| 2 | gitattributes | `linguist-vendored` | vendored | Linguist | marked vendored in .gitattributes |
| 3 | gitattributes | `linguist-documentation` | docs | Linguist | marked documentation in .gitattributes |
| 4 | gitignored | `.gitignore` | build output | git | git ignores it (any ignore source): not source |
| 5 | file name | `README README.* CHANGELOG CHANGELOG.* LICENSE LICENSE.* LICENCE LICENCE.* COPYING COPYING.* CONTRIBUTING CONTRIBUTING.* NOTICE NOTICE.* AUTHORS` | docs | well-known names | project documents |
| 6 | file name | `*_test.go` | tests | Go | the go command builds these only for go test |
| 7 | file name | `test_*.py *_test.py` | tests | pytest | pytest's default test file names |
| 8 | file name | `*.test.js *.test.jsx *.test.ts *.test.tsx *.test.mjs *.test.cjs *.spec.js *.spec.jsx *.spec.ts *.spec.tsx *.spec.mjs *.spec.cjs` | tests | Jest, Vitest | the runners' default test file names |
| 9 | file name | `*.schema.json openapi.json openapi.yaml openapi.yml swagger.json swagger.yaml swagger.yml` | schemas | JSON Schema, OpenAPI | schema and API contract files |
| 10 | file name | `Makefile GNUmakefile justfile .justfile Dockerfile Dockerfile.* Containerfile Taskfile.yml Taskfile.yaml` | tooling | make, just, Docker, Task | build and task runner files |
| 11 | file name | `Cargo.lock package-lock.json npm-shrinkwrap.json yarn.lock pnpm-lock.yaml poetry.lock uv.lock Pipfile.lock go.sum Gemfile.lock composer.lock flake.lock` | config | package managers | lockfiles |
| 12 | file name | `Cargo.toml package.json pyproject.toml setup.cfg go.mod go.work Gemfile Pipfile requirements.txt requirements-*.txt CMakeLists.txt tsconfig.json tsconfig.*.json rust-toolchain rust-toolchain.toml` | config | package managers | manifests and toolchain files |
| 13 | file name | `.gitignore .gitattributes .gitmodules .editorconfig .env .env.* .npmrc .nvmrc .prettierrc .prettierrc.* .eslintrc .eslintrc.* .clang-format rustfmt.toml .rustfmt.toml clippy.toml .clippy.toml` | config | well-known names | tool settings |
| 14 | tool convention | `/.github/workflows/*.yml /.github/workflows/*.yaml` | CI | GitHub Actions | workflows live in .github/workflows at the repository root |
| 15 | tool convention | `/.gitlab-ci.yml` | CI | GitLab CI/CD | the default pipeline file at the repository root |
| 16 | tool convention | `/.circleci/config.yml` | CI | CircleCI | the pipeline file at the repository root |
| 17 | tool convention | `/Jenkinsfile` | CI | Jenkins | the default Pipeline script path |
| 18 | tool convention | `/azure-pipelines.yml` | CI | Azure Pipelines | the default pipeline file at the repository root |
| 19 | tool convention | `/.ai/tickets/T-*.toml` | tickets | Studio | the only folder Studio reads tickets from |
| 20 | tool convention | `{Cargo.toml}/build.rs` | tooling | Cargo | the package's build script |
| 21 | tool convention | `{Cargo.toml}/tests/**` | tests | Cargo | integration tests and their data |
| 22 | tool convention | `{Cargo.toml}/benches/**` | tests | Cargo | benchmarks |
| 23 | tool convention | `{Cargo.toml}/examples/**` | docs | Cargo | example programs that show the package's use |
| 24 | extension | `*.rs *.py *.pyi *.js *.mjs *.cjs *.ts *.mts *.cts *.tsx *.jsx *.go *.c *.h *.cc *.cpp *.cxx *.hpp *.hh *.hxx *.cs *.java *.kt *.kts *.swift *.rb *.php *.lua *.dart *.scala *.zig *.vue *.svelte *.html *.htm *.css *.scss *.sass *.less *.sql *.wgsl *.glsl *.hlsl *.vert *.frag *.comp *.shader` | code | languages | source, markup, styles and shaders |
| 25 | extension | `*.sh *.bash *.zsh *.fish *.ps1 *.psm1 *.bat *.cmd *.mk *.cmake` | tooling | shells, make, CMake | scripts |
| 26 | extension | `*.md *.markdown *.mdx *.rst *.adoc *.txt *.org *.tex *.pdf` | docs | document formats | prose |
| 27 | extension | `*.proto *.graphql *.gql *.xsd *.avsc *.thrift *.capnp` | schemas | schema languages | contracts between programs |
| 28 | extension | `*.toml *.yaml *.yml *.json *.jsonc *.json5 *.ini *.cfg *.conf *.properties *.xml *.plist *.env *.csv *.tsv *.jsonl *.ndjson` | config | data formats | settings and data |
| 29 | extension | `*.png *.jpg *.jpeg *.gif *.webp *.bmp *.ico *.svg *.tga *.dds *.psd *.exr *.hdr *.tif *.tiff *.ttf *.otf *.woff *.woff2 *.wav *.ogg *.mp3 *.flac *.mp4 *.webm *.mov *.fbx *.gltf *.glb *.blend *.bvh` | assets | media formats | images, fonts, audio, video, models |
| 30 | extension | `*.et *.ent *.layer *.layout *.emat *.edds *.xob *.meta` | assets | Enfusion | engine resources: prefabs, worlds, UI layouts, materials, textures, models, resource metadata |
| 31 | extension | `*.log *.o *.a *.lib *.so *.dylib *.dll *.exe *.pdb *.pyc *.pyo *.class *.rlib *.rmeta` | build output | compilers, loggers | objects, binaries, bytecode, logs |
| 32 | fallback | `*` | other | none | no rule above matched |

## Tool conventions and their sources

Each convention cites the tool's own documentation. Wherever a class from a convention is shown
(map cards, `sources_report`, the Files district), its basis is shown with it: "tool convention: Cargo".

| Basis | Source |
|---|---|
| GitHub Actions | https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions (workflow files are stored in `.github/workflows`) |
| GitLab CI/CD | https://docs.gitlab.com/ee/ci/yaml/ (`.gitlab-ci.yml` at the repository root by default) |
| CircleCI | https://circleci.com/docs/configuration-reference/ (`.circleci/config.yml`) |
| Jenkins | https://www.jenkins.io/doc/book/pipeline/jenkinsfile/ (`Jenkinsfile` at the repository root by default) |
| Azure Pipelines | https://learn.microsoft.com/en-us/azure/devops/pipelines/get-started/yaml-pipeline-editor (`azure-pipelines.yml` at the root by default) |
| Studio | `crates/studio_sources/src/tickets.rs` (`T-*.toml` directly in `.ai/tickets`) |
| Cargo | https://doc.rust-lang.org/cargo/reference/cargo-targets.html#target-auto-discovery (`tests/`, `benches/`, `examples/` beside `Cargo.toml`) and https://doc.rust-lang.org/cargo/reference/build-scripts.html (`build.rs` beside `Cargo.toml`) |

The file-name rows that name a tool cite it the same way:

| Basis | Source |
|---|---|
| Linguist | https://github.com/github-linguist/linguist/blob/main/docs/overrides.md |
| Go | https://pkg.go.dev/cmd/go#hdr-Test_packages |
| pytest | https://docs.pytest.org/en/stable/explanation/goodpractices.html#conventions-for-python-test-discovery |
| Jest, Vitest | https://jestjs.io/docs/configuration#testmatch-arraystring and https://vitest.dev/config/#include |

## Heavy folders

A heavy folder is listed with its totals but not descended, so a dependency tree or a build cache
never slows the first layout. A folder is heavy when:

1. it is version control metadata: `.git`, `.hg`, `.svn` (the VCS's own folders);
2. git ignores it; or
3. it holds one of the markers below, which the tool that fills the folder writes into it. The check
   is one stat per marker on the scan thread, while the folder is met; `CACHEDIR.TAG` also needs its
   first 43 bytes to be the specification's signature line
   (`Signature: 8a477f597d28d172789f06886806bc55`).

| Marker | Heavy reason | Tool | Source |
|---|---|---|---|
| `CACHEDIR.TAG` | build cache | Cache Directory Tagging (Cargo, pytest, mypy, ruff, …) | https://bford.info/cachedir/ |
| `pyvenv.cfg` | dependencies | Python venv (PEP 405) | https://peps.python.org/pep-0405/ |
| `.package-lock.json` | dependencies | npm 7+ hidden lockfile | https://docs.npmjs.com/cli/configuring-npm/package-lock-json#hidden-lockfiles |
| `.modules.yaml` | dependencies | pnpm | https://github.com/pnpm/pnpm/tree/main/pkg-manager/modules-yaml |
| `.yarn-integrity` | dependencies | Yarn 1 (classic) | https://classic.yarnpkg.com/en/docs/cli/check |
| `.yarn-state.yml` | dependencies | Yarn 2+ node-modules linker | https://github.com/yarnpkg/berry/tree/master/packages/plugin-nm |

There are no manifest-anchored heavy rules (such as `node_modules/` beside `package.json`): npm,
pnpm and Yarn each write a marker into the folders they fill, and the marker is preferred.

## What is never assumed

- No folder is heavy, and no file gets a class, for a folder's name alone: not `node_modules`,
  `target`, `build`, `dist`, `vendor`, `tests`, `__pycache__` or any other. A `tests/` folder is tests
  only beside a `Cargo.toml`; a `node_modules/` is heavy only with a marker or when git ignores it.
- File contents are not read to classify (the `CACHEDIR.TAG` signature is the one exception, and it
  only makes a folder heavy).
- Nothing is learned from other projects or from history: the same paths and the same inputs give the
  same classes.
- The parser classifies with what the scan knows (every path, and what git ignores). Linguist
  attributes come from git and are applied by the viewer with the same table.
