# Studio

A desktop code visualizer that lays out a repository as an infinite node canvas. Files, folders, types and functions become cards. Imports and calls become wires.

Studio is under active development. Editing is being moved onto the canvas and is currently unavailable. See the [roadmap](docs/ROADMAP.md) for the plan and the standards every feature follows.

Built in Rust with [egui](https://github.com/emilk/egui) and [wgpu](https://wgpu.rs). Runs on Linux, Windows and macOS.

## Features

- **One world**: the whole project on a single canvas, in five districts that sit in the same place in every project: Pipeline north, Files west, Code in the centre with the Desk below it, Run east, Changes south. The camera flies between them.
- **Nothing hidden**: the canvas shows every file and folder on disk, including empty folders, gitignored files, binaries, images, large files and symlinks, laid out as the real nested folder tree. See [What the canvas shows](#what-the-canvas-shows).
- **Polyglot parsing**: Rust through `syn`, 17 more languages through tree-sitter, plus Markdown and Bohemia Enforce Script (Arma Reforger / DayZ). See [Supported languages](#supported-languages).
- **Wires**: imports, calls and Markdown links between files and between individual members, drawn on the GPU in instanced batches. There is a CPU fallback.
- **Fast reopen**: parsed graphs are cached per project with zero-copy `rkyv`. The cache is invalidated when any source file changes.
- **Search**: <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> or <kbd>/</kbd> searches symbols across the project.
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

## Using it

| Action | Input |
|---|---|
| Go to a district | click it, or its button in the compass at the top right |
| The whole project | <kbd>Home</kbd>, **All** in the compass, or click the project name in the title bar |
| Pan | drag with any mouse button |
| Zoom | scroll over the canvas, <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + scroll, pinch, or <kbd>=</kbd> / <kbd>-</kbd> |
| Back to 100% | <kbd>0</kbd>, or the zoom under the compass |
| Scroll inside an open code card | scroll over the card's code |
| Select and trace | click a card: everything upstream and downstream of it is highlighted, the rest dims |
| Search symbols | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd>, or <kbd>/</kbd> when no text field has focus |
| Close / deselect | <kbd>Esc</kbd> |
| Open a file on the Desk | double-click its card, pick it in the Desk's navigator, or **Open on the Desk** in its context menu |
| See a tool's commands | click its tile in the Run district; click a command to open where it is defined |
| Follow a flow | in the Pipeline district, click a step to open its file on the Desk at its line |
| Open or close a group of flows | click its header in the Pipeline district, or its pill at the top to open it and go there |
| Build, Test, Trace and pinned tools | the Dock at the bottom; hover an icon for exactly what it runs |
| Open a folder | double-click it: it opens in place and fills the view |
| Go back up | click a folder or the district in the breadcrumb in the title bar |
| Change a folder's detail level | the three buttons in its header (minimised, node view, open) |
| Show or hide wires | **View** menu: all wires, member wires, and each kind (documentation is off by default; cards show "docs N" chips instead) |
| Open another project | the arrow next to the project name in the title bar |
| Debug panel and project stats | <kbd>F3</kbd> |
| Move / maximise the window | drag / double-click an empty part of the title bar |
| Resize the window | drag a window edge or corner |

The Run district lists the tools a project defines, found by reading its files: Cargo binaries and examples, `.cargo/config.toml` aliases, the subcommands of clap command-line tools (nested, including enums from other crates), `package.json` scripts, Makefile targets, justfile recipes and GitHub Actions workflows. Nothing is run to find them.

The Pipeline district shows how the project runs: one lane per entry point (each route of an axum route table, each program's `main`, each command of a command-line tool), with its steps left to right by how far they are from the entry. Steps and links come only from facts: route tables read from the code, `@route` and `@contract` tags in comments of any language (`// @route POST /api/v1/items/{id}`, `// @contract item.schema.json#/definitions/Item`), contracts found by file name and JSON pointer, and Rust calls resolved by path. A link's line style is its evidence: proven links are solid, links to one of a few candidates are long dashes with a "1 of n" chip, observed links are dotted, and unresolved links are short, dimmer dashes. Every link has a chip naming its tier; a route that only exists under a condition (such as `if dev`) says "conditional", and a link from one language to another names the contract it goes through. Flows that cross languages come first and are open; the other groups (endpoints by router file, programs, commands, entry points with no resolved links) start closed. A lane shows at most three steps per column and seven columns, then "+k more". A project with no routes, tags or programs says "No entry points found". The Code map draws its wires in the same line styles.

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
  - build caches marked with `CACHEDIR.TAG` (such as Cargo's `target/`);
  - dependency trees (`node_modules`, Python virtualenvs, `__pycache__`, …).

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
| `studio_parser` | Project scanning (`git ls-files`, Cargo workspaces), language detection, extraction (`syn`, tree-sitter, Markdown, Enforce), graph builders, save and re-parse, cache. |
| `studio_canvas` | Infinite canvas: the world and its districts, the camera, input, spatial hash culling, card and wire rendering, the wgpu wire pipeline. |
| `studio_sources` | Everything read about a project besides its code (packages, settings, tools, git, agent sessions), read-only and off the UI thread. |
| `studio_ui` | Theme, color tokens, syntax highlighting (`syntect`), Markdown rendering, card widgets. |
| `studio_viewer` | The eframe application (`studio_viewer` binary), telemetry, and the `bench_scale` benchmark. |

Adding a language means two things:
- a grammar entry in `crates/studio_parser/src/extractor/treesitter/registry.rs`;
- a query file in `treesitter/queries/` that uses the shared capture names documented at the top of `treesitter/mod.rs`.

## Data locations

- **Graph cache**: the OS cache directory under `tbd_studio/projects/<name>_<hash>/`, for example `~/.cache/tbd_studio` on Linux, `~/Library/Caches/tbd_studio` on macOS, or `%LOCALAPPDATA%\tbd_studio` on Windows. It is safe to delete. The **Force Reparse** button ignores it.
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

## Development

```bash
cargo test --workspace
```

CI (`.github/workflows/ci.yml`) runs `cargo fmt --check`, `cargo clippy -D warnings` and the tests on Linux, Windows and macOS.
