# Studio

A desktop code visualizer that lays out a repository as an infinite node canvas. Files, folders, types and functions become cards. Imports and calls become wires.

Studio is under active development. Editing is being moved onto the canvas and is currently unavailable. See the [roadmap](docs/ROADMAP.md) for the plan and the standards every feature follows.

Built in Rust with [egui](https://github.com/emilk/egui) and [wgpu](https://wgpu.rs). Runs on Linux, Windows and macOS.

## Features

- **One world**: the whole project on a single canvas, in five districts that sit in the same place in every project: Pipeline north, Files west, Code in the centre with the Desk below it, Run east, Changes south. The camera flies between them.
- **Nothing hidden**: the canvas shows every file and folder on disk, including empty folders, gitignored files, binaries, images, large files and symlinks, laid out as the real nested folder tree. See [What the canvas shows](#what-the-canvas-shows).
- **Polyglot parsing**: Rust through `syn`, 17 more languages through tree-sitter, plus Markdown and Bohemia Enforce Script (Arma Reforger / DayZ). See [Supported languages](#supported-languages).
- **Wires**: imports, calls, Markdown links and package dependencies, between files and between individual members, drawn on the GPU in instanced batches. There is a CPU fallback. A wire's line says how sure Studio is: solid only when proven.
- **Fast reopen**: parsed graphs are cached per project with zero-copy `rkyv`. The cache is invalidated when any source file changes.
- **Go to anything**: <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> or <kbd>/</kbd> opens a search field in the title bar. It finds files, folders, symbols, commands and tools, and what Studio read about the project, grouped by kind. Start with <kbd>&gt;</kbd> for commands only, or <kbd>@</kbd> for symbols only. The same query always gives the same order.
- **Debug panel**: <kbd>F3</kbd> shows project numbers, frame timing, memory, CPU, disk, GPU and canvas statistics.

## Build and run

You need Rust 1.95 or newer. Install it with [rustup](https://rustup.rs).

```bash
cargo run --release -p studio_viewer -- /path/to/project
```

Without a path, Studio reopens the last project. If there is none, it opens the current directory when that is a Cargo project, and otherwise shows an **Open folder** prompt.

Platform notes:

- **Linux**: needs a Vulkan or OpenGL driver. Wayland and X11 both work. Building needs the windowing headers (`libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev` on Debian and Ubuntu).
- **Windows**: uses DirectX 12 or Vulkan. No extra setup.
- **macOS**: uses Metal. No extra setup.
- The tree-sitter grammars are compiled from C during the build, so a C compiler must be available. It usually is: Xcode command line tools, MSVC build tools, or `gcc`/`clang`.

To use a specific graphics backend, set `WGPU_BACKEND` (`vulkan`, `metal`, `dx12` or `gl`).

If the GPU fails to start, Studio falls back by itself: first wgpu on OpenGL, then the OpenGL (glow) renderer, which draws wires on the CPU. A GPU that fails in the first seconds after the window opens restarts Studio on OpenGL. Each fallback prints one line to stderr ending in the hint to set `WGPU_BACKEND=gl` to start on OpenGL directly. With `WGPU_BACKEND` set, Studio uses that backend and does not fall back.

## Using it

| Action | Input |
|---|---|
| Go to a district | click it, or its button in the compass at the top right |
| The whole project | <kbd>Home</kbd>, **All** in the compass, or click the project name in the title bar |
| Pan | drag with any mouse button |
| Zoom | scroll over the canvas, <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + scroll (even over code), pinch, or <kbd>=</kbd> (or <kbd>+</kbd>) / <kbd>-</kbd> |
| Back to 100% | <kbd>0</kbd>, or click the zoom under the compass |
| Scroll inside an open code card | scroll over the card's code |
| Select and trace | click a card: everything upstream and downstream of it is highlighted, the rest dims |
| Go to anything | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd>, or <kbd>/</kbd> when no text field has focus |
| Only commands / only symbols | type <kbd>&gt;</kbd> / <kbd>@</kbd> first; <kbd>&gt;</kbd> alone lists every command and tool |
| Move through results | <kbd>↑</kbd> / <kbd>↓</kbd> |
| Open a result on the Desk | <kbd>Enter</kbd> (a symbol opens at its line) |
| Show a result on the map | <kbd>Shift</kbd>+<kbd>Enter</kbd>, or <kbd>Shift</kbd>+click it |
| Close the results | <kbd>Esc</kbd>, or click outside them |
| Every key and gesture | the **Keyboard shortcuts** command (type <kbd>&gt;</kbd> shortcuts); <kbd>Esc</kbd> or a click outside closes the sheet |
| Close / deselect | <kbd>Esc</kbd> |
| Open a file on the Desk | double-click its card, pick it in the Desk's navigator, or **Open on the Desk** in its context menu |
| Put a Desk card away / show it on the map | its X / its crosshair |
| Read a Markdown card's source | **Source** on the card (**Rendered** goes back) |
| Follow a link in a Markdown card | click it: a file opens on the Desk at the lines it names; a web link opens in the browser |
| See a tool's commands | click its tile in the Run district; click a command to open where it is defined |
| Follow a flow | in the Pipeline district, click a step to open its file on the Desk at its line |
| Open or close a group of flows | click its header in the Pipeline district, or its pill at the top to open it and go there |
| Build, Test, Trace and pinned tools | the Dock at the bottom; hover an icon for exactly what it runs. Build, Test and All tools fly to their part of the Run district, a tool to its tile, Trace to what the selection connects to (or the Pipeline); nothing runs yet |
| Where tools come from | **Add a tool** at the right end of the Dock |
| Go to the project's worktree, or today's agent sessions | the pills in the title bar ("main · 3 changes", "4 sessions today"); both fly to the Changes district |
| Open a folder | double-click it: it opens in place and fills the view |
| Go back up | click a folder or the district in the breadcrumb in the title bar |
| Change a folder's detail level | the three buttons in its header (minimised, node view, open) |
| Show or hide wires | **View** menu: all wires, member wires, and each kind, dependencies included (documentation is off by default; cards show "docs N" chips instead) |
| Open another project | the arrow next to the project name in the title bar, or the **Open folder…** command |
| Debug panel and project stats | <kbd>F3</kbd> |
| Move / maximise the window | drag / double-click an empty part of the title bar |
| Resize the window | drag a window edge or corner |

The Run district lists the tools a project defines, found by reading its files: Cargo binaries and examples, `.cargo/config.toml` aliases, the subcommands of clap command-line tools (nested, including enums from other crates), `package.json` scripts, Makefile targets, justfile recipes and GitHub Actions workflows. Nothing is run to find them.

The Pipeline district shows how the project runs: one lane per entry point (each route of an axum route table, each program's `main`, each command of a command-line tool), with its steps left to right by how far they are from the entry. Steps and links come only from facts: route tables read from the code, `@route` and `@contract` tags in comments of any language (`// @route POST /api/v1/items/{id}`, `// @contract item.schema.json#/definitions/Item`), contracts found by file name and JSON pointer, and Rust calls resolved by path. A link's line style is its evidence: proven links are solid, links to one of a few candidates are long dashes with a "1 of n" chip, observed links are dotted, and unresolved links are short, dimmer dashes. Every link has a chip naming its tier; a route that only exists under a condition (such as `if dev`) says "conditional", and a link from one language to another names the contract it goes through. Flows that cross languages come first and are open; the other groups (endpoints by router file, programs, commands, entry points with no resolved links) start closed. A lane shows at most three steps per column and seven columns, then "+k more". A project with no routes, tags or programs says "No entry points found — no route tables, no @route tags, no programs." The Code map draws its wires in the same line styles.

Nothing on the canvas changes what the map says: wires come from the code, so they cannot be drawn, cut or deleted by hand, and cards stay where the layout puts them.

A project opens on the whole world. In the Code district, the project shows at its top level: its main folders, closed, with one wire per pair of them. The number on a wire is how many file pairs it stands for, and heavier wires are thicker. A file or folder used by most of its neighbours shows "used by N" instead of its wires. See [docs/VISUAL_LANGUAGE.md](docs/VISUAL_LANGUAGE.md).

Studio draws its own title bar. On macOS the native window buttons stay at its left end.

## Supported languages

| Language | Extensions | Extracted |
|---|---|---|
| Rust | `rs` | items, impls, traits, calls, `use` (via `syn`) |
| Python | `py` `pyi` `pyw` | classes, `Enum` subclasses, methods, decorators, fields, imports |
| TypeScript / TSX | `ts` `mts` `cts` `tsx` | classes, interfaces, enums, functions, arrow functions, imports |
| JavaScript | `js` `mjs` `cjs` `jsx` | classes, functions, arrow functions, `import` / `require` |
| C | `c` | structs, enums, functions, `#include` |
| C++ | `cpp` `cc` `cxx` `hpp` `hh` `h` … | classes, bases, out-of-line `Type::method`, `#include` |
| C# | `cs` | classes, structs, records, interfaces, enums, properties, `using` |
| Java | `java` | classes, interfaces, enums, records, fields, imports |
| Kotlin | `kt` `kts` | classes, objects, interfaces, enum classes, properties, imports |
| Go | `go` | structs, interfaces, functions, receiver methods, imports |
| Swift | `swift` | classes, structs, protocols, enums, extensions, imports |
| PHP | `php` `phtml` | classes, traits, interfaces, enums, `use`, includes |
| Ruby | `rb` `rake` `gemspec` | classes, modules, methods, `require` |
| Scala | `scala` `sc` | classes, objects, traits, enums, imports |
| Dart | `dart` | classes, mixins, enums, methods, imports |
| Lua | `lua` | functions, `M.f` / `M:f` methods, `require` |
| Zig | `zig` | structs, enums, functions, `@import` |
| Bash | `sh` `bash` `zsh` | functions, `source` |
| Markdown | `md` `markdown` | headings, links, code blocks |
| Enforce Script | `c` (in Enforce projects), `ens` `es` `enforce` | classes, `modded class`, methods, fields, enums, attributes |

Other text files (JSON, YAML, configs) appear as plain file cards without parsed members.

## What the canvas shows

Every folder is a box nested inside its parent, and every file is a card in its folder. Nothing is filtered out.

- **Empty folders** show as empty boxes.
- **Binary files, images, files over 1.5 MB, symlinks and unreadable files** get a card with their kind and size. They are listed but not parsed.
- **Heavy folders start collapsed** and show their totals, for example `node_modules · 48,213 files · 312 MB · dependencies`. Click one to load its contents in the background. Heavy folders are:
  - version control (`.git`, `.hg`, `.svn`);
  - folders git ignores;
  - folders a tool marked as its own: build caches with `CACHEDIR.TAG` (such as Cargo's `target/`), Python virtualenvs (`pyvenv.cfg`), and `node_modules` folders that npm, pnpm or Yarn filled.

  No folder is heavy for its name alone. The full rules are in [docs/CLASSIFICATION.md](docs/CLASSIFICATION.md).

  Folders nested inside a loaded one that are themselves heavy stay collapsed until clicked.

A `.c` file is treated as Enforce Script when any of these is true:
- it is inside a folder that contains a Reforger `*.gproj`, a DayZ `$PBOPREFIX$`, or a `config.cpp` that declares `CfgMods`;
- it sits in an engine script-module folder (`Scripts/Game`, `scripts/4_World`, …);
- it uses syntax plain C doesn't have (classes, `proto`, `override`, `array<>`).

Otherwise it is parsed as C.

## Project layout

| Crate | Role |
|---|---|
| `studio_graph` | Graph model: nodes, ports, edges, clusters, member nodes. Serializable with `rkyv`. |
| `studio_parser` | Project scanning (`git ls-files`, Cargo workspaces), file classification (`classify`), language detection, extraction (`syn`, tree-sitter, Markdown, Enforce), code analysis (`analysis/`: the Rust path resolver, crate graph, axum route tables, `@route`/`@contract` tags, contracts), graph builders, the search index, save and re-parse, cache. |
| `studio_canvas` | Infinite canvas: the world and its districts, the camera, input, spatial hash culling, card and wire rendering, the wgpu wire pipeline. |
| `studio_sources` | Everything read about a project besides its code (disk, packages, settings, tools, git, agent sessions, tickets, pipeline), read-only and off the UI thread; see [docs/DATA_SOURCES.md](docs/DATA_SOURCES.md). |
| `studio_ui` | Theme, color tokens, syntax highlighting (`syntect`), Markdown rendering, card widgets. |
| `studio_viewer` | The eframe application (`studio_viewer` binary), telemetry, and the `bench_scale` benchmark. |

Adding a language means two things:
- a grammar entry in `crates/studio_parser/src/extractor/treesitter/registry.rs`;
- a query file in `treesitter/queries/` that uses the shared capture names documented at the top of `treesitter/mod.rs`.

## Data locations

- **Graph cache**: the OS cache directory under `tbd_studio/projects/<name>_<hash>/`, for example `~/.cache/tbd_studio` on Linux, `~/Library/Caches/tbd_studio` on macOS, or `%LOCALAPPDATA%\tbd_studio` on Windows. It is safe to delete; Studio builds it again on the next open. The `sources/` folder inside it holds the git and agent-session caches, readable only by you.
- **Settings** (last project): eframe's app data directory for `tbd-studio`, for example `~/.local/share/tbd-studio` on Linux.

## Benchmark

`bench_scale` covers four things:
- loads a real project;
- checks the cache round trip;
- stress-tests the spatial index and frame simulation at 200k and 5M synthetic nodes;
- measures GPU wire batching.

It prints per-stage RAM, CPU, disk and fault telemetry, using what the OS can report.

```bash
cargo run --release -p studio_viewer --bin bench_scale -- /path/to/project --target-fps 144 --ram-budget-gb 8
```

By default it uses the current directory, half of system RAM, and 60 FPS.

`--palette` runs only the search benchmark. It loads the project the way the app does and times building the search index (entries per kind, memory). Then it types every prefix of 40 fixed queries, each one searched from scratch, three times over. It prints p50, p95, p99 and max query times and the five slowest queries. It exits with an error when p95 is over 8 ms; `--target-ms` changes that limit.

```bash
cargo run --release -p studio_viewer --bin bench_scale -- /path/to/project --palette
```

## Development

```bash
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings` and the tests on Linux, Windows and macOS.
