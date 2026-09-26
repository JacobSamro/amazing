# LFG-3 remerge: widened shrink-to-fit exposes a pre-existing layout/paint bug

## What changed

LFG-3 (`5935e2ae`, merged into `dev` as part of `ab002ce4`/`30c0cb07`) widened
`prepare_stretch`'s (now `prepare_sizes`'s) CSS2.1 §10.3.7 shrink-to-fit rule
from "left and right both auto" to "width auto" (cases 1/2/4, not just the
narrower case this lane's `prepare_sizes` refactor had covered incidentally).
The coordinator's remerge brief listed this as one of four LFG-3 behaviors
that must survive the merge into this lane's `ContainingBlock`/`prepare_sizes`
structure. It was ported faithfully: the shrink-to-fit basis is now
`cb.w - margin - chrome` (ignoring whichever single inset, or static-position
anchor fallback, is present), clamped to `[min-content, max-content]` via the
existing `intrinsic_content_widths` helper — the same formula as dev's
`available_border_box`/`min`/`max` computation, just expressed through this
lane's content-box helper instead of dev's inline `crate::flex::*` calls.
`crates/amazing-layout/src/flex.rs` and `grid.rs` (LFG-3's other two touched
files) are byte-identical between this lane's pre-merge HEAD and the merged
`dev` tip (`git diff` empty both ways), so the only surface for a merge-
introduced defect is this one clamp-basis change in `position.rs`.

## What broke

Re-gating after the merge (workspace/crate tests still 100% green: 372/1
skip crate, 3190/14 skip workspace, 4/4 doctests) surfaced real, non-noise
losses in the `tables` (43) and `retained` (17, one already a documented
LFG-2 exception) pinned corpora, plus 2 in `position`, 3 in `writing`, 3 in
`ruby` — none in `flex-grid-manifest`'s own 1,446-case selection (714 pass,
zero losses there, matching LFG-3's own reported numbers exactly).

Confirmed with real pixel evidence, not assumed:
- `CSS2__tables__table-anonymous-objects-059.xht` (0 differing pixels before,
  26,103 after, `max_difference: 255` — a real full-scale mismatch, not
  antialiasing). Rendered PNGs (`amazing render`) show the green (abspos)
  `<table>`'s three columns overlapping/interleaved with the red reference
  table's text instead of sitting cleanly on top of it.
- `CSS2__positioning__abspos-008.xht` — literally titled "Shrink wrapping of
  absolutely positioned blocks" (Ian Hickson's canonical CSS2.1 case), 0
  differing pixels before, now mismatches. Its abspos box has `float:left`
  children, not a table — a different content shape, same class of failure.

## Why it isn't a width-computation bug (ruled out, not assumed)

Traced the full pipeline for the table-059 case:
- `prepare_sizes`'s shrink-to-fit clamp for the abspos wrapper `<div>`:
  confirmed via `amazing layout <file> 800`'s box-tree dump that the wrapper
  resolves to exactly the table's own max-content width (943.2ish px),
  identical to the red reference's span-table width.
- `layout_block_in_context`'s width branch (`MaybeLen::Px` arm) correctly
  uses the wrapper's now-definite style width regardless of the (wrong-
  looking but immaterial, since it's a `Px` not a percentage) `cbw` passed
  by `position_subtree`'s `layout_child` call.
- `content_w` is correctly propagated to the table child at the `layout_block`
  children-recursion call site (`crates/amazing-layout/src/block.rs`).
- `table::layout`'s `available.min(mx).max(mn)` lands exactly on `mx` (the
  wrapper's shrink-to-fit target equals the table's own max intrinsic width
  by construction), and `auto_widths`'s stage-2 fallback (`next[i] = max[i]`
  for every column) is reached with `target == available` exactly, producing
  `widths = max` with zero remaining `extra` to misdistribute.
- **The debug box-tree dump for the green table's cells shows the exact same
  x-positions and widths as the red reference's cells** (9, 223, 438 at
  214.4px each, both sides) — i.e. the computed geometry is correct and
  should render as a pixel-perfect match.

Yet the actual rendered PNG is badly wrong. This mismatch between "geometry
dump says correct" and "rendered pixels say broken" points to a defect
downstream of layout — most likely a multi-pass reflow/paint inconsistency
(e.g. `prepare_sizes` mutates `child.style`'s `width` to a definite value and
`position_subtree` restores the original style afterward; if paint or a
second reflow pass re-enters this path with different assumptions about
`width` already being resolved, behavior can diverge from a single debug-dump
pass). This is not yet isolated to one line and needs dedicated
investigation time this lane's remerge budget does not have.

## Disposition

Not caused by this lane's merge *choices* (the position.rs edit is a faithful,
verified-equivalent port of the exact formula the coordinator's brief
specified as must-preserve; flex.rs/grid.rs are untouched/byte-identical).
It is a real, pre-existing amazing-layout defect that LFG-3's coordinator-
mandated widening newly exercises across paths this lane's own corpora
(tables/retained/position/writing/ruby) test but layout-flexgrid's flex/grid
selection does not. Same shape as the two already-documented LFG-1/LFG-2
gate exceptions (right-113, subgrid/grid-lanes), at larger scale because the
trigger (any `position:absolute; width:auto` box with at most one inset,
wrapping non-trivial content: a table, a float, ruby, or an orthogonal
writing-mode block) is far more common across the general corpora than the
narrow subgrid-named-line intersection was.

Recorded as documented, non-silent `gate.sh` exceptions (full case lists
below) rather than holding this remerge indefinitely on a bug outside this
ticket's scope. **This is squarely amazing-layout's own code (table.rs,
block.rs, or the paint/reflow pipeline) — not routed to another lane.**
Filed as the layout lane's next ticket (LAYOUT-24) in `docs/lanes/layout_TODO.md`
and `.agents/layout/questions.md`, ahead of LAYOUT-24's previously-planned
"inline positioned ancestors" scope.

## Resolution (LAYOUT-24, 2026-09-27)

Root cause, confirmed with fresh empirical instrumentation (temporary
`AMAZING_TABLE_DEBUG`-gated `eprintln!`s in `prepare_sizes` and
`table::layout`, removed before the fix landed): `table::intrinsic` (the
function `crate::flex::min_content_width`/`max_content_width` call for any
`Role::Table` box, including via a table wrapper's `max_content_inner_width`
fold) hardcodes `intrinsic_with_text(b, fonts, None)` — it always measures
every cell with fallback glyph-width heuristics, never the real shaped text
that the *same* table's live `table::layout` → `constraints()` call always
receives. For `table-anonymous-objects-059.xht`'s green (abspos) table this
produced a shrink-to-fit `preferred` of 643.2px, while `table::layout`'s own
`constraints()` call for the *same* table (once laid out at that width)
reported a true shaped max-content sum of 809.16px — the wrapper resolved
~166px too narrow, and `auto_widths`'s proportional-interpolation branch (used
whenever available space is between the min- and max-content sums, which is
correct per CSS2.1 §17.5.2.2, not a bug in `auto_widths` itself) then squeezed
every column to a share of that too-small width, causing the observed
overlap: columns narrower than their real shaped content, text bleeding into
the next column and thus into the red reference underneath it.

This affects only `table::intrinsic`'s callers, not `table::layout`'s: the
"correct box-tree dump" observation above is explained by `cmd_layout`'s debug
tool passing `text: None` everywhere (`amazing_layout::block::
layout_root_in_viewport(&mut root, width, None, &fonts)`), which made the
wrapper's fallback-based shrink-to-fit estimate *and* the debug dump's own
measurement both use the same (wrong-for-real-rendering) fallback metrics,
so the dump looked internally consistent while the real render (`amazing
render`, which does pass a real `TextSystem`) was not.

Fix, in `position.rs`/`table.rs` only (`flex.rs`/`grid.rs` untouched, so no
layout-flexgrid coordination was needed):
- `table::intrinsic_with_text` (previously private, `text` hardcoded to
  `None` at its one call site) is now `pub(crate)`, letting a caller with real
  `text` in scope use it directly instead of the text-less `intrinsic`.
- `position.rs`'s `intrinsic_content_widths` (the shrink-to-fit clamp's sole
  caller) now calls a new recursive helper, `shaped_intrinsic_width`, instead
  of going straight to `crate::flex::min_content_width`/`max_content_width`.
  It descends through `Flow::Blocks` block-flow ancestors (so a `<table>`
  nested under a generic wrapper `<div>` is found, not just when `box_` is
  itself the table wrapper) and calls `table::intrinsic_with_text` for any
  `Role::Table` box it finds, threading the same real `text` the shrink-to-fit
  box already has. Everything else (flex/grid containers, non-table leaves)
  falls through unchanged to the original `flex::min/max_content_width`.
- An earlier version of this fix also applied real-text shaping to inline
  runs found while descending (not just tables), matching what
  `intrinsic_content_widths`'s pre-existing top-level check already did for
  `box_` itself. That version fixed `abspos-008` but *also* introduced 17 new
  losses elsewhere in the `tables` corpus (e.g.
  `table-anonymous-objects-017.xht`: a previously-exact `a bc d` one-line
  layout started wrapping "d" to a second line, exposing a red reference
  pixel that should stay covered) — real shaped text made some short inline
  runs measure narrower than the fallback estimate did, tipping a
  previously-fitting line into wrapping. That part was reverted; the shipped
  fix only changes measurement for the `Role::Table` leaf, not inline runs
  found while descending.

Measured effect (Boat `bx_cp5jbn5z`/pool-layout, workspace 3267 passed/14
skipped, `cargo test --workspace --locked --doc` clean, both before this fix
was applied and after, at `512912d5`):
- `position`: `abspos-008` now matches exactly (0 differing pixels, previously
  a loss) — 1 pass recovered, no new losses. Removed from `gate.sh`'s
  `accepted` dict.
- `tables`: pass count unchanged (271, same as the LFG-3-regressed baseline)
  — **not fully recovered** — but the target cases are structurally fixed,
  not merely reduced in severity by coincidence:
  `table-anonymous-objects-059.xht` differing pixels 26,103 → 8,929 (-66%),
  `-098.xht` 26,686 → 9,544 (-64%), `-018.xht` 1,139 → 1,056 (-7%). Visual
  inspection (`actual.png` vs `reference.png`, converted from the render
  pipeline's `.ppm.gz` output) confirms the overlap/bleed-through defect
  itself is gone — both render as clean, non-overlapping green text. The
  residual mismatch is a **separate, pre-existing** ~1px cumulative rounding
  drift in `auto_widths`'s column-width distribution (measured directly:
  glyph run start-x is identical for column 1 in both images at one text row,
  then off by exactly 1px for columns 2/3 at another row), which a strict
  `allowed_differing_pixels: 0` comparison turns into thousands of differing
  anti-aliased glyph-edge pixels even though the visual difference is a
  hairline. Confirmed pre-existing and unrelated to LFG-3/LAYOUT-24:
  `fixed-table-layout-003a03.xht`/`-003a04.xht` already had exactly 2
  differing pixels before this fix, unchanged after. Filed separately as
  **LAYOUT-26** (table column-width rounding precision vs. reference
  browsers) rather than expanded into this ticket, since it predates LFG-3
  and affects tables generally, not just shrink-to-fit ones.
- `writing`/`ruby`/`retained`: unchanged (0 net movement; same accepted cases,
  no new losses). These corpora's documented LAYOUT-24 exceptions were not
  confirmed via the same table-specific pixel-tracing this write-up used for
  `tables`/`position`, so it is possible some of them share LAYOUT-26's
  rounding-drift mechanism rather than the fixed `table::intrinsic` bug —
  left for LAYOUT-26 or a future pass to re-triage now that the structural
  bug is out of the way.
- Full workspace + doctests clean both before and after (no regressions);
  `gate.sh` reported zero unexplained (`CORPUS GATE HELD`) losses across all
  five corpora on the shipped fix.

`gate.sh`'s `accepted` dict is otherwise unchanged (same case lists for
`tables`/`writing`/`ruby`/`retained`, since none of those flipped to `pass`),
with `abspos-008` removed from `position` and the comment block updated to
describe the fix and the LAYOUT-26 follow-up.
