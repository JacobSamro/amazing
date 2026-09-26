# RC-1 revised — image adapter and measured render #7 scope

Form controls were removed from RC-1 by the integrator. Layout owns LAYOUT-2
(box construction); the integrator owns GPUI control descriptors. CSS color
parsing is unchanged pending CSS-2. No media-element lifecycle is implemented.

## Measured priority

WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, baseline engine
`e9c1e21`, 800×600 at scale 1. The dedicated selection is now committed under
`tests/wpt-replaced-content/`; it replaces the earlier inconclusive screen of
the unrelated 60-case paint diagnostic. Eight SHA-ranked HTML match candidates
per cluster produce 48 entries / 41 unique upstream tests. Direct source
markers overlap; the full inventory and selection are retained for audit.

| Cluster | HTML candidates | Scripted candidates | Pass | Mismatch | Unsupported | Renderer error | Non-pass rank |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Canvas | 533 | 532 | 0 | 0 | 0 | 8 | 1 (8/8) |
| Controls | 40 | 26 | 1 | 2 | 0 | 5 | 2 (7/8) |
| Images/fitting | 308 | 72 | 2 | 5 | 0 | 1 | tied 3 (6/8) |
| SVG | 353 | 82 | 2 | 4 | 1 | 1 | tied 3 (6/8) |
| Embedded content | 132 | 23 | 2 | 5 | 0 | 1 | tied 3 (6/8) |
| Document media | 61 | 13 | 2 | 5 | 0 | 1 | tied 3 (6/8) |

Timeouts: zero throughout. Totals: 9 pass / 21 mismatch / 0 timeout /
1 unsupported / 17 renderer errors (the runner's `crash` classification).
Before and after counts are identical. These rank observed non-passes in the
bounded sample, not estimated global feature gains. Matching pixels can be
inert; renderer errors include missing JS prerequisites. No causal assertion
that implementing one surface fixes every case in its cluster is justified.

Dependency-aware sequence:

1. Deliver bounded raster-image decoding and fitting now; this needs no new DOM
   lifecycle and is the revised ticket's owned implementation.
2. Deliver controls through LAYOUT-2 plus integrator-owned presentation/state
   descriptors; the measured sample has 7/8 non-passes.
3. Establish DOM/JS canvas/context/worker bindings before Canvas 2D rendering:
   all eight selected canvas cases currently exit with renderer errors and
   532/533 screened canvas candidates contain scripts. In parallel, scope
   SVG geometry/paint independently; it ties the image/embedded/media sample
   in non-pass count and should not be smuggled into the image decoder.
4. Add embedded-document surfaces after document lifecycle, origin and process
   seams; add HTMLMediaElement after DOM state/events/timing. Equal measured
   non-pass counts do not establish an order between those two.

## Implementation and ownership

- `amazing-image`: safe adapter using exact `image` 0.25.9 with PNG/JPEG/GIF/
  WebP/ICO/BMP/PNM enabled, explicit format allowlist, input/dimension/output
  caps, decoder allocation budget and panic containment. Output is the owned
  `amazing_media::Image`; dependency types never cross the adapter.
- Integrator-directed placement: `amazing-image` is a member of the standalone
  `amazing-shell-gpui` workspace and shares its lockfile. It has no separate
  workspace marker or lockfile. Root workspace membership/dependencies stay
  unchanged. Cargo metadata and the locked shell check accept the arrangement.
- `amazing-media::ImageSize`: natural dimensions, definite axis overrides,
  one-auto-axis aspect ratio, explicit zero, invalid/overflow rejection.
- `amazing-paint::image`: all five object-fit modes; content-box clipping,
  keyword/percentage/pixel object positions and common edge-offset syntax;
  provider hook plus `StyledImages` retains per-inline image presentation.
  Unsupported position expressions such as calc() fall back to center.
- Raster sampling is bounded, validates buffer sizes and preserves straight
  alpha through effect layers. No paint color-parser changes.
- Minimal authorized shell patch delegates decoding, uses the size helper,
  passes computed image styles into paint and caps retained image RGBA at
  128 MiB. GPUI still owns presentation and keeps its Frame/RgbaImage use.

Limits default to 16 MiB encoded input, 8192 per dimension, 64 MiB returned
RGBA and 128 MiB decoder working budget per image. Codec-internal limits are
best-effort; no total-process memory bound is claimed. Animation playback,
EXIF orientation and ICC conversion are outside this slice.

## Evidence and boundary

The static PNG diagnostic moves 2/12 raw matches to 0/12, with 12 mismatches
and no timeouts/unsupported/errors after decoding. The two former matches
were inert: references still omit background-image painting, but actual images
now paint. Floats and inline CSS geometry remain additional dependencies.
This is **zero credited WPT gains**, not a completed CSS Images conformance gate.
The unchanged focused paint selection remains 24/24 in both repeated runs.
The 48-entry scope selection remains 9/48 raw passes. Full results and hash
projections are under `tests/wpt-replaced-content/`.

Windows validation: paint 62 passed, media 7 passed; image adapter 4 unit plus
3 integration tests passed; full root workspace 733 passed, 0 failed,
3 ignored; standalone shell `cargo check --locked` passed. The workspace
excludes the shell/adapter, which are validated separately.

Integration follow-ups: root driver PageMedia still decodes BMP/PPM only and
needs an explicit product decision before adopting shell-side codecs. Layout
must preserve CSS used image sizes, borders/padding and containing-block
constraints for inline replaced content. Background-image painting and floats
are prerequisites for crediting the retained upstream PNG reftests. Resolve
these in owning lanes; do not modify upstream references to obtain passes.

Implementation references: [CSS Images 3 object fitting](https://drafts.csswg.org/css-images-3/#the-object-fit),
[image 0.25.9 reader](https://docs.rs/image/0.25.9/image/struct.ImageReader.html),
[decoder limits](https://docs.rs/image/0.25.9/image/struct.Limits.html).

# RC-2 — background longhands and bounded static SVG

Base: `57f0e69` (includes the integrator's DOM-2 and NET-3 refresh). The Mac
worktree remains authoritative; all lane changes were re-synced after the
refresh. A stale Cargo net artifact initially exposed an older ParsedUrl API
although source hashes matched. Cleaning package artifacts fixed it without
net source edits; baseline binaries were rebuilt before collecting final data.

## Decisions and delivered behavior

- Background layer descriptors are constructed in `display_list`; it remains
  the only paint stage walking LayoutBox. Rasterization consumes the existing
  stacking, clip and effect command order. Background color is below the last
  image, the first listed image is topmost, and inset shadows/borders/content
  retain their existing order.
- `ImageProvider::background_image(node,url)` supplies already decoded owned
  media pixels. Paint does no fetching or decoding. The shell resolves and
  fetches URLs through its existing session and delegates to amazing-image.
  The static reftest artifact uses the same adapter for local resources.
- Longhands cover repeat/no-repeat/repeat-x/repeat-y/space/round, keyword and
  percentage/pixel positions including edge offsets, auto/explicit sizing,
  contain/cover, border/padding/content origin and rounded clip, comma lists,
  shorter-list cycling and `none` slots. Token-aware splitting retains quoted
  commas and CSS URL escapes. The bottom layer selects background-color clip.
- Background descriptors have 32-layer/64-KiB-value bounds; the raster frame
  has a 64-Mi sample budget. Inverse sampling avoids loops proportional to
  tiny repeated tile sizes. Exhaustion produces the existing closed white
  frame. The shell bounds attempted background URLs at 32 and shares the
  128-MiB retained RGBA budget with img resources.
- A half-pixel edge mismatch between replaced images and backgrounds was
  fixed by using pixel-centre coverage for replaced images. A direct raster
  regression checks matching coverage at half-pixel boundaries.
- The next independent cluster is **static SVG images**, an explicit bounded
  slice of the SVG cluster tied third in RC-1's measured non-pass ranking.
  Canvas had 8/8 renderer errors and 532/533 scripted candidates; controls
  require layout/DOM work, and embedded documents/media require lifecycle
  seams. Static SVG shapes can produce owned pixels without those prerequisites.
  This is not a claim that this slice resolves the broader SVG cluster.
- SVG has its own small strict-subset parser/raster module behind the existing
  amazing-image decode entry point. It supports explicit integral intrinsic
  dimensions, viewBox and preserveAspectRatio meet/slice/none, rectangles,
  circles, ellipses, inherited presentation fill/currentColor and shape alpha.
  Unsupported features reject the whole image. Work is capped at 1024 elements,
  depth 32 and 64-Mi pixel/shape samples, alongside adapter input/output limits.
  The canonical amazing-css color parser is reused; paint color parsing is
  unchanged. The CSS dependency moved from adapter dev-only to normal use;
  no external engine dependency or root workspace membership changed.

## Boundaries and routed work

CSS-3 owns background shorthand expansion and stylesheet-relative URL
provenance, accepted by the integrator. RC-2 uses unmodified upstream explicit
longhands and does not wait for CSS-3. Layout retains inline replaced/control
box ownership. Background attachment, gradients, logical/calc positions,
full unit resolution (rem currently uses the engine's 16px root default),
root/body image propagation and inline decoration fragmentation are not claimed.

SVG CSS/inline style, paths, strokes, text, transforms, filters, external
references, animation, fractional/percentage intrinsic sizing and antialiasing
remain unsupported. The paired SVG WPT resource files use inline CSS fill;
raw matches with those images omitted receive zero conformance credit.
Root-driver PageMedia adoption of the richer adapter remains integrator-owned.
No GPUI screenshot proof was collected; the standalone shell gate is a locked
compile check, not an interactive visual claim.

Windows validation after refresh and re-sync: root workspace **927 passed,
0 failed, 3 ignored**; paint **74 passed** including 12 new background/coverage
checks; adapter **4 unit + 3 presentation + 5 SVG tests passed**; standalone
shell `cargo check --locked` passed. All 13 changed crate source files match
between Mac and Windows by SHA-256. Evidence is in
`tests/wpt-replaced-content/rc2/validation.json` and `artifacts.json`.

## RC-2 pinned results

WPT remains pinned at `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The retained PNG selection moves **0/12 → 12/12**. The references now paint
decoded backgrounds, and the two fractional object-position cases also use
consistent pixel-centre coverage. This is a selected static reftest result,
not a general CSS Images conformance claim or a substitute for LAYOUT-2.
The SVG counterpart selection is **12/12 raw → 12/12 raw**, with **zero SVG
conformance credit** because the resources require unsupported inline CSS.
The 48-entry scope remains **25 pass / 5 mismatch / 1 unsupported / 17 renderer
errors**, with no timeouts; its improvement over RC-1 belongs to the new dev
baseline and is not credited to RC-2. The focused gate remains **24/24**.

Reports preserve every classification, differing-pixel count and pixel hash
under `tests/wpt-replaced-content/rc2/`. Reproduction and interpretation are in
that directory's README; `analyze-rc2.py` compares both saved runs per phase.

Next boundary: merge review of this owned slice and the minimal shell loader
patch; CSS-3 and LAYOUT-2 remain independently owned. A future SVG ticket must
add CSS through the canonical computation seam and select non-inert SVG
reftests before increasing any conformance count. Do not proceed automatically.

All eight before/after repeatability pairs are stable in classifications and
actual/reference/diff hashes. Every PNG reference changed, while only the two
half-pixel object-position actuals changed. SVG actual and reference hashes
are unchanged across phases, corroborating zero credited SVG gains.

## Artifact-freshness correction and integration

All pre-fix Windows evidence was superseded. The lane repeated its builds and
measurements at base 57f0e69 after first-party artifact invalidation in all three
target directories with CARGO_INCREMENTAL=0. These are saved-executable lane
measurements, distinct from the merged product head. RC-2 was merged as
d8437f3. The integrator reports the authoritative clean merged-head workspace
gate: **992 passed, 0 failed, 3 ignored across 52 targets**, lock prefix
3FEAF813. No new feature work follows RC-2.

## RC-3 — control painting and the payload boundary

Implemented chrome for TextInput, Checkbox, Radio, Button, Select and Textarea
from layout's `styled_replaced_fragments()` view. Only the display-list builder
enumerates the view; rasterization consumes flattened commands and supplied
geometry, retaining clipping, stacking and compositing. Author visible
backgrounds/borders and appearance:none are honored. Default chrome is a bounded
platform-neutral design; specialized appearance keywords are not implemented.

`amazing_paint::controls::ControlProvider` supplies checked/disabled and borrowed,
already-positioned `LineBox` glyph runs. The additive
`paint_document_with_text_resources_and_controls` entrypoint consumes these
through the shared shaped-glyph raster path, clipped to the control content
rectangle. No DOM state lookup, label selection, shaping or measurement is added.
Existing entrypoints default to unchecked/enabled/no label. The integrator routed
production state to DOM and control-label shaping/placement to layout; production
state/value/selected-label behavior remains unclaimed until those are wired.
Only controls exposed by the inline replaced view are covered by this slice.

Integrator decision: computed initial transparent background and none borders
cannot distinguish explicit author defaults. Treat ambiguous defaults as absent
styling; do not infer provenance. CSS owns the queued provenance follow-up.
The transitional paint color parser is unchanged.

Pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`: 63 static eligible
candidates, 24 unique selected by path hash, four per control kind with greedy
uniqueness. Before and after are both **21 matches / 2 mismatches / 1 unsupported**;
zero new conformance passes claimed. Twenty matches change both sides, one
changes neither. Base-select explicit-size and writing-mode cases still mismatch.
Zero-size checkbox is unsupported because the integrator-owned harness resolves
its root-relative blank reference against the test directory, despite the pinned
reference being present. This request is routed; no upstream files were changed.

CSS Images remains **12/12** and focused paint **24/24**, both before and after.
All six repeated pairs have stable classifications, difference counts and pixel
hashes. Final Windows gate on refreshed `0cc0ea0` plus owned changes: **1,029 passed,
0 failed, 3 ignored across 52 targets**. Paint **86 passed**, including 12 new
control tests. The image example builds with `--locked` after the integrator's
standalone-shell lock fix. First-party artifacts were invalidated, baseline
rebuilt, final owned sources re-synced and rebuilt with CARGO_INCREMENTAL=0;
source hashes also match. Full evidence and reproduction live in
`tests/wpt-replaced-content/rc3/`. No shell/workspace/lock edits in RC-3.

Boundary: deliver for review and stop. Follow-ups are provider integration,
CSS provenance, specialized appearance/layout coverage, and root-relative WPT
reference resolution. None is silently credited by raw matching reftests.

RC-3 merge preflight refresh: rebased cleanly onto origin/dev `156cc13`
(implementation `0991e80`), ran the authorized lane-only refresh, re-scp'd all
owned changes and rebuilt on Windows. Workspace **1,035 passed / 0 failed /
3 ignored across 53 targets**; paint **86 passed**. Controls, CSS Images and
focused paint retain the same counts and pixel hashes both across repeats and
against the pre-rebase after runs. Evidence is preserved in
`tests/wpt-replaced-content/rc3/rebase-156cc13/`. The integrator accepted routing
of the root-relative reference harness bug; questions.md marks it RESOLVED.

## RC-4 — author provenance and the next unblocked cluster

CSS-4's `ComputedStyle::is_author_specified` is now adopted for control
background and border longhands. Explicit author transparent/none and initial
defaults suppress native chrome; absent author styling retains it. The RC-3
explicit-default ambiguity is resolved by the CSS-owned surface. Author rollback
through explicit longhands is tested. A distinct `border:revert` shorthand
provenance defect was exposed and routed to CSS, not guessed around in paint.
The DOM state/layout glyph provider integration is still separately owned.

Remeasured the unchanged 48-entry scope at WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` on the CSS-4 baseline:

| Remaining cluster | Pass | Mismatch | Unsupported | Renderer error | Dependency-aware priority |
| --- | ---: | ---: | ---: | ---: | --- |
| Canvas | 0 | 0 | 0 | 8 | Blocked: DOM/JS context/worker contracts, driver script resource loading |
| Controls | 2 | 1 | 0 | 5 | Remaining state/text providers and specialized widget/layout behavior |
| SVG | 4 | 2 | 1 | 1 | Highest unblocked; RC-4 bounded geometry transforms |
| Images/fitting | 6 | 1 | 0 | 1 | Remaining geometry/resource cases; retain dedicated 12-case PNG gate |
| Embedded | 7 | 0 | 0 | 1 | Blocked: document lifecycle/origin/process seams |
| Document media | 7 | 0 | 0 | 1 | Blocked: DOM state/events/timing |

Totals: **26 matches / 4 mismatches / 1 unsupported / 17 renderer errors**,
no timeout. These are overlapping bounded samples, not broad conformance rates
or causal estimates. Canvas errors include root-relative script loading failures
before any paint work; the integrator confirmed Canvas exclusion and routed
script/reftest-link resolution to integrator-assist. A later ranking should
remeasure after those blockers land rather than retain these errors as paint debt.

RC-4 expands static SVG image decoding with group/shape matrix, translate,
scale, rotation with optional pivot, and skew attribute lists. Matrix composition
and inverse sampling reuse bounded rectangle/ellipse rendering. Singular
transforms hide their subtree. Parsing has a 32-transform limit; invalid or
nonfinite/overflowing input fails the complete image. Existing input, depth,
element, allocation and raster-work limits remain. No dependencies, root graph,
lockfiles, shell, DOM, layout or CSS files were changed. The existing strict SVG
subset parser remains; root SVG transforms, CSS, paths, strokes, references,
text, filters and animation remain unsupported.

Owned headless render proof (not WPT): **0/9 before → 9/9 after**, twice.
Three cases prove explicit-default provenance changes pixels; six compare SVG
transforms against equivalent untransformed references. Fixture canvases are
fixed at 64x64. The retained SVG WPT resource selection still requires inline CSS,
so no raw match there receives new SVG conformance credit. The next SVG boundary
is canonical CSS computation/presentation metadata and a non-inert upstream
selection; general inline SVG geometry remains a separate architectural slice.

Original Windows workspace at dev `3d0c2b0` plus owned implementation:
**1,135 passed / 0 failed / 3 ignored across 56 targets**. Paint **87 passed**;
isolated image adapter **17 passed** (4 unit, 3 presentation, 10 SVG), with --locked
example build green. Before artifacts are from `cc3761e`; intervening dev includes
DOM/HTML namespace integration and is not credited to RC-4. The lane-only refresh,
re-sync and isolated-target artifact invalidation precede the final gates.
Evidence and reproduction are in `tests/wpt-replaced-content/rc4/`.

Original RC-4 repeats: scope **26/4/1/17**, controls **21/2/1**, CSS Images **12/12**,
SVG **12/12 raw with zero credit**, focused **24/24**, unchanged before/after.
All ten WPT repeated pairs are stable; actual/reference hashes are unchanged
across phases. Owned proof **0/9 → 9/9** has stable repeated hashes, changed
actuals and unchanged references. Source equality was checked across 180 tracked
Rust/Cargo inputs with zero mismatches. Stop for review; no next ticket started.


RC-4 merge gate refreshed on dev `11dd8a8aefd81318fcfe858334e865ed66694cd8`,
compiled lane HEAD `51ba19fbb4c55386e0822715d27062d4b14b9271`:
root **nextest 1,209 passed / 0 failed / 3 skipped across 41 binaries**;
separate **doctests 2 passed / 0 failed** (HTML 1, JS 1; 18 targets).
Paint contributes 87 passing tests. Isolated image adapter **nextest 17 passed**,
with **0 doctests present** in its separate doc run. Both renderer builds and
root/standalone metadata checks pass with `--locked`. No shell source grant or
lane lockfile edit is needed. The primary's artifact-invalidating fleet refresh,
owned-file re-sync, and separate image-target invalidation precede these gates;
191 Rust/Cargo inputs match the Mac. Original before reports and saved executable
hashes remain unchanged. The partial acb57e5 gate was superseded by the fleet
refresh and is preserved separately, without a final repeatability claim.

Fresh required gate pairs are complete: scope **26/4/1/17**, controls **21/2/1**,
PNG **12/12**, SVG **12/12 raw with zero credit**, focused paint **24/24**.
All five pairs have stable classifications, pixel differences and image hashes,
also unchanged from original before/after. Owned proof remains **9/9 twice**,
unchanged from original after; original **0/9** before is preserved. Evidence:
`tests/wpt-replaced-content/rc4/rebase-11dd8a8/`. RC-4 is ready for co-integrator
review; no push, shell grant, or next-ticket work.


RC-4 subsequent preflight checkpoint: on dev f1d0f50 + compiled lane552fd1f,
Windows nextest **1261/0/3 skipped**, separate docs **2/0**; image **17/0**, docs0;
owned proofs9/9 twice. Both pinned runs completed with prior counts unchanged,
but the fleet SSH outage prevented collecting reports for hash comparison.
Evidence is labeled incomplete in `rc4/rebase-f1d0f50/`, not a ready gate.
Local lane rebased through IA-1 dev `9bf1224`, which changes the driver and WPT
root mapping; fresh validation of that newer source is unrun pending restored
server access. No new SSH/SCP attempts after stand-down. REFRESH-HOLD removed
after the running process ended. Existing before and paired evidence preserved.

### RC-4 current-source resume and co-integrator preflight

After the outage, saved `f1d0f50` reports and executable hashes were collected
without overwriting the earlier evidence. Their five pinned selections and
owned 9/9 proof agree across the two runs on classification, pixel differences,
and actual/reference hashes. They remain historical: compiled lane `552fd1f`
on dev `f1d0f50` predates IA-1.

RC-4 was rebased to dev `ffb143b6fdc07df7a9681e597f04a8da40f23d06`
(compiled lane `95dabe37a6b510e5271dee2fc51770eb6b0459e7`). The primary shared
coordinator refreshed only replaced-content; owned files were re-synced. On
win-001, 328 Rust/Cargo/harness/fixture inputs matched the Mac, both root and
isolated-image `cargo metadata --locked` checks passed, and WPT stayed pinned at
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Root nextest passed
**1263/1263** with 3 skipped across 42 binaries; separate root doctests passed
**2/2** across 19 targets. Paint contributed 87 passes. Isolated image nextest
passed **17/17** across 3 binaries; separate image doctests found 0. Both
renderers rebuilt. Full gates used the primary shared build slot. Logs and
artifact hashes are recorded in `tests/wpt-replaced-content/rc4/rebase-ffb143b/`.

Each of five current pinned selections ran twice with stable classification,
pixel-difference and actual/reference hashes: scope **26 pass / 4 mismatch /
0 timeout / 0 unsupported / 18 renderer errors**, controls **22 / 2 / 0 / 0 /
0**, PNG **12/12**, SVG **12/12 raw with zero credit**, focused paint **24/24**.
Owned 64x64 proof stayed **9/9 twice**, versus the preserved original **0/9
before**. RC-4 claims no new WPT conformance pass. SVG resources requiring
unsupported inline CSS still receive zero credit; the bounded transform proof
covers the owned fixtures only.

Compared with the historical packet, the driver/root mapping now resolves the
blank reference for `input-checkbox-zero-size.html`, changing its control case
from unsupported to a raw pass with equal blank hashes. In the broader scope,
`svg/use-descendant-combinator-003.html` changes from missing-reference
unsupported to a renderer script TypeError. Those are IA-1 driver/root mapping
effects and receive no RC-4 credit. All other selected classifications and
actual/reference hashes are unchanged. The larger root test count likewise
reflects intervening dev changes, not RC-4.

Stop at RC-4 for co-integrator review and authoritative merged-head validation.
No remote branch/PR publication or next ticket.

### RC-4 HTML-2 named-source workspace checkpoint

After `ffb143b`, dev added HTML-2 `parse_bytes`/encoding (no production caller)
and a refresh-script change. The only changed paths are `amazing-html`
`encoding.rs`/`lib.rs`, HTML decision notes, and the refresh script; there are
no RC, driver, render harness, or fixture changes. RC-4 rebased cleanly to dev
`4e9d71c390d949d03b6adbdd48e55b72fb3ea4b1`, compiled lane
`043292f1968ef71b99f9d7aff9b2225fc7b08a7d`. The primary coordinator
refreshed only the RC Windows tree from origin/dev, RC files were re-synced,
and 329 Rust/Cargo/harness/fixture hashes matched. Root locked metadata passed.
Under the primary shared build slot, root nextest passed **1270/1270** with
3 skipped across 42 binaries and separate doctests passed **2/2** across 19
targets. See `tests/wpt-replaced-content/rc4/rebase-4e9d71c/`.

The five pinned render pairs and owned proof were not rerun: their committed
pixel evidence is explicitly for `ffb143b` compiled source. This checkpoint
claims only the named-source workspace gate. Integrator will stage/rebase an
integration snapshot on later dev and authoritative-gate the merged head.

### RC-4b — pinned render repeat on current dev

Fixed base `253d9ec56a3612df8e495ea0cac49813fffbe2ee`, WPT pin
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Primary coordinator
refreshed only replaced-content from origin/dev. Root and isolated-image locked
metadata passed; 339 current Rust/Cargo/harness/fixture hashes matched before
and after the counterfactual build. Before was current dev with only the five
RC-4 source-file changes removed (four first-parent files, new transform module
absent); after was unmodified current dev. Both renderer pairs were rebuilt on
win-001. Builds and the complete 20-run pinned sweep used the primary shared
build slots under REFRESH-HOLD.

All ten within-phase pairs have stable classifications, pixel differences and
actual/reference/diff hashes. Before and after per-case projections are
identical. In each phase and repeat: scope **26 pass / 4 mismatch / 0 error /
0 timeout / 0 unsupported / 18 renderer errors**; controls **22 / 2 / 0 / 0 /
0 / 0**; PNG **12/12**; SVG **12/12 raw, zero conformance credit**; focused
paint **24/24**. No result moved. Current after projections also match the
older ffb143b after packet exactly (aside from an added explicit `error: 0`
report-count field). Full evidence is in
`tests/wpt-replaced-content/rc4b/base-253d9ec/`. Owned 9-case proof was not
part of this five-selection rerun. Stop at RC-4b for integrator review.

## RC-5 — non-inert static SVG resource styling

Fixed dev base `0964385d49bb1a0da1822446a4e1572d91d7c925`; pinned WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. A four-case subset of the
prior CSS Images SVG selection was frozen before implementation, together with
an owned 64×64 styled/reference/blank SVG fixture. Prior WPT results were 4/4
raw inert matches and **zero SVG credit**. The saved predecessor executable
rendered the styled fixture exactly like blank and unlike the reference.

The bounded image decoder now applies inline `fill`, `color`, `fill-opacity`,
and `opacity` through `amazing-css::parse_declarations` and its canonical color
parser. Inline values outrank presentation attributes, inline `!important`
wins over normal duplicates, and `currentColor` resolves against the effective
inherited color. Unsupported declarations and values reject the whole image.
Scripts, external references, animation, unimplemented geometry/paint, and
the existing input, depth, element, output, and raster-work caps remain strict.
No CSS, DOM, root graph, lockfile, shell, paint, or media source was changed.

After a lane-only refresh and owned-file re-sync, 10 source/fixture hashes
matched Mac and Windows both before and after final gates. Windows root
nextest **1,396 passed, 3 skipped**; separate root doctests **2 passed**.
Isolated image nextest **19 passed**; separate image doctests **0 present**.
The exact four-case WPT selection is **4/4 twice** with stable classifications,
pixel differences, and actual/reference/diff hashes; all four actual and
reference hashes changed from the inert prior. The owned fixture matches its
reference in both runs and differs from blank at all **4,096 pixels**. Raw
WPT pass gain is zero; the four selected passes are now non-inert headless
resource evidence, without a broader SVG conformance claim. Full hashes and
reproduction details are in `tests/wpt-replaced-content/rc5/`.

CSS-wide keywords, `var()`/custom properties, stylesheet cascade, and SVG
presentation metadata need a CSS-owned standalone computation API. The lane
requested that contract in `questions.md` and did not add a parallel CSS
grammar. Paths, text, strokes, paint servers, filters and group opacity remain
outside the static renderer. Stop at RC-5 for co-integrator review; no next
ticket, push or PR.

## RC-6 — CSS-7 computed static SVG paint style

Fixed base `945ca38` (CSS-7 merge). The SVG image adapter replaces RC-5's
literal declaration winner logic with
`amazing_css::compute_element_paint_style(parent, hints, author, inline)`.
It passes each parent's computed style and keeps SVG presentation attributes
in the lower-priority hint slice. Inline declarations may include custom
properties. The static image subset has no matched stylesheet declarations,
so the author slice is empty. Typed color, fill, fill-opacity and opacity are
consumed; explicit `Unsupported` values reject the whole resource. Existing
geometry, reference, script, animation, unsupported paint and work limits
remain strict. No CSS, DOM, paint, media, root graph or lockfile source changed.

At pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the four-case
SVG resource control stays **4/4** on the CSS-7 fixed base and in both after
runs, with identical classifications, pixel differences and
actual/reference/diff hashes. Raw WPT gain is zero. Four owned 64×64 cases
cover CSS-wide `inherit`/`initial`, inherited `var()`, and presentation versus
inline ordering. They move from **0/4 inert reference matches** to **4/4
non-inert matches**, twice; every after case differs from blank at 4,096
pixels. Evidence and reproduction: `tests/wpt-replaced-content/rc6/`.

The remaining boundary is full SVG stylesheet/selector integration and other
SVG paint/shape surfaces, not a missing literal parser. No broad SVG
conformance claim follows from four selected headless reftests. Stop at RC-6
for co-integrator preflight; no push, PR, or next ticket.

Final win-001 gate after lane-only refresh and re-sync: workspace nextest
**1,441 passed, 0 failed, 3 skipped**; separate workspace doctests **2 passed**.
Isolated image nextest **21 passed**, separate image doctests **0 present**.
Twenty-two source/manifest/fixture hashes matched Mac and Windows both before
and after, unchanged across the gate. The final gate is on fixed CSS-7 base
`945ca38`; later dev merges were not chased.

## RC-7 — bounded straight-line SVG path fill

Fixed base `1a8c222` (RC-6 merge). The image adapter accepts one explicitly
closed outline using `M/m`, `L/l`, `H/h`, `V/v`, `Z/z`, including implicit
lineto coordinate pairs after moveto. It samples the outline through the
existing viewport and inverse transform path and applies RC-6 computed fill,
color and opacity. Path token and point counts, finite coordinates, and a
vertex-weighted raster work budget bound parsing and rendering. Multiple
subpaths, open outlines, curves, arcs, fill rules, strokes, paint servers,
references, scripts and animation reject the whole image.

The pinned WPT CSS Images SVG resource control has four previously non-inert
cases. Its sparse checkout contains no `<path>` SVG resource. These controls
therefore carry **zero new WPT credit** for RC-7. The owned 64×64 HTML image
corpus has absolute, relative, concave and transformed/alpha path cases with
shape-based references and a blank control. The fixed-base before executable
was **0/4**, each actual identical to blank. The first after render was
**4/4 non-inert**, with all actuals equal to references and 576–1,024 pixels
different from blank. Final repeat and gate evidence: `tests/wpt-replaced-content/rc7/`.

### RC-7 preflight correction — default nonzero path fill

Code review found the path containment check counting even-odd crossings,
although SVG's initial `fill-rule` is `nonzero`. A literal single closed
`M/H/V/Z` path tracing one square twice in the same direction was accepted
but rendered blank at its center. The image regression failed before the fix
with transparent versus red at `(2,2)`. The containment check now accumulates
signed ray crossings, so winding `2` fills the square. Explicit `fill-rule`
remains unsupported by the strict static decoder. The owned HTML resource
corpus adds a matching double-wound/reference/blank case, moving the expanded
five-case proof from **4/5** before the correction to **5/5 non-inert** after.
Final repeated hashes and Windows package gate are in the RC-7 evidence packet.

## RC-8 — bounded SVG polygon fill

Fixed dev base `52222f8` (RC-7 merge). The `amazing-image` static SVG decoder
accepts `<polygon points="...">` with finite coordinate pairs, SVG comma or
whitespace separators, and a negative second coordinate adjacent to the first
as allowed by the SVG points grammar. At least three vertices are required;
512 vertices/1,024 numeric tokens are the explicit cap. Malformed values,
unsupported attributes, `<polyline>`, strokes, fill rules, paint servers,
references and animation reject the entire resource. Accepted polygons share
the RC-7 closed-outline nonzero winding sampler, viewport/transform path,
computed fill/color/alpha and vertex-weighted raster work budget. No other
engine crate, driver, shell, root graph or lockfile was edited.

The owned 64×64 HTML image corpus has simple, concave, transformed/alpha and
same-direction double-wound polygons, each paired with a shape-only SVG
reference and blank control. The fixed-base predecessor renderer was **0/4**,
with every actual identical to blank. The polygon renderer is **4/4 non-inert
in two runs**, identical per-case classifications and actual/reference/blank
hashes; colored pixels differ from blank at 1,024, 624, 576 and 1,024 pixels.
The pinned CSS Images SVG resource selection has no `<polygon>` resource and
remains **4/4 raw before and twice after** with stable pixel hashes. This is
**zero new WPT polygon credit**. Evidence: `tests/wpt-replaced-content/rc8/`.

## RC-9 — bounded SVG polyline fill

Fixed dev base `03b26ec` (includes RC-8 merge). The strict static SVG image
subset accepts `<polyline points="...">` only for fill. It reuses RC-8's
bounded finite points parser (three to 512 vertices, comma/whitespace pairs,
4,096-character attribute cap) and RC-7's default nonzero winding sampler.
For fill, the closing edge from the final to first vertex is implicit. The
raster work budget charges each vertex. No stroke is painted: any stroke,
fill-rule, unsupported attribute, malformed points, reference or paint server
rejects the entire SVG resource. Paths and polygons retain their behavior.

The owned 64×64 HTML image corpus covers simple, concave, transformed/alpha,
and three-vertex endpoint-closure cases, each with a literal shape-based SVG
reference and a shared blank control. The fixed-base predecessor executable
renders all four actuals exactly like blank, for **0/4 non-inert matches**.
After implementation, all four actuals match their references and differ
from blank by 1,024, 1,024, 576 and 528 pixels; both runs have identical
classifications and per-page hashes. The pinned four-case CSS Images SVG
resource selection contains no applicable polyline resource and remains
**4/4 raw passes** before and twice after, with identical pixel hashes. It is
a regression control with **zero new polyline WPT credit**. Final proof,
hashes, and Windows gates are in `tests/wpt-replaced-content/rc9/`.

## RC-10 — implicit closure for straight-line SVG path fill

Fixed dev base `e6577dc` (RC-9 merge). The bounded, one-subpath `M/L/H/V`
static SVG path subset accepts an open outline for fill. The existing default
nonzero winding sampler joins its last and first vertices when testing fill;
explicit `Z` retains the same result. Paths still require at least three
finite vertices. The path token/point cap, 4,096-character attribute cap and
vertex-weighted raster work budget are unchanged. Multiple subpaths, curves,
arcs, malformed commands, explicit fill-rule, strokes, references and paint
servers reject the whole image. No stroke behavior is inferred from fill.

The owned 64×64 HTML `<img>` corpus pairs open triangle, concave and
transformed/alpha paths with shape-based references, and directly compares an
open path with the same path ending in `Z`. A shared blank control prevents
inert credit. The fixed-base predecessor was **0/4 non-inert**, with each open
path actual identical to blank. After implementation the owned corpus is
**4/4 non-inert twice** with identical classifications and per-page hashes;
the fills differ from blank at 528, 1,024, 576 and 1,024 pixels. The four
pinned CSS Images SVG resource controls use SVG files without an applicable
open `<path>` outline and remain **4/4 raw before and twice after** with
identical hashes. They carry **zero new path WPT credit**. Exact repeated
render and Windows gate evidence is under `tests/wpt-replaced-content/rc10/`.

## RC-11 — external SVG shape/painting conformance screen

Fixed dev base `592e542` (RC-10 merge), WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. A deterministic selection
froze all 35 script-free HTML `rel=match` reftests under `svg/shapes` and
`svg/painting` before any source edit (manifest SHA-256
`2fdbe12184173e8808ad2c09a6dc2ed41254affaa4bd1d84054e1880e110cfb0`).
The inventory retains 45 candidate HTML reftests and their dependency
blockers, including nine scripted matches and one mismatch relation excluded
by the selection rule. Exact-revision wpt.fyi Chrome 154 run
`5182906107691008` and Safari 26.6 run `5141363439173632` give 20
both-pass cases, 10 Chrome-only passes and five both-fail cases.

The fixed-base predecessor renderer produced **22 pass, 13 mismatch** in both
Windows sweeps, with no errors, timeouts, unsupported cases or crashes.
Classifications, differing pixel counts and actual/reference/diff hashes are
stable per case. The 20-case both-browser-pass subset has **11 raw passes and
nine mismatches**. Sixteen raw passes (nine consensus passes) are all-white
actual/reference matches and do not prove SVG painting. Forty-five source,
runner, manifest, lockfile and selected WPT page hashes matched Mac and Windows
and were unchanged across the paired baseline.

The nine consensus mismatches are blocked upstream of an image/paint-only fix:
seven use inline SVG that the HTML reftest artifact does not paint, and two use
SVG data URIs that its resource resolver treats as filesystem paths. The
`<img>` data URI case also requires CSS zoom, line stroke and vector-effect.
Another selected case uses an SVG `<iframe>` and has an embedded-document
lifecycle dependency. These are source-based dependency inferences recorded
beside the measured first mismatch metrics in the RC-11 evidence packet.

There is **no actionable both-browser-pass case within replaced-content owned
paths** in the frozen selection. RC-11 stops with a ranked evidence report and
cross-lane requests in `questions.md`. No image/paint source was changed; no
after sweep or conformance gain/loss is claimed. Evidence and reproduction:
`tests/wpt-replaced-content/rc11/` and `rc11-inventory.json`.

## RC-12 — shared SVG document model for byte resources and inline DOM

RC-12 introduces a public bounded `SvgDocument` model in `amazing-image`. The
existing byte parser builds it; the inline path takes a `Dom`, SVG root
`NodeIdx`, `StyledDom`, and content viewport, then builds the same model.
`render_svg` rasterizes either model at the requested viewport and DPR under
the existing geometry, output-byte, shape and raster-work limits. The byte
`<img>` decoder still uses its intrinsic viewport. Literal byte/DOM trees
compare RGBA pixels, including a document selector overriding a presentation
hint. The DOM builder preserves SVG hints when the current whole-document CSS
cascade only exposes initial paint values; explicit author declarations equal
to an initial value remain ambiguous until CSS supplies hint provenance.

`amazing-paint::ImageProvider` has a defaulted `svg(node, width, height, dpr)`
method. Paint asks it for a raster at a replaced content rectangle, then uses
its existing clipped, source-over image blit. The driver can return `None` for
non-SVG nodes. Integrator-assist owns that adapter and WPT artifact. LAYOUT-9's
`ReplacedKind::Svg` control exclusion will be applied once its seam merges.
The frozen 32 inline WPT outcomes are the conformance gate; these unit tests
and paint seam alone receive no WPT credit. External before/after results,
source hashes, workspace gate and final status belong in the RC-12 report.

Primary integration decision after LAYOUT-9 merged: RC-12 lands on isolated unit
evidence so IA-SVG can compile the driver adapter. The frozen 32-inline
before/after WPT outcome and retained render checks are a joint IA-SVG gate
after this merge; RC-12 claims no WPT gain from its fixtures. Fixed base
`859bb53` includes LAYOUT-9 `ccea7b0` and HTML-13. Windows image and paint
crate tests plus separate doctests passed after a fresh tree refresh and
source audit. No RC-12 workspace or after-WPT result is claimed.

## RC-13 — retained SVG recovery and crate split

Fixed dev base `cb3064b` includes CSS-10's SVG presentation attributes in
the document cascade. The inline DOM builder now takes paint values from that
computed style and no longer guesses whether an initial fill came from a
presentation hint. Unsupported DOM descendants degrade by skipping that
subtree while preserving renderable siblings; the byte resource parser remains
strict. Nested SVG viewports, viewBox transforms and clipping, basic clip
shapes, rectangular masks and foreign-object backgrounds cover the retained
reftest cases. The renderer still has a bounded static subset and does not
implement general SVG text, filters or paint servers.

The frozen RC-11 35-case selection at WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` is rendered with
integrator-assist's exact `7e48a0e` adapter. Against the pre-RC-12 `859bb53`
result of **22 pass / 13 mismatch**, the recovered renderer measures **26 pass
/ 9 mismatch, 23 nonblank passes**, twice with identical per-case actual and
reference hashes. There are **zero retained pass to mismatch transitions**.
Three passes remain blank matches and receive no painting credit. The
after-move sweep repeated **26 / 9 and 23 nonblank** twice, preserving every
classification and actual/reference/diff hash from both pre-move runs. The
paired reports and pixel counts are under `tests/wpt-replaced-content/rc13/`.

The SVG model and software renderer move into root-workspace `amazing-svg`.
`amazing-image` re-exports the SVG API for its existing callers and keeps the
raster codec adapter. The six DOM model tests move to an image integration test
so the image test count stays **40/40** across the split. The shell lockfile
adds only the `amazing-svg` package and the image-to-SVG dependency; the
IA adapter's scratch fetch/net dependencies are not product dependencies.

## RC-14 — routed paint order and corner radius

The reviewed 12-case selection is frozen in
`tests/wpt-replaced-content/rc14/focused-manifest.json` before the paint
changes. It pins WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` and
contains the two LAYOUT-11 routes with related radius, positioning and
stacking cases. `fixture-hashes.json` records the upstream test/reference
inputs; owned fixtures receive no WPT credit.

An auto-z positioned subtree now places its normal-flow descendants before
its positioned descendants while preserving normal-flow sibling order. Paint
uses `amazing_layout::values::resolve_corner_radii` for CSS math and each
axis's percentage basis, then applies its existing radius normalization. The
layout box does not yet retain the selected font's measured `ch` width, so
paint passes the layout-style `0.5em` fallback for `ch` expressions.

Primary identified two lost-paint cases within the retained RC-13 SVG set:
`ellipse-auto-rx-textpath` and `non-scaling-stroke-007` changed from nonblank
matches to blank matches. RC-14 also restores bounded Ahem text along a
referenced ellipse, resolving `rx`/`ry: auto` from the other radius, and
straight-line path strokes with non-scaling width through a nested viewBox.
Both cases require a painted match; blank equality receives no credit.

The fixed base is pushed `582a6af` (RC-13 plus HTML-15). On Windows, the
focused 12-case WPT gate moved from **8 pass / 4 mismatch** to **10 pass / 2
mismatch** twice. All passes were painted. The two gains are exactly
`all-prop-002` and `calc-border-radius-1`; there were zero pass losses,
timeouts, unsupported cases or crashes. Classifications and image hashes
were stable within each before and after pair.

With the exact frozen IA `7e48a0e` adapter, the retained 35 SVG cases stayed
**26 pass / 9 mismatch** twice. Painted passes rose **23 to 25**. Both routed
SVG cases remain raw passes and now have matching painted actual/reference
images: textPath **0 to 5,776** nonwhite pixels, non-scaling stroke **0 to
1,900**. One raw pass (`pathlength-005`) remains blank. There were zero raw
or painted pass losses; all 35 after classifications and image hashes were
stable. The bounded textPath and stroke support does not cover general SVG
text or arbitrary paths.

The production shell lock needed Cargo's generated `amazing-encoding` entry
and the `amazing-html` dependency edge from HTML-15. Primary explicitly
granted this narrow lock change; the IA scratch adapter has no product
dependency edge. On the fixed Windows tree, paint nextest passed **91/91**
twice, image nextest with the IA scratch manifest passed **42/42**, and root
workspace nextest passed **1,720/1,720** with three skipped. With the granted
production lock update, image nextest also passed **42/42** and its separate
doctests **0/0**. Separate workspace doctests passed **2/2**. Paired reports,
pixel counts and input hashes are
under `tests/wpt-replaced-content/rc14/`.

## RC-15 — bounded SVG stroke geometry and dashes

The retained RC-11 selection still has six non-scaling stroke mismatches and
one blank raw pass (`pathlength-005`). The SVG 2 stroke and vector-effect
rules require stroke geometry in the host viewport for non-scaling stroke,
odd dash arrays repeated to even length, negative dash offsets wrapped in the
pattern, and `pathLength` calibration for dashes. The shared renderer now
builds bounded strokes for rect, circle, ellipse, path, line, polygon and
polyline. It accepts stroked byte SVG resources, including viewBox-only line
images used by the data-URL cases, and retains strict rejection of curves and
unsupported paint. A fresh paired sweep exposed two reference strokes that
became painted while their tests' embedded SVG data images remained empty.
The DOM renderer now accepts bounded SVG data images, including fractional
source viewports, and rasterizes them at the embedding viewport so a
non-scaling stroke remains one host pixel wide. External SVG images and
recursive embedded images remain outside this bounded path.

At fixed base `f8bdb33` and WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, the exact retained 35 SVG
cases moved from **26 pass / 9 mismatch, 25 painted passes** to **28 pass /
7 mismatch, 28 painted passes**. The two raw gains are
`non-scaling-stroke-009` and `non-scaling-stroke-010`; the previously blank
`pathlength-005` pass now paints. There were zero raw or painted pass losses,
and zero errors, timeouts, unsupported cases or crashes. Two fresh after runs
have identical per-case classifications and actual/reference/diff hashes.
Reports and pixel counts are in `tests/wpt-replaced-content/rc15/`; an earlier
stale-binary pair was discarded. The source recovery commits are `64833d3`,
`aeb3820` and `53f9b29`.

## RC-16 — compositor display-list view

ADR 0006's compositor receives a borrowed, read-only `DisplayListView` from
paint. Ordered items expose stable indices for one view, conservative
pre-transform bounds, transform/clip/effect/scroll state IDs and parent tables,
plus direct `will-change`, fixed, sticky, video and scroller hints. The caller
identifies video nodes because layout retains node identity but not element
names. Public item records contain no `LayoutBox` references.

`raster_range` replays the private paint command stream with its original
effect and clip stack, selects a contiguous range of drawable items, then
translates and clips the resulting RGBA ink into the caller's surface. This
preserves the existing full-document entry points and their pixels. The
initial seam uses a full-viewport scratch surface for each call; tile-local
raster allocation and command culling remain a compositor performance followup.

At the same fixed base, win-002 locked gates passed image **47/47**, paint
**94/94**, workspace **1,791/1,791** with four skipped, and workspace
doctests **2/2**. Image and paint doctests each had zero cases and passed.
The 12-case focused WPT selection stayed **8 pass / 4 mismatch** before and
after RC-16, with identical per-case classifications and image hashes. The
35-case retained SVG selection stayed **28 pass / 7 mismatch, 28 painted
passes**, also with identical hashes to RC-15 after. Reports are in
`tests/wpt-replaced-content/rc16/`. The fixed-base gate snapshot was
`dd2fbee` with root lock SHA-256 `ADD98EE1…` and standalone shell lock
`EA93BD5…`; it is a local test snapshot, not a pushed branch.

## RC-17a — raw local-space compositor tiles

`DisplayListView::raster_raw_local` draws a selected item range into a caller's
tile using an untransformed document-space tile origin. It leaves group
transform, opacity and overflow clipping to the compositor. Decoration and
shaped-text items are separable; controls and replaced-fragment content return
an explicit `UnseparableItem` error without changing the target. This is an
additive path; `raster_range` retains its full-paint behavior.

Fixed base `9571c04`, source commit `ca868ad`: win-002 locked paint nextest
**98/98**, workspace **1,918 passed / 5 skipped**, and workspace doctests
**2/2**. The pinned WPT focused selection stayed **10 pass / 2 mismatch**;
the retained SVG selection stayed **25 pass / 9 mismatch / 1 crash**. The SVG
crash is the same reference page script error before and after. All **47**
per-case result records, including rendered image hashes, matched exactly;
there were zero pass losses. Reports, source hashes and comparison are in
`tests/wpt-replaced-content/rc17a/`. Generated-box ordering and URL-keyed
images remain RC-17b after CSS-13 and LAYOUT-13 merge.

## RC-17b — generated order and URL-keyed images

Paint orders every sibling box by layout's DOM pre-order `source_order` when
present, retaining the old NodeIdx fallback for manually built boxes. The
display-list regression creates a newer arena node, inserts it before an older
sibling, and verifies the resulting paint command order. Existing positioned
and stacking buckets still determine the painting phase.

Generated replaced fragments use `(NodeIdx, source_order)` to recover their
styled URL and paint through `ImageProvider::image_by_url(&str)`. A missing URL
resource does not reuse the originating node's image. Ordinary replaced
elements continue to use `image(NodeIdx)`. Layout owns the parallel
`ReplacedElementProvider::intrinsic_size_by_url(&str)` query; the production
driver must fill both URL lookups from its decoded cache. The two-URL paint
test verifies distinct pixels for generated runs sharing one origin node.

At fixed base `ba83010`, the locked win-002 gate passed paint **103/103**,
image **47/47**, workspace **2,026 passed / 5 skipped**, and workspace
doctests **2/2**. A granted image test correction replaced an obsolete
unsupported `lab()` expectation with unsupported `context-fill`; the former
failed on the clean base after CSS-12 added Lab color support. The pinned
generated-content seven-case selection stayed **2 pass / 5 mismatch** across
two exact before/after pairs. RC-17a's focused 12 stayed **10/2**, and its SVG
35 stayed **25/9/1 reference script crash**. All **54** per-case records and
image hashes were identical, with zero pass losses. These are raw counts; the
missing production URL lookup means the WPT selection shows no URL rendering
gain yet. Reports, fixture hashes and comparison are in
`tests/wpt-replaced-content/rc17b/`.

## RC-18 — generated gradient raster and compositor geometry metadata

Paint display-list items now expose bounded untransformed content ink estimates,
scrollable extents with an optional adapter-supplied initial offset, and sticky
insets plus containing-block bounds. Decorations with uncertain shadow overflow
retain the viewport fallback. The raw local tile path and full painter remain
unchanged. Generated-image keys that contain CSS linear, radial or conic
functions, including repeating variants, raster through the same replaced
fragment path as URL images. Paint uses the CSS tokenizer and color parser;
fragment geometry and font size determine sampling. Unsupported gradient
syntax returns no image rather than painting a guessed value.

At fixed base `e762837`, win-002 locked gates passed paint **107/107**,
standalone image **47/47**, workspace **2,081 passed / 5 skipped**, and
workspace doctests **2/2**; paint and image doctest sets have zero cases.
The pinned four-case generated-gradient/scroll selection moved **3 pass /
1 mismatch** to **2 pass / 2 mismatch**. The raw pass loss is
`css/css-lists/list-style-image-gradients.html`: before, both the gradient
marker and its data-SVG reference marker were absent; after, paint draws the
gradient but the driver still leaves the data-SVG reference absent. Do not
count this as a conformance gain. The retained focused 12 stayed **10/2** and
SVG 35 stayed **25/9/1 reference script crash**, with identical per-case
classifications and actual/reference/diff hashes in all four runs and zero
losses. Evidence and source/hash audit are in `tests/wpt-replaced-content/rc18/`.

CSS currently rejects gradient image functions in `content`, so generated
pseudo content needs the routed CSS parser seam before it reaches paint.
Layout's no-intrinsic-size fallback makes gradient markers 300×150 until its
marker sizing rule is supplied; the driver also needs its data-SVG reference
resource. These cross-lane follow-ups are in the replaced-content questions.

## RC-19 — stable paint item keys and conservative display-list deltas

Paint items now carry an optional stable key `(DOM NodeIdx, arena generation,
paint phase, local ordinal)`. `DisplayListView::with_dom_generations` uses
`Dom::node_generation` from the fixed base; older constructors remain the full
paint oracle and deliberately cannot publish a delta without generations. The
64-bit compositor adapter hash is checked for duplicates and collisions.

`old.update(&new)` compares bounded, freshly constructed
views at one frozen epoch. It returns changed keys and contiguous item ranges,
old/new local ink bounds expanded one pixel, and retired URL/generated/replaced
resource identities. It compares each item raster against the full painter and
falls back explicitly for order changes, unknown/reused generation, property
state changes, unseparable effect groups, unsafe bounds, raster failures and
large views. The first implementation is a correctness seam, not yet an
incremental builder: it may raster each item across the full viewport and caps
comparison at 512 items / 1,048,576 pixels. The caller keeps the fresh full
view for presentation when the result is `RequiresFullRebuild`. The compositor
must still validate chunk containment before accepting old/new bounds.

Paint exports the integer source-over helper and `quantize_opacity` adopted by
the compositor in the paired merge. Half-opacity over opaque white rounds to
127 under the documented premultiplied rule. The fixed RC-19 delta base is
pushed `5860d66`, which also contains the DOM generation resolver, per-test
storage isolation and the granted driver assertion updates. The refreshed
win-001 tree matched 14/14 audited source, fixture, test and lockfile hashes
and the pinned WPT revision. The synthetic mutation tests assert three delta
updates, ten explicit full fallbacks and one DOM resolver case; details are
retained in `tests/wpt-replaced-content/rc19/mutation-results.json`. On the
fixed base, win-002 paint passed 118/118, workspace passed 2223/2223 with five
skipped, and workspace doctests passed 2/2. The retained 51-case WPT pair is
recorded separately in `tests/wpt-replaced-content/rc19/final-comparison.json`:
focused 10/2, SVG 26/8 plus the known reference-script crash, and
gradient/scroll 2/2 in both rounds. There were no pass losses. SVG
`non-scaling-stroke-010` changed from mismatch to pass, and the gradient marker
remains a mismatch with fewer differing pixels (18,864 to 3,072). Those two
cases alone changed actual/diff hashes across the newer merged base; every
reference hash and both rounds agree.

## RC-20 bounded retained paint update (2026-09-24)

COMP-11 property ownership landed first as `2bee652`: non-root transform,
opacity/effect and scroll property nodes carry `PaintNodeKey { node, generation }`.
`DisplayListView::property_owner_mapping()` returns view-local IDs for each CSS
property kind. A missing or duplicate owner returns `None`, requiring scene
rebuild; synthetic root nodes are excluded.

`RetainedPaintView::from_view` now owns the seed item's metadata, local raw
raster and resource set. `apply_damage` accepts keyed `PaintDamage::Remove` and
`PaintDamage::Upsert` values from layout's geometry/style damage. It validates
the whole damage set before mutation, rasterizes only upserts, keeps untouched
pixel buffers allocated, and reports changed ranges, added/removed keys,
conservative old/new bounds and resources no longer referenced by any item.
Both individual tile and aggregate retained allocations are bounded. Unstable
identity, property-table changes, unsafe geometry, and unsupported item raster
return a full-rebuild reason. The old two-full-view comparison remains an
oracle for same-epoch parity checks.

The pushed RC-20 base is `f66be2b`. `LAYOUT-17` and the requested
`runtime_host_replaced-content.rs` adapter are not present on this base; the
paint-local delta type is ready for their damage list when the owning lanes
publish it. IA-ORACLE-PAINT's 31 ms p95 cannot be credited until that driver
adopts the retained path and the Windows release trace measures it.

## RC-21 — responsive/lazy images, scroll fetch and the COMP-14 owner mapping

Landed on `lane/replaced-content-rc21`, merged dev `988b1c4d` (includes
COMP-13b/14, IPC-10, CSS-15, DOM focus, IA-REAL-PAGES). Three owned changes:

- **Paint scroll owners across reopened clips** (`a70711f0`). A stacking or
  positioned context re-opens the overflow clips it inherits, so one CSS
  scroller opened two `ScrollNode`s in a single display list. The duplicate
  `(node, generation)` owner made `DisplayListView::property_owner_mapping`
  return `None`, which made `RetainedPaintView::from_view` report
  `AmbiguousPropertyOwner` for the COMP-14 effects-scroller oracle page. Paint
  now reuses one scroll node per owner; clip nodes stay per scope. Regression:
  `effects_scroller_property_owners_stay_unique_across_reopened_clips`.
- **Script-detached `<img>` loading** (`c18d13f9`). `load_page_media_in_viewport`
  now visits every live arena slot for an `img` the document walk cannot reach
  (`new Image()`), so the request starts and a pending `decode()` promise can
  settle; detached loading is always eager.
- **Shell lazy fetch after scroll** (`a5734b23`, `cb84e972`, `b8f413ab`). Every
  successful scroll (wheel, trackpad, keyboard, scrollbar drag) reads the
  retained root scroll offset from COMP-13b and fetches deferred lazy images
  whose laid-out box entered the viewport, then repaints the scrolled frame.
  Layout already reserved each deferred box, so pixels paint into it until the
  next DOM snapshot relayouts with the real intrinsic size.

Boundaries, not silently credited: inline replaced content still forces the
baked paint fallback, whose property trees are empty, so a page with an inline
`<img>` has no compositor root scroll node and the lazy-scroll fetch currently
exercises block-level images only until RC-17/paint makes inline replaced items
separable in `raster_raw_local`. `new Image()` still needs the DOM-bindings
legacy factory global; only the loader side is done. `image-orientation:none`
through the sandboxed utility remains routed to IPC.

**Linux-first gate** (per the 2026-09-26 orchestrator change; Windows render
sweeps deferred). Base `988b1c4d`, Boat sandbox warm:

- `cargo nextest run --workspace --locked --no-fail-fast`: **2,543 passed,
  0 failed, 10 skipped**.
- `cargo test --workspace --locked --doc`: **2 passed, 0 failed**.
- `amazing-paint` includes the new regression; Linux paint was **126/126**
  before the dev merge.
- Pre-Linux-first Windows run at `b8f413ab`: shell
  `scrolled_deferred_lazy_image_is_fetched_and_repainted` passed, shell
  `--locked` build green. The shell is a Windows-only GPUI workspace and is not
  part of the Linux workspace gate.

The RC-21 pinned render (226) and testharness (117) selections and their
reproduction scripts live in `tests/wpt-replaced-content/rc21/`. Both runners
are Windows-only (`scripts/run-render-wpt.ps1` and the `wpt-harness` subcommand
that byte-pins against the checkout), so no Linux corpus before/after is
claimed here; the recorded Windows baseline is 185/6/9/26 render and the
harness candidate 4/58/45/8/2. These are not conformance movement.

## RC-22 — separable inline replaced content in the raw tile path (2026-09-26)

Fixed on top of the RC-21 branch. `raster_raw_local` returned
`UnseparableItem` for any `PaintContent` box whose lines carried a replaced
fragment, so a page with an inline `<img>` fell back to the baked painter,
which has empty property trees and therefore no compositor root scroll node.
That is why the RC-21 scroll-triggered lazy fetch exercised block-level
replaced images only.

The full painter and the raw tile path now share `paint_replaced_fragments`:
it resolves a line's replaced fragments (generated URL, SVG root, or provider
image) against the **original** box, whose inline flow still carries the
styled-fragment metadata, with an optional document-space offset. The raw
content path paints replaced fragments from the original box at the tile
offset and the shifted glyphs from a local line clone; it still skips group
transforms, opacity and clips, which remain compositor-owned.
`raster_raw_local` now accepts `PaintContent` items with or without replaced
fragments. No pixels changed: the raw tile equals the full painter for the
item, and `RetainedPaintView::from_view`/`apply_damage` accept such items.

Linux gate at `d7d67c76`: workspace nextest **2,703 passed / 0 failed / 10
skipped** with `--no-fail-fast`; separate workspace doctests **4 passed / 0
failed**. The focused `amazing-paint` + `amazing` bulk run was **342 passed /
4 skipped**, including the new paint regression
`raw_tile_rasterizes_inline_replaced_content_and_retains_it` and the adapter
test `inline_replaced_content_retains_and_keeps_the_root_scroll_owner`.

Known adjacent gap, not claimed: the retained path's per-item resource set
(`retained_paint::resources_for_box`) still records no resource for inline
replaced fragments on a content item, unlike the old/new delta's `resources()`.
Seeding and damage must agree before enabling retirement for them, so this
stays a follow-up when the integrator-assist damage adapter lands.

## RC-20 — gradient background-image layers (2026-09-26)

Deferred from the RC-18/RC-19 reprioritization. `BackgroundLayer` now carries an
optional serialized `<gradient>` function beside the existing `url()`.
`paint` resolves a gradient layer through the same RC-18 `gradient::raster` at
its used size: a gradient has no intrinsic dimensions, so `auto` resolves to the
background positioning area and the tile is rasterized there; `cover`,
`contain` and explicit sizes scale from the positioning area. URL layers keep
the provider's decoded pixels and natural size. Layer order (first listed
topmost), repeat modes, position, `background-origin`/`background-clip` and the
shared per-frame sample budget stay in the existing loop. No CSS parsing and no
duplicate rasterizer were added; unsupported gradient syntax paints no layer
rather than guessing.

Recognized functions: `linear-gradient`, `repeating-linear-gradient`,
`radial-gradient`, `repeating-radial-gradient`, `conic-gradient`,
`repeating-conic-gradient`. Nothing outside `amazing-paint` changed.

Four new focused paint tests assert pixels: linear fill of the positioning area,
repeating tiling through `background-size`, first-listed-layer-on-top ordering,
and unsupported syntax painting nothing. Linux gate at `e3e475fd`: workspace
nextest **2,707 passed / 0 failed / 10 skipped** with `--no-fail-fast`; separate
workspace doctests **4 passed / 0 failed**. The focused `amazing-paint` crate run
was **131 passed / 0 skipped**, including the four new cases.

The ticket's frozen WPT gradient reftest selection and its two-round
before/after are Windows-only and were not run under the Linux-first deferral,
so RC-20 claims **zero WPT conformance credit**; the retained RC-18 focused 12,
SVG 35 and gradient/scroll 4 selections are likewise unclaimed. Reproduction and
rationale: `tests/wpt-replaced-content/rc20/README.md`. Per-paint gradient
caching is a performance follow-up, not a correctness gap.

## RC-23 — positioned and shaped radial/conic gradient geometry (2026-09-26)

Follow-up to RC-18/RC-20 in `amazing-paint::gradient`. Any leading geometry part
other than an exact `circle`/`ellipse` or a bare `from Ndeg` previously made the
whole gradient paint nothing: the optional-geometry parser could not distinguish
an unsupported geometry part from a color stop and fell through to treating it
as a stop, which then failed and rejected the image.

The first gradient part is now classified by whether it begins with a parseable
color. If it does, it is a stop; if not, it must parse as valid geometry or the
entire gradient is rejected. Supported geometry:

- radial: `[circle|ellipse]? [closest-side|farthest-side|closest-corner|
  farthest-corner|<length>]{0,2}? [at <position>]?`, defaulting to
  farthest-corner. Explicit radii are `px`/`em`/`rem` and share circle/ellipse
  sampling; corner keywords use a per-axis circle approximation for ellipses.
- conic: `[from <angle>]? [at <position>]?`, `<angle>` in `deg`/`grad`/`rad`/
  `turn`.
- linear angles also accept `grad`/`rad`/`turn` now.

`at <position>` reuses `crate::image::position` (keywords, percentages, px,
edge-offset syntax). Percent radii, unsupported units, malformed positions and
unknown keywords reject the gradient rather than guessing. No CSS, layout,
compositor or driver source changed.

Focused paint tests: positioned/shaped radial, explicit-radius radial, rotated
conic, and a background-level positioned radial, plus a rejection table for
percent radii, empty/oversized positions, duplicate size keywords and bad
angles. Linux gate at `6988c124`: workspace nextest **2,737 passed / 0 failed /
10 skipped**; separate workspace doctests **4 passed / 0 failed**; `amazing-paint`
**134 passed / 0 skipped**.

The ticket's WPT gradient selection and before/after remain Windows-only and
deferred under Linux-first, so RC-23 claims **zero WPT conformance credit**;
ellipse corner-keyword radii and percentage radii remain bounded approximations
or rejections, recorded here rather than claimed conformant.

## RC-24 — mix-blend-mode and isolation effect-group reference (2026-09-26)

Answer to the compositor lane's COMP-15 request for a full-painter reference.
`GroupEffect` and the public `DisplayListView::EffectNode` now carry a CSS
`BlendMode` and an `isolate` flag. `effect_for` opens an effect group for a
non-normal `mix-blend-mode` or `isolation: isolate`, and `composite_group`
composites the group layer with the new integer
`amazing_paint::blend_group(mode, src, dst, opacity)`: blend each channel
against the backdrop, then source-over, per CSS Compositing and Blending.
All 17 modes are implemented, including plus-lighter and the non-separable
hue/saturation/color/luminosity formulas (W3C algorithm, rounded once);
`Normal` reduces exactly to the shared `blend_source_over`. The blend math is
integer so the reference is bit-deterministic across platforms, matching the
existing source-over rule.

Focused tests: separable blend modes against white/black, alpha/opacity
compositing, non-separable color keeping the backdrop luminance, a
paint-document multiply child over a gray parent, and `isolation: isolate`
opening a group without changing normal pixels. No CSS, layout, compositor,
driver or lockfile source changed.

Linux gate at `608e546c`: workspace nextest **2,742 passed / 0 failed / 10
skipped**; separate workspace doctests **4 passed / 0 failed**; `amazing-paint`
**139 passed / 0 skipped**.

Compositor handoff: `crates/amazing-compositor/src/paint_view.rs` still passes
`BlendMode::Normal` to `trees.add_effect`; map `node.blend`/`node.isolate` into
its `EffectOperations` and adopt a matching reference. Filters, backdrop-filter,
clip-path and mask are not part of RC-24 and remain a separate paint slice.
No WPT credit is claimed.

## RC-25 — CSS filter color-function reference (2026-09-26)

Second COMP-15 paint reference slice, the color half of `filter`. A new bounded,
`Copy` `amazing_paint::filter::FilterPlan` parses the computed filter list's
color functions (`brightness`, `contrast`, `grayscale`, `hue-rotate`, `invert`,
`opacity`, `saturate`, `sepia`) in source order into a capped fixed array of
`FilterOp`. Blur, drop-shadow, `url()` and `calc()` functions are skipped rather
than guessed (the painter previously deferred the whole filter property).
`effect_for` opens a group for a non-empty plan, `GroupEffect`/`EffectNode`
carry the plan, and `composite_group` applies it to the group layer before
blending, so the filter covers the element's own decorations and its subtree.
Color math is the Filter Effects definitions (luma 0.2126/0.7152/0.0722, the
sepia and hue-rotate matrices), clamped and rounded per channel.

Focused tests: plan order and unsupported-function skipping; grayscale,
brightness(0), invert(1) and opacity(0.5) pixel values; and a paint-document
test that the filter changes only the element subtree. No CSS, layout,
compositor, driver or lockfile source changed.

Linux gate at `4176b349`: workspace nextest **2,745 passed / 0 failed / 10
skipped**; separate workspace doctests **4 passed / 0 failed**; `amazing-paint`
**142 passed / 0 skipped**.

Still outstanding for COMP-15: `backdrop-filter`, blur/drop-shadow filters,
clip-path basic shapes and mask-image, which need the compositor's
`EffectOperations` metadata contract and raster transforms. No WPT credit.

## RC-26 — `blur`/`drop-shadow` filters and `backdrop-filter` reference (2026-09-26)

Third COMP-15 paint reference slice, answering the compositor's
`PaintFilterSource`/`PaintFilterInput` producer-boundary preparation
(`.agents/compositor/questions.md`, `crates/amazing-compositor/src/paint_filters.rs`
at fixed source base `431a0a04`): ordered resolved filters, `backdrop_filters`
and `backdrop_bounds` for an existing non-root effect group.

- `amazing_paint::filter::FilterOp` gains `Blur(std_px)` and
  `DropShadow { dx, dy, std, color }`; `FilterPlan::from_css` now takes the
  element's resolved `current_color` (for `drop-shadow()`'s default and
  explicit `currentcolor`) and parses both functions. Only an already-resolved
  absolute length (the CSS parser folds `in`/`cm`/`mm`/`q`/`pt`/`pc` into
  `Px`) is accepted for blur radius/shadow offsets; `em`/`rem`/percentage/
  viewport-unit arguments are skipped, not guessed, matching the existing
  color-function policy.
- `FilterPlan::apply_to_surface` is the new whole-surface entry point: color
  functions still map every pixel independently, but `blur`/`drop-shadow` read
  neighboring pixels and can only run this way. `composite_group` now always
  filters the whole child layer once before its per-destination-pixel
  transform sampling, instead of calling a per-pixel `apply` inside that loop;
  nearest-neighbor sampling makes this exactly equivalent for the color-only
  case and is required for the two spatial functions.
- Blur is the SVG/CSS Filter Effects three-box-blur Gaussian approximation
  (box size `d = floor(std * 3 * sqrt(2π) / 4 + 0.5)`), run in premultiplied
  space with the surface edge clamped outward, and using the spec's exact
  odd/even pass radii (an even `d` cancels its two-pixel-biased passes with
  one pass shifted left, one shifted right, then a centered `d+1` pass) so
  the result stays centered rather than drifting. Drop-shadow tints the
  source's alpha into the resolved shadow color, blurs that shadow, shifts it
  by the offset (a shift past the surface edge is discarded, matching this
  painter's per-surface, not per-halo, filter model), and draws the original
  content over it.
- `backdrop-filter` is new on `GroupEffect`/`EffectNode`, with the group's
  border box as `backdrop_bounds`. `RasterState::begin_group_with_mask` now
  seeds a group's fresh, otherwise-transparent layer with the *filtered*
  snapshot of the surface directly behind it (before this box's own
  decorations are drawn), cropped to that border box; the box's own
  decorations then paint over that seed exactly as they would over any other
  backdrop (existing decoration painting already alpha-blends, not
  overwrites). A box with only `backdrop-filter` (no opacity/transform/blend/
  filter/isolation) now opens a group, the same filter-only-group policy RC-25
  established for `filter`.

Focused tests: blur/drop-shadow parsing order and relative-length skipping;
`drop-shadow()`'s offset/blur/color resolution including bare `currentcolor`
defaults; a uniform field's interior staying unblurred while a single spike
spreads into (and stays bounded within) its neighbors; a drop shadow drawn
offset and behind its source; and three `paint_document` end-to-end tests
(`backdrop_filter_group_filters_the_snapshot_behind_it`,
`blur_filter_group_softens_its_own_content_edge_into_its_surrounding_transparency`,
`drop_shadow_filter_group_draws_its_shadow_behind_its_own_content`). No CSS,
layout, compositor, driver or lockfile source changed.

Linux gate: workspace nextest **2,754 passed / 0 failed / 10 skipped**;
workspace doctests **4 passed / 0 failed**; `amazing-paint` **149 passed / 0
skipped** (up from 142 at RC-25).

Still outstanding for COMP-15: `clip-path` basic shapes/`path()` and
`mask-image`/`mask-*`, and the reference-space transform contract for
arbitrary local-space CSS filters under a 3D/perspective transform (this
slice's filters run in the group's own post-transform raster space, matching
`PaintFilterSpace::Raster`). No WPT credit is claimed under the Linux-first
deferral.

## RC-27 — `clip-path` basic-shape reference (2026-09-26)

Fourth COMP-15 paint reference slice: `inset()`, `circle()` and `ellipse()`.

- New `amazing_paint::clip::ClipMask { x, y, width, height, radii: [(f32,f32);4] }`
  (top-left/top-right/bottom-right/bottom-left order) resolves `inset()`
  directly and maps `circle()`/`ellipse()` onto their own bounding box with
  every corner sharing one radius pair — the same per-corner elliptical test
  this painter already uses for rounded-rectangle overflow clipping
  reproduces an exact circle/ellipse when the box's width/height equals
  `2*rx`/`2*ry`. `inset()`'s offsets and `round` radii both use the CSS
  four-value shorthand expansion (1/2/3/4 values); `circle()`'s keyword
  radius resolves against the closest/farthest of all four sides per CSS
  Shapes 1, while `ellipse()`'s two keyword radii resolve per axis
  independently. `polygon()`, `path()`, a `url()` reference and a bare
  `<geometry-box>` with no shape are skipped, not guessed, matching this
  painter's existing policy for unsupported filter functions; so are
  `em`/`rem`/viewport-relative lengths, which need font/viewport context this
  bounded resolver does not have. Every shape resolves against the box's own
  border box regardless of an explicit `<geometry-box>` keyword (this
  resolver does not track a separate padding/margin reference box) — a
  documented simplification, not a spec claim.
- `GroupEffect`/`EffectNode` gain `clip: Option<ClipMask>`; a `clip-path`
  alone (no opacity/transform/blend/filter/isolation) now opens a group, the
  same filter-only-group policy RC-25/RC-26 established. `RasterState::
  begin_group` converts the resolved `ClipMask` into the same `RoundedRect`
  mask `push_overflow_clip`'s rounded-corner overflow clip already uses, so
  clip-path composites through the identical, already-tested per-pixel mask
  check in `composite_group`.

Focused tests: `inset()`'s offset/round four-value expansion and percentage
resolution; `circle()`'s closest-side default and centering; `circle()`'s
explicit radius/position producing an exact disc (inside/outside/corner
checks); `ellipse()`'s independent per-axis radii; `polygon()`/`path()`/a bare
geometry keyword/`none` all resolving to no mask; and a `paint_document`
end-to-end test (`clip_path_circle_clips_the_elements_own_content_to_the_disc`)
confirming the box's own painted content is clipped to the circle while its
bounding-rect corners show the parent's background through. No CSS, layout,
compositor, driver or lockfile source changed.

Linux gate: workspace nextest **2,761 passed / 0 failed / 10 skipped**;
workspace doctests **4 passed / 0 failed**; `amazing-paint` **156 passed / 0
skipped** (up from 149 at RC-26).

Still outstanding for COMP-15: `polygon()`/`path()` clip-path shapes and
`mask-image`/`mask-*`. No WPT credit is claimed under the Linux-first
deferral.

## RC-28 — `<img src="*.svg">` rendered nothing (2026-09-26)

Visible bug, reported against `tests/real-pages/pages/docs-index.html`: the
header logo and all six card thumbnails (`../assets/logo.svg`,
`thumb-N.svg`) painted nothing. `amazing render-page` on that page reported
`amazing render: image "...logo.svg": UnsupportedSvg` for all seven images,
in ~1.2s with a clean `"status":"ok"` exit — a genuine decode rejection, not
an IPC/utility-process timeout (the layout lane's held-image-loss hypothesis
did not reproduce here; the sandboxed decode path and its Linux fallback both
call the same `amazing_svg::decode`, and neither hung).

Root cause: `amazing_svg::svg::parse` (the byte decoder behind `<img
src="*.svg">`, distinct from the DOM-based `from_dom`/`visit_dom` used for
inline `<svg>`) rejects the *whole* document on the first unrecognized
element, per its documented "never return a partial successful SVG" policy.
Both fixtures pair a supported `<rect>` background with a `<text>` label;
`<text>` was not in the element dispatch at all, and the tokenizer also
rejected any non-tag character data (the text content itself), so either one
alone would already have failed the file.

Fix, consistent with the DOM path's existing behavior for `<text>` (render
only the WPT-only Ahem-square heuristic; otherwise resolve to nothing rather
than reject): `append_element` now recognizes `text` structurally, like `g`
— it validates a small attribute allow-list (`x`, `y`, `dx`, `dy`,
`font-family`, `font-size`, `font-weight`, `text-anchor`) and contributes no
shape. The tokenizer allows raw character data only while the open-element
stack's top is `text`, skipping to the next `<` instead of erroring;
everywhere else, stray character data is still rejected, so the "reject
unsupported structure as a whole" policy is unchanged for every other
element. This does not render glyphs (out of scope per the module's "no
general text" subset) — it stops a label next to a supported shape from
blanking the entire image, which is what the corpus fixtures need. Intrinsic
size and paint were otherwise already correct once decode succeeds: the
`Image` carries the SVG's own declared width/height, and layout/paint's
existing replaced-content sizing needed no change.

Regression coverage:
- `amazing-svg`: `rect_next_to_a_text_label_still_decodes_and_paints_the_rect`
  and `text_before_a_shape_still_decodes` prove the fix; three negative tests
  (`unbalanced_text_tag_is_still_rejected`,
  `character_data_outside_text_is_still_unsupported`,
  `unknown_attribute_on_text_is_still_rejected`) prove the strictness policy
  still holds everywhere else.
- `crates/amazing/tests/svg_image_render.rs` is new: it renders
  `docs-index.html` end to end through `amazing render-page` and asserts the
  logo's `#0a58ca` fill and a thumbnail's `#2f6f4e` fill both actually paint
  (a tolerant ≥200-pixel color match for the CSS-scaled logo, an exact
  ≥60-pixel horizontal run for the thumbnail painted at intrinsic size),
  plus that `render-page`'s stderr no longer contains `UnsupportedSvg`. The
  existing `real_pages_corpus_renders_without_crashes` test continues to pass
  across the full ~50-page corpus.

No CSS, layout, compositor, driver or lockfile source changed; only
`amazing-svg`'s byte decoder and a new `amazing` integration test.

Linux gate: workspace nextest **3,032 passed / 0 failed / 12 skipped**;
workspace doctests **4 passed / 0 failed**; `amazing-svg` **5 passed / 0
skipped** (new); `amazing` `svg_image_render` **1 passed**; `real_pages`
corpus **52 pages, 0 crashes, 0 blank pages** (unchanged pass/fail shape,
image decode failures gone from stderr). The jump from RC-27's 2,761 to
3,032 workspace tests reflects `dev` catching up other lanes' merged work
(`git merge dev` before this ticket), not this fix alone. No WPT credit
claimed; the pinned RC-21 render/testharness selections stay Windows-only
under the Linux-first deferral.

## RC-29 — `clip-path` `polygon()`/`path()` and `mask-image`/`mask-mode` reference (2026-09-26)

The COMP-15 remainder filed in `questions.md` after RC-27: the two
`clip-path` basic shapes still skipped, plus a new `mask-image`/`mask-*`
paint reference slice.

**`clip-path`: `polygon()` and a straight-edged `path()`.** `ClipMask`
(`crates/amazing-paint/src/clip.rs`) is now an enum: `Rect` (the existing
`inset()`/`circle()`/`ellipse()` per-corner rounded-rect test, unchanged)
and a new `Polygon { points: [(f32, f32); MAX_CLIP_POINTS], len, even_odd }`
tested by winding rule (`crate::clip::point_in_polygon`, shared with the
raster mask below). `polygon()`'s vertices resolve directly (no path-string
parsing: they are already `(LengthPercentage, LengthPercentage)` pairs).
`path()` gets a new bounded straight-edge-only data parser
(`parse_straight_path`): `M`/`L`/`H`/`V`/`Z`, absolute or relative, one
closed subpath. A curve/arc command (`C`/`S`/`Q`/`T`/`A`), a second subpath,
more than `MAX_CLIP_POINTS` (32) vertices, or malformed syntax is skipped
rather than approximated — the same policy RC-27 used for `polygon()`/
`path()` itself and for relative-unit filter arguments. The vertex cap keeps
`ClipMask` `Copy`, like `FilterPlan`, so it stays inside the copyable paint
command and effect-node records.

`amazing_paint::paint`'s raster side gets a matching `RasterMask` enum
(`Rounded`/`Polygon`) so `RasterGroup`/`begin_group`/`composite_group` no
longer assume every mask is a rounded rect; `push_overflow_clip`'s existing
CSS-overflow rounded clip now wraps in `RasterMask::Rounded`, unchanged in
behavior.

**`mask-image`/`mask-mode`.** New `crates/amazing-paint/src/mask.rs`:
`MaskLayer::from_css` resolves a *single* `mask-image: url(...)` layer (the
first `url()` reference; gradients, `image()`/`image-set()`/`element()`/
`cross-fade()`/`paint()`, more than one layer, and `mask-composite` are
skipped — the element paints unmasked, which is what CSS Masking specifies
for an unresolved mask reference anyway). `mask-mode: alpha` reads the
referenced image's alpha channel directly; otherwise (`luminance`, or
`match-source`'s default for a raster image) the same Filter Effects luma
coefficients this painter already uses for `grayscale()` convert color to a
mask value, weighted by the pixel's own alpha. The image is stretched to
exactly fill the masked box's own border box (`mask-size: 100% 100%`,
`mask-position: 0 0`, `mask-repeat: no-repeat`); `mask-size`/`mask-position`/
`mask-repeat`/`mask-clip`/`mask-origin` are not read, a documented
simplification in the same spirit as `clip-path` always resolving against
the border box regardless of an explicit `<geometry-box>`. `GroupEffect`/
`EffectNode` gain `mask_image: Option<MaskLayer>` and `mask_bounds: Rect`; a
`mask-image` alone now opens a group too, the same filter/clip-path-only
group policy RC-25/26/27 established. Applying the mask needed `images: &dyn
ImageProvider` at composite time, which `end_group`/`pop_clip` did not
previously receive; both now take it and thread it from all three raster
command loops that already had it in scope. `composite_group` folds the
per-pixel mask value into the same opacity factor `blend_group` already
takes, so mask-image, opacity and blend mode compose correctly together.

Focused tests: `polygon()` nonzero-winding containment and percentage
resolution against the reference box; `path()` with only straight commands
producing the same closed-outline containment, its `evenodd` keyword
threading through, and a curve command / too many vertices being skipped;
`mask-image`'s `none`/gradient skip, single-`url()` resolution (both the
unquoted-URL-token and quoted-string-function tokenizer forms), a second
layer being ignored, `alpha` vs `luminance` mode, and out-of-extent sampling
masking out; two new `paint_document`/`paint_document_with_resources`
end-to-end tests (`clip_path_polygon_clips_the_elements_own_content_to_the_triangle`,
`mask_image_group_uses_the_referenced_images_luminance_as_alpha`). No CSS,
layout, compositor or driver source changed; the two `GroupEffect`/
`EffectNode` field additions are backward-compatible (no other crate reads
`ClipMask`'s fields directly yet — compositor's own `EffectOperations`
adoption is still open per `questions.md`).

Linux gate: workspace nextest **3,044 passed / 0 failed / 12 skipped**;
workspace doctests **4 passed / 0 failed**; `amazing-paint` **168 passed / 0
skipped** (up from 156 at RC-27: net +6 in `clip.rs`, +5 new in `mask.rs`,
+2 new end-to-end tests in `paint.rs`, -1 retired
`polygon_path_and_bare_geometry_box_are_skipped_not_guessed`, replaced by
positive coverage plus a narrower negative test). `amazing-compositor` and
`amazing` both build clean against the changed `GroupEffect`/`EffectNode`
shape. No WPT credit claimed; the pinned RC-21 render/testharness selections
stay Windows-only under the Linux-first deferral.

COMP-15 is now fully covered by this lane's paint reference: `filter`,
`backdrop-filter`, `mix-blend-mode`/`isolation`, and every `clip-path` basic
shape plus a single-layer `mask-image`. Outstanding for a future ticket, if
requested: multi-layer masks and `mask-composite`, `mask-size`/
`mask-position`/`mask-repeat`, and curved `path()` (needs a bezier/arc
flattener, deliberately out of scope here to keep this ticket bounded).

## RC-30 — SVG coverage for real content (2026-09-26)

integrator-assist counted 121x `UnsupportedSvg` across the 52-page real-pages
corpus. Two independent fixes, both inside `amazing-svg`.

**Byte-decoder tolerance.** `amazing_svg::svg::parse` (the `<img src="*.svg">`
byte decoder, distinct from the DOM inline-`<svg>` path) treated any element
outside a small dispatch table as a fatal parse error, rejecting the whole
document — not the SVG 2 error-handling model, where an unrecognized element
(and its subtree) is skipped, not fatal. Fixed with a new `ignored:
Vec<String>` stack alongside the existing frame stack: an open tag not in
`is_known_element`'s whitelist (`svg g text rect circle ellipse path line
polygon polyline`) pushes onto `ignored` instead of `append_element`, and
everything under it — character data, further unknown children, malformed
attributes — is silently skipped until its balanced close tag; the existing
strictness for genuinely malformed *known* elements (duplicate attributes,
mismatched close tags, depth limits) is unchanged. `tests/real-pages/
generate.py`'s SVG template now prepends a `<title>` to every generated
asset, deliberately reintroducing a realistic unknown element as a
reproducible before/after fixture.

**Real `<text>` rendering.** SVG `<text>` only ever drew pixels when
`font-family` contained the literal substring `"Ahem"` (the WPT font); any
real `font-family` hit an early return and drew nothing. Real text now
routes through the same pipeline HTML text painting uses —
`amazing_layout::font::FontSource` for advances, `amazing_paint::text`'s
`OutlineGlyphProvider`/`RectInkProvider` for rasterized coverage bitmaps —
with `fill`, `stroke` (an 8-compass-direction glyph-run splat at a clamped
radius, since the glyph rasterizer has coverage bitmaps, not vector outlines,
to stroke exactly), `text-anchor`, and `x`/`y`/`dx`/`dy`, in both the
byte-decoder path (`finalize_text`) and the DOM path (`append_dom_text`,
concatenating direct `Text` children). The Ahem-exact WPT path
(`append_ahem_text`/`append_ahem_text_path`) is unconditionally unchanged,
gated on `font-family` containing "Ahem" exactly as before. The rasterized
run is embedded as a `Shape::Image` item so the existing per-pixel
compositing loop paints it with no renderer changes. New `amazing-svg` ->
`amazing-layout`/`amazing-paint` Cargo dependency edges (confirmed acyclic:
neither, nor their transitive deps, depend back on `amazing-svg`).

18 new/extended tests in `amazing-svg` cover unknown-element tolerance
(nested, self-closing, mismatched-close-inside-ignored, unbalanced-at-EOF,
unknown-root-still-rejected) and real-text rendering (fill, stroke widening,
anchor shift, dx/dy offset, absurd font-size rejected, whitespace-only draws
nothing). `crates/amazing-image/tests/svg.rs`'s
`unsupported_svg_fails_whole_image_without_partial_success` had asserted
`<script>`/`<image>` were fatal — true only because of the old catch-all
policy this ticket deliberately reverses — so it was split: genuinely
invalid cases stay in that test, a new
`unrecognized_elements_are_skipped_not_fatal` asserts the corrected
tolerant behavior.

**Measured (real-pages corpus, 52 pages):** `UnsupportedSvg` 121 -> 0.
**Linux gate:** `amazing-svg` 18/18; `amazing-image` 50/50; workspace
nextest and doctests carried in the combined RC-30+RC-31 gate below.

## RC-31 — `<geometry-box>` resolution and a Linux `css-masking`/`css-images` reftest pass (2026-09-26)

Ran `scripts/linux-render-wpt.sh replaced-content css/css-masking` (also
tried every `svg/*` pinned-WPT subdir — see `questions.md`, none produced
any eligible reftest, a harness discovery gap not an engine bug) and fixed
the largest fixable-in-lane cluster.

**Root cause.** `ClipMask::from_css` (`crates/amazing-paint/src/clip.rs`)
always resolved a `clip-path` basic shape's percentages/keywords, and a bare
`<geometry-box>` keyword with no shape, against the border box — regardless
of which box the CSS actually named. A bare box keyword alone was skipped
entirely (painting fully unclipped) rather than clipping to that box.

**Fix.** New `GeometryBoxes { margin, border, padding, content }` carries
all four reference rects (computed from `LayoutBox`'s margin/padding/
border/border_box/content_box in `display_list.rs`'s `effect_for`, the only
non-test call site) into `ClipMask::from_css`. A bare box keyword now clips
to that box's exact rect; `margin-box`/`border-box`/`padding-box`/
`content-box` each resolve against their own box. Per CSS Masking's used-
value table for non-SVG content, `fill-box` behaves like `content-box`;
`stroke-box`/`view-box` behave like `border-box` — confirmed directly
against the `<meta name="assert">` text of WPT's own
`clip-path-polygon-010/011/012.html`. (First pass lumped all three SVG-only
keywords into `content-box`, which is right for `fill-box` but wrong for
`stroke-box`/`view-box`; caught by re-running the reftest sweep after a
compile sanity check and diffing the before/after mismatch lists, which
surfaced `-011`/`-012` as new regressions — fixed by splitting the
`MaskBox` match arm, no scope change.) No CSS or layout grammar/data was
missing — everything needed (`amazing_css::effects::MaskBox`,
`amazing_layout::LayoutBox`'s public box fields) already existed — so this
stayed entirely inside `amazing-paint`, no cross-lane edit.

**Bonus in-lane fix, same series.** While clustering `css-images`
mismatches, found `amazing_paint::gradient::raster` rejected any gradient
with fewer than two color stops outright (three redundant length guards),
even though CSS Images 3 allows a single color-stop gradient
(`linear-gradient(green)`), rendering as a solid fill. `sample()`/
`resolve_stops()` already degenerate correctly for one stop; only the
early-rejection guards were wrong (loosened `< 2` to `< 1`/`is_empty()`).
One pre-existing test asserted the old wrong rejection for
`linear-gradient(to right, red)` and was split into a corrected positive
test (`single_stop_gradient_paints_a_solid_fill`).

**Measured, Linux numbers (`scripts/linux-render-wpt.sh`):**
- `css/css-masking`: pass 50 -> 55, mismatch 53 -> 48 (zero regressions,
  full before/after diff reconciled).
- `css/css-images`: pass 85 -> 92, mismatch 56 -> 49.

Remaining `css-masking` mismatches are out of lane scope (see `questions.md`
for the full breakdown): legacy `clip` property and CSS Shapes 2 `rect()`/
`xywh()`/`shape()`/`circle()` corner keywords/`polygon() round` all need new
`amazing-css` grammar (~28 tests total); `marginBox-1b/1c/1d` and
`contentBox-1e` need `outline` painting, which doesn't exist yet for any
element (4 tests, not a clip bug); multi-layer `mask-composite` is RC-29's
already-documented deferred scope (4 tests). Four `gradient-single-stop-*`
variants (hue-interpolation-method specific) remain unresolved in
`css-images`, unrelated to the single-stop fix.

**Linux gate (combined RC-30 + RC-31):** workspace nextest **3,092 passed /
0 failed / 13 skipped**; workspace doctests **4 passed / 0 failed**
(`amazing-a11y`, `amazing-devtools`, `amazing-html`, `amazing-js`);
`amazing-paint` **172 passed / 0 failed** (up from 168 at RC-29: +2 in
`clip.rs`'s test module net, +2 in `gradient.rs`, +1 corrected in
`background_tests.rs`); `amazing-compositor` unaffected — `ClipMask`'s own
enum shape didn't change, only `from_css`'s parameter type — confirmed with
a fresh `cargo build -p amazing-compositor --locked` plus its own test
suite (paint_filters 4/4, paint_view_pages 10/10, parallel_tile_pages 1/1).
