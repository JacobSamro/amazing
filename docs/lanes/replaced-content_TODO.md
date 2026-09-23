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
