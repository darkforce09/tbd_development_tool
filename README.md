# Studio

A desktop code visualizer that lays out a repository as an infinite node canvas. Files, folders, types and functions become cards. Imports and calls become wires. You can read and edit code in place, and saving re-parses the file and updates the graph.

Built in Rust with [egui](https://github.com/emilk/egui) and [wgpu](https://wgpu.rs). Runs on Linux, Windows and macOS.

## Features

- **Views**: Files & Folders, every item, public API only, or one card per module.
- **Nothing hidden**: the Files view shows every file and folder on disk, including empty folders, gitignored files, binaries, images, large files and symlinks, laid out as the real nested folder tree. See [What the Files view shows](#what-the-files-view-shows).
- **Polyglot parsing**: Rust through `syn`, 17 more languages through tree-sitter, plus Markdown and Bohemia Enforce Script (Arma Reforger / DayZ). See [Supported languages](#supported-languages).
- **Wires**: imports, calls and Markdown links between files and between individual members, drawn on the GPU in instanced batches. There is a CPU fallback.
- **Inspector and editor**: syntax-highlighted source for any file, item or member. Edits to a snippet are spliced back into the file. Saving is refused if the file changed on disk.
- **Fast reopen**: parsed graphs are cached per project with zero-copy `rkyv`. The cache is invalidated when any source file changes.
- **Spotlight**: <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> or <kbd>/</kbd> searches symbols across the project.

## Build and run

You need Rust 1.95 or newer. Install it with [rustup](https://rustup.rs).

```bash
cargo run --release -p studio_viewer -- /path/to/project
```

Without a path, Studio reopens the last project. If there is none, it opens the current directory when that is a Cargo project, and otherwise shows a built-in showcase graph.

Platform notes:

- **Linux**: needs a Vulkan or OpenGL driver. Wayland and X11 both work. Building needs the windowing headers (`libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev` on Debian and Ubuntu).
- **Windows**: uses DirectX 12 or Vulkan. No extra setup.
- **macOS**: uses Metal. No extra setup.
- The tree-sitter grammars are compiled from C during the build, so a C compiler must be available. It usually is: Xcode command line tools, MSVC build tools, or `gcc`/`clang`.

To use a specific graphics backend, set `WGPU_BACKEND` (`vulkan`, `metal`, `dx12` or `gl`).

## Using it

| Action | Input |
|---|---|
| Pan | right-drag, or <kbd>Space</kbd> + left-drag |
| Zoom | scroll over the canvas, <kbd>Ctrl</kbd>/<kbd>⌘</kbd> + scroll anywhere, or pinch |
| Scroll inside an open code card | scroll over the card's code |
| Select / inspect | click a card or a member row |
| Save the editor buffer | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>S</kbd> |
| Spotlight search | <kbd>Ctrl</kbd>/<kbd>⌘</kbd>+<kbd>K</kbd> or <kbd>/</kbd> |
| Close / deselect | <kbd>Esc</kbd> |
| Collapse / expand a folder | click its header, or the collapsed folder card |

Editing works on whatever the inspector shows: the whole file, or the snippet for a single item or member.

- **Saving a snippet** replaces exactly that text in the file and leaves everything else alone.
- **If a snippet can't be found**, because the file changed elsewhere or the same text appears twice, the save is refused and nothing is written. Use **Load Full File** to edit the whole file instead.
- **Every save** goes to a temporary file first, which is then renamed over the original. The file's line endings (LF or CRLF) are kept.
- **Unsaved changes**: switching to another card asks whether to save, discard or cancel.

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

## What the Files view shows

Every folder is a box nested inside its parent, and every file is a card in its folder. Nothing is filtered out.

- **Empty folders** show as empty boxes.
- **Binary files, images, files over 1.5 MB, symlinks and unreadable files** get a card with their kind and size. They are listed but not parsed. In the inspector:
  - images are previewed;
  - large text files show their first 64 KB;
  - binaries show a hex dump;
  - symlinks show their target;
  - every file has **Open in system app** and **Show folder** buttons.
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
| `studio_canvas` | Infinite canvas: transform, input, spatial hash culling, card and wire rendering, the wgpu wire pipeline. |
| `studio_ui` | Theme, color tokens, syntax highlighting (`syntect`), Markdown rendering, card widgets. |
| `studio_viewer` | The eframe application (`studio_viewer` binary), telemetry, and the `bench_scale` benchmark. |

Adding a language means two things:
- a grammar entry in `crates/studio_parser/src/extractor/treesitter/registry.rs`;
- a query file in `treesitter/queries/` that uses the shared capture names documented at the top of `treesitter/mod.rs`.

## Data locations

- **Graph cache**: the OS cache directory under `tbd_studio/projects/<name>_<hash>/`, for example `~/.cache/tbd_studio` on Linux, `~/Library/Caches/tbd_studio` on macOS, or `%LOCALAPPDATA%\tbd_studio` on Windows. It is safe to delete. The **Force Reparse** button ignores it.
- **Settings** (last project, view, panel state): eframe's app data directory for `tbd-studio`, for example `~/.local/share/tbd-studio` on Linux.

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
