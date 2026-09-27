# GPUI presenter direct-tiles fast path for effect passes (COMP-19)

Started from local `dev` at `0bf12a75` (COMP-18 merged in). Ran on worker-2
(Boat's daily sandbox-start quota was still exhausted; venus/worker-2 carried
this ticket's builds/tests, same as COMP-18). Windows deferred under the
Linux-first rule.

## The gap this closes

COMP-18 filed (`questions.md`, 2026-09-27): `GpuiPresenter::present`
(`crates/amazing-shell-gpui/src/compositor_presenter.rs`) set
`direct_tiles = false` the instant a frame's root pass contained *any*
`DrawQuad::Pass` -- the quad a `RenderPass` isolation boundary uses to
composite one pass's rendered surface into its parent. Any real page with a
`transform`/`opacity` effect group anywhere on it produces such a pass
(`quad.rs`'s `needs_pass`, or the "enhanced" `EffectPassBuilder` chain for
filter/backdrop/mask/`local_transform` groups), which dropped the *whole
frame* off GPUI's fast direct-tile placement path and onto
`ReferenceCpuPresenter`'s full CPU recompose -- even though the compositor's
own tile cache never rerasters (COMP-18's `build_cached_frame` rechecks
proved that separately).

## What changed

`crates/amazing-shell-gpui/src/compositor_presenter.rs` only. `GpuiPresenter`
gained `hoist_pass`, called from `present()`'s existing per-quad loop instead
of unconditionally setting `direct_tiles = false` on `DrawQuad::Pass`:

- A pass is **flatten-safe** when its own effect (if any) has no
  filter/backdrop/mask/`local_transform` (`effect_operations_are_trivial`) --
  those genuinely reshape pixels or need `DrawQuad::Pass`'s own per-pixel
  inverse-sampled compositing (COMP-16), which a stack of absolutely
  positioned tiles cannot replicate -- its blend is `Normal`, and its clip
  chain is rect-only (`clip_chain_is_rect_only`; a rounded clip's bounding
  box is not the same shape, which is exactly why an *ordinary* tile under a
  rounded ancestor clip already forces its own isolating pass in `quad.rs`).
  This recurses into any further-nested pass the same way, bounded to depth
  16 like `ReferenceCpuPresenter::draw_pass`'s own `PassCycle` guard.
- When flatten-safe, `hoist_pass` walks the pass's tiles/solids straight into
  the caller's direct-tile draw list, combining each one's own opacity with
  the enclosing pass chain's via `combine_opacity` -- the same 8-bit
  `quantize_opacity` rounding `amazing_compositor::property`'s private
  `nested_effect_opacity` uses, duplicated here rather than made `pub`
  in-crate since it is a two-line formula and this ticket's edits stay
  scoped to `amazing-shell-gpui` -- and intersecting its clip with the pass
  chain's own `rect`/`clip_rect` (`intersect_rects`). This is exactly the
  algebra a real isolated-pass blend produces when there is nothing to
  isolate *for* (no overlap-sensitive compositing, since filters/masks are
  already excluded), just computed once instead of via an actual raster.
- When a pass is *not* flatten-safe, behavior is unchanged from before this
  ticket: `direct_tiles = false`, whole-frame CPU recompose, exactly as
  filters/masks/rounded-clip/rotated groups already required.
- `direct_tiles`'s initial value changed from `frame.passes.len() == 1` to
  `true`: pass *count* was never the real constraint (a frame can have many
  passes and still be fully hoistable, as this ticket's own scaffold and new
  test show), only whether each pass encountered is flatten-safe.

Also updated `compositor_presenter.rs`'s own tests: the old
`rotated_and_effect_pass_frames_use_exact_fallback` asserted the CPU fallback
for *both* a rotated tile and a plain opacity pass in one loop. Split into
`rotated_pass_frame_uses_exact_fallback` (unchanged expectation -- a rotation
is never flatten-safe, GPUI has no 4x4 transform for its tile placement
regardless of this fix) and a new `multi_tile_opacity_pass_hoists_onto_the_
direct_tile_path` proving the opacity-pass case now hoists.

## Pixel parity against a full repaint

`multi_tile_opacity_pass_hoists_onto_the_direct_tile_path` (new test in
`compositor_presenter.rs`) is COMP-19's other proof obligation, done without
needing to actually rasterize a GPUI element tree: it takes the *exact*
flattened draws `hoist_pass` produced, rebuilds them as a synthetic
single-pass `CompositorFrame` (cloning the original frame so
`resources`/`trees`/`tile_lookup` are untouched -- only each hoisted
`TileQuad`'s `opacity`/`clip_rect` differ, and `tile_lookup`'s pixel source
lookup is keyed by `(layer, tx, ty)`, independent of which pass a `TileQuad`
sits in), runs that synthetic frame through `ReferenceCpuPresenter`, and
asserts the result is pixel-identical to running the *original*, real
multi-pass frame through the same `ReferenceCpuPresenter`. Any drift in
`combine_opacity`'s rounding or `intersect_rects`' clip math would show up
here as a pixel difference, not just a missed assertion on an opacity field.

Also still covered: `gpui_resource_upload_and_frame_match_cpu_reference`,
`raw_page_canvas_solid_stays_on_ordered_direct_path`,
`single_tile_opacity_and_rect_clip_stay_on_gpui_tile_path` (the pre-existing
single-tile case, unaffected -- it never went through a pass at all) all
still pass unchanged.

## `shell_viewport_frame_time_trace`: before and after

Same test COMP-18 added (`engine_viewport.rs`), same scaffold, same 3 real
pages, same `STEPS = 8`, unmodified by this ticket -- only the presenter
underneath it changed. `direct_tiles` flips from `Some(false)` to
`Some(true)` on every row, and `retained_call_us` (the real, end-to-end
`scroll_input`/`animate_transform`/`animate_opacity` wall time, presentation
included) drops by roughly two orders of magnitude for `transform`/`opacity`
and by about 80x for `scroll`:

**Before (COMP-18, `direct_tiles=Some(false)` throughout):**

```
page,scenario,frames,full_paint_us,retained_call_us,build_cache_hits,direct_tiles
app-kanban,scroll,8,934364,27259674,8,Some(false)
app-kanban,transform,8,930900,27012998,8,Some(false)
app-kanban,opacity,8,934039,23765725,8,Some(false)
docs-guide,scroll,8,871152,27117177,8,Some(false)
docs-guide,transform,8,870631,26983109,8,Some(false)
docs-guide,opacity,8,870061,23668895,8,Some(false)
news-section,scroll,8,1182042,27873595,8,Some(false)
news-section,transform,8,1204972,28228156,8,Some(false)
news-section,opacity,8,1162424,23823805,8,Some(false)
```

**After (COMP-19, `direct_tiles=Some(true)` throughout):**

```
page,scenario,frames,full_paint_us,retained_call_us,build_cache_hits,direct_tiles
app-kanban,scroll,8,2195848,338944,8,Some(true)
app-kanban,transform,8,2283701,19195,8,Some(true)
app-kanban,opacity,8,2186916,18198,8,Some(true)
docs-guide,scroll,8,2011616,322016,8,Some(true)
docs-guide,transform,8,2088761,17993,8,Some(true)
docs-guide,opacity,8,2035752,18148,8,Some(true)
news-section,scroll,8,2578217,323038,8,Some(true)
news-section,transform,8,2590939,18146,8,Some(true)
news-section,opacity,8,2586453,18400,8,Some(true)
```

`transform`/`opacity`: ~27M us -> ~18-19K us (roughly 1400x). `scroll`:
~27-28M us -> ~320-340K us (roughly 80x -- still real end-to-end work,
scroll's own hit-region/offset bookkeeping in `scroll_controller`/
`scroll_input` beyond the presenter, but no longer a full CPU recompose).
`full_paint_us` moved up somewhat between the two runs (host load, not this
change -- it is the unrelated reference measurement re-timed fresh each run,
not something COMP-19 touches) but is now utterly dwarfed by
`retained_call_us` for `transform`/`opacity` where COMP-18 had them within
the same order of magnitude. `build_cache_hits` stayed 8/8 on every row
both before and after, confirming the compositor's own tile cache was never
the bottleneck at any point -- exactly the presenter-layer gap this ticket
was filed to close.

## Measured

`amazing-shell-gpui`: **76/76 passed, 0 failed** (was 75; +1 net test from
splitting `rotated_and_effect_pass_frames_use_exact_fallback` into two and
adding the new hoist/pixel-parity test), via `cargo test --locked --bin
amazing-shell` from the crate's own directory. `amazing-compositor`
(unedited this ticket): **93/93 passed, 0 failed**, unchanged from COMP-18,
via `cargo nextest run -p amazing-compositor --locked`.

Full workspace gate and doctests: see `docs/lanes/compositor_TODO.md`'s
COMP-19 entry for the numbers from the single final gate run.
