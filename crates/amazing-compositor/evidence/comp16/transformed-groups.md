# COMP-16: matrix-aware compositing for transformed filter/clip/backdrop groups

## What changed

`PainterFilterSource::from_view` (`src/paint_filters.rs`) no longer rejects the
whole view merely because a nonidentity transform exists somewhere alongside a
filter/backdrop/clip group. It now resolves, per qualifying group (one with a
filter, backdrop-filter or `clip_path`), the single `transform` id common to
every item in that group's own subtree (`group_transform`), and accepts it as
the group's own `EffectOperations::local_transform` when:

- the transform's own parent in the transform tree is the identity root
  (no ancestor transform to conjugate through);
- no other qualifying group is a strict ancestor or descendant of this one
  (composing two local surfaces would need conjugating one matrix through the
  other, not the single inverse-mapped sample this ticket adds); and
- the filter/backdrop-filter list has zero outset (no blur, no drop-shadow):
  those read neighboring source pixels outward, and that read is not yet
  itself transform-sampled.

Everything else with a filter/backdrop/clip alongside a transform still
returns `BakedFallback`, unchanged from COMP-15.

`Compositor::build_frame` (`src/quad.rs`) renders a `local_transform` group's
own tiles at `Matrix4::IDENTITY` (into its local, pre-transform surface)
while tile selection and damage tracking keep using the group's real
accumulated matrix -- what is actually visible and dirtied on screen.
`DrawQuad::Pass` then:

- filters and mask-images that local surface exactly as the untransformed
  case already did;
- tests `clip_path` at the exact fractional point the final sample lands on
  (`effect_clip::clip_path_contains`) rather than the local surface's integer
  pixel centers -- matching `composite_group`'s own mask test, which runs at
  the same fractional point it later floors for the color sample. Testing
  `clip_path` on the local surface's integer grid instead (as a first attempt
  did) reproduced translation exactly but lost pixels under scale/rotate,
  where the two roundings can disagree at the boundary;
- forward-samples the parent surface through the same matrix into the local
  grid to seed a backdrop-filter snapshot, so it lines up with that grid;
- samples the (now filtered/clipped) local surface into its parent with one
  `Matrix4::unmap` per destination pixel, nearest, no interpolation --
  exactly the pattern `DrawQuad::Tile` already used for a transformed tile,
  and exactly `composite_group`'s own transform sampling.

## Root-caused during implementation

Painter's `EffectNode`/`TransformNode::owner` is only populated when the view
is built with real DOM generations (`with_dom_generations`); the plain
`DisplayListView::new()` constructor used throughout this crate's tests
always passes `owner: None`. An owner-matched "same `BeginGroup` pushed this
transform and this effect together" design (the first attempt) therefore
silently found no transform for every group, and the blanket-relaxed nesting
checks never fired -- so the very first test run showed both new corpora
recognizing `RawLocal` where the geometry was in fact wrong. The fix drops
owner-matching entirely and resolves `local_transform` purely from item
transform-id uniformity within each qualifying group's own subtree, which
does not depend on generation tracking.

## Measured (Linux, Boat sandbox, `pool-paint`)

`cargo nextest run -p amazing-compositor --locked`: 92/92 passed (was 91/91
at COMP-15; +1 new test `transformed_filter_clip_and_backdrop_groups_match_
full_paint`, and `transformed_clipped_and_oversize_filters_use_exact_full_
paint_fallback` now covers 5 cases instead of 6, with 3 of the previous 5
transform+filter cases moved to the success corpus and 3 new nesting/parent-
transform/blur-under-transform rejection cases added).

New transformed-effects corpus (`tests/support/paint_filter_parity.rs::
run_transformed`), 4 transforms (`translate`, `scale`, `rotate`, a composed
`scale rotate translate`) crossed with 5 cases (zero-outset filter, backdrop-
filter, two clip-path shapes, filter+clip-path together) = 20 combinations,
each checked for `RawLocal` mode, zero-loss `classify_pixel_parity` against
the real full-painter reference, `EffectsFrameSnapshot` round-trip, and
cached-frame equality: **20/20 zero losses.**

Existing corpora, rerun unchanged for zero regressions:

- `real_painter_filter_directory` (32 filter/backdrop/clip cases): 32/32,
  zero losses.
- `paint_blends` (85-case blend/isolation directory): unchanged, passing.
- Real-pages IA oracle and long-article scroll trace, nested opacity,
  scroller-512, style-change trace, border-collapse regression: all passing.

Full workspace gate on the same base:
`cargo nextest run --workspace --locked`: **3151/3151 passed, 0 failed, 14
skipped.**
`cargo test --workspace --locked --doc`: **4/4 passed** (amazing-a11y,
amazing-devtools, amazing-html, amazing-js), 0 failed.

## Still open (routed, not this ticket)

- Blur/drop-shadow under a `local_transform` still forces `BakedFallback`
  (see above); transform-sampling their outward neighbor read is a follow-up.
- Two qualifying groups nested, or one qualifying group's transform sitting
  under another transform, both still force `BakedFallback` -- composing two
  local surfaces or conjugating two matrices, respectively.
- `mask-image` under a `local_transform` is untested (no corpus case); its
  existing integer-grid `coverage()` sampling is left as-is on the local
  surface, unlike `clip_path`'s new fractional test.
- CSS clip-path/mask-image live deltas and a painter `mask-image` renderer
  remain routed to the CSS and replaced-content lanes respectively (COMP-15
  evidence, unchanged).
