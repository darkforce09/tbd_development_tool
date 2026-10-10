# Visual language

How Studio draws a project: which way things flow, where ports sit, how wires travel, and what each colour means. These rules are the same for every project and every language, so once you can read one project you can read them all.

Everything here is deterministic. The same project always produces the same picture.

## Direction

A project reads **left to right, from providers to consumers**.

If `main.rs` uses something from `utils.rs`, then `utils.rs` provides it and `main.rs` consumes it:

```
[utils.rs] ──out → in── [main.rs]
  provides                 uses
```

Libraries and helpers end up on the left. The application's entry points end up on the right.

## Ports

| Side | Port | Meaning |
|---|---|---|
| Left, top | **Documentation port** (sky blue) | Documentation that describes this file or folder links in here. It sits above the code inputs because documentation is not a code input. |
| Left | **Inputs** | What this file or folder uses from elsewhere. |
| Right | **Outputs** | What this file or folder provides to others. |

Cards (files) and folders follow the same rules.

## Layout laws

**L1. Providers are left of consumers.** Inputs enter on the left and outputs leave on the right. If A feeds B, A is left of B. If C takes from A and feeds B, then A is left of C and C is left of B.

**L2. Nothing sits on a wire.** A card or folder never sits on top of a wire, and a wire never passes through, over or under a card or folder that it does not connect. Wires crossing other wires are kept to a minimum. Some graphs cannot be drawn without any crossings, so zero is not guaranteed.

**L3. Wires cross folder edges through gates.** A folder has **input gates** on its left edge and **output gates** on its right edge. A wire entering a folder always arrives at an input gate from the left and continues inside to the file it feeds. A wire leaving a folder always leaves a file, travels to an output gate and exits to the right. This holds even when the file sits at the bottom of a tall folder: the wire goes to the folder's edge first.

- **One input gate per distinct visible source feeding in.** If one outside file feeds three files inside, that is one gate that fans out inside.
- **One output gate per distinct visible source feeding out.** If one inside file feeds three files outside, that is one gate that fans out outside.
- **A closed folder stands in for its files.** Outside a closed folder, every wire from the files in it counts as coming from the folder: they share one gate per folder edge and one wire, labelled with how many file pairs it carries and drawn thicker the more it carries. Opening the folder splits it back into its files.
- **Documentation has its own gates.** They sit on the same edges (inputs left, outputs right) but at the top of the folder, in its documentation strip, one per documentation file whose links cross that edge. They are sky blue like documentation wires.

**L4. Loops are boxed.** When files depend on each other in a loop (A uses B, B uses A), "provider left of consumer" cannot hold. The files in a loop are grouped into a marked **cycle box** that is placed as one unit. Everything outside the box stays strictly left to right. Inside a box, wires that run backwards are routed around the cards, never through them. Since order cannot hold inside a box anyway, its contents are packed into a compact block of columns (in loop order, aiming at a 16:10 shape) instead of one long row.

**L5. Folders have three detail levels.**

| Level | Shows |
|---|---|
| 1. Minimised | One input gate and one output gate. |
| 2. Node view | Every gate, listed like the pins of a ComfyUI or Unreal node. The inside is hidden. |
| 3. Open | The same gates as level 2, with the wires continuing inside to the files. |

A project opens at its **top level**: the root folder is open and every folder in it is in node view, so you see the project's main parts and one wire per pair of them. Open a folder to look inside it; its own subfolders start closed. You move through a project one level at a time.

Changing a folder's detail level never reorders the folders around it: their wiring can change (a closed folder shares its gates, an open one splits them), but every folder keeps its columns and the order of its items. Neither does opening a card: the layout keeps every folder's columns and order and only moves things aside to make room. A full layout (opening the project, loading a folder, new wires) may order things afresh.

In node view the pins are listed top to bottom on each side: documentation pins first, then code pins, each in alphabetical order of the file or closed folder whose wires they carry, and each pin is labelled with that name. An open folder places the same gates level with what they feed, so the order can differ between levels; the set of gates is the same.

Every folder header has three buttons, one per level (minimised, node view, open); the current level is highlighted. Folders whose contents are not loaded yet (dependencies, build output, version control) start minimised.

**L6. Hubs are badges.** A file or folder that feeds at least half of the other connected items in its folder, and at least 8 of them, is a **hub** (a shared types file, a prelude). Its wires would say little ("everything uses it") and bury everything else, so they are not drawn or laid out. The hub shows a "used by N" badge instead, and its wires appear when it is selected. A hub still sits left of everything it feeds (L1).

**L7. Districts keep their places and never overlap.** Every project gets the same five districts, always in the same place: **Pipeline** north, **Files** west, **Code** in the centre with the **Desk** below it, **Run** east, **Changes** south (see [Districts](#districts) and [Desk](#desk)). Wires never leave the Code map.

## Districts

The map of the code (everything above) is one district of a larger world. The world is laid out around the Code map's bounds, which the layout engine owns: a district growing never moves the map, and the map growing moves the districts while the camera stays on the one it is at.

| District | Where | What it holds | Colour |
|---|---|---|---|
| Pipeline | north | the flows a run takes, across processes and languages | yellow `#FFD60A` |
| Files | west | the disk as it is: folders, sizes, ignore rules, settings | teal `#63E6E2` |
| Code | centre | every package and folder, and how they connect | blue `#0A84FF` |
| Desk | below Code | open files and plans, side by side | cyan `#64D2FF` |
| Run | east | tools, apps, builds, tests and the console | green `#30D158` |
| Changes | south | commits, worktrees and agents at work | purple `#BF5AF2` |

- Districts other than Code are drawn in their own units, scaled with the size of the map, so the world view stays in proportion for a project of any size and each district reads at 100% when the camera visits it.
- From far away each district shows its name at a constant size, and short labels in the gaps say how they relate ("feeds →", "makes →", "↑ runs as", "↓ changes").
- At a district, the others dim. Clicking a dimmed district goes there.
- A district with nothing to show says so plainly ("No git repository here"). It never shows sample content, and it never disappears or moves the others.
- The camera flies between districts in 0.85 s, zooming out on long trips so the way stays visible. Any wheel, pinch or drag stops it where it is. A flight that ends at 100% lands on whole pixels.

### Files

- The disk tree is in Finder order, flowed into columns. Each row ends with its marks, joined by " · ": **ignored**, **heavy** with its reason ("heavy: build cache"), **LFS**, **generated**, **vendored**, **hidden**, then its size. Example: "ignored · heavy: build cache · 2.0 KB".
- Ignored and hidden rows have dimmed names; the marks of an ignored row are in the district's teal. Something inside an ignored folder is ignored too.
- LFS, generated and vendored come from `.gitattributes` as git reads it, and mark files only. Hidden means the name starts with `.`. Heavy folders are listed with their totals but not opened (see [CLASSIFICATION.md](CLASSIFICATION.md#heavy-folders)).
- In the size map, ignored entries are grey; the rest are teal.
- "Tracked files by class" lists each class with its files and bytes and the rule that decided most of them; hovering a class lists all its rules.

### Changes

- Session facts come from agents' logs and are **Observed**: the session lane is headed "AGENT SESSIONS · observed", and each session chip's tip says "observed".
- A ticket named in a branch is **Unresolved** (matched by the branch name): its chip is dashed and muted, and its tip says "matched by branch name · unresolved".
- A ticket whose `shipped_at` commit git confirms is **Proven**: its card has a solid outline in the district's purple.

### Pipeline

- One lane per flow; steps run left to right by how far they are from the entry point.
- A lane shows at most 3 steps per column and 7 columns. The rest fold into a "+k more" card, in the column or at the lane's end.
- Every link has a chip naming its tier ("1 of n" for a Possible set); a route that exists only under a condition says "conditional".
- Groups of flows collapse; the district grows north to fit and the map never moves.

## Desk

The Desk is where files and plans are read. It sits below the Code map and is part of the world: it moves and zooms with the camera like everything else.

- Its text is real text: you can select and copy it.
- The navigator is on the left: Finder-like columns from a part down to a file. Open cards sit to its right, side by side, and the row scrolls sideways.
- At most 12 cards are open. Opening a 13th puts away the one opened longest ago.
- Below 45% zoom the Desk shows only outlines; its text comes back as you zoom in.
- Each card has an X (put it away) and a crosshair (show it on the map). A Markdown card switches between **Rendered** and **Source**.
- Links in Markdown open their file on the Desk, at the lines they name (`#L10`, `#L10-L20`); web links open in the browser.
- A plan card shows a session's plan and the edits the session made, under the step they belong to when the session recorded steps. It is **Observed**: it says what an agent did. An edit still found exactly once in the file is lit; found at several places it is a Possible set; gone, it is stale.
- Nothing on the Desk changes the map. Opening, closing or reading a card never moves a card, a folder or a wire.

## Placement of things that are not code flow

- Documentation is shown as a chip on the card it documents ("docs 2") by default. Its wires can be turned on in the View menu.
- Files with no code wires in their folder sit in a grid below the folder's code columns.
- Documentation wires and asset wires do not affect the order. Documentation cannot create fake loops or pull code out of order.
- Documentation wires follow L2 and L3 like code wires. Every folder has a documentation strip across its top, under its name. A documentation wire climbs from its file to the strip, runs along it on its own track, and comes down to the card it documents, entering that card's documentation port from the left. The strip grows when more documentation tracks cross the folder; that can move a folder's contents down, never reorder them.
- A wire between two parts of the same file is only drawn when that file's card is open.

## Colours

A wire takes the colour of the kind of thing it connects. Symbols listed on cards use the same colours, so a green wire leads to a green function.

| Kind | Wire | Symbol on a card | Colour |
|---|---|---|---|
| Call | calls a function | function, method | emerald `#34D399` |
| Type use | uses a type | struct, class | orange `#FB923C` |
| Implements | implements or inherits | trait, interface | violet `#A78BFA` |
| Import | imports or includes a file or module | module | slate `#94A3B8` |
| Documentation | documentation links to code | documentation port | sky blue `#38BDF8` |
| Asset | references an asset (planned) | | pink `#F472B6` |
| Dependency | a package depends on another (its manifest says so) | | slate `#94A3B8`, like imports until it has its own colour |
| | | enum | amber `#FBBF24` |

**A wire's line style is its evidence: solid only when proven.** Colour says what a wire connects; the line says how sure Studio is that the link exists.

| Evidence | Meaning | Line | Alpha | Chip (on hover) |
|---|---|---|---|---|
| Proven | resolved exactly (compiler rules, manifest, exact path, tag, route, pointer) | solid | normal | none |
| Possible set | one of a known set of candidates | long dash, 14 px on / 8 px off | normal | "1 of n" |
| Observed | seen happening in a run, trace or session log | round dots, every 7 px | normal | "observed" |
| Unresolved | matched by name only | short dash, 6 px on / 6 px off | × 0.6 | "unresolved" |

Dash sizes are in screen pixels and do not change with zoom. A dash pattern starts again at each corner of a routed wire. A wire drawn once for several links (a shared stretch, a bundle, a closed folder's wire) shows the strongest evidence among them. Colour-blind variants come later.

Dependency wires come from the package manifests, so they are always Proven and solid. Each runs from the dependency's `Cargo.toml` card to the dependent's, one per pair of packages; the View menu's **Dependencies** toggle shows or hides them.

## Limits

- Where many documentation files cross the same narrow gap, their vertical tracks share space and can overlap each other (never a code wire).

- Wire-on-wire crossings are minimised by heuristics, not guaranteed to be zero.
- In Rust, `mod.rs` files and their submodules often form loops, so much of a crate can end up in one cycle box.
- A column far taller than its folder's 16:10 target is split into side-by-side columns. Files in one column never wire to each other, so L1 still holds.
- A busy folder can have many gates when it is open. Closing the folders around it (they then share their gates) or minimising it folds them.
- A bundle carrying wires of several kinds is drawn in the colour of the kind it carries most; its count covers all of them.
- The hub rule is a fixed threshold. It is exact and repeatable, but it can call something a hub that you would not, or miss one.
- The layout is only as correct as the wires. Proven wires (Rust paths the resolver settles, package dependencies from manifests, documentation links) are right; Unresolved wires are name matches and may be wrong, and the layout follows them anyway until every language has a compiler-grade resolver (roadmap Phase 3).
