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
