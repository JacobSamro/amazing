# Retained effects

`PropertyTrees::set_effect_operations` attaches immutable resolved effect values
to an existing `EffectId`. Filters process the entire effect group once, across
paint chunks and tile boundaries. Nested groups preserve their ordering and
isolation; backdrop filters read the preceding contents of their backdrop root.
The CPU presenter executes these passes; GPUI's existing pass fallback can
present the resulting surface. This is not a GPU shader implementation.

`EffectOperations` accepts blur, drop-shadow, brightness, contrast, grayscale,
hue-rotate, invert, opacity, saturate, sepia, backdrop filters, all 16 standard
blend modes, isolation, inset/circle/ellipse/polygon/path clipping, and an owned
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
blur/drop-shadow; synchronous and worker raster paths use the same visibility
rule. Filter-list mismatches and blend/geometry/resource changes interpolate
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

Full CSS effect parity is pending the replaced-content lane's full painter
implementation and raw effect metadata. The current painter exports opacity
only and explicitly defers filters and blend modes. CSS property registration
and the effect-only style classifier, plus driver scheduling and GPU snapshot
transport adoption, are routed to their owners. No completed COMP-15 CSS gate
is claimed by these native APIs.

References: [Filter Effects 1](https://www.w3.org/TR/filter-effects-1/),
[backdrop-filter operation](https://drafts.csswg.org/filter-effects-2/#backdrop-filter-operation),
[Compositing and Blending 1](https://www.w3.org/TR/compositing-1/),
[CSS Masking 1](https://www.w3.org/TR/css-masking-1/), and
[ADR 0010](../../docs/adr/0010-incremental-style-layout-paint-invalidation.md).
