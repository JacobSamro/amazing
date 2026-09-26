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


