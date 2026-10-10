# Data sources

Where everything Studio shows comes from. Each source reads files, or runs a read-only command, off the UI thread, and every link it makes carries an evidence tier and a basis ([ROADMAP.md](ROADMAP.md) S1, S2). Nothing here changes the project: the only files Studio writes are its own caches.

The code is `crates/studio_sources` for everything but the code itself, and `crates/studio_parser` for the code map. `sources_report` prints what every source reads about a project, with timings, without opening a window:

```bash
cargo run --release -p studio_sources --bin sources_report -- /path/to/project [--du] [--tests <prefix>] [--classes]
cargo run --release -p studio_sources --bin sources_report -- /path/to/project --sessions [--cold]
```

## The framework

### Sources and the hub

There is one `SourceHub` per open project. Each source runs as a job on its own thread and reports through one channel. The UI drains the channel once per frame, which costs no more than swapping in what arrived. The sources are disk, packages, settings, tools, git, agent sessions, tickets and pipeline (`SourceKind`).

A job reports `Running` first, then one of:

| State | Means | On the canvas |
|---|---|---|
| `Running` | still reading | the district says what it is reading: "Reading the project's tools…", "Reading the disk…", "Reading git…", "Reading routes, tags and entry points…" |
| `Ready` | done, with how long it took | the district shows what was read |
| `Unavailable(reason)` | the project has nothing for this source | the reason, said plainly as the district's empty state ("No git repository here"). Never shown as an error. |
| `Failed(reason)` | it should have worked and did not | "*Source* could not be read: *reason*", e.g. "The pipeline could not be read: …" |

A district without content shows its source's reason instead of "Reading…", so a source that has nothing never looks like it is still loading.

Jobs start once the project's files are known. Packages, disk, settings and git history start together. Tools start when packages arrive, or with no packages when packages are Unavailable or Failed. The pipeline starts when tools arrive, or with no tools when tools are Unavailable or Failed. Tickets start once git has answered. Agent sessions and the last commit per folder wait until the first layout is on screen, so they never delay it.

### Commands

Every command goes through `exec.rs`. Nothing else in Studio runs a program to read a project.

- Only two programs: `git` and `cargo`.
- Git runs only these subcommands: `cat-file`, `check-attr`, `check-ignore`, `diff`, `for-each-ref`, `log`, `ls-files`, `rev-list`, `rev-parse`, `show`, `status`, `version`, and `worktree` only as `worktree list`. Anything else is refused before it runs.
- Every git command starts with `--no-optional-locks --no-pager -c core.quotepath=off`: it never takes the index lock, so it never blocks your own git commands; it never pages; paths come back as raw bytes.
- Cargo runs only `cargo metadata --format-version 1 --offline --no-deps --manifest-path <manifest>`.
- Every command runs with `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0`, `RUSTUP_AUTO_INSTALL=0` and `CARGO_TERM_COLOR=never`: plain, untranslated output that never asks for a password and never installs a toolchain.
- At most 4 commands run at once per project (`MAX_CHILDREN`).
- Each command has a timeout: 20 s by default (`DEFAULT_TIMEOUT`), 120 s for the whole-history pass of the last commit per folder.
- Closing the project cancels every job: running commands are killed, and nothing waits for them, so closing never stalls the UI.
- A non-zero exit is an error that keeps the exit code and the first 3 lines of stderr. A missing program makes the source Unavailable ("git is not installed").

### Caches

Caches live in the project's user cache folder, next to the graph cache, under `sources/` (`store.rs`).

- One file per cache, `<name>.rkyv`.
- Each file carries a format version and a key (a fingerprint of what it was read from). A file whose version or key differs is ignored, never trusted.
- Files are written atomically: a crash leaves the old cache or the new one.
- The folder is `0700` and the files `0600`: only you can read them.

Two sources cache: git (the files of each commit) and agent sessions. Everything else is read again on open.

## Disk

What git and the file system say about the files: the Files district's tree header, marks and size map.

Commands, in order:

1. `git rev-parse --is-inside-work-tree`
2. `git ls-files -z` (the number of tracked files)
3. `git check-attr --stdin -z filter linguist-generated linguist-vendored linguist-documentation`, fed every tracked path
4. `git ls-files --others --ignored --exclude-standard --directory -z`
5. `git check-ignore -v -z --stdin --no-index`, fed every ignored entry: the exact `.gitignore` file, line and pattern for each
6. `git status --porcelain=v2 -z --untracked-files=normal`

Rules:

- A file is in LFS when its `filter` attribute is `lfs`. It is generated, vendored or documentation when `linguist-generated`, `linguist-vendored` or `linguist-documentation` is set (`set` or `true`).
- Tracked files by class: every tracked path is classified with the [classification table](CLASSIFICATION.md), with git's ignore and Linguist answers as inputs. Each class shows its file count, the sum of its files' lengths, and the rules that decided it, most files first.
- Sizes are measured like `du -s --block-size=1`: allocated blocks, each hard-linked file once, never crossing into another file system. Elsewhere than Unix, file lengths. Every top-level entry and every ignored entry is measured, on 4 threads, and arrives as it is measured.
- `check-ignore` exits 1 when a path is not ignored; it still prints the ones that are, so that exit is not an error.

The Files district's "Tracked files by class" panel shows those totals; hovering a class lists its rules.

Marks on tree rows: "ignored", "heavy: *reason*" (see [CLASSIFICATION.md](CLASSIFICATION.md#heavy-folders)), "LFS", "generated", "vendored", "hidden" (a name starting with `.`), then the size. Ignored and hidden rows are dimmed.

Evidence: these are facts git reports about each path, not links.

Cache: none.

Failure modes:

- No repository: the source is Unavailable at once, git's facts are skipped and sizes are still measured. The Files
  tree's header says "No git repository here" (or "Git could not be read: …" when git fails), and so does the
  Changes district.
- The size map says "Measuring where the space goes…" until the first size arrives.

## Packages

Cargo packages: what each builds and which other packages of the project it depends on. The Run district's build-target wall, the "crate *name* · *n* binaries" mark on package folders, and the input to tools and the pipeline.

Command: `cargo metadata --format-version 1 --offline --no-deps --manifest-path <manifest>`, once per `Cargo.toml` that has a `[workspace]` table.

Files read: every `Cargo.toml` the scan found.

Rules:

- Packages cargo describes are taken as cargo describes them.
- A workspace cargo cannot read (no toolchain, a broken manifest), and a package outside every workspace, is read from its `Cargo.toml` with cargo's own target rules: `src/lib.rs`, `src/main.rs`, `src/bin/*`, `examples/*`, `tests/*`, `benches/*`, `build.rs`, `[lib]`, `[[bin]]` and the other target tables, `autobins` and the other auto flags, and an `edition.workspace = true` through the workspace's `[workspace.package]`. Each such workspace is listed with what went wrong.
- A package's project dependencies are its path dependencies, also through `workspace = true`.
- Packages are sorted by name, then manifest.

Cache: none (cargo is fast: 158 packages in about 100 ms on TBD-Reforger).

Failure modes:

- No `Cargo.toml`: Unavailable, "No Cargo packages here". The Run district's package wall says so, and tools are still read.
- Packages failed: the wall says "Packages could not be read: *reason*".
- No package found: "No packages found".

## Settings

The files that configure how a project builds, formats, lints and is versioned, each fact with the line it comes from. The Files district's settings cards; each card opens its line on the Desk. Nothing is written.

Files read, at the project root:

| Card | File |
|---|---|
| Cargo workspace | the root `Cargo.toml`: members, shared dependencies with versions and how many packages use each, profiles |
| Toolchain | `rust-toolchain.toml`, else `rust-toolchain` |
| Cargo settings | `.cargo/config.toml`, else `.cargo/config` |
| Lints | `clippy.toml`, else `.clippy.toml` |
| Formatting | `rustfmt.toml`, else `.rustfmt.toml` |
| Editor settings | `.editorconfig` |
| Git attributes | `.gitattributes`: what is kept in Git LFS, and other attributes |
| `.gitignore` | grouped by its comment banners (a `# ====` line next to a titled comment) when it has them, else at each comment after a blank line |
| Data shapes | every `.json` file that declares `$schema`: its title, type, definitions, required keys, formats and `$ref` count |

Evidence: each fact names its file and line; a setting found through ⌘K opens there.

Cache: none.

Failure modes: a missing or unreadable file has no card. The source is never Unavailable.

## Tools

The commands a project defines, found by reading files, never by running anything. The Run district, the Dock and the commands in ⌘K.

Files read:

- Cargo binaries and examples, from the packages.
- Cargo aliases: `[alias]` in `.cargo/config.toml`, else `.cargo/config`.
- `package.json` scripts, in file order. The runner is `pnpm run` when a `pnpm-lock.yaml` is beside it or in a folder above, else `yarn run` for a `yarn.lock`, else `npm run`.
- Makefile targets (`Makefile`, `makefile`, `GNUmakefile`), read without running make. Pattern rules, variables and `.PHONY` lines are not targets. The description is a `##` comment on the line, else the comment line above.
- justfile recipes (`justfile`, `Justfile`, `.justfile`), with the comment above as the description.
- GitHub Actions workflows (`.github/workflows/*.yml` and `*.yaml`) and their jobs, read with yaml-rust2.
- clap subcommands of Rust binaries that use clap's derive API, nested up to 4 levels: `#[derive(Parser)]`, `#[derive(Subcommand)]` and `#[derive(Args)]` items in the `.rs` files of the binary's package and the packages it depends on.

Each tool records where it is defined (file and line), its description and its exact invocation: `cargo run -p <package> --` (or `--bin <name>` when the package has several binaries, or the alias when one runs exactly that package), `cargo run -p <package> --example <name>`, `cargo <alias>`, `npm run <script>` (with `cd <folder> &&` outside the root), `make <target>`, `just <recipe>`. CI workflows and jobs have no invocation.

Rules for clap:

- A binary's parser is the one in its own source file, else the one it calls with `::parse` or `::try_parse`, else, for `src/main.rs`, the package's only parser outside `src/bin`.
- A variant's name is `#[command(name = "…")]`, else the variant in kebab case. `#[command(skip)]` variants are left out.
- Known weakness: a subcommand enum is looked up by its bare type name over the dependency closure, the binary's own package first. Two enums with the same name in different modules or packages collide, and an enum re-exported under another name is not found. The path resolver will replace this lookup (roadmap, [S9]).

Evidence: the tool's file and line come from the file that defines it. Line numbers for aliases, scripts and jobs are the first line naming them.

Cache: none.

Failure modes:

- Nothing found: "No tools found: no Cargo binaries or aliases, npm scripts, make, just or workflows".
- An unreadable file only loses its own tools; the source is never Unavailable.

## Git

Branches, worktrees, changes and history. The Changes district, the title bar's worktree pill, and the last commit on each folder of the map.

Commands for the history:

1. `git rev-parse --is-inside-work-tree`
2. `git for-each-ref --format=%(refname:short)%1f%(objectname)%1f%(upstream:short)%1f%(upstream:track)%1f%(committerdate:unix)%1f%(subject) refs/heads` (local branches, with ahead and behind)
3. `git worktree list --porcelain`, then in each worktree folder:
   - `git status --porcelain=v2 -z --untracked-files=normal`
   - `git diff --numstat -z -M --no-relative HEAD` (changed files with + and −)
4. `git log --no-show-signature --format=%H%x1f%h%x1f%an%x1f%ct%x1f%P%x1f%s%x1f%(trailers:key=Co-authored-by,valueonly,separator=%x1d)%x1e -n5000` (HEAD only, newest first, with `Co-authored-by` trailers)
5. `git rev-list --count HEAD`

Files of one commit, read when asked for: `git show --numstat -z -M -C --diff-merges=first-parent --no-show-signature --format= <commit>`.

The last commit per folder: `git rev-parse --verify -q HEAD`, then `git log --name-only --no-renames -z --no-show-signature --format=%x1e%H%x1f%h%x1f%ct%x1f%s HEAD` over the whole history (120 s timeout). Each folder gets the newest commit that touched a file under it. Merges list no files.

Rules:

- The first worktree git lists is the main one.
- Commit ids are 40 or 64 hex digits (SHA-1 or SHA-256).
- The history holds at most 5,000 commits; the commit count is git's own.
- A project inside a repository marks only the folders under it.

Evidence: commits, refs and file changes are what git reports.

Cache: the files of each commit, as `commit-<id>.rkyv` (format version 1, keyed by the commit id). A commit never changes, so the cache never goes stale.

Failure modes:

- No repository: Unavailable, "No git repository here". The Changes district says so; if agents worked in the folder, their sessions still show in one "project folder · no git" row with that note.
- A repository with no commit yet: `git log` fails, so the history reads "Git could not be read: *git's message*".
- Git failed: "Git could not be read: *reason*".

## Agent sessions

What coding agents did in the project, read from Claude Code's session logs. The Changes district's session lanes, the sessions pill, plan cards on the Desk, and edits lit on the map.

Files read:

- `$HOME/.claude/projects/*/*.jsonl` (main logs) and `$HOME/.claude/projects/*/<session>/subagents/*.jsonl` (subagent logs). Every folder is scanned: folder names are lossy, so they never decide anything.
- Plans: the file an `ExitPlanMode` call names, else `$HOME/.claude/plans/<slug>.md`.

Rules:

- A session belongs to the project when the canonical `cwd` its lines carry is inside the project, or inside the project's folder in another worktree of the same repository (symlinked spellings included). The longest root wins.
- A log is decided by its first `cwd`; when that is outside every root, by the first later `cwd` inside one. A subagent log follows its session's main log.
- Sessions in a parent folder of the project are not mapped: such a session may have worked anywhere below it.
- Indexed: `Read`, `Edit`, `MultiEdit` and `Write` calls with the lines they touched, edits a Bash command reported, `ExitPlanMode` plans, and `TaskCreate`/`TaskUpdate` steps in main logs.
- Kept: paths, ids (as hashes), slugs, branch names, times, line numbers and byte offsets into the logs. Never prompts, file contents, commands, patches, plans or task text. Text a plan card shows is read from the log when the card opens, and never cached.
- A session is shown by its slug, else the first 8 characters of its id, and its start time.
- An edit attaches to the plan step that was in progress when it was made, when the session recorded steps; otherwise edits are listed in order.

Evidence: everything from a session log is Observed: it says what an agent did, not what the code is. When a plan card opens, each edit is checked against the file as it is now: found exactly once, it is lit (Observed); found at several places, it is a Possible set; not found, it is stale.

Cache: `agents.rkyv` (format version 2). Its key is a fingerprint of the Claude folder, the roots and the options. Each log is remembered with its length, modification time and a hash of its first 4 KB; a log that only grew is read from where the last read stopped, and one that changed otherwise is read again from the start. A cache that cannot be written only costs time on the next run.

Failure modes:

- No Claude folder, no `projects` folder, or no session mapped: Unavailable, "No agent sessions for this project".
- A worktree with none of the sessions: "No sessions in this worktree".

## Tickets

Studio's own tickets, an optional adapter. The Changes district's tickets column.

Files read: `.ai/tickets/T-*.toml`, directly in that folder. Nothing is cached.

Rules:

- `id` and `status` are required; `title`, `kind`, `priority`, `order`, `parent`, `shipped_at`, `spec` and `plan` are optional. A file that does not parse is listed by name with why, never with its text.
- Tickets sort by the numbers in their id. Next up: `ready` before `queued`, then priority, order and id.
- `shipped_at` is checked with `git cat-file --batch-check`, fed `<id>^{commit}` per ticket.
- A branch name links to the tickets it names: exact `T-<n>(.<n>)*` tokens, case-sensitive (`slice/T-940.11` names T-940.11; `XT-9` and `t-5` name nothing).

Evidence:

| Link | Tier | Basis | Drawn |
|---|---|---|---|
| ticket → the commit git confirms for `shipped_at` | Proven | commit id | solid outline |
| branch → ticket named in the branch | Unresolved | branch name | dashed and muted, "matched by branch name · unresolved" |

A `shipped_at` git does not know reads "commit not in this repo"; when git could not be asked, "not checked".

Failure modes:

- No `.ai/tickets`: Unavailable, "No tickets here (.ai/tickets)".
- Only unreadable files: "*n* ticket files could not be read".

## Pipeline

Routes, senders, handlers, contracts and the flows through them. The Pipeline district.

Inputs: the packages, the tools and the project's files. It runs no command. Rust code is read through the path resolver (below).

### Route tables

Axum route tables are evaluated from the code, only in library and binary module trees: never `tests/`, benches, examples or `#[cfg(test)]` code.

- Router functions are those whose return type resolves to `axum::Router`, plus serve sites (`axum::serve(…)`, `.into_make_service*()`). `Router::new()` must resolve to axum's.
- Followed: `.route(path, m)`, `.nest(prefix, r)`, `.merge(r)`, layers and state; `let` bindings and reassignment; another router function's result.
- An `if` makes the routes inside it conditional, with the condition as written (`!(cond)` in the `else`). Any `#[cfg(…)]` other than `test` does the same.
- Method routers: `get(h)`, `post(h)`, …, `any(h)` and `on(MethodFilter::X, h)`, only when they resolve to `axum::routing`.
- Anything else that touches a router (a loop, a macro, a closure, an unknown call) is listed as skipped with its line and why. Nothing is guessed.

### Tags

`@route` and `@contract` tags count only inside a comment of a code file, as the first word of the comment line. Markdown, data and config files never carry tags.

- `@route <METHODS> <TEMPLATES>`: methods and templates separated by `|`, e.g. `// @route POST /api/v1/items/{id}`.
- `@contract <file>[#<pointer>] [partial]`, e.g. `// @contract item.schema.json#/definitions/Item`.
- A malformed tag is kept with what is wrong with it and makes no link.
- Route templates unify by shape: `:id` and `{id}` are the same parameter, names never matter, and a trailing `/`, query or host changes nothing. Wildcards and partial segments (`{a}-{b}`, `:id.json`) never unify.

### Contracts

A contract tag names a file and a JSON pointer.

- The file is found by exact name, or by path suffix when the reference holds a `/`. No file: Unresolved ("no file named …"). Several: a Possible set.
- One file: the pointer must exist in it (RFC 6901, taken as written, no percent-decoding). Found: Proven, at the pointer's line. Not found: Unresolved ("pointer … not found in …").
- `#/` stays Unresolved: in RFC 6901 the pointer `/` names the member whose key is the empty string `""`, not the whole document. The whole document is `#` (an empty pointer). So `file.json#` is Proven at line 1, and `file.json#/` is Proven only in a document that has a `""` key; otherwise it says "pointer `/` names the empty key; whole document is `#`".

### Links

| Link | Tier | Basis |
|---|---|---|
| route → its handler | Proven when the handler resolves to one item; else Unresolved | route table |
| router function → the routes it mounts | Proven | route table |
| sender (`@route` tag) → route | Proven when exactly one route matches and its handler resolves; Possible set when several match; else Unresolved, to a step made from the tag | route table (tag when Unresolved) |
| function → function it calls | Proven, only for calls the path resolver resolves to one function, outside `#[cfg]`-gated code | path resolution |
| step → contract | as in [Contracts](#contracts) | JSON pointer |

The pipeline makes no Observed links. Two links between the same steps keep the stronger tier; at equal tiers an unconditional one wins, else the conditions join with `||`.

### Flows

- Entry points: each route, each binary's `fn main`, each command-line command and Cargo alias with a known line.
- A flow walks forward from its entry over links at Possible set or better, at most 12 layers and 400 steps; senders that reach the entry come first.
- A flow crosses languages when a sender in one language reaches a route handled in another. Contracts are data and never count as a crossing.
- Groups, in this order: Cross-language (open), Endpoints (one group per router file), Programs, Commands, Entry points with no resolved links. All but the first start closed.

Cache: none.

Failure modes:

- Nothing found: "No entry points found — no route tables, no @route tags, no programs."
- The job could not start: "The pipeline could not be read: *reason*".

## Parts and part facts

What the map's folder cards say about each part, worked out on the loader thread after a scan (`studio_parser`), then marked with what the sources read.

- Files and tests inside the folder at any depth. Rust tests are counted by attribute, the way the harness finds them: `#[test]`, `#[tokio::test]`, `#[rstest]`, `#[wasm_bindgen_test]`, `#[quickcheck]`, and one per `#[test_case(..)]`, at any depth, including inside macro calls like `proptest!`. Comments and strings never count. Python and Go tests are counted by their runners' names (`test*` functions in `test_*.py` and `*_test.py`; `Test*` functions in `*_test.go`).
- The folder's three largest file classes, with the tool convention that decided any of them: "12 code, 3 test files (tool convention: Cargo), 1 docs". The classes come from [CLASSIFICATION.md](CLASSIFICATION.md).
- The first sentence of the folder's README (badges, headings and HTML skipped, at most 200 characters), as what the part is for.
- The last commit that touched it: short id and age ("1a2b3c4 3 d ago"), from git.
- A package's folder is marked "crate *name*", with its binaries: "crate *name* · 2 binaries".

### Code map wires

| Wire | Tier | Basis |
|---|---|---|
| Markdown link to a file that exists | Proven | doc link |
| Rust `use` or path call the path resolver resolves to exactly one item | Proven | path resolution |
| package → package it depends on (dependency wire) | Proven | manifest |
| any other import or call matched by name | Unresolved | name match |

**The path resolver (R1)** resolves Rust paths through modules, `use` and re-exports, the way the compiler does. It leaves these Unresolved, keeping the name match:

- ambiguous names: two globs that provide it, or a `use` naming items in two files;
- macros: `macro_rules!` names, and names a macro in the module or statement may define;
- `#[path]` modules, and modules whose file is missing or does not parse;
- method calls, and `Self::` in a trait or a blanket impl;
- anything reached through a `#[cfg]`-gated item, variant, import, module or dependency, and any reference written in `#[cfg]`-gated code;
- a glob whose base cannot be resolved, and a `pub(in path)` item seen through a glob from outside its module;
- paths through an associated item or a type alias, and `Self` outside an impl.

A path into another crate is not drawn: if the name match had linked it to a workspace file, that wire is dropped. Test code (`#[cfg(test)]`, `#[test]`) is not read by the resolver and keeps its name-match wires.

**Dependency wires** come from the manifests:

- One wire per pair of packages joined by a path dependency in `[dependencies]` or `[build-dependencies]`, including `[target.'…'.dependencies]`, optional entries and `workspace = true` entries.
- The wire runs from the dependency's `Cargo.toml` card to the dependent's, and its line is the dependency's entry in the dependent's manifest.
- Dev-dependencies make no wire: a wire could not tell that only the package's tests use the other. Registry and git dependencies make none either.
- A `Cargo.toml` inside a heavy folder loaded later gets no wire until the next full load.

The View menu's **Dependencies** toggle shows or hides them.

## Classification

Every file gets one class from one table, and heavy folders are found by markers and gitignore only. See [CLASSIFICATION.md](CLASSIFICATION.md).
