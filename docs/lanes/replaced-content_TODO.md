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
