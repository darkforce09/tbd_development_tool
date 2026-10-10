# Roadmap

Studio lays out any repository on one infinite canvas so a solo developer, or a small team working with AI agents, can see how the whole project fits together: where each part lives, what depends on what, and which way data flows. It opens a folder with no setup, and everything it shows is derived from the code, the build metadata or the running program. Nothing on the canvas is guessed.

This document has three parts:

1. **Standards**: rules every feature must follow. A feature that breaks one is not done.
2. **Phases**: the order of work, each with a goal and exit criteria.
3. **Launch**: what must be true before Studio is advertised.

Work on a project-specific fix only through the general mechanism. If something only works for TBD-Reforger or AnyPS5, it is not done.

## Standards

### S1. Correct data only

Every node, wire and label is derived deterministically from one of:

- source code, through a parser or a compiler-grade indexer;
- build metadata (`cargo metadata`, `compile_commands.json`, `package.json`, …);
- observation of the running program (traces).

No data on the canvas comes from an LLM. AI may read the graph and draw its plans on top of it, but it never adds, removes or labels graph data.

### S2. Every wire states its evidence

Each wire carries a kind, a tier and its provenance (which resolver produced it, from which source line).

| Tier | Meaning | Example |
|---|---|---|
| **Proven** | Resolved by the compiler or an equivalent resolver. | `foo()` resolves to `crate::a::foo`. |
| **Possible set** | Statically one of a known, exact set. | A `dyn Trait` call: one of these 4 impls. |
| **Observed** | Recorded while the program ran. | This handler ran during the trace. |
| **Unresolved** | The reference exists but its target is unknown. | Lookup by a runtime string. |

Name matching is not evidence. A wire that cannot reach at least the Possible set tier is shown as Unresolved or not at all.

How kinds look on the canvas (colours, ports, layout laws) is defined in [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md).

### S3. Language-agnostic core

The graph model, layout and canvas know nothing about any language. Languages plug in through adapters that output symbols, references and evidence tiers in one shared format. Adding a language never changes the core.

### S4. Zero setup

Opening the project folder is the only step. Build metadata is generated automatically where possible (for example running CMake configure with `CMAKE_EXPORT_COMPILE_COMMANDS`). When it cannot be, Studio says what is missing and shows only what it can prove.

### S5. One canvas

There is one canvas per project. Views, regions, filters and overlays change what that canvas shows. They never open a separate canvas.

### S6. Files stay the source of truth

Studio never replaces git or the files in the repo. Settings panels (gitignore, editorconfig, MCP config, …) are a visual layer over the real files, so every other tool and agent keeps working.

### S7. Measured correctness

Correctness is measured, not judged by eye:

- **Fixture repos**: small repositories per language with hand-verified expected wires. Studio must produce no wire that is not in the expected set (precision 100%). Recall is measured and tracked.
- **Real repos**: this repository, TBD-Reforger and AnyPS5. Counts per wire kind and tier are recorded so regressions show up.

### S8. Performance budget

- A 17,500-file project shows its folder layout in under 1 s and is fully parsed in the background.
- The canvas stays at 60 fps at every zoom level.
- `bench_scale` checks both on every change that touches the loader or canvas.

### S9. UX is reviewed in every phase

Every phase lists UX acceptance criteria alongside technical ones, and ends with a review on the real repos. Anything on screen that does not help someone understand the project is removed.

### S10. Safe writes

Edits touch only the region that was loaded, are refused when the file changed on disk, are written atomically and keep line endings.

## Phases

Phases run roughly in order. A later phase can start early only when it does not depend on an unfinished earlier one.

### Phase 0: Clean slate

Goal: remove everything that is mock, duplicated or in the way, so later work builds on what is real.

- [x] One canvas: remove the Codebase Maps / Architecture Map / Flows & Traces tabs. Flows & Traces showed hard-coded mock data.
- [x] Remove the right Inspector panel and the Split Editor. Editing returns on the canvas in Phase 6.
- [x] Filters sidebar hidden when the app opens.
- [x] Remove the STUDIO brand label and the bottom status bar.
- [x] Project stats move into the F3 debug panel, alongside frame timing, process, GPU and canvas telemetry. The old "Architecture Graph Valid" title is dropped because nothing was validated.
- [x] Remove the view dropdown (Files & Folders / All Items / Public API / Modules) from the UI.
- [x] Remove the Items, Public API and Modules graph builders from `studio_parser`, and granularity from the cache key.
- [x] Remove the Ingress / Compute / State / Egress filter categories and the Spotlight buttons that created such nodes. They came from the abandoned lanes idea and are not derived from code.
- [ ] Remove those archetypes from the graph model, the mock pipeline graph and `bench_scale` (needs a cache version bump; fold into Phase 1).
- [x] Remove the Repo, Showcase and Force Reparse buttons. With no project open, the canvas shows an Open folder prompt.
- [x] Wire toggles move into a View menu (Wires, Member wires) built so new toggles are one line each.
- [x] Remove the Spotlight "create node" buttons; it is now Search. They added nodes that no code backs.
- [x] Integrated title bar: no OS decorations; search, open and the View menu on the left, the project name in the centre, window buttons on the right. The window edges resize it.
- [x] Filters button removed; the sidebar opens from an arrow tab under the title bar (Eden editor style).
- [x] Messages and errors: the debug panel shows the last message and any load error, and a failed load puts a warning on the title.
- [ ] Keyboard shortcut help, which went away with the bottom bar.
- [x] Visual language standard: direction (providers left), ports, layout laws, gates, folder detail levels, colours. See [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md).
- [x] Wire kinds on every edge, Markdown links as documentation wires into a documentation port, provider-to-consumer direction, and keep every wire into an input (no fan-in loss).
- [x] Left-to-right layout engine: lifting wires through folder gates, cycle boxes, layering, crossing reduction, coordinates.
- [x] Wire routing: orthogonal routes that never pass through cards or folders, crossing folder edges only at gates.
- [x] Route documentation wires: through documentation gates and each folder's documentation strip, never over cards or folders (L2, L3).
- [x] Rendering of routes, gates, documentation ports and cycle boxes; folder detail levels (minimised, node view, open) switched from the folder header.
- [x] Geometry-only relayout when a card expands or a folder changes level: columns and order are kept, only the folders around the change are measured again.
- [x] One level at a time: a project opens at its top level (folders closed); a closed folder stands in for its files (one gate per visible source, one bundled wire with a count, thicker for more); double-click opens a folder in place and fits it; a breadcrumb in the title bar goes back up.
- [x] Hubs (L6): an item feeding at least half its folder's connected items, and at least 8, shows "used by N" instead of its wires.
- [x] Documentation as "docs N" chips; documentation wires off by default.
- [x] Click to trace: selecting a card highlights everything upstream and downstream and dims the rest.
- [x] Quieter wires, faint folder fills, and folder names drawn at a readable size when zoomed out.
- [ ] Skip laying out documentation wires while they are hidden, so their strips take no space.

Exit: every control on screen does something real, and nothing shows mock data for a real project.

### Phase 1: Foundation

Goal: a graph model that can hold correct data from any language.

- Edge kinds as a typed enum: contains, imports, calls, references type, implements or inherits, data flow, doc link, asset reference.
- Evidence tier and provenance on every edge (S2).
- Any number of wires per port. Today a new wire into a port deletes the previous one, so a header included by 298 files keeps 1 incoming wire.
- Stable node identity (path plus symbol path) that survives a reparse.
- Language adapter interface (S3). Move the `syn`, tree-sitter, Markdown and Enforce extractors behind it.
- Fixture repos and the correctness test harness (S7).
- Real graph checks to replace the old "valid" label, for example wires whose target no longer exists.

Exit: all extractors run through the adapter interface, fixture tests run in CI, and the real-repo counts are recorded.

### Phase 2: Organization

Goal: open any repo and see a structure that is the same for every project, so you know where to look before reading code. This comes before wire work because messy wires are a symptom of missing organization.

- Fixed canvas regions, assigned by deterministic file classification:
  - code
  - tests
  - tooling and scripts
  - docs
  - schemas and contracts
  - assets (images, models, textures, audio)
  - configuration and settings
  - build output, caches and logs
  - CI
  - tickets (Phase 8)
- Classification rules are a documented table: extensions, well-known names, package-manager metadata and `.gitignore`.
- The code region is arranged by the package graph: Cargo workspaces, CMake targets, npm workspaces, Python packages, Enfusion addons. Packages are laid out in dependency layers so flow reads in one direction.
- Size so one huge folder does not take half the screen: aggregate cards and detail by zoom level.
- Enforce Script: addon and script-module structure as the code region's packages.

UX exit: on TBD-Reforger and AnyPS5, with zero setup, a newcomer finds "the mission creator UI" and "the shader recompiler" in under a minute.

### Phase 3: Verified dependencies and calls

Goal: replace name matching with compiler-grade resolution.

| Language | Source |
|---|---|
| Rust | `cargo metadata` for crates, rust-analyzer (SCIP) for symbols and calls |
| C / C++ | `compile_commands.json`, generated automatically from CMake, or with `bear` for Make, then scip-clang or clangd |
| Python | scip-python |
| Enforce Script | Studio's own resolver; no external indexer exists |
| Others | SCIP indexers where available; otherwise structure only, wires Unresolved |

- Tree-sitter remains for the instant first view and for structure. Its references never become Proven.
- The indexer runs in the background, and verified wires replace provisional ones when it finishes.

Exit: fixture precision is 100% for Rust, C++, Python and Enforce, recall is tracked, and both real repos are indexed with zero setup. For AnyPS5 that includes fetching submodules.

### Phase 4: Data flow and pipelines

Goal: show which way data moves, not just that two things are connected.

- Direction per wire from typed information: arguments flow caller to callee, return values flow back, channel sends flow to receives, field writes flow to reads, and events flow from emitters to handlers.
- Entry points by framework convention: `main` and bin targets, egui `App::update`, HTTP routes, Enforce event overrides, PRX exports.
- A pipeline is everything reachable from an entry point, named after the entry point.
- Every wire links to the source line that proves it.

Exit: pipelines are generated automatically on both real repos, and each one can be checked against the source by clicking through.

Pipeline district (Step 7):

- [x] Axum route tables evaluated from the code: `route`, `nest`, `merge`, `let` reassignment, and `if` marking a route conditional; handlers resolved by path. Route tables are read only from library and binary module trees, never tests.
- [x] `@route` and `@contract` tags in comments of every language; route templates unify `:x` and `{x}`.
- [x] Contracts resolved by exact file name and JSON pointer; several matching files make a Possible set, a missing pointer stays Unresolved.
- [x] Flows walked from entry points (routes, program `main`s, command-line commands) over links at Possible set or better, with language crossings recorded.
- [x] One lane per flow, steps left to right, every link labelled with its tier; clicking a step opens its code on the Desk. Groups collapse; the district grows north to fit and the map never moves.
- [x] Rust `use` and path-call wires on the Code map are Proven when the path resolves to exactly one item; the rest stay Unresolved. Wire line styles per tier (solid, long dash, dotted, short dash).
- [x] Exit: on TBD-Reforger the fleet-command report routes link the Enforce sender to their Rust handlers as Proven; fixture precision is 100%.
- [ ] Enforce string folding: check a sender's `@route` tag against the URL its code builds (constants from other classes, concatenation, `if`/`else`). [S11]
- [ ] Per-process swimlanes for a selected flow, with runtime traces. [S11]
- [ ] Resolve a saved file's wires again by path on save; until then, saving never makes a wire Proven by name. [S10]
- [ ] The clap command reader finds enums through the path resolver instead of by name over the dependency closure. [W1C]

### Phase 5: Wire presentation

Goal: wires that are readable at any zoom. Routing and the basic look are set in Phase 0 by the [visual language](VISUAL_LANGUAGE.md); this phase builds on it.

- Toggles per evidence tier (S2).
- Line styles and colour-blind variants.
- Tune the hub threshold and bundle thickness from real use.

### Phase 6: On-canvas editing and inspection

Goal: read and edit everything in place, with no side panel.

- Edit code directly in the card, keeping the S10 save rules (the logic in `studio_parser::save`).
- Preview images, hex dumps, symlink targets and large-file heads on cards. This restores what the Inspector had.
- Previews for 3D models (glTF, OBJ; Enfusion `.xob` needs an export path).
- Asset usage: which code, prefabs or configs reference each asset, including Enfusion `{GUID}` resource references, and which assets nothing references.

### Phase 7: Integrated tooling

Goal: run and understand the project without a terminal.

- Test and compile results on the nodes they belong to: cargo test JSON, ctest, pytest JUnit, compiler diagnostics.
- Tool panels detected from the project: Cargo bins, `package.json` scripts, justfiles, Makefiles and CMake targets, shown with buttons and output.
- Settings panels over real files (S6): gitignore, gitattributes, editorconfig, MCP config.
- Logs as a viewable region instead of raw text.

### Phase 8: Git and collaboration

- Working tree overlay: changed nodes highlighted, unchanged ones dimmed.
- Branches and worktrees.
- Pull request overlay: which functions open PRs touch, to avoid duplicate work.
- Tickets placed on the canvas and linked to code.
- Docs linked to symbols, not paths, with a "possibly stale" flag when the linked code changes.

### Phase 9: AI integration

- An MCP server exposing the graph, so agents query real structure.
- Agent plan overlay: what the agent will touch, highlighted on the canvas, with the ability to add or remove nodes from the plan.
- Live highlighting of agent activity.
- AI never writes graph data (S1).

### Phase 10: Live UI editing, per framework

- Map a rendered UI element to the code that builds it and the code that handles it.
- Frameworks one at a time: web (HTML/React), egui, Enfusion layouts.

### Phase 11: Runtime (last, optional)

- Observed-tier traces from a running program, drawn as live wires.
- Assets actually used during a run.
- Running the application inside Studio.

Not planned: replacing git, inventing a new language or file format.

## Launch

Studio is advertised when:

- Phases 0 to 10 are complete.
- Rust, C/C++, Python and Enforce Script meet the Phase 3 exit criteria.
- TBD-Reforger and AnyPS5 pass every phase's UX exit with zero setup.
- Adding a language is documented and needs no change to the core (S3).

## Test repositories

| Repository | Why |
|---|---|
| This repository | Small Rust workspace; fast feedback. |
| TBD-Reforger | 175 crates, ~17,500 files, Rust and Enforce Script, many assets. |
| AnyPS5 | ~275k lines of C++ and Python on CMake; 101 files named `Export.cpp`; submodules; a vibe-coded project that needs new contributors. |
| Fixture repos (Phase 1) | Small hand-verified repos per language for precision tests. |
