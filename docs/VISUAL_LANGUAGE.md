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

- **One input gate per distinct outside item feeding in.** If one outside file feeds three files inside, that is one gate that fans out inside.
- **One output gate per distinct inside item feeding out.** If one inside file feeds three files outside, that is one gate that fans out outside.
- **Documentation has its own gates.** They sit on the same edges (inputs left, outputs right) but at the top of the folder, in its documentation strip, one per documentation file whose links cross that edge. They are sky blue like documentation wires.

**L4. Loops are boxed.** When files depend on each other in a loop (A uses B, B uses A), "provider left of consumer" cannot hold. The files in a loop are grouped into a marked **cycle box** that is placed as one unit. Everything outside the box stays strictly left to right. Inside a box, wires that run backwards are routed around the cards, never through them. Since order cannot hold inside a box anyway, its contents are packed into a compact block of columns (in loop order, aiming at a 16:10 shape) instead of one long row.

**L5. Folders have three detail levels.**

| Level | Shows |
|---|---|
| 1. Minimised | One input gate and one output gate. |
| 2. Node view | Every gate, listed like the pins of a ComfyUI or Unreal node. The inside is hidden. |
| 3. Open | The same gates as level 2, with the wires continuing inside to the files. |

Changing a folder's detail level never reorders the folders around it.

In node view the pins are listed top to bottom on each side: documentation pins first, then code pins, each in alphabetical order of the file whose wires they carry, and each pin is labelled with that file's name. An open folder places the same gates level with what they feed, so the order can differ between levels; the set of gates is the same.

Every folder header has three buttons, one per level (minimised, node view, open); the current level is highlighted. Folders open at level 3, except folders whose contents are not loaded yet (dependencies, build output, version control), which start minimised.

## Placement of things that are not code flow

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
| | | enum | amber `#FBBF24` |

All wires are solid lines for now. Line styles and colour-blind variants come later.

## Limits

- Where many documentation files cross the same narrow gap, their vertical tracks share space and can overlap each other (never a code wire).

- Wire-on-wire crossings are minimised by heuristics, not guaranteed to be zero.
- In Rust, `mod.rs` files and their submodules often form loops, so much of a crate can end up in one cycle box.
- A column far taller than its folder's 16:10 target is split into side-by-side columns. Files in one column never wire to each other, so L1 still holds.
- A busy folder can have hundreds of gates. Minimised view (level 1) is the way to fold them.
- The layout is only as correct as the wires. Until wires are compiler-verified (roadmap Phase 3), the layout correctly follows wires that may themselves be wrong.
