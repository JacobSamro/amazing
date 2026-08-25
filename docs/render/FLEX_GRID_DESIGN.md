# Flexbox and Grid convergence design

Phase #5 asks which Flex/Grid failures remain after shared geometry and text
primitives are stable, and which specification algorithms should replace the
old one-pass subsets.

## Evidence selection

- WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`
- Viewport: 800×600 at device scale 1
- Fixed point: `c8c371d`
- Clean behavior-stratified gate: 24 static HTML reftests
- Fixed-point baseline: 10 pass / 14 mismatch / 0 unsupported / 0 crash
- Final: 24 pass / 0 mismatch / 0 unsupported / 0 crash
- Exact-revision Blink/WebKit consensus: 24/24
- Capability-credited movement: 10/24 → 24/24
- Stable final runs: `run-20260825T115503119Z-2748` and
  `run-20260825T115549381Z-7060`
- Final projection SHA-256:
  `e86e102590a6c34b9c424a4300ad1b50535df5099e174c5093dfe810ae71c30f`

The first 200-case deterministic diagnostic measured 19/100 Flex and 16/100
Grid passes. It intentionally sampled later features and cross-dependencies.
The final diagnostic is 31/100 Flex and 16/100 Grid, or 47/200 overall. Grid's
flat broad count is expected because most sampled Grid failures are subgrid or
Grid-Lanes cases; the clean Level-1 shard is the convergence milestone.

The reusable inventory now accepts include-path, exclude-path, and blocked-
content regexes. The gate excludes references using deferred float, table,
inline-block, gradient, embedded surface, subgrid, or Grid-Lanes behavior.
This prevents reference-renderer defects from masquerading as Flex/Grid
failures. Raw equal pixels at the baseline are not counted as new capability.

Exact browser oracles use Chrome 154 run `5182906107691008` and Safari 26.6
run `5141363439173632`. The applicable specifications remain authoritative:
[CSS Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/) and
[CSS Grid Layout](https://www.w3.org/TR/css-grid-1/).

## Prototype decision

The deleted Rust terminal prototype exercised Flex grow/shrink clamping and
Grid `fr` restart cases. One generic constraint solver was rejected:

- Flex needs flex base and hypothetical sizes, min/max clamps, grow factors,
  scaled shrink factors, outer chrome, total violation, and repeated freezing.
- Grid needs track bases, growth limits, flex factors, gaps, and a restart when
  a base is larger than its provisional flex fraction.

The durable result is two deep pure modules, `flex_algorithm` and
`grid_algorithm`. Parsing, intrinsic measurement, box layout, and physical
placement stay outside their interfaces.

## Implemented slice

- Flex §9.7 iterative freeze/distribute/clamp resolution in row and column
  axes, including infinite computed grow factors and partial shrink sums
  without NaN propagation.
- Modern `min-width:auto`, overflow-dependent automatic minimums,
  `min-content`/`max-content` constraints, and distinct preferred versus
  content size suggestions.
- Token/component-value validated `flex-flow`, `flex`, and supported Level-1
  `grid-template` shorthands and longhand track lists, including
  fixed-size/single auto-repeat grammar, reserved line-name rejection, and
  post-`var()` validation without losing cascade rank; validated legacy Grid
  gap aliases; and cascade-ordered logical margin mapping that preserves
  `auto`. Invalid shorthand or longhand winners emit no cascade candidates,
  so they cannot override earlier valid declarations.
- Auto main-margin absorption before `justify-content` in both the row and
  column axes, with placement steps taken from post-absorption outer sizes.
- Correct reverse-axis margin use, negative-margin outer sizes, one-time box
  chrome accounting, line fitting tolerance, and multi-line stretch.
- Grid `none` templates without phantom tracks; implicit track cycling;
  definite/indefinite row auto-repeat; fixed-track subtraction; `minmax()`
  floors plus finite growth-limit maximization; named-line occurrences through
  repeat with identifier case preserved; rejection of grammar-invalid nested
  `repeat()` before recursion; RTL inline alignment; bounded track/occupancy
  work; and item auto margins that do not contaminate track sizing.
- A separate restart-based flexible-track solver for `fr` tracks.

Forty-one public integration behaviors exercise the full HTML → cascade →
box-tree → layout path; private implementation changes are not the test seam.

## Remaining broad clusters

The phase gate is complete, not the full Flex/Grid corpus. Remaining broad
work is explicit: baseline alignment, transferred aspect-ratio sizing,
column wrapping and wrap-reverse, cross-axis auto margins, inline outer display
for `inline-flex`/`inline-grid`, template areas, auto-fit empty-track collapse,
full intrinsic contribution distribution across spanning tracks, subgrid, Grid
Lanes, and references blocked on float/table/inline-block/paint capabilities.
Those dependencies belong to later measured phases rather than being relabeled
as Phase #5 passes.

GPUI remains the sole production presentation layer. These algorithms emit
provider-neutral layout geometry; no GPUI or platform type enters the engine.

## GPUI presentation proof

The final release shell at
`target/release/amazing-shell.exe` (locked release build of
`crates/amazing-shell-gpui`, built 2026-08-25T12:16:20Z on the server's default
stable toolchain, deployed byte-identical) ran as PID 18460 in RDP session 2 —
attached to the console, so the desktop composited normally — and navigated to the pinned
`grid-auto-repeat-minmax.html` fixture. `PrintWindow` captured the unobscured
1096×768 shell window containing the 960×570 browsing frame at
`target/diagnostics/phase5-flex-grid-gpui.png`; the page shows the required
filled green square inside the production GPUI chrome. This visual proof
complements, and does not replace, the 24/24 WPT gate.
