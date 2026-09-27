# layout-flexgrid lane decision map

Owns `crates/amazing-layout/src/flex*` and `grid*`: flexbox and grid layout
algorithms, split from `layout` (which keeps block/inline/table/position/floats
and the shared `box_tree.rs`/`values.rs`). Shared-file edits are routed through
`.agents/layout/questions.md`.

## LFG-1 — pinned flex/grid WPT convergence (in progress)

Selection: `crates/amazing-layout/evidence/layout18/flex-grid-manifest.json`,
**1,446 eligible static cases** (518 flexbox, 928 grid) at pinned WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800 × 600. No errors,
unsupported, crashes or timeouts.

### Baseline (source `6db18c89`)

`flex-grid-before.json`: **614 pass / 832 mismatch**. Root-cause families by
first two path segments (from `.agents/layout-flexgrid/analysis/cluster.py`):

| Family | Mismatches | Notes |
| --- | ---: | --- |
| `css-grid/grid-lanes` | 443 | Grid 3 draft; subgrid-to-lanes 102, auto-repeat 61, dense packing 33, alignment 111 |
| `css-grid/abspos` | 88 | static-position / self-alignment of absolutely-positioned grid items |
| `css-grid/alignment` | 36 | content distribution with collapsed tracks (24 now green) |
| `css-grid/subgrid` | 31 | writing modes, line names, auto-fill |
| `css-flexbox/balance` | 16 | flex-line balancing (`balance-*`) |
| flexbox singles | ~90 | wrap-reverse, baseline, table-as-item, aspect-ratio |

### Adopted base (lane/layout LAYOUT-18)

Flex/grid was split out of lane/layout mid-LAYOUT-18. This lane adopted the
committed LAYOUT-18 flex/grid work (`c2cfe242` WIP used sizing, `8ebd1dcb`
auto-fit/automatic repeat, `83e497a0` cyclic percentage/chrome contributions)
plus that lane's loss-repair working tree. Changes:

- A single `ActiveAxis` track-size pass (`src/grid_repeat.rs`) unifies ordinary
  grid and grid-lanes: auto-fit empty tracks collapse, `auto-fill` repeats by
  definite contribution, automatic repeat counts gaps, repeated line names merge.
- Intrinsic contributions count padding/border once; cyclic percentages
  contribute zero to intrinsic tracks; `minmax(auto)` maxima stretch.
- Used-size math consumes resolved `calc()`/`min()`/`max()` and `attr()` lengths.
- Cross axis honors writable-axis mapping before `wrap-reverse`.
- Grid Lanes fill-reverse packs and its stacking axis sizes like grid.

Measured on the pinned selection (harness, Linux debug renders): **614 → 656
pass**, 47 gains, 5 losses before the loss-repair pass. The retained 449-case
union moved 276 → 277 (1 loss) and the 17-case CSS-values selection 4 → 7 (0
losses). Loss-repair pass brings the crate tests to 302 passed / 1 skipped and
targets the five regressions:

```
css-flexbox/flexbox_rtl-flow-reverse.html
css-flexbox/flexbox_rtl-order.html
css-grid/grid-lanes/track-sizing/auto-repeat/column-auto-repeat-017.html
css-grid/grid-lanes/track-sizing/auto-repeat/row-auto-repeat-015.html
css-grid/grid-lanes/subgrid/grid-subgridded-to-grid-lanes/baseline/row-grid-lanes-container-baseline-subgrid-001.html
```

### Measurement tooling

Linux is the final gate (AGENTS Linux-first, 2026-09-26). `run-render-sweep-linux.py`
is the Linux twin of `scripts/run-render-wpt.ps1`: same manifest schema,
reftest/fuzzy handling and pass/mismatch classification, fetching pinned
test/reference files from `raw.githubusercontent.com`. Windows remains deferred;
Linux numbers are the gate until lifted.

### LFG-1 result (done)

Linux full selection, `$OUT/lfg-flexgrid/report.json`, source `d7621074`:

| Selection | Before | After |
| --- | --- | --- |
| flex/grid (1,446) | 614 pass / 832 mismatch | **688 pass / 758 mismatch** (+74) |
| LAYOUT-18 loss cases (6) | 6 pass | **5 pass** (the 6th needs CSS `stretch`) |
| collapsed-tracks group (24) | 0 | **24 exact** |

The five flex/grid regressions from the adopted LAYOUT-18 base all render
exactly; the sixth loss is a CSS-owned `stretch` parsing gap. Crate gate
`cargo nextest run -p amazing-layout` green.

## LFG-2 — absolute-positioned grid items (in progress)

`css-grid/abspos` is the second-largest family (88 mismatches; ~40 are the
`grid-abspos-staticpos-*` reftests). Two spec rules share the same geometry:

- **Static position (§10.2):** the spec text says an abspos grid child's static
  position is the container's content edges, but the pinned WPT references and
  both engines place it in the child's **grid area** when the axis has a definite
  `grid-placement`. Implemented by keeping resolved track geometry on the grid
  container (`GridTracks` in `grid.rs`), plumbing it through `block.rs` onto
  `LayoutBox::grid_tracks`, and letting `position.rs::grid_child_static_anchor`
  align within the area rect (content edges remain the fallback for unplaced
  children).
- **Containing block (§10.1):** when the grid container generates the abspos
  containing block, each child resolves insets and percentages against its grid
  area (`grid_area_containing_block`), padding box for an auto-placed axis. This
  drives `positioned-grid-items-*` and the `descendant-static-position` cases.

Covers self-alignment (start/center/end/self-start/self-end) and RTL; vertical
writing mode axis mapping and `last baseline` remain.

### LFG-2 progress (2026-09-26)

Two bugs blocked almost all of the abspos/grid-area work above from ever
taking effect, both fixed in this pass:

- **`grid-area` cascade override.** `grid-area` is not a registered CSS
  property (the cascade never expands it into the four longhand
  `grid-*-start`/`-end` properties the way it does `grid-column`/`grid-row`),
  but an unrecognized declared value still survives verbatim in the computed
  style. `parse_placement`'s `grid-area` fallback added a `row_base`/`col_base`
  from it, but the unconditional longhand-override checks right after it always
  fired too — `grid-column-start` etc. are real registered properties whose
  *initial* value is the string `"auto"`, so `style.get("grid-column-start")`
  is `Some("auto")` even when the author only ever wrote `grid-area`, clobbering
  the just-computed base. Fixed by skipping the longhand checks whenever
  `grid-area` is present (`crates/amazing-layout/src/grid.rs::parse_placement`).
  A second, unrelated bug in the same area: the crate test's `#first`/`#unplaced`
  selectors (plain `#id`) had *lower* specificity than the sibling
  `#grid>div` rule (id+type beats id alone), so the universal rule's
  `grid-area` always won — a test-authoring bug, fixed by matching
  specificity (`#grid>div#first`).
- **Shrink-to-fit width for unconstrained abspos children.** CSS2.1 §10.3.7 /
  css-position-3 §3.5.1: when `left`, `right` and `width` are all auto, the
  used width is shrink-to-fit (`min(max(min-content, available), max-content)`),
  not the whole containing block. `prepare_stretch` only handled the
  "insets both set → stretch" case; with neither inset set (the common case
  for a grid item's static position) the box fell through to `layout_child`
  treating the given containing width as its own auto width, over-stretching
  every unconstrained abspos grid child to its full track/grid-area size.
  Fixed by adding the shrink-to-fit branch, reusing `flex::min_content_width`/
  `max_content_width` (`crates/amazing-layout/src/position.rs::prepare_stretch`).

Also added: `GridTracks::reverse_x`/`reverse_y` record when Grid Lanes
`fill-reverse` packs the waterfall (auto-placed) axis from the far edge; an
out-of-flow descendant's static position on that axis (no real tracks exist
there, so `GridTracks::area()` returns `None`) is now mirrored the same way
real items are, instead of anchoring to the near content edge
(`css/css-grid/grid-lanes/abspos/*-oof-fill-reverse-*` and
`css/css-grid/grid-lanes/tentative/item-placement/*-out-of-flow-*`).

Measured on Linux (source `3e101baa` + this series), release binary,
`run-render-sweep-linux.py`:

| Selection | Before (`6db18c89`, pre-LFG-1) | After |
| --- | --- | --- |
| flex/grid (1,446) | 614 pass / 832 mismatch | **698 pass / 748 mismatch** (+84) |
| `css-grid/abspos` (113, static-position + self-alignment + RTL) | ~25 pass / 88 mismatch (clusters.txt) | **32 pass / 81 mismatch** (+7) |
| LFG-1 loss-repair set (6) | — | **5 pass / 1 mismatch** (the 6th is CSS-owned: `stretch` keyword parsing, `.agents/layout/questions.md`) |
| collapsed-tracks group (24) | 0 | **24 exact** |

Remaining `css-grid/abspos` mismatches cluster into: vertical-writing-mode /
orthogonal-container axis mapping (31 of 81 — not yet attempted, flagged
above), `last baseline` self-alignment (12 of 81 — not yet attempted), and a
general bucket (41 of 81, e.g. `positioned-grid-items-*`,
`descendant-static-position-*`) that likely shares more of the same root
causes as the two bugs above; not fully triaged this pass.

Crate gate: `cargo nextest run -p amazing-layout --locked` 303/303 (1 skipped
timing test), doctests 0/0. Workspace gate:
`cargo nextest run --workspace --locked` 2,752/2,752 (10 skipped), workspace
doctests 4/4. Both green on the lane's Linux sandbox (Linux-first, AGENTS.md).

## LFG-3 — css-grid/abspos remainder, round 2 (2026-09-26)

Merged dev at `a38fe2f0` (brings LFG-1/LFG-2 back in as `c78edbfc`) before
starting; base for this series is `93540eb0` (merge commit on
`lane/layout-flexgrid`).

Root-caused and fixed four more bugs, all in files this lane already owns or
was granted (`flex.rs`, `grid.rs`, `position.rs`):

1. **`GridTracks` positions go stale after any later shift of the grid
   container.** `GridTracks::col_positions`/`row_positions` are absolute
   physical pixel positions computed once during the grid container's own
   layout, but neither `flex::translate` (used for float placement, among
   other things) nor `position::shift_box` (used for `position: relative`
   offsets) updated them when the box itself moved afterward — every other
   sibling in a run of `float: left` grid containers (the WPT static-position
   test family's standard "put N variants in a row" layout) kept its *first*
   provisional position's tracks, desyncing every abspos child's grid-area
   containing block/static position from where the container actually ended
   up. Root-caused with a temporary debug print comparing `parent.content_box`
   against `tracks.col_positions` for two sibling floated `.container`s in
   the same test file — the second one's tracks matched the *first*
   container's origin, not its own. Fixed by shifting both position arrays
   alongside `content_box`/`border_box` in both functions. This is the
   highest-leverage fix in this series — most WPT static-position tests in
   this family float multiple `.container`s in a row.
2. **Box Alignment axis mapping for `align-self`/`justify-self` didn't
   account for the grid container's own writing mode.** `align-*` always
   targets the block axis and `justify-*` the inline axis (css-align-3 §5);
   `grid_child_static_anchor` hard-coded `justify→X`/`align→Y`, correct only
   for `horizontal-tb`. For a `vertical-rl`/`vertical-lr` container the block
   axis is physical X and the inline axis physical Y, so `align-self`
   controls X and `justify-self` controls Y, with `vertical-lr`'s block-start
   at physical left vs. `vertical-rl`'s at physical right. Rewrote
   `grid_child_static_anchor`/`static_align` (`position.rs`) to resolve which
   property drives which physical axis, and its Start/End polarity, from the
   container's `writing-mode` (and `direction` for whichever axis is inline).
   `left`/`right` also needed splitting three ways: invalid for `align-*`
   (not in its grammar — both keywords fall back to the unset default, never
   distinct Start vs. End, confirmed against the WPT reference), literal
   physical for `justify-*` on a physically-horizontal inline axis, and
   start/end-equivalent for `justify-*` on a non-horizontal one
   (css-align-3 §8.3). This alone does **not** fully fix the `vertWM`/
   `orthogonal` family — see "Known gap" below.
3. **`last baseline` synthesized to the wrong edge.** `static_align` read
   only the last word of the value (`"baseline"`), so `last baseline` was
   indistinguishable from plain `baseline` and both fell back to the
   start-edge default. Neither engine computes a real baseline for these
   fixed-size/empty boxes, so per css-align-3 §9's fallback alignment,
   `baseline`/`first baseline` should synthesize to the start edge and
   `last baseline` to the end edge — confirmed against the WPT reference,
   which literally names its CSS classes `alignStart`/`alignEnd`. Fixed by
   checking for a leading `last` before the `baseline` keyword.
4. **`prepare_stretch`'s width shrink-to-fit only covered "both insets
   auto."** CSS2.1 §10.3.7 cases 2 and 4 (exactly one of `left`/`right` set,
   `width` auto) also shrink-to-fit the width, solving the *other* inset from
   it — only case where both are set stretches. Widened the condition from
   `left.is_none() && right.is_none()` to just `width_auto` (the stretch
   branch above it already `else if`s past the both-known case).
5. **`parse_item_align` (grid.rs, normal in-flow item alignment, also reused
   for the abspos `-self` override lookup) never matched `self-start`/
   `self-end`**, silently falling back to `Stretch` — verified via a WPT
   reference file using an ordinary (non-abspos) grid item with
   `align-items: self-end` that stayed at the top instead of the bottom.
   Mapped both to `Start`/`End`; a fully orthogonal-aware resolution would
   need the item's own writing mode threaded through, which nothing calling
   this function currently carries, so this is the common-case default, not
   a complete fix.

**Known gap, not attempted:** the `vertWM`/`orthogonal` abspos family (30 of
the original 81) needs more than alignment-keyword polarity. Grid's own axis
convention is that **rows run along the row axis (parallel to the inline
axis) and columns along the column axis (parallel to the block axis)** —
for `vertical-rl`/`vertical-lr` those are physically swapped from the
`horizontal-tb` assumption baked into `layout_grid`'s row/column-to-physical
mapping throughout `grid.rs` (track sizing, positions, gaps, line names,
`justify-content`/`align-content`), not just into the abspos static-position
code in `position.rs`. Confirmed by hand-computing a `vertical-rl` test
case's expected grid-area rect two ways (physical-column-from-row-template
vs. the reverse) and finding our renderer's *internal* math self-consistent
under the current (wrong) row↔Y/column↔X assumption, while the correct
physical rect requires the swap. Retrofitting this into `grid.rs` touches
enough of the track-layout algorithm that it deserves its own ticket rather
than a rushed change late in this one; see `questions.md`.

**Also found but not fixed** (see `questions.md` for the exact repro): a
grid container with *zero* in-flow items (every child out-of-flow) computes
its own auto width from something far smaller than the containing block,
instead of filling it like an ordinary block-level box — affects
`descendant-static-position-*` (a positioned descendant nested one level
below the grid item wrapper) and some `positioned-grid-descendant-
containing-block-*` cases.

Measured on Linux, release binary, `run-render-sweep-linux.py`, same base:

| Selection | Before this series | After |
| --- | --- | --- |
| flex/grid (1,446) | 698 pass / 748 mismatch | **714 pass / 732 mismatch** (+16) |
| `css-grid/abspos` (113) | 32 pass / 81 mismatch | **48 pass / 65 mismatch** (+16) |
| LFG-1 loss-repair set (6) | 5 pass / 1 mismatch | **5 pass / 1 mismatch** (unchanged, still CSS-owned) |
| collapsed-tracks group (24) | 24 exact | **24 exact** (unchanged) |

Remaining `css-grid/abspos` mismatches (65): vertWM/orthogonal 30 (the known
gap above), last-baseline 6 (all `-vertWM-last-baseline-`/RTL-last-baseline
combinations that also need the axis-swap gap closed), rtl 4, other 27 (the
`descendant-static-position`/`positioned-grid-descendant-containing-block`
zero-in-flow-item width bug above accounts for 8 of these; the rest are
`positioned-grid-items-*` cases still blocked on the css lane's `place-items`
shorthand gap, filed previously).

Crate gate: `cargo nextest run -p amazing-layout --locked` 303/303 (1
skipped). Workspace gate: `cargo nextest run --workspace --locked`
2,949/2,949 (11 skipped, 1 slow), workspace doctests 4/4. Both green on the
lane's Linux sandbox.

Also updated `evidence/layout-flexgrid/run-render-sweep-linux.py` to render
into a scratch tempdir and only persist mismatch images under `--output`
(pass-case PPMs used to accumulate gigabytes per full-selection run on the
shared `pool-layout` sandbox, per the 2026-09-26 disk-pressure incident).

## LFG-4 — largest remaining mismatch cluster in flex/grid, in bulk (2026-09-27)

Merge base: `855b2ca9` (`dev`, picked up after LFG-3 merged as `ab002ce4`,
`layout`'s LAYOUT-23/24 re-merge as `512912d5`, and an unrelated
`amazing`-binary compile break the integrator fixed as `855b2ca9`). Per the
coordinator: stayed off `position.rs`'s shrink-to-fit path this round
(`layout` is separately fixing the tables loss it caused, as LAYOUT-24).

Clustered the post-merge flex/grid (1,446) mismatches by WPT subpath. The
largest bucket by far was `css-grid/grid-lanes` (427-428), inside which
`subgrid/grid-subgridded-to-grid-lanes` (102) was the single biggest named
cluster, followed by `track-sizing/auto-repeat` (54), `tentative/item-
placement` (34) and `item-placement/dense-packing` (33).

1. **Attempted first, kept but ineffective on its own:** `grid.rs`'s
   `intrinsic_lanes_width` (an atomic-inline Grid Lanes container's own
   width) summed *every* item's intrinsic width for a row-lanes container
   (rows explicit, columns the free waterfall axis) as if each item got its
   own column — correct only with exactly one row track. Rewrote it to
   mirror `layout_row_lanes`'s real per-row-lane packing (`choose_lane` /
   `parse_placement`) with intrinsic widths standing in for laid-out outer
   heights. Verified via `amazing layout` on
   `grid-lanes/subgrid/grid-subgridded-to-grid-lanes/row/grid-lanes-
   subgrid-001c.html`: container width went from 184.0 to 93.6 (closer to
   the reference's 62.8, since the reference's container legitimately uses
   a *different*, non-masonry algorithm that happens to produce the same
   pixels for that file's specific numbers — real progress, but a clean
   isolated before/after sweep (512912d5, with vs. without this one change)
   showed **zero tests flipped either way** across the whole 1,446-case
   selection, so it doesn't clear the fuzzy-match bar on its own for that
   cluster. Kept anyway (strictly more correct, zero regressions) and moved
   on rather than chase pixel-exact parity with a structurally different
   reference algorithm for the rest of this ticket's budget.
2. **Real fix, verified, broader than grid-lanes:** `place_row_flow`'s fully
   -automatic (both `row`/`col` auto) placement loop — used by *both* plain
   `display:grid` auto-flow and grid-lanes' explicit-column-track branch —
   wrapped to the next row only when the *start* column exceeded the wrap
   `limit` (`if c > limit`), not when the item's whole span would. A
   spanning item that didn't fit the remaining columns of the current row
   therefore overflowed into a spurious implicit column past the wrap
   boundary instead of wrapping, both overflowing the container and
   creating an extra (usually near-zero-width, since `repeat(auto-fill,…)`
   sizing is computed independent of it) implicit track. Fixed by computing
   `last_start = limit - (span - 1)` and wrapping once the start column
   would exceed *that*, in both the dense (rescan) and sparse (cursor)
   sub-branches. Verified via `amazing layout` on
   `grid-lanes/track-sizing/auto-repeat/column-auto-repeat-001.html`
   (`repeat(auto-fill,100px)` columns, a `grid-column:span 2` item that
   must wrap to a 3rd row): the spanning item moved from `(208,108)
   100×100` (wrongly overflowing into an implicit 4th column) to `(8,208)
   200×100`, now byte-for-byte matching the reference.

Measured on Linux (venus-003, release binary, `run-render-sweep-linux.py`,
base `855b2ca9`):

| Selection | Before this series | After |
| --- | --- | --- |
| flex/grid (1,446) | 702 pass / 744 mismatch | **709 pass / 737 mismatch** (+7, 0 lost) |

Both numbers measured at the same base (`855b2ca9`, lane edits stashed for
the "before" run) so the +7/0-lost figure is attributable entirely to this
ticket's two fixes, not to sibling-lane drift; a per-test diff of the two
`report.json`s confirms 7 gained (all `grid-lanes/track-sizing/auto-repeat`,
the `place_row_flow` wrap fix) and 0 lost.

Crate gate: `cargo nextest run -p amazing-layout --locked` 372/372 (1
skipped, venus-003). Workspace gate: `cargo nextest run --workspace
--locked --no-fail-fast` 3,279/3,279 (2 slow, 15 skipped); doctests all
`ok` (`amazing-js` 1/1, `amazing-html` 1/1, everything else 0/0). Both
green on venus-003.

Cross-lane note: mid-ticket, Boat exhausted its 200 sandbox starts/day and
the fleet moved to direct Linux hosts (`venus-001`/`venus-003`, `worker-2`,
`k-test`); host routing flapped a few times while the orchestrator tuned
`scripts/linux-run.sh` live, and one `dev` commit briefly broke the
`amazing` binary's build (`PageScript.node`, devtools/HTML lanes, fixed
upstream as `855b2ca9` before this ticket's own numbers were taken) — neither
was caused by or is owned by this lane; noted for the record, not filed as a
new question since both were already resolved by the time of the final gate.


