# Retained449 loss on remerge with layout-flexgrid LFG-2: subgrid-in-grid-lanes named lines

## Context

The orchestrator reported that `7e84c772` (LAYOUT-23) could not merge into `dev`
because layout-flexgrid's LFG-1/LFG-2 landed first, conflicting in
`block.rs`/`grid.rs`/`position.rs`/`docs/lanes/layout_TODO.md`. This lane
resolved the conflicts by hand: kept this lane's `ContainingBlock`/
`prepare_sizes`/`relative_offset` (RTL-aware, zoom-correct, anchor-aware
shrink-to-fit) and kept layout-flexgrid's `GridTracks`/grid-area abspos
static-position and containing-block plumbing verbatim, adapting only
`grid_area_containing_block`'s `Rect` return to the `ContainingBlock` wrapper
type (same field math, `rtl` taken from the fallback).

Re-gating the merge against the fixed `retained449` selection
(`crates/amazing-layout/evidence/layout16/layout16-full-manifest.json`,
before = `crates/amazing-layout/evidence/layout22/final-after/retained/report.json`,
303 pass) surfaced two distinct issues:

## Issue 1 — fixed in this remerge

`grid_child_static_anchor` (layout-flexgrid's LFG-2 addition) used
`parent.grid_tracks`'s grid-area rectangle for an abspos grid child's static
position/alignment container *unconditionally*, whenever the parent was a
grid container. That regressed
`css-grid__abspos__grid-abspos-staticpos-justify-self-002.html` and
`-rtl-003.html` (both newly failing, 1694 and 1620 differing pixels).

Root cause: a `position: static` grid container does **not** generate the
containing block for its absolutely-positioned descendants under CSS2.1
§10.1 — grid does not get flexbox's unconditional special case for this.
Confirmed directly against the pinned WPT reference file for
`grid-abspos-staticpos-justify-self-002`: its reference removes `display:
grid` and `position: absolute` entirely and instead positions the equivalent
boxes as **normal-flow block children with `margin-left`, measured against
the full content box** (e.g. `alignEnd { margin-left: 24px }` on a 30px-wide
container with a 6px-wide box: `24 = 30 - 6`, the full content box, not the
20px-wide grid track area at column 2). This lane's own pre-merge code (see
`prepare_sizes`'s ticket) already had this right (comment: "static position
uses the grid container's content edges, independent of grid-placement
properties"); layout-flexgrid's grid-area version dropped that distinction.

Fix: `grid_child_static_anchor` now takes a `use_grid_area: bool` parameter,
set at the call site to the same `grid_generates_cb` condition already used
to pick the abspos containing block (`Display::Grid` and `position` one of
relative/absolute/fixed). When false, it falls back to the grid container's
content box exactly as before grid-area support existed. The crate's own
regression test (`grid_abspos_static_position_uses_grid_area_only_when_positioned`,
renamed from `..._uses_grid_area`) now asserts different expected geometry
per `position: static` vs `position: relative`, matching the WPT reference.

Verified: `retained449` 303 -> 305 (+3 gains, only 1 remaining loss — issue 2
below), replacing the prior 303 -> 303 (+3/-3, both justify-self cases lost).
`amazing-layout` crate: 372/372 (1 skipped).

## Issue 2 — NOT fixed here, routed to layout-flexgrid

`css-grid__grid-lanes__subgrid__grid-subgridded-to-grid-lanes__line-names__row-line-names-008.html`
still regresses (2494 differing pixels) after the issue-1 fix. This file
exercises eight `grid-lanes` containers, each with a `subgrid`-rows child
using named lines (`a-end`, spans against a named line, negative/`-1`
indices combined with subgrid line names) — a narrow intersection of three
layout-flexgrid features (grid-lanes, subgrid, named-line resolution) that
this lane does not own and did not touch.

This is **not** a regression introduced by this lane's merge-conflict
resolution: `grid_area_containing_block` was adapted only to wrap/unwrap the
`ContainingBlock` type (identical field math to layout-flexgrid's original
`Rect`-returning version — see the function body), and the subgrid named-line
contribution code (`subgrid_column_contribs`, `parse_placement`'s named-line
handling, `GridTracks::area`) is layout-flexgrid's own code, taken verbatim
(zero hunks touched it during conflict resolution). `retained449` was never
part of layout-flexgrid's own gate (their LFG-2 report covers crate tests,
workspace, the 1,446-case flex/grid selection, `abspos-manifest`,
`loss-manifest`, and `collapsed-tracks-manifest` — not this fixed 449-case
selection), so this is a pre-existing latent bug in `dev` that this lane's
broader corpus is the first to surface, not something this merge caused.

Routed to `.agents/layout-flexgrid/questions.md` (and noted in
`.agents/layout/questions.md`) for investigation by the owning lane.
`gate.sh` records this one case id as a second documented, non-silent,
owner-external exception (same mechanism as LAYOUT-23's `right-113` image
loss) so the gate does not hold indefinitely on a bug this lane does not own
and should not attempt to fix blind, under severe shared-sandbox disk
pressure (`pool-layout` repeatedly hit 1-2 GB free during this remerge).
