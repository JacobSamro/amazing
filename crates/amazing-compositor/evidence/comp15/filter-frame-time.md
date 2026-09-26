# Filter animation joins the real-page frame-time trace

Fixed source base `1cf5678f` (this lane's real painter filter/backdrop/clip
import). Linux Boat pool-paint, compositor's isolated target
`/home/user/amazing/targets/compositor`. Windows deferred under the
Linux-first rule.

`paint_view_pages.rs::retained_real_page_property_trace` already measured
scroll/transform/opacity on a real multi-item page (the effects/scroller
fixture plus a long article, 512x512), proving each produces zero tile
uploads and a `build_frame` cost under 10% of a full repaint. It now adds a
`filter` scenario: the same effect node used for the `opacity` scenario
(`EffectOperations` carries filters/blend/clip, not opacity, so the two
scenarios don't interact) animates `filter: blur(0)` to `blur(6px)` over 20
steps via `reserve_effect_animation` once, then `sample_effect_operations` +
`set_effect_operations` per step — the same source-free tick path
`effect_tests::reserved_blur_animation_and_worker_jobs_keep_offscreen_tiles_ready`
already proved on a synthetic single-item fixture, now measured on a real
page. Reserving the wider halo can itself dirty tiles that need the extra
margin; that one-time cost is absorbed by an untimed `build_frame` call
before the scenario loop starts, so it doesn't land inside the `scroll`
scenario's zero-upload assertion (it did, before that ordering fix — see the
one commit's history: a first attempt without the absorbing call failed
`scroll` with 25 unexpected uploads).

## Measured

Compositor **91/91**, 0 doctests, unchanged from the fixed source. Workspace
**3019 passed**, **11 skipped**, doctests **4/4**, unchanged.

```
scenario,frames,full_paint_us,retained_build_us,uploads
scroll,20,1223030,10345,0
transform,20,1220947,9354,0
opacity,20,1219605,9094,0
filter,20,1216385,9275,0
```

`filter`'s `retained_build_us` (9275) is in the same range as the other three
properties and about 0.76% of `full_paint_us`, with zero tile uploads across
all 20 steps: filter animation gets the same source-free, no-reraster
treatment on a real page that scroll/transform/opacity already had.

This is a `build_frame`-assembly measurement, matching the existing three
scenarios' scope — it does not include `Presenter::present`'s per-pixel
filter kernel cost (that only runs at present time, in `quad.rs`'s
`draw_pass`), GPUI upload/presentation, network or JavaScript, same
exclusions as COMP-7's original measurement.
