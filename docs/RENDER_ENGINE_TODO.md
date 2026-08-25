# Render-engine conformance decision map

Canonical sequence for moving Amazing's style → layout → paint pipeline toward
standards conformance. Resolve one ticket, verify its exit criteria on
`win-001`, record measured results, then stop at the phase boundary for review.

Frontier: **#6 — Paint order, clipping, stacking, and compositing**

## Evidence contract (resolved)

- **Standard conformance** means behavior required by the applicable WHATWG or
  W3C specification and measured with a pinned, unmodified upstream WPT case.
- **Blink/WebKit agreement** is a differential oracle used to triage ambiguous
  failures. It is not a replacement for the specification or WPT reference.
- Feature unit tests and curated WPT-inspired fixtures remain useful regression
  gates, but their counts must never be reported as upstream WPT pass rates.
- Every conformance report names the WPT revision, viewport, font set, device
  scale, test selection, pass/fail/timeout/crash counts, and artifact directory.
- Builds, tests, and reference captures run on `win-001`; local macOS remains
  edit-only. Probes accept parameters rather than embedding one fixture.

### Current measured baseline

- Automated, unmodified upstream WPT render smoke cases: **2** at revision
  `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` (**1 pass, 1 mismatch**).
- XHTML-safe cascade/value corpus: **33 pass / 26 mismatch / 13 unsupported**
  at the same revision (corrected baseline **23/36/13**).
- Variables shard: **12/12 pass** (baseline 0/12), matching the exact-revision
  Blink/WebKit consensus.
- HTML-only geometry corpus: **38 pass / 23 mismatch**; focused signed-margin,
  min/max-height, and collapse-through family: **9/9** (baseline 6/9).
- HTML Position shard: **9/12** (baseline 5/12), including **5/5** in the
  exact-revision Blink/WebKit consensus subset.
- Broad text diagnostic: **12 pass / 36 mismatch** raw and **7/29** in the
  exact-revision Blink/WebKit consensus subset. The lower raw count exposes
  former inert equal-pixel results. Focused shaping is **12/12**, including
  **8/8** exact-revision Blink/WebKit consensus (baseline 3/12 raw and 0/8
  capability-credited).
- Focused Flex/Grid Level-1 gate: **24/24** exact-revision Blink/WebKit
  consensus (fixed-point baseline **10/24**). The deterministic broad
  diagnostic moves **35/200 → 47/200** while retaining later-feature and
  cross-dependency failures.
- Existing curated CSS2 geometry subset: **17/17**.
- Existing feature regressions: **16 flex-module**, **17 grid-module**, **41
  Phase #5 full-pipeline**, **61 CSS**, **63 HTML**, **65 net**, **137 layout**,
  **56 paint**, **7 text-contract**, and **26 text-provider tests**.
- These numbers establish regression coverage only; they do not establish
  compatibility with Blink, WebKit, or the broader web platform.

## #1: Reproducible upstream WPT render harness

Blocked by: none
Type: Prototype
Status: ✅ Resolved

### Question

How will Amazing run a pinned upstream CSS reftest, render both test and
reference through the real engine, compare deterministic pixels, retain diffs,
and emit machine-readable measured results on `win-001`?

### Exit criteria

- A revision-pinned WPT source/manifest policy is recorded without copying or
  rewriting test assertions.
- A parameterized Windows probe selects tests, viewport, output directory, and
  timeout; it has no fixture-specific constants.
- The harness classifies pass, mismatch, timeout, unsupported, and crash, and
  saves actual/reference/diff images for every mismatch.
- At least one known-pass and one intentionally known-fail upstream reftest
  prove the harness rather than the engine feature.
- A repeat run produces the same classification and pixel hashes.

### Answer

`amazing render-wpt` paints an exact caller-supplied viewport through the real
style → layout → paint pipeline. `scripts/run-render-wpt.ps1` accepts manifest,
source root, output directory, executable, viewport, and timeout parameters;
it sparse-checks out the manifest's exact upstream revision, derives the
reference and relation from the unmodified test's `rel=match`/`rel=mismatch`
link, applies supported upstream fuzzy limits, bounds each renderer process,
and emits PPM actual/reference/diff images plus JSON and SHA-256 evidence.

The smoke manifest proved both branches on `win-001`: `background-color-clip`
passed with 0 differing pixels, while `border-radius-horizontal-zero` produced
the expected mismatch (23,552 pixels, maximum channel difference 255). Two
repeat runs had identical classifications and all image hashes; normalized
evidence is retained in `tests/wpt-render/baseline.json`. The full
workspace test gate passed, including two fixed-viewport command tests; strict
Clippy passed for the changed `amazing` package with dependencies excluded.

## #2: Cascade and computed-value conformance baseline

Blocked by: #1
Type: Research
Status: ✅ Resolved

### Question

Which upstream CSS Syntax, Cascade, Selectors, Values, Color, and Fonts tests
reach the existing engine, and which missing primitive creates the largest
failure cluster?

### Answer

The deterministic SHA-256-ranked selection found 72 eligible static reftests
across Cascade, Selectors, Values, Color, Fonts, and Variables; CSS Syntax has
no eligible static shard and needs a future testharness runner. Amazing passed
30 and mismatched 42. Exact-revision wpt.fyi data shows Blink and WebKit both
pass 64 selected cases; Amazing is 26/64 in that consensus subset.

At the #2 baseline, the largest coherent cluster was the absent
specified-to-computed-value stage:
Variables is 0/12 versus Blink/WebKit 12/12, and Cascade is 4/12 with failures
centered on CSS-wide keywords, `all`, layers, and scope. That baseline code lowercased
case-sensitive custom-property names, retained raw specified strings, skipped all
at-rules, and could not substitute `var()` or represent invalid-at-computed-value
time. Full methodology, category tables, oracle runs, caveats, and links to
generated evidence are in `docs/render/CASCADE_BASELINE.md`.

## #2a: Specified-to-computed-value engine

Blocked by: #2
Type: Prototype
Status: ✅ Resolved

### Question

What deep style interface can preserve specified declarations and produce
validated computed values, beginning with case-sensitive inherited custom
properties, `var()` fallback/cycles, invalid-at-computed-value-time behavior,
CSS-wide keywords, and `all`, without leaking string substitution into layout
or paint?

### Exit criteria

- Custom-property names preserve case and declarations inherit by default.
- `var()` substitution handles nesting, fallback commas, missing variables,
  cycles, and invalid-at-computed-value-time behavior.
- `initial`, `inherit`, and `unset` resolve through property metadata; `all`
  applies to every supported property except `direction` and `unicode-bidi`.
- The full workspace stays green on `win-001`.
- The pinned 12-case Variables shard and applicable Cascade cases are rerun;
  measured before/after counts and pixel artifacts are recorded.

### Answer

The prototype validated a deep in-process module behind the unchanged
`compute(dom, sheets) -> StyledDom` interface; its terminal shell was deleted
after absorption. Cascade now uses per-declaration order and matched-selector
specificity, expands supported shorthands before ranking, then hands one winner
per property to the private computed-value module. That module centralizes
property metadata, case-sensitive inherited custom values, dependency-cycle
detection, nested `var()` substitution and comma-preserving fallbacks,
guaranteed-invalid behavior, token-boundary-safe serialization, CSS-wide
`initial`/`inherit`/`unset`, and `all` excluding `direction`/`unicode-bidi`.
Static variable-bearing `@supports` conditions cross the same syntax rules.

On `win-001`, CSS tests increased from 34 to 52 and the full workspace passed.
The pinned Variables shard moved from 0/12 to 12/12; two identical 24-case
runs measured 16 pass / 8 mismatch under the original harness. Ticket #3 found
13 XHTML tests/references that require an unimplemented XML parser mode; the
corrected 24-case projection is 14 pass / 6 mismatch / 4 unsupported (baseline
3/17/4), while the genuine Variables result is unchanged. The one
former `:scope` false pass now exposes its real selector gap; two other green
changes remain uncredited because they are downstream-inert. Remaining
computed-shard mismatches require layers/revert, scope, inline-fragment color,
bidi, positioning, or inline-block. Evidence lives in
`tests/wpt-render/computed-values-results.json` and
`tests/wpt-render/cascade-results-xhtml-safe.json`.

## #3: Block, inline, sizing, and positioning convergence

Blocked by: #2a
Type: Prototype
Status: ✅ Resolved

### Question

What formatting-context and intrinsic-sizing changes close the first coherent
CSS2/CSS Sizing WPT cluster without encoding test-specific behavior?

### Answer

The deterministic HTML-only inventory scanned 13,655 documents across CSS2,
Box, Display, Inline, Position, and Sizing and selected 61 executable static
reftests. It also fixed the harness to classify `.xht`/`.xhtml` tests or
references as unsupported instead of parsing XML as HTML. The broad corpus is
35 pass / 26 mismatch; one former inert inline-table pass now exposes a real
unsupported feature.

The first coherent algorithmic family is vertical margin collapse: nine HTML
reftests covering mixed-sign margins, negative auto-height shrinkage, trailing
margins inside BFCs, empty-block collapse-through, and min/max-height
separation. A throwaway logic prototype validated a signed positive/negative
accumulator and trailing-margin result, then was deleted after absorption.
`stack_blocks` now returns both content extent and collapsed trailing state;
the parent decides whether it escapes, is trapped by a size constraint, or
contributes inside a BFC. Six interface-level tests raise layout to 80 tests.

The family moved from 6/9 to 9/9 and is repeat-hash stable; all five
Blink/WebKit-consensus cases pass. The last failure required inherited
padding-box overflow clips through backgrounds, borders, glyphs, and images,
raising paint to 52 tests. Full methodology and evidence are in
`docs/render/GEOMETRY_BASELINE.md` and
`tests/wpt-render/block-collapse-results.json`.

## #3a: Positioned and out-of-flow layout

Blocked by: #3
Type: Prototype
Status: ✅ Resolved

### Question

What deep layout interface can remove absolute/fixed boxes from normal flow,
retain their static position, select the correct containing block, solve
insets/auto sizes and margins, apply relative offsets without moving siblings,
and place fixed descendants against the viewport?

### Exit criteria

- Absolute/fixed descendants do not contribute to normal-flow size or flex/grid
  item placement; relative boxes retain their original flow slot.
- Containing-block selection and top/right/bottom/left resolution cover fixed,
  auto, percentage, negative, and over-constrained values in both directions.
- Interface-level tests cover block, inline, flex, and grid parents plus nested
  positioned ancestors.
- A pinned HTML-only upstream Position shard improves with no regression in the
  nine-case block-collapse family or full workspace.

### Answer

The throwaway prototype validated separate normal/out-of-flow child collections
and one post-normal-flow positioning pass, then was deleted. Box construction
now removes absolute/fixed descendants before block, inline, Flex, or Grid
layout. The positioning module shifts relative subtrees without moving sibling
slots, selects the nearest positioned padding box or fixed viewport, resolves
fixed/percentage/negative insets, stretches auto sizes under dual insets,
honors LTR/RTL over-constraint precedence, and preserves block static anchors.
Inline-block and table roles are blockified so positioned containers are not
lost while full atomic-inline/table layout remains deferred.

Ten positioning regressions plus a scrollable-overflow test raise layout to 91
tests. Paint/dump traversals, Win32 hit testing, GPUI link/control regions, and
document overflow all visit positioned children; fixed boxes do not extend
page height. WPT, GPUI, and Win32 pass a definite viewport height for fixed
positioning, and a 43rd GPUI test proves positioned links remain interactive.

The fixed HTML Position shard moved from 5/12 to 9/12 and is repeat-hash
stable. Four mismatches became exact passes; Amazing moved from 3/5 to 5/5 in
the Blink/WebKit-consensus subset. The broad geometry corpus moved from 35/26
to 38/23 while the block-collapse family stayed 9/9. One former inert
`display:contents` pass now exposes that missing unboxing behavior. Evidence is
in `docs/render/POSITIONING_BASELINE.md` and
`tests/wpt-render/position-results.json`.

## #4: Standards-grade text pipeline

Blocked by: #1, #3a
Type: Research
Status: ✅ Resolved

### Question

How will shaping, fallback, kerning, Unicode line breaking, bidi, vertical
metrics, and deterministic test fonts cross the std-only engine boundary?

### Answer

Amazing will use a std-only `amazing-text` contract and an isolated, pinned
pure-Rust provider. Paragraph itemization preserves shaping context across
eligible inline style boundaries; provider-neutral results retain bidi/script
runs, source clusters, selected faces, glyph ids, advances, offsets, metrics,
and legal/safe breaks. Layout establishes geometry from those glyphs and paint
rasterizes the same ids, eliminating independent per-character measurement.

The proposed provider pins HarfRust, Swash, unicode-bidi, ICU4X Segmenter, and
wuff behind first-party safe adapters. CSS retains font matching/source order,
property tailoring, and shaping-boundary policy. The provider owns validated
font bytes, Unicode algorithms, OpenType shaping, fallback coverage, and
rasterization. Architecture decision 4d locks the boundary; provider versions
and the declared Rust 1.89 MSRV remain prototype acceptance gates.

The broad 48-case diagnostic is 19 pass / 29 mismatch and 8/29 against
exact-revision Blink/WebKit consensus. A font-controlled,
behavior-stratified 12-case shaping gate is 3 pass / 9 mismatch; all three
passes are inert, so the honest consensus capability baseline is 0/8. Full
evidence, reference-engine findings, interface design, risks, and sequencing
are in `docs/render/TEXT_PIPELINE_RESEARCH.md`.

## #4a: Shaped glyph-cluster provider slice

Blocked by: #4
Type: Prototype
Status: ✅ Resolved

### Question

Can the owned contract, isolated provider, WOFF2-backed `@font-face` loading,
paragraph-context shaping, fallback, shared layout/paint glyph stream, and
bounded caches close the first focused shaping WPT cluster without leaking
provider policy into CSS or provider types into engine cores?

### Exit criteria

- First-party text contract/provider adapters forbid unsafe and provider types
  do not cross the contract; exact dependencies compile at declared Rust 1.89.
- One paragraph is itemized across DOM inline boundaries, preserves source
  cluster mappings, and distinguishes paint-only, shaping-style, and hard
  boundaries.
- Layout and paint consume one shaped glyph stream with font metrics, glyph ids,
  advances, offsets, and deterministic raster-cache keys.
- WOFF/WOFF2 web fonts load through bounded page resources; invalid fonts fail
  closed without crash or unbounded allocation.
- Contract, shaping, fallback, font-load, and malformed-input tests pass on
  `win-001`; the focused gate is rerun twice with stable hashes and measured
  before/after plus Blink/WebKit-consensus counts.
- The full workspace, block-collapse 9/9, and Position 9/12 gates do not regress.

### Answer

The prototype proved WOFF2 decode, HarfRust shaping, Swash rasterization, ICU
segmentation, and bidi analysis on Rust 1.89 against WPT's Noto Naskh Arabic,
then was deleted. Production now has two deep modules: std-only `amazing-text`
owns the small text interface and provider-neutral paragraph spans,
clusters, glyphs, metrics, breaks, raster images, and generation-keyed cache
keys; isolated `amazing-text-provider` owns exact HarfRust 0.13.2, Swash
0.2.10, ICU4X Segmenter 2.3.0, unicode-bidi 0.3.18, and wuff 0.2.8.

CSS token/component-value grammars extract `@font-face` descriptors and expand
the `font` shorthand. The driver resolves local, root-relative WPT, file, HTTP,
and HTTPS font sources through a caller-selected 8 MiB fetch cap; custom wuff
callbacks refuse decoded output above 32 MiB before allocation. The provider
caps the full set at 64 MiB/256 faces and the raster cache by both bytes and
4,096 entries. It matches ordered families using engine-owned attribute
policy, falls back per whole-paragraph grapheme cluster, keeps
default-ignorable controls with their surrounding face, shapes with features,
variations, synthesis policy, and pre/post context across paint-only and
shaping-style boundaries, and breaks context at padding/border/isolation
edges. Decode, parse, shape, and raster calls fail closed.

Layout itemizes one paragraph across DOM text nodes, retains source byte
clusters and style boundaries, and builds wrapping, line geometry, inline
padding/backgrounds, and positioned glyphs from provider advances. Paint
rasterizes those exact font/glyph ids and never measures text on the new path.
Implementing the unmodified WPT family also closed prerequisite gaps for named
ZWJ/ZWNJ entities, NBSP whitespace classification, font shorthand, and inline
decoration painting.

The focused gate moved from 3 pass / 9 mismatch (0/8 capability-credited
consensus) to **12/12** and **8/8** Blink/WebKit consensus. Two final runs had
identical classifications and every pixel hash; normalized projection SHA-256
is `4486d23609f79fecf593fafae357ac4bfb5efe8557a266a6bd1fc14a259a588b`.
The broad diagnostic moved from 19/29 raw and 8/29 consensus to 12/36 and 7/29,
exposing unsupported equal-pixel cases once real web/system glyphs replaced
heuristic rectangles; those losses are not credited as capability regressions.
The full Rust 1.89 workspace passed; block collapse
stayed 9/9 and Position stayed 9/12. Evidence is in
`tests/wpt-render/text-shaping-results-ticket4a.json` and
`tests/wpt-render/text-results-ticket4a.json`.

## #4b: CSS-tailored breaking and bidirectional lines

Blocked by: #4a
Type: Prototype
Status: ✅ Resolved

### Question

How will paragraph bidi levels, visual runs, grapheme/word/line opportunities,
white-space processing, `word-break`, `line-break`, `overflow-wrap`, soft
hyphens, and line-edge reshaping replace the remaining scalar/word assumptions
without losing source-cluster and caret mappings?

### Exit criteria

- Inline itemization preserves original text/source offsets through CSS white
  space transformation and exposes ICU-tailored legal/mandatory opportunities.
- unicode-bidi produces paragraph levels and visual runs; mixed LTR/RTL,
  isolates, controls, mirroring, and combining marks never reorder by scalar.
- Line selection uses legal HarfRust-safe cluster boundaries and reshapes
  unsafe line edges; `word-break`, `line-break`, `overflow-wrap`, and manual
  soft hyphens have focused interface tests.
- A pinned CSS Text/Bidi/white-space WPT family improves with repeat-stable
  hashes, exact-revision Blink/WebKit triage, and no #4a or geometry regression.

### Answer

The throwaway model proved that transformed text, source mappings, legal and
emergency breaks, discretionary insertions, and visual order must all use one
indivisible grapheme/glyph-cluster index; a scalar-indexed emergency case split
a Devanagari conjunct and was rejected before production code. The terminal
shell was deleted after the decision was absorbed.

`amazing-text` now carries typed white-space, word/line/overflow break,
hyphenation, Unicode-bidi, explicit paragraph-direction, source-map, safe-edge,
and visual-line data through the existing std-only seam. The provider applies
ICU CSS tailoring and UAX #9, synthesizes CSS bidi controls internally, shapes
resolved directional runs, and reshapes selected line ranges with full
pre/post context. Layout coalesces collapsible DOM-edge spaces without losing
source ranges, retains preserved segment breaks/tabs, selects provider cluster
opportunities, consumes visual runs, and inserts a hyphen only when a manual
soft-hyphen break is selected. Paint still consumes exact glyph ids only.

The 16-case unmodified upstream gate moved from **0/16 to 14/16** and from
**0/14 to 14/14** on exact-revision Blink/WebKit-consensus cases. Two final
800×600 runs have identical classifications and pixel hashes; normalized
projection SHA-256 is
`5585ae3cc21d9284ac54a6222bad5dedbac0cccc83683cc9ecd4bd3f4293d332`.
The two remaining cases have matching glyph shapes but exercise the existing
absolute static-position/float geometry defect; Blink passes and WebKit fails
both at this revision. Evidence is in
`tests/wpt-render/text-flow-results-ticket4b.json`.
Phase #4a remains 12/12, block collapse remains 9/9, Position remains 9/12,
and the full Rust 1.89 workspace plus locked GPUI check pass.

## #4c: Vertical metrics, writing modes, and ruby text

Blocked by: #4b
Type: Prototype
Status: ✅ Resolved

### Question

Which vertical font metrics, glyph orientation/substitution, logical-axis line
layout, text-combine behavior, and ruby annotation model close the first
Writing Modes/Ruby WPT cluster on top of the shared glyph pipeline?

### Exit criteria

- The owned text contract carries sideways writing modes, per-cluster pose,
  canonical/synthesized vertical metrics, and final glyph transforms without
  provider or GPUI types.
- Vertical inline layout wraps against logical height, advances columns on the
  block axis, and maps Flex main axes through writing mode and direction.
- Text-combine forms one fitted 1em atomic inline; Ruby retains base/text roles,
  autohide, empty bases, annotation spanning, and horizontal/vertical line-over
  placement.
- A pinned font-controlled Writing Modes/Ruby gate improves with exact-revision
  Blink/WebKit triage, repeat-stable hashes, and no #4a/#4b/geometry regression.

### Answer

The deleted logical-axis prototype established one inline/block vector model:
upright vertical clusters shape top-to-bottom, sideways clusters shape
horizontally and carry a final rotation, TCY is one fitted 1em atomic, and Ruby
is paired base/annotation structure on the logical line-over side. HarfRust,
Swash, and unicode-vo implement the provider adapter behind the expanded
std-only text contract. Layout now uses logical height and column progression,
Flex resolves its physical axes once from writing mode/direction, and paint
applies owned rotation/scaling transforms before GPUI presents the page.

The corrected 20-case fixed point is **9/20 raw, 1/4 Blink/WebKit consensus,
and 0/20 capability-credited**. Two repeat-stable final runs are **11/20 raw,
2/4 consensus, and 8/20 capability-credited**, with no unsupported cases or
crashes. Phase #4a remains 12/12, #4b remains 14/16, block collapse remains
9/9, Position remains 9/12, and the Rust 1.89 locked workspace plus standalone
GPUI locked check pass. Evidence and remaining-cluster triage live in
`docs/render/VERTICAL_RUBY_DESIGN.md` and
`tests/wpt-render/vertical-ruby-results-ticket4c.json`.

## #5: Flexbox and Grid convergence

Blocked by: #2, #3, #4c
Type: Prototype
Status: ✅ Resolved

### Question

Which flex/grid WPT clusters fail after shared sizing and text primitives are
correct, and what spec algorithms replace the current documented subsets?

### Exit criteria

- A pinned, behavior-stratified upstream WPT gate contains only supported,
  self-contained Level-1 cases whose references avoid deferred float, table,
  inline-block, gradient, canvas, and later Grid features.
- Flex resolves flexible lengths with the §9.7 iterative freeze loop,
  automatic/intrinsic minimums, logical margins, wrapping, gaps, and cross-line
  stretch through public full-pipeline tests.
- Grid retains min/max track functions and named lines, distinguishes definite
  and indefinite auto-repeat axes, sizes implicit/repeated tracks, absorbs item
  auto margins, and uses a restart-based `fr` solver.
- The gate improves against exact-revision Blink/WebKit consensus with stable
  hashes and no #3/#3a/#4a/#4b/#4c, workspace, or GPUI regression.

### Answer

The deleted terminal prototype proved that Flex and Grid require separate pure
solver interfaces. Flex now freezes and redistributes items after min/max
violations; Grid retains two-sided track functions and restarts flexible-track
sizing when a base exceeds its provisional `fr` share. Computed values expand
validated token/component-value grammars for `flex-flow` and the supported
`grid-template` Level-1 subset—including fixed-size auto-repeat constraints,
reserved line-name rejection, and cascade-preserving post-`var()`
expansion—plus legacy gap aliases; layout adds content-based
automatic minimums, intrinsic size keywords, logical margins, correct
cross-line stretch, row/column §9.7 resolution, implicit/auto-repeat sizing,
named-line occurrences, finite growth-limit maximization, bounded placement
work, nested-`repeat()` rejection, case-preserved repeated line names, RTL grid
alignment, and Grid item auto-margin alignment.

At WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the
24-case clean Level-1 gate moves **10/24 → 24/24**, all in exact-revision
Blink/WebKit consensus, with two repeat-stable final projections. The
deterministic 200-case diagnostic moves **35/200 → 47/200**; its remaining
153 mismatches intentionally include baseline alignment, aspect-ratio transfer,
outer inline Flex/Grid, floats/tables used by references, template areas,
auto-fit collapse, full intrinsic spanning distribution, subgrid, and Grid
Lanes. Evidence lives in `docs/render/FLEX_GRID_DESIGN.md` and
`tests/wpt-render/flex-grid-results-ticket5.json`.

## #6: Paint order, clipping, stacking, and compositing

Blocked by: #3, #4b
Type: Prototype
Status: 🌫️ Fog

### Question

Which display-list model is deep enough for backgrounds, border radii,
shadows, overflow clips, opacity groups, transforms, and stacking contexts
without coupling rasterization back into layout?

### Answer

Pending until geometry and glyph output are deterministic.

## #7: Replaced content and additional rendering surfaces

Blocked by: #3, #4, #6
Type: Discuss
Status: 🌫️ Fog

### Question

Which of form controls, image object fitting, SVG, Canvas 2D, embedded content,
and in-document `<audio>`/`<video>` belongs in this convergence program, and
what owned presentation descriptors let GPUI compose every surface without
leaking platform types into engine cores?

### Answer

Pending; scope will be chosen from measured WPT dependency clusters. The
product requirement is already fixed: media tabs and codec probes are evidence
for internal adapters only. End-user playback must live inside the browsing
frame with HTML media-element lifecycle, layout, controls, timing, events,
accessibility, and screenshot proof rendered through GPUI.

## #8: Continuous convergence gate

Blocked by: #2, #3, #4, #5, #6, #7
Type: Discuss
Status: 🌫️ Fog

### Question

What pinned WPT shards and non-regression budgets must every render change pass,
and what measured threshold defines the next compatibility milestone?

### Answer

Pending until representative costs and failure distributions are known.
