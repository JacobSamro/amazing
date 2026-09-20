# Layout lane decision map

## LAYOUT-1 — Floats and clearance (completed prototype; stopped at review boundary)

### Design

- `floats.rs` owns margin-box exclusion geometry: left/right placement,
  source-order top constraints, next-bottom search, side-specific clear and
  line-band queries. Ordinary blocks share a context; independent BFCs own one.
- `block.rs` removes floats from normal stack advancement, resolves shrink-to-fit
  with the existing intrinsic-width interface, and includes contained floats in
  auto heights. BFC border boxes avoid float bands over their full height.
- Clearance preserves locked decision 4b: stacking returns content extent,
  trailing collapsed margin and whether clearance prevents that group escaping.
  An empty cleared box still collapses with following empty siblings.
- `inline.rs` consults exclusion bands for legacy and shaped line placement,
  moves unbreakable first content below an obstruction, and restores full line
  width below float bottoms. Anonymous blocks inherit explicit zero line-height.
- Float-bearing shaped documents resolve text during block placement so following
  blocks see provider line heights. Float-free documents keep the existing path.
- Box construction blockifies floating inline elements while preserving Flex/Grid
  inner formatting contexts. No changes to form-control construction in this ticket.

Rules consulted: [CSS 2.1 float placement](https://www.w3.org/TR/CSS2/visuren.html#float-position),
[float widths](https://www.w3.org/TR/CSS2/visudet.html#float-width), and
[clearance and collapsing margins](https://www.w3.org/TR/CSS2/box.html#collapsing-margins).

### Measured evidence

Windows layout unit tests: **138/138 before → 150/150 after**. Twelve new
interface-level tests cover packing, margins, side-selective clear, BFC height
and isolation, line shortening/recovery in both text paths, overwide words,
shrink-to-fit, full-height BFC avoidance, zero line-height, cleared empty margin
groups, and floated inline-flex formatting.

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
All runs use an 800 × 600 viewport. Counts below are pass / mismatch; every
selection has zero timeout, unsupported and crash results.

| Selection | Before | After |
| --- | --- | --- |
| Broad geometry | 32 / 29 | 31 / 30 |
| Eight-case float shard | 2 / 6 | 6 / 2 |
| Block collapse | 9 / 0 | 9 / 0 |
| Position | 9 / 3 | 9 / 3 |
| Text flow | 14 / 2 | 14 / 2 |
| Focused flex/grid | 24 / 0 | 24 / 0 |
| Focused paint | 24 / 0 | 24 / 0 |

Geometry and float selections have two identical normalized pixel-hash
projections both before and after. The brief's historical 38/61 geometry count
does not reproduce with the unchanged current renderer (32/61 measured).
Broad raw regressions: `float-nowrap-3.html` and
`float-table-align-left-quirk.html`; gain: `float-nowrap-3-ref.html` (an upstream
mismatch reftest). These remain visible in the evidence, not reclassified.

Full Windows workspace: **737 passed, 0 failed, 3 ignored**, exit **0**.
Ignored tests require external HTTPS or an interactive D3D11 device. An initial
PowerShell `Start-Process -Wait` wrapper failed to return after the passing test
process exited; the cached workspace run was repeated through `cmd /c` to
capture exit 0. No local builds or tests were run.

Normalized results, selection, run ids and actual/reference/diff hashes:
[`layout1-results.json`](../../crates/amazing-layout/evidence/layout1-results.json)
and the seven sibling `*-results.json` files. The final executable is
`C:\Users\server\target\layout\layout1-final.exe`, SHA256
`4f18246d7fade784742391e576de16eeafaee4be89a6108daf67649c21a577e7`.
The lane rebased to `origin/dev` at `e9c1e21`; the integrator-owned lockfile
SHA256 is `aa0a0d8fd69000e161ad26ac31d2ea3a988152cce75fdcb4e1277ff8480b2d25`.
No root files or other lanes' source were edited.

The source checkout is lane-local at `C:\Users\server\target\layout\wpt`.
The shared WPT tree omitted most selected files; incomplete-source probe runs
are excluded. `crates/amazing-layout/evidence/run-layout1.ps1` reproduces the
selection, two repeats and the five regression gates with a pinned executable.

### Pre-merge review: the two raw losses were inert passes

Inspected the retained baseline/final **actual, reference and diff** PPMs for
both cases, verified their SHA256s against `geometry-results.json`, and viewed
lossless PNG conversions. The labeled comparison crops include all non-white
pixels; the full 800×600 conversions and original PPM hashes are retained in
[`evidence/review`](../../crates/amazing-layout/evidence/review/regression-review.json).
Raw WPT results remain mismatches; this classification explains lost credit
rather than changing the measured pass count.

1. **`css/CSS2/floats/float-nowrap-3.html` — previously inert pass exposed.**
   Reference: `css/CSS2/floats/float-nowrap-3-ref.html`.
   Before: test/reference have identical pixels **and zero blue pixels**: the
   empty inline float was absent on both sides. After: both paint the same
   **2,304 blue pixels** (48×48); the **781 differing pixels** are the text
   “my parent.”, which the test moves below the float while the reference keeps
   it on the preceding line. The old equality did not demonstrate float or
   nowrap conformance. The remaining inline-continuation defect is real
   unsupported behavior in the prototype, but this raw loss is not a regression
   from a working float implementation.
   [Retained image comparison](../../crates/amazing-layout/evidence/review/float-nowrap-3-comparison.png).

2. **`css/CSS2/floats/float-table-align-left-quirk.html` — previously inert pass exposed.**
   Reference: `css/CSS2/floats/float-table-align-left-quirk-ref.html`.
   Before: all eight digits are stacked like the un-floated reference. After:
   the **209-pixel diff** shows overlap/displacement where CSS floats now
   participate but legacy `align="left"` tables still behave as ordinary blocks.
   Three single-variable diagnostic copies were rendered with both preserved
   executables on win-001. Widening every containing block from 0 to 100px,
   disabling CSS floats, or replacing legacy align with CSS floats each leaves
   the **baseline byte-identical** to its original/reference image. Its supposed
   pass was insensitive to float semantics. With the new renderer, replacing
   **only** `align="left"` with `style="float:left"` restores **exactly** the
   retained reference hash; disabling CSS floats also restores it. This isolates
   the exposed missing legacy alignment/presentational-hint mapping, not a
   newly broken float-placement constraint. The wider-space probe now changes
   placement, as expected from active floats.
   [Retained image comparison](../../crates/amazing-layout/evidence/review/float-table-align-left-quirk-comparison.png),
   [probe images](../../crates/amazing-layout/evidence/review/table-probes.png).

All five invariant table probe outputs have PPM SHA256
`c7ba3123aba39ee1dec40df17c8b0e6240e5b3390d5695b15b407e7d92da7d0a`.
The final wider-space output is
`37727e10fd1cd17016affc81f03ba629b955050e064ad6560cc3c77a7e0149ca`.
The diagnostic copies are **not** WPT results. Reproduce them on Windows with
`crates/amazing-layout/evidence/review/reproduce-table-probes.ps1`;
the script checks the distinguishing hashes. No engine code change is needed
for this review classification. Missing legacy alignment mapping belongs in
cross-lane follow-up; the current raw result remains a mismatch.

### Baseline discrepancy for the integrator's shared decision map

`docs/RENDER_ENGINE_TODO.md:30` advertises current HTML geometry as
**38 pass / 23 mismatch**. That is also the historical ticket #3a result near
line 258. It is not the LAYOUT-1 starting baseline. The unchanged pre-float
renderer at the lane's `c50ee3f` starting point reproduces **32 / 29**, twice,
with **0 timeout / 0 unsupported / 0 crash**. This does not invalidate the
historical ticket #3a measurement; it means the current-summary number is stale.
No edit to that integrator-owned file was made.

Reproduction inputs:

- Unchanged executable `C:/Users/server/target/layout/baseline.exe`, preserved
  **before** syncing the float implementation; SHA256
  `c4c378513791e2d46071ac27e806a63da1eb4fe4c466b2675501a6f3b2096fc9`.
- Existing **unchanged 61-case** `tests/wpt-render/geometry-manifest.json` and
  WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` in the complete lane-local
  checkout `C:/Users/server/target/layout/wpt`; viewport 800×600, timeout 20s,
  runner's default font set (recorded in `geometry-results.json`).
- Run ids `run-20260920T163435252Z-11080` under `baseline-geometry` and
  `run-20260920T164122852Z-19128` under `baseline-repeat-geometry`; both live below
  `C:/Users/server/target/layout/evidence`. Both normalized projections hash to
  `e8c9dd377767cd3d657f951178a1291d8db742efe3760eadd2b019386bea94b7`.
- The initial incomplete shared-checkout probe had missing files and is excluded;
  both credited reproductions have zero unsupported cases.

Run the following from the Windows lane tree using PowerShell
`-ExecutionPolicy Bypass` (over SSH, pass it via UTF-16LE `-EncodedCommand`):

```powershell
foreach ($repeat in 1, 2) {
    & scripts\run-render-wpt.ps1 `
        -ManifestPath tests\wpt-render\geometry-manifest.json `
        -WptRoot C:\Users\server\target\layout\wpt `
        -ExecutablePath C:\Users\server\target\layout\baseline.exe `
        -OutputDirectory "C:\Users\server\target\layout\evidence\baseline-review-$repeat" `
        -ViewportWidth 800 -ViewportHeight 600 -TimeoutSeconds 20 `
        -SkipBuild -SkipFetch
}
```

Comparing committed `tests/wpt-render/geometry-results-positioned.json` with
our unchanged baseline confirms **the same 61 test ids**, with eight historical
passes now mismatching and two historical mismatches now passing (net −6).
Historical passes now mismatching:

- `css-box__margin-trim__block-container-block-end-collapsed-margins.html`
- `css-box__margin-trim__block-container-block-start-self-collapsing-item-has-larger-block-end.html`
- `css-box__margin-trim__block-container-block-start-self-collapsing-item-larger-block-start.html`
- `css-box__margin-trim__flex-block-end-trimmed-only.html`
- `css-box__margin-trim__flex-column-inline-multiline.html`
- `css-box__margin-trim__grid-block-end.html`
- `css-display__display-contents-sharing-001.html`
- `css-display__display-contents-td-001.html`

Historical mismatches now passing: `CSS2__floats__float-nowrap-3.html` and
`css-display__display-contents-fieldset-nested-legend.html`. These changes
predate LAYOUT-1. Their causes have not been bisected in this ticket.

### Prototype boundary / known limitations

- Mid-inline floats still split anonymous inline runs instead of resuming one
  source line. The float shard retains `float-nowrap-3` and
  `floats-line-wrap-shifted-001` as mismatches.
- The geometry corpus includes a floated table/quirks case whose former equal
  pixels change when float placement becomes active. Table formatting is still
  deferred; the pre-merge review above classifies the old pass as inert.
  Retain the raw mismatch, not supported-pass credit.
- Float shrink-to-fit uses the existing legacy intrinsic-width approximation;
  provider-shaped intrinsic widths, vertical writing-mode exclusion, exact
  inline static-position rectangles and broader table/inline-block behavior
  remain outside this prototype's supported slice.
- Do not equate the raw reftest pass count with complete CSS float conformance.

## LAYOUT-2 — Atomic inline replaced boxes (completed; stopped at review boundary)

### Contract and ownership decisions

`ReplacedElementProvider::intrinsic_size(NodeIdx)` returns **natural content
width and height only**. Neither HTML width/height attributes nor CSS used
sizes, borders or padding may be applied by the provider. Layout needs the
original image ratio when resolving an auto axis. The integrator explicitly
approved this contract during LAYOUT-2 and owns driver/shell migration.
`PageMedia` at the `26c64de` baseline violates it by returning
HTML-attribute-adjusted `used_size`; RC/shell providers can also return
CSS-adjusted `ImageSize`. These are integration violations, not alternate
contracts or a reason to infer natural dimensions from already-sized values.

The integrator approved the additive `LayoutBox::styled_replaced_fragments()`
view: control identity, computed style, content and border rectangles, resolved
padding/border/margin edges and computed font size. Existing `ReplacedFrag`
rectangles remain **content rectangles**. Consumers recurse through normal and
positioned children. Paint/control-adapter adoption is routed by the integrator
to replaced-content after merge. Layout does not copy checked/value/selected/
disabled state and does not import a platform control implementation.

### Implementation

Both direct and nested inline construction retain inputs (including text-like
variants, checkbox and radio), buttons, selects and textareas as atomic boxes.
Their descendants do not leak into surrounding text. Hidden and display:none
controls are omitted. Explicit inline-block replaced elements stay in inline
flow. Blockified/floating boxes retain the existing block path.

`replaced.rs` owns natural-size fallback, CSS used-size resolution, and the
paint-facing view. Control defaults are deterministic engine policy, not claims
of platform-native sizing: text input 20 × 0.6em by 1.2em, checkbox/radio 13px
square, button/select 5em by 1.5em, textarea 20 × 0.6em by 2 × 1.2em.
Positive size/cols/rows attributes override the default character/row counts
(capped at 10,000); an existing provider overrides natural defaults. Control
preferred dimensions do not create an image aspect ratio.

Width percentages and all padding/margin percentages use the full containing
block content width, not an inline wrapper or a float-shortened line. Height
percentages require a definite containing-block height; auto content heights
and min-height alone are not a basis. Natural image ratios, one-auto-axis
sizes, min/max constraints, explicit zero, and border-box sizing are resolved
before fitting. The CSS2.2 both-auto min/max rules preserve ratio except where
opposing constraints force distortion.

Both text paths fit and reorder the outer atomic extent, including its edges,
then convert final fragments to content rectangles. The additive view exposes
already-resolved decoration geometry, so paint does not redo layout. Repeated
layout resolves from the retained natural dimensions and style each time.

Sizing reference: [CSS2.2 used dimensions and min/max rules](https://www.w3.org/TR/CSS22/visudet.html).

### Measured validation

Windows layout tests: **150 → 164 passed**, zero failed/ignored. Fourteen new
public-pipeline tests cover atomic direct/nested controls, hidden controls,
inline-block replaced boxes, defaults/size attributes, provider overrides,
image ratios, definite/indefinite percentage bases, min/max constraints,
chrome/line fitting, border-box sizing, repeat layout, bidi metadata, and invalid
provider sizes. The existing provider-driven visual-order test now also asserts
styled content/border geometry after atomic reordering.

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Viewport 800 × 600, device scale 1. Every selection was measured **twice before
and twice after**, with identical result/pixel-hash projections within each
pair. Before/after projections also match exactly: no classification changes,
no pixel changes in these selections, and no WPT gain claimed. All selections
have **0 timeout / 0 unsupported / 0 crash**.

| Selection | Before pass / mismatch | After pass / mismatch |
| --- | --- | --- |
| Broad geometry | 31 / 30 | 31 / 30 |
| Float shard | 6 / 2 | 6 / 2 |
| Block collapse | 9 / 0 | 9 / 0 |
| Position | 9 / 3 | 9 / 3 |
| Text flow | 14 / 2 | 14 / 2 |
| Focused flex/grid | 24 / 0 | 24 / 0 |
| Focused paint | 24 / 0 | 24 / 0 |

Full Windows workspace: **929 passed / 0 failed / 3 ignored**, exit **0**.
The ignored tests require external HTTPS or an interactive D3D11 device.
The baseline is `origin/dev` **133292f** (DOM-2 plus NET-3). The final lane
implementation was re-synced after the integrator refresh. All **142 tracked
Rust/build files** were hash-verified against the Mac before and after the
credited workspace gate and again during final evidence collection.

A subsequent server overwrite reset four layout files to exact dev content
after the initial verification. The first successful workspace/WPT results
were excluded. The lane re-synced its source, repeated the workspace gate with
before/after source guards, and repeated all WPT measurements with preserved
executables. `collect.ps1` now rejects source drift as well as ambiguous runs.
No source drift remained at completion. No local builds/tests were run.

Credited WPT directories are `baseline-verified` and `final-verified` below
`C:/Users/server/target/layout/layout2-evidence`. The pristine baseline unit
count comes from the retained `133292f` baseline build; final 164/164 was
rechecked after restoring the lane files.

- Baseline executable: `C:/Users/server/target/layout/layout2-baseline-133.exe`,
  SHA256 `20830897f08778708cc8227d0eb48d39ee9384a1646fbaac477867482e142a90`.
- Final executable: `C:/Users/server/target/layout/layout2-final-verified.exe`,
  SHA256 `b7246e76141d4735e6203dc6ba14d00c1fe611fb7478f562bdc81688901bbea1`.
- [Measured summary](../../crates/amazing-layout/evidence/layout2/results.json),
  [reproduction notes](../../crates/amazing-layout/evidence/layout2/README.md),
  and [verified build-source manifest](../../crates/amazing-layout/evidence/layout2/build-source-sha256.json).
  Per-suite JSON retains case ids, counts, run directories and actual/reference/
  diff hashes. Earlier LAYOUT-1 evidence is untouched.

### Supported boundary

This ticket supplies layout geometry and an adoption seam, not native control
painting or a live form-state model. Provider adapters must migrate to natural
sizes before CSS/HTML sizing combinations can be claimed end to end. The root
driver still uses its existing BMP/PPM path; the separate PNG CSS Images
selection is not measured by this ticket and its prior 0/12 is not promoted.
Undecoded images retain their existing omission behavior. General non-replaced
inline-block layout, blockified control defaults/content, vertical replaced
placement, and platform/content-dependent control sizing remain outside this
inline-box slice. Image HTML dimension presentational hints are not introduced
here; they belong in the upstream style/hint path after removing provider-side
used sizing.


## LAYOUT-3 — Table formatting (in progress)

- Anonymous table object generation follows CSS 2.1 §17.2.1, grouping adjacent
  misparented siblings before descending into wrappers. HTML parser repair is
  not a substitute for CSS box repair.
- Fixed widths use columns, first-row cells, then equal remaining columns.
  Automatic widths distribute cell min/max contributions, including spans.
  Row spans clamp at row-group boundaries; zero spans run to the group end.
  Overlap follows the source-order option permitted by CSS 2.1 §17.5.
- **Resolved CSS dependency:** `border-spacing` inherited computed metadata is
  queued to the CSS lane after CSS-3 provenance. Layout reads computed style
  only and defaults an absent value to the initial `0`; it does not implement
  cascade/inheritance or edit CSS. Explicit computed table spacing is consumed.
- **Resolved paint dependency:** the integrator approved an additive
  `LayoutBox::table_decoration()` seam. `TableDecoration` exposes suppression
  of ordinary borders/backgrounds, absolute background holes for hidden empty
  cells, and table-owned `TableBorderSegment` values carrying the **resolved
  winner** (rectangle, orientation, style, RGBA color, source node) per edge.
  Conflict resolution stays in layout; paint must not reconstruct candidates.
  Hidden winners suppress geometry; inset/outset map to ridge/groove. Paint
  adoption belongs to replaced-content after merge; no paint files are edited.
- Fresh Windows baseline built from `d8437f3` after removing first-party
  artifacts and disabling incremental compilation. Existing 164 layout tests
  and the first 10 new table regressions pass on Windows; final gates pending.
