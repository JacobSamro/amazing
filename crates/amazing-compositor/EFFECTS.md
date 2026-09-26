# Retained effects

`PropertyTrees::set_effect_operations` attaches immutable resolved effect values
to an existing `EffectId`. Filters process the entire effect group once, across
paint chunks and tile boundaries. Nested groups preserve their ordering and
isolation; backdrop filters read the preceding contents of their backdrop root.
The CPU presenter executes these passes; GPUI's existing pass fallback can
present the resulting surface. This is not a GPU shader implementation.

`EffectOperations` accepts blur, drop-shadow, brightness, contrast, grayscale,
hue-rotate, invert, opacity, saturate, sepia, backdrop filters, all 16 standard
blend modes plus additive `plus-lighter`, isolation, inset/circle/ellipse/polygon/path clipping, and an owned
alpha/luminance mask. `EffectClip::from_path` flattens resolved line, quadratic,
cubic and elliptical-arc segments with a bounded tolerance. The producer resolves
CSS reference boxes, units, image decoding/origin checks, mask positioning and
repetition before submission. Coordinates are raster pixels. Importing transformed
CSS effect groups still requires the painter's reference-space metadata; these
APIs do not claim to parse CSS or apply local-space filters before arbitrary 3D
transforms.

For an animation, resolve both endpoints once, call
`reserve_effect_animation(id, &from, &to)` before initial raster, then sample with
`sample_effect_operations` and update using `set_effect_operations`. Subsequent
`build_cached_frame` calls have no paint source and cannot rebuild a paint view
or raster a tile. Missing or dirty tiles return `MissingCachedTile`, preserving
the caller's last complete frame. The reserved halo includes offscreen input to
blur/drop-shadow. Reservations are retained per effect node: nested ancestor
halos add, independent sibling halos take the maximum, and a later smaller
reservation cannot shrink the prepared envelope. Synchronous and worker raster
paths use the same visibility rule. Filter-list mismatches and blend/geometry/resource changes interpolate
discretely. `apply_bound_effect_changes` validates stable owner generations for
the whole batch before updating any effect.

Promotion hints cover will-change transform/opacity, animations, 3D transforms,
video/canvas, fixed and sticky. Inferred matrix and positioning reasons supplement
producer hints. Only adjacent chunks sharing exactly the same retained state
are squashed; video/canvas resources remain independent. A scene exceeding 1024
layers returns `TooManyLayers` so the producer can use full paint. Layers,
passes, nesting, filter lists, curve points, masks and frame scratch allocations
have explicit limits. Property-only edits never invalidate source tiles.

Effects require the additive `EffectsFrameSnapshot` V2 envelope. V1 snapshots
of an effect-bearing tree carry an unsupported version and must be rejected;
this prevents old GPU transports silently dropping effects. V1 ordinary scenes
remain compatible. The IPC owner must encode the new envelope before effect
frames can use the split GPU process.

## Evidence boundary

Native tests cover numerical color/blend checks, isolated/nested groups,
clip/mask coverage, tile-crossing blur and offscreen input, mirrored backdrop
edges, snapshot validation and source-free animation. A full-surface filter
kernel check is **not** an independent full-painter oracle.

`examples/effects_trace.rs` measures layer counts and ADR 0010 parity on the
existing IA reflow/effects-scroller pages and article. The four HTML files under
`fixtures/comp15/` cover the requested CSS effects for the later full-painter
comparison. They are prepared fixtures, not claimed passing WPT results.

Full CSS effect parity is pending the CSS lane's live effect-delta classifier
and driver scheduling/GPU snapshot transport adoption. The painter now exports
opacity, blend, isolation (RC-24), filter/backdrop-filter (RC-25/26) and
`clip-path` inset/circle/ellipse (RC-27); `PainterFilterSource::from_view`
imports all of those at scene construction (see "Filter/backdrop/clip
producer import" below). It still lacks `mask-image` reference rendering, and
`resolve_css_effect_delta` still does not accept a live `clip-path`/
`mask-image` change. CSS property registration and the effect-only style
classifier, plus driver scheduling and GPU snapshot transport adoption, are
routed to their owners. No completed COMP-15 CSS gate is claimed by these
native APIs.

References: [Filter Effects 1](https://www.w3.org/TR/filter-effects-1/),
[backdrop-filter operation](https://drafts.csswg.org/filter-effects-2/#backdrop-filter-operation),
[Compositing and Blending 1](https://www.w3.org/TR/compositing-1/),
[CSS Masking 1](https://www.w3.org/TR/css-masking-1/), and
[ADR 0010](../../docs/adr/0010-incremental-style-layout-paint-invalidation.md).

## CSS value adoption

`resolve_css_filters` adopts the CSS lane's typed filter list. It retains list
order, converts hue angles from CSS radians to kernel degrees, resolves omitted
drop-shadow color from the supplied current color, and requests relative lengths
from the producer. Percent filter lengths, calc expressions, resource filters
and values outside kernel limits fail explicitly.

`resolve_css_effect_delta` returns an all-or-nothing proposed value for filter,
backdrop-filter, mix-blend-mode and isolation edits. It preserves unchanged masks
and clips. The caller supplies backdrop bounds in the same raster reference
space. Geometry and resource edits select fallback;
this converter does not authorize a stacking-context change or invent layout
geometry. Sample its resolved endpoints with `sample_effect_operations`.

The current driver deliberately rejects all nonempty CSS effect deltas before
changing opacity/transform or committing styles, until painter effect groups
and reference-space metadata can be imported. This prevents silent no-op frames.
`examples/css_effects_corpus.rs` processes every inline effect declaration in
`fixtures/comp15/`; resolved/fallback counts are value-adoption evidence, not
full-painter or unmodified WPT results.

`plus-lighter` follows [Compositing 2 §9.1.15](https://drafts.csswg.org/compositing-2/#porterduffcompositingoperators_plus_lighter):
add premultiplied color and alpha, clamp both at one, then return straight RGBA.
Its 500-case numerical grid, translucent group cross-fade, cached presentation
and V2 snapshot round-trip cover the retained operator independently of CSS
full-painter adoption.

## Filter/backdrop/clip producer import

`PaintFilterSource` supplies a batch of `PaintFilterInput` values keyed to
existing, non-root paint effect IDs (filters, backdrop filters plus bounds,
and now an optional `clip_path: Option<EffectClip>`).
`PaintViewSource::with_filters` snapshots and validates the entire batch
before exposing a source. `paint_view_metadata_with_filters` imports the
values without changing blend, isolation, opacity or ancestry. Empty input
preserves the ordinary path.

`PainterFilterSource::from_view` is the real-painter producer: it reads
`amazing_paint::display_list_view::EffectNode::{filter, backdrop_filter,
backdrop_bounds, clip}` directly, retains the painter's own `FilterPlan` so
the retained pass runs its exact pixel kernel (`painter_filters`), and
converts a resolved `clip-path` shape with `import_clip`. Filters and clip
masks both execute on the group's local, pre-transform surface in the
painter; `from_view` returns `BakedFallback` whenever the view has any filter
or clip alongside a nonidentity transform, since the retained `DrawQuad::Pass`
stage that applies them has no matrix-aware compositing.

Duplicate/unknown/root IDs, invalid kernels, missing backdrop bounds,
nonfinite/empty bounds, an unresolvable clip shape, and baked sources fail
explicitly. No partial import escapes. Filter/clip-only elements must already
have painter effect groups; the adapter cannot synthesize topology from CSS.

Only `PaintFilterSpace::Raster` is supported: filters act on the post-transform
render-pass image. `Local` requests fail until local reference-space handling
exists. A producer may only label values Raster when this execution order is
equivalent to its intended effect; it must rebuild/recompute values and bounds
when transforms or scrolling change that reference space. This is not permission
to relabel arbitrary transformed CSS blur/drop-shadow as raster-space filters.

Clip-path masking must match the painter's hard, single-center-point mask
test (`RoundedRect::contains` in `composite_group`) rather than
`EffectClip`'s antialiased `coverage()`, or an imported rounded/circular
clip loses pixels at its edge; `effect_clip::apply_masks` samples
`EffectClip::contains` once per pixel for `clip_path` for exactly this
reason (see `evidence/comp15/real-filter-import.md`).

Native/corpus tests cover ten filter operations, backdrop bounds/cached
pixels, `inset()`/`circle()`/`ellipse()` clip import alone and combined with
a filter or backdrop-filter, and invalid batches, against the real painter
with `classify_pixel_parity`; they are not independent full-painter filter
comparisons against unmodified WPT. `mask-image` is not imported: the painter
has no reference renderer for it yet.
