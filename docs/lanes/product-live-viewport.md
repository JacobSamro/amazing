# Live shell viewport

Recorded coherent base: `dcee3e6202fa31b8543c2ef83a28cafe73f13dc1`.
Original shell producer source is preserved on `lane/product-viewport-preserved-pre-dpr`.
Source preparation only; no build or product qualification yet.

The actual 1280×800 window proof contains a fixed 960×570 engine surface.
Replacing those constants with another constant or stretching that bitmap does
not implement resize. The shell must measure the inner content canvas in GPUI
logical pixels and read the same window's `scale_factor()` during prepaint.
Measurements publish through a deferred entity update, guarded by the captured
active tab and actual retained document/runtime owner. Hidden/zero-size bounds
keep the last valid frame. A new document must inherit the most recent valid
measurement rather than reset to the constructor default.

## One unit contract

`ViewportMetrics` retains fractional CSS width/height and actual device scale,
with checked `ceil(css * scale)` physical extents. Legacy direct constructors
keep 960×570 at scale 1. Live measurement never substitutes that default for an
invalid measurement. Layout, media queries, responsive source selection, lazy
intersection, hit testing, scroll positions and accessibility geometry use CSS
pixels. Raster buffers, tiles and uploads use physical pixels. A presenter maps
physical coordinates back to GPUI logical coordinates exactly once. Authored
CSS zoom is independent of the outer device scale.

## Shell publication and source ownership

Static pages must retain their originally parsed owned DOM, stylesheet inputs,
font sources and resource loading context. They reflow from that same document;
resize must not reparse HTML or fetch the main resource. Script pages use the
actual `ShellDomHost.dom` in their existing runtime. A resize-only rendering
method must not call `snapshot`, which consumes navigation/history directives.
Timer/click snapshots and lazy resource completion use the current metrics.
Resource selection may refetch a newly selected responsive image outside raster.
The active old scroll offset is reapplied through the new real scroll tree and
clamped to its new extents, without copying obsolete node IDs into a new owner.

CSS computation receives the actual `MediaEnvironment`; the resulting single
`StyledDom` is passed to `layout_styled_document_from_root`, paint and AX build.
The current separate default-environment computation after layout must go.
Links/control rectangles come from the newly laid-out root. Pointer conversion
uses independent horizontal and vertical ratios only while a prior frame is
presented during deferred reflow; once geometry is current GPUI logical and CSS
pixels agree. Scroll events remain logical externally. The shell maps deltas, targets and momentum into the physical compositor lattice once and converts reported offsets back to CSS pixels.

The content element fills measured available space. The measurement canvas
must not request its size from the previous raster. Input handlers retain the
same bounds and document owner as the presented frame; retired callbacks cannot
hit a new active tab. Find/editor overlays and the accessibility bridge receive
current CSS geometry and scroll translation. AX node identity/parent structure
must remain stable while platform bounds become actual content bounds rather
than the existing zero-size off-screen shadows.

## Lower raster dependency requiring owner review

At this base, full paint, `DisplayListView::new` and
`PaintViewSource::{compositor,build_frame}` accept integer target dimensions but
no outer raster transform. `ImageProvider::device_scale` selects image resources;
it does not scale ordinary box/text geometry. Merely doubling the output sizes
would leave content occupying the original CSS pixel dimensions.

The required additive lower contract is a genuine logical viewport plus device
scale and checked physical extent, accepted by both full and retained paint.
Raster sample centers map physical target pixels to CSS document coordinates;
glyph coverage is requested at real device density, preserving authored zoom.
The same mapping covers clips, gradients, decoded/generated images, controls,
scroll offsets, effects and compositor damage/tiles. Missing support yields a
checked unavailable result rather than a stretched scale-1 frame. Existing
scale-1 wrappers retain their current semantics. No shell-side cloned/scaled
layout/style tree or post-raster resize is acceptable.

Paint and compositor owners must freeze exact APIs before the shell's final
DPR adapter is written. This is a real source dependency, not a proposed new
certificate/custody framework or an inferred merge blocker.

## Whole qualification

Grouped source tests retain existing default controls and cover fractional
measurements, invalid bounds/scale, physical overflow, multiple densities,
independent pointer axes, scroll clamping, owner-retired measurement, runtime
updates retaining metrics, responsive/media changes, and AX bounds. Boat runs
workspace nextest plus docs and the shell's relevant corpus in one bulk cycle.
Actual bounded window proof uses the same fixture at two window sizes, with
resize reflow plus scroll/click behavior, executable/source/PID/viewport receipts.
Physical-density proof must measure actual platform scale rather than advertise
a requested value. No universal browser-readiness claim follows this ticket.

## Source preparation checkpoint

The shell now retains owned static input and measured metrics, and has a
resize-only runtime render path distinct from directive-consuming snapshots.
One media-aware styled root feeds layout, paint and AX. Responsive/lazy source
environments use those same metrics. Measurement defers through an immutable
weak presented-owner receipt so it does not keep the mutable retained page
shared; stale tab/frame measurements and input callbacks are rejected. Root
scroll projection uses the new root ID and clamp, with offsets in CSS units.
Presentation has an explicit physical-to-logical scale entry point.

The coherent source remains unbuilt. Full paint and every retained view now use
the genuine RasterViewport/TextSystem density APIs. EngineCompositor rejects a
changed viewport signature even when its rounded physical extent is unchanged.
CSS transforms are conjugated into device coordinates; Arrow keys preserve the
40 CSS pixel step. Page keys use actual physical container bounds. Presentation
converts raster bounds into logical GPUI bounds once. The scaled scene's scroll
offsets and asynchronous events are converted back before DOM/AX use.

A checked resize failure preserves the prior valid frame and publishes a
visible failure. A no-op does not clear it; only a checked replacement clears
it. Invalid or missing measurements retain the old metrics. The native global
setter validates finite positive density before any global or Env mutation and
preserves navigation/history/timer state. Initial scripts receive the measured
metrics. No intermediate source-only output is product evidence.

Text-control editing now remaps by the actual node generation into the new
control list, preserving the edited buffer when media reflow changes indices
and cancelling only a genuinely unavailable/replaced control.

## Actual-window proof hooks

The public capture tool accepts `--device-scale` only through GPUI's native X11
platform `GPUI_X11_SCALE_FACTOR` input. Actual prepaint still reads
`window.scale_factor()` and the real measured content/window extent. The shot
manifest validates the native shell PID, source/executable custody, ceil raster
extent and the measured logical window against the actual X window dimensions.
Latest render failures or changing frame receipts reject the shot. Existing
1x callers remain supported. Real resize/click actions operate on the owned
window; no image is resized or cropped. Metadata fixtures cover wrong PID,
source, density, raster extent, missing/failed frame and wrong window geometry.


Current-frame failure recovery
------------------------------

A failed current runtime DOM/style render retains the previous pixels with a
visible error. Cached scroll success and no-op movement preserve that error; they
cannot authorize DOM hits, form editing, find results or accessibility geometry.
Only a newly checked current runtime/static reflow, including successful scroll
projection, replaces the frame and clears failure. Navigation/history directives
remain consumed and delivered even when their accompanying render fails.
Three grouped native controls exercise genuine 2x generated border-image resource
failure, cached scroll/no-op, stale frame identity, checked runtime recovery and
failed/successful current-DOM resize and authentic timer recovery without suspending
the failed page runtime. These controls are source-prepared, unrun.

First qualification attempt (immutable ae563): baseline GPUI 93/93 passed.
Baseline render sweep stopped on native /tmp ENOSPC before a following command
could start; candidate and actual-window proofs were not reached. No zero-loss or
DPR product claim is made from that attempt. Raw remains Boat.

## Fresh published-base preparation

The complete parked `db4133b60c80c4f19f6272f589b41814983c643d` area is replayed on actual published `d8656a54f7da95ef4bd1da68b5abab4cba5295d8` after the separate supported build-tooling repair. All 37 non-interpreter area files were byte-identical during replay; the interpreter only unions the current published call-resource include/test rows with the existing viewport setter/test addition. Current MessageEvent, Abort, Flex and DTD source stays on the new base. Unqualified navigation/editing WIP is excluded.

This is fresh source preparation, not a retry or reuse of old engine proof. The two previous supported failures, baseline-only20 maps and missing candidate/window proof remain unchanged. Qualification still requires main workspace/docs, full GPUI groups, original20 scale1 matched directories and actual document1x/2x resize/PID/source/raster window proof on this new before/after base after tooling publication. No local engine build has run.

## Published tooling base and bulk compiler correction

The source is replayed on published `d1919c42a290f7bb0a5ef518165c5521ef545285`, which contains the independently qualified native-Cargo bootstrap repair. First fresh candidate `f400b82edae1637921fcd90d76e607d702a78808` had test-compilation failures: the GPUI compositor field must preserve the lower optional viewport identity, two native timer controls must use the public `amazing_js::value::JsValue` path, and the Paint density fixture must construct attached native DOM instead of importing an absent HTML dependency. These are corrected together, preserving every assertion and legacy unit-view identity. No Cargo dependency or raster algorithm changed.

The failed candidate still completed docs4/release0 and all20 scale1 matched directories with1327→1327 passing identities,0loss/0gain. The baseline passed4607/17skip/docs4 and GPUI93. These partial receipts are preserved; they do not qualify candidate native controls or actual window behavior. Full corrected native and window qualification remains required. Raw logs stay on Boat; only compact receipts/maps/compiler context are local.

### Fractional physical transform sampling

The corrected candidate preserves the original device-lattice equality control. Authentic Paint scenes now carry the compositor Nearest sampling policy to match full paint; generic compositor sources retain Bilinear. GPUI direct and hoisted tile admission explicitly consult that policy and retain whole-physical-pixel translation admission, using the existing exact CPU composed image for fractional transforms. A grouped native companion covers integer/positive/negative fractional transforms, opacity passes and cached composed-image reuse. Native/window qualification remains required; prior density1.25 failure stays recorded.

## Current published-base whole replay

The exact owned delta `d1919c42a290f7bb0a5ef518165c5521ef545285` → `b51e39cc08a5ceef196cae3d9a54fd190f2eb53f` is replayed onto recorded published `43e959578e44e2539921d16178ea2788461e61b9`. Its only three-way conflict is the interpreter test-module tail: current DynamicSource/EvalGlobal/PTC tests remain present, with the viewport module added separately. All original GPUI109 controls, lower device-raster/sampling/visibility controls and20 render selections remain intact. No Cargo/dependency change or held Weak source is imported. Historical109/0 proof is provenance, not a claim about this replay.

The published native-storage plan wrapper will admit the complete per-side workspace +GPUI +render +window command, retain measured runnable ELFs/generated source/raw outputs and retire only verified recoverable target artifacts after actual completion. Both sides use line-table debug/test information and incremental0, with separately fresh targets and exact baseline artifact manifest. Public window capture accepts an explicit storage-managed target via `CAPTURE_CARGO_TARGET_DIR`; ordinary captures retain their existing dedicated native/tmp target. No allocation or local engine test has run for this source.

### Current published-base replay

The complete source is replayed onto recorded origin/dev
`132b9010dc6a6352909b06f7319beed51eab028d`. Drift since the earlier
`43e95957` base changes only the current Net/process sibling
`network_v2.rs` among these crates; it remains untouched. Qualification
uses independent native storage lifecycles with explicit corpus roles,
actual pinned-input discovery and capacity envelopes, and full real
before/after pass-set comparisons. Historical selected-ID equality is
not an execution or merge prerequisite. The already-qualified current
baseline workspace is reused where source identity permits; the candidate
receives a fresh whole workspace/docs gate. Original 109 GPUI controls,
all 20 render selections and real scale1/scale2 action sequences remain.
