# Compositor lane follow-ups

## COMP-4 boundary

The RC-16 display-list view replays the painter's transform, clip and opacity
groups while rasterizing an item range. COMP-4 therefore uses one neutral
property-state chunk for pixel-exact real-page output. Its property and hint
metadata is imported separately for inspection. The 36-frame win-002 real-page
trace is exact but its baked frame build takes about twice full repaint time.

1. COMP-5 adopted RC-17a raw local-space tiles on fixed base `7b26163`.
   Article, effects, and two pinned replaced-content fixtures match full paint
   pixel for pixel. The 512 px win-002 real-page trace retains all tiles for
   scroll, transform and opacity updates: 0.92%, 0.75%, and 0.67% of full
   repaint time, with zero tile uploads. Controls and replaced content use the
   pixel-exact baked fallback until their commands can be separated.
2. Paint view should expose local ink bounds for content items, scrollable
   content extents and computed sticky insets. Until then layout supplies
   scroll extents, snap points and sticky constraints to the compositor.
3. Integrator-assist wires `GpuiFrame::element()` into the live viewport and
   retains `GpuiPresenter` per scene. Its frame damage tells the shell when to
   request repaint; resource deltas require continuity across frames.
4. When an active win-001 RDP session is available, run the staged
   `C:\Users\server\target\diagnostics\comp4_capture.ps1` through SSH and
   collect the proof PNG. The scheduled tasks clean up in `finally`.
5. A future GPU transform ticket needs a project ADR: the pinned GPUI color
   sprite API has no matrix field. COMP-4 draws axis-aligned tile sprites
   directly and uses a CPU-composed image for rotation, perspective, rounded
   clips, blend modes and effect passes.

## COMP-6 animation selection

The native compositor unit harness projects timing and event assertions from
three files at WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`:

- `web-animations/timing-model/animation-effects/simple-iteration-progress.html`
  and its paired `current-iteration.html`: before/after fill, fractional
  iterations, iteration start, and the two-iteration spec timing example.
- `css/css-animations/animation-iteration-event-001.html`: a seven-iteration,
  one-second animation with a negative four-second delay emits iteration
  elapsed times five and six seconds, then an end event.
- `css/css-transitions/event-dispatch.tentative.html`: run/start/end order and
  elapsed times for a delayed transition; cancellation and negative-delay
  elapsed times are covered by additional native tests.

These are projected native assertions, not an unmodified browser WPT run. The
shell does not yet install CSS Animation/Transition DOM event or Web Animations
bindings. Integrator-assist must connect the compositor's typed style and
keyframe inputs to CSS-14, apply main-thread updates, and dispatch returned
events and promise signals before the unmodified selections are measurable.

## COMP-7 full mutation baseline

`examples/mutation_trace.rs` alternates a class on a live parsed DOM, then
runs full style/layout, constructs a paint view, builds a new compositor frame,
and checks its pixels against the full painter. It measures from mutation to
CPU-composed frame on two existing compositor fixtures, 20 frames each,
512×512, fallback text, win-001 release build. Source was local `c167fb3`
plus COMP-7 edits, transferred as archive SHA-256
`73de3b76055b9e8efb12fd8fd0736e27fcfb47a07267114fc757d40ae68d4137`.

| Fixture | Mutation→frame p50 / p95 | Style+layout p50 | View p50 | Frame p50 | Items / uploads per frame |
| --- | ---: | ---: | ---: | ---: | ---: |
| Article class | 37.546 / 40.501 ms | 7.103 ms | 0.518 ms | 29.826 ms | 16 / 4 |
| Effects scroller class | 61.819 / 63.946 ms | 6.481 ms | 0.417 ms | 54.894 ms | 14 / 12 |

The measurement includes a fresh tile raster and reference CPU presentation
but excludes the separate parity paint, GPUI upload/presentation, network and
JavaScript. Every measured frame matched full-painter RGBA pixels. The trace
CSV and summary are under `crates/amazing-compositor/evidence/comp7/`.

## COMP-13 compositor scroll handoff

The compositor scroll controller accepts wheel, trackpad and touch pan deltas,
keyboard commands, scrollbar fractions, direct scrollTo targets, and resolved
smooth targets. It clamps offsets to layout's scroll extents, bubbles deltas to
scrollable ancestors, samples smooth and decaying momentum motion at vsync,
and coalesces asynchronous scroll events per scroll node. The driver adapter
returns a cached frame with no style, layout or paint-view call; an uncached
tile or stale owner binding requires the full rendering path. Existing
property-tree positioning applies fixed and sticky constraints to both input
and animated frames.

The DOM owner mapping gives each queued scroll event a node generation before
main-thread dispatch. Cancelable wheel/touch input must be gated against the
DOM event path's resolved passive flags; unknown listener state uses the
main-thread path. Dom-bindings owns the read-only path query, and
integrator-assist owns the input/vsync and event-task hooks. The later pinned
`css/css-scroll-snap` WPT selection will validate snap details after those
driver hooks land; the current controller uses the existing layout-supplied
snap points and `settle_scroll` API.


## COMP-14 resumed integration boundary

The owner-mapping regression reported after the document scroll merge is
already corrected by COMP-13b (`6ca61021`). Its regression now explicitly checks
four bindings: the document viewport at scroll ID 1 plus the nested element's
transform, opacity and scroll. Synthetic scroll ID 0 has no DOM binding.

Remaining adoption work is routed to the owning lanes: integrator-assist retains
`TileRasterPool`, collects ready tiles at vsync and drains generation-checked
scroll events; replaced-content resolves the effects-scroller owned snapshot's
`AmbiguousPropertyOwner`. These do not block the compositor crate. GPU transform
support remains deferred by the recorded project decision, and interactive
window proof still requires an active win-001 session. No new compositor-owned
ticket is listed in the current brief after COMP-14.

## COMP-15 — composited effects (active)

Move CSS filter (blur, drop-shadow and color filters), backdrop-filter,
mix-blend-mode/isolation, clip-path basic shapes/path and mask-image onto
retained compositor effect nodes. Use the full painter as reference.
Effect-only filter/blend changes must animate without rebuilding paint.
Promote will-change transform/opacity, animations, 3D transforms, video/canvas
and fixed/sticky; squash compatible layers to bound layer count.

Exit: fixtures for every effect with exact pixels or only recorded one-channel
layer rounding under ADR 0010; effect-only frames with no paint rebuild;
measured layer counts for IA oracle pages. Iterate remotely on Linux in bulk,
then run one Windows crate/workspace nextest and doctest gate plus parity
evidence. GPUI screenshot proof remains pending and nonblocking.

### COMP-15 native implementation boundary

The retained compositor now owns typed effect operations, nested passes,
filter/blend sampling, clip/path/mask coverage, backdrop scopes, and bounded
layer promotion/squashing. Filter animation can reserve its endpoint halo
before raster and use source-free cached frames thereafter. See
`crates/amazing-compositor/EFFECTS.md` for the producer contract and limits.

Boat iteration: 75/75 compositor tests, 0 doctests; IA reflow, effects-scroller
and article have 1/4/1 layers and zero pixel losses against the existing painter.
These are Linux iteration numbers, not the Windows gate. Four CSS fixture
pages are staged in `crates/amazing-compositor/fixtures/comp15/`.

Remaining completion criteria: replaced-content must supply full-painter
effects and raw effect metadata (the current painter only exports opacity);
CSS owns registration/classification; IA owns scheduler adoption; IPC owns the
additive EffectsFrameSnapshot V2 wire format. Arbitrarily transformed CSS
effect groups need reference-space metadata before import. Native API tests
do not establish full CSS effect parity, so COMP-15 is not marked done.

### COMP-15 Windows native gate

`661ebb62` on fixed base `f5f97c39`: win-001 compositor **75/75**, 0 skipped,
**0 doctests**; workspace **2495/2495**, 9 skipped, **2/2 doctests**. The IA
reflow/effects-scroller/article have **1/4/1 layers**, each **262144 exact
pixels, 0 layer-rounding, 0 losses**. Evidence is committed under
`crates/amazing-compositor/evidence/comp15/`. The native compositor area is
gated; full COMP-15 remains open for the CSS/painter/driver adoption and the
new effect-fixture parity criterion described above.

### COMP-15 restart recovery

`5196e881` completes the interrupted nested-animation halo correction. Endpoint
reservations are per effect: sum ancestors, maximize siblings, retain larger
previous reservations. Cached and worker raster paths use that same envelope.
Boat iteration: **76/76**, **0 doctests**. Windows gate remains pending: the
fixed-commit archive upload lost connection before the gate; no new Windows
result is claimed. Full CSS-effect parity still depends on painter reference
effects and metadata, plus CSS/driver adoption.

### COMP-15 Linux-first final gate

Final source `642ffc69` (halo fix `5196e881`, dev base `4518bea7`): Boat
**2526/2526 workspace tests**, **10 skipped**, **2/2 doctests**; compositor
**76/76**, **0 doctests**. Before `27de5230` and after corpora both have
**1/4/1 layers**, **262144 exact pixels per page**, zero rounding/losses.
Evidence: `crates/amazing-compositor/evidence/comp15/linux.md`. Native recovery
is complete and ready to merge. Windows is deferred and no longer a blocker.
Full COMP-15 remains pending painter reference effects/metadata and adoption.

### COMP-15 CSS value adoption and plus-lighter

CSS filter/backdrop/isolation/blend deltas now have an atomic typed converter;
relative lengths and resource/reference-space decisions remain with producers.
All 17 CSS blend values map, including new premultiplied plus-lighter. The
driver refuses effect deltas before committing accompanying opacity or cached
style while painter metadata is unavailable.

Isolated-target Linux final on source `3c6295d7`: **2741/2741 workspace**, **10 skipped**,
**4/4 doctests**; compositor **82/82**, **0 doctests**. Four curated fixture files:
**37 resolved values, 14 explicit fallbacks, 0 failures**. IA before/after remains
**1/4/1 layers**, **262144 exact pixels/page**, **0 losses**. Details:
`crates/amazing-compositor/evidence/comp15/linux-css-adoption.md`.

This adoption area is complete. Next COMP-15 item remains importing painter
effects/reference spaces and measuring the four fixtures against independent
full paint. The current painter exposes opacity only, so full COMP-15 and
unmodified WPT effect conformance remain open; no additional owned ticket is
listed in the brief.

### COMP-15 pooled-target incident revalidation

Merged `86c44031` via dev and cold-built in
`/home/user/amazing/targets/compositor`. Authoritative gate on `3c6295d7`:
**2741 passed, 10 skipped**, **4 doctests**, compositor **82 passed**.
Both corpora reran: **37 resolved/14 fallback/0 failures**, IA exact with
**0 losses**. Earlier pooled 2723/620 results are superseded; timestamp refresh
was not an isolation guarantee. Implementation `7e6601ea` is ready again.
Next painter import remains blocked on opacity-only EffectNode metadata.

### COMP-15 RC-24 paint blend/isolation adoption

Imported painter EffectNode blend/isolate into retained EffectOperations using
the same effect IDs, preserving opacity and ancestry. All 17 modes covered
across 85 full-painter/cached-frame cases: **53/85 → 83/85**, **30 gains,
0 lost passes**. Two explicit painter differences remain: color-burn integer
rounding and translucent plus-lighter; **1152 loss pixels**, **0 unexpected
differences**. These are routed to replaced-content; no zero-loss claim.

Linux isolated final: **2765 workspace passed**, **10 skipped**, **4 doctests**;
compositor **83 passed**. Existing IA pages remain exact and four CSS value
fixtures unchanged. Evidence: `crates/amazing-compositor/evidence/comp15/paint-blends.md`.
This importer item is complete. Filters/backdrop-filter/clip-path/mask await the
later paint metadata slice; no other newly unblocked item is listed.

### COMP-15 filter/backdrop producer boundary (prepared)

Added PaintFilterSource/PaintFilterInput and atomic paint-view import of resolved
filter/backdrop lists/bounds, preserving other group properties. Native pixel,
cached-frame and rejection checks: compositor **86/86**; isolated Linux workspace
**2775/2775**, **10 skipped**, **4 doctests**. All existing corpora unchanged.
Evidence: `crates/amazing-compositor/evidence/comp15/filter-import.md`.

Live filter/backdrop import is **not complete**: dev431a0a04 has no producer
filter fields, filter-only groups or independent reference rendering. Replaced-
content has the exact metadata contract request. Local-space inputs fail
explicitly until reference-space handling exists. Clip/mask share that later
producer dependency; no implementation claim is made for those inputs.
