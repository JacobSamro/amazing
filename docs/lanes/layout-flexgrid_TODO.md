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
