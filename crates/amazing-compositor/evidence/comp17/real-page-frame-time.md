# Real-page frame-time report for the shell (COMP-17)

Fixed source base `a2de5c96` (this lane's COMP-16 matrix-aware compositing).
Linux Boat pool-paint, compositor's isolated target
`/home/user/amazing/targets/compositor`. Windows deferred under the
Linux-first rule.

COMP-15/COMP-16 measured scroll/transform/opacity/filter on one synthetic
multi-item fixture (the effects/scroller page plus a long article). COMP-17
repeats that same measurement on 5 real-page fixtures from
`tests/real-pages/pages/`, so the retained-compositor-vs-full-repaint claim
has numbers for actual page content rather than only a synthetic scene.

## Method

`paint_view_pages.rs::real_page_frame_time_report` prepends the same
`fixed`/`scroller`/`moving`/`fade` scaffold `retained_real_page_property_trace`
already uses inside each fixture's own `<body>` (discarding its `<head>`,
which layout never executes anyway). Prepending keeps the scaffold's element
order first, so it gets the same deterministic property ids that test
already relies on (`ScrollId(2)`, `TransformId(1)`, `EffectId(2)`), while the
fixture's own body content drives realistic scene scale. Each page is
rendered at 1024x1024.

Five pages were picked because they resolve to `PaintViewRasterMode::RawLocal`
with the scaffold attached (required for the compositor's tiled retained path
to run at all): `app-kanban`, `docs-guide`, `news-section`, `shop-category`,
`wiki-portal`. Several other fixtures were checked and rejected: pages with a
`<form>`/`<input>`/`<select>`/`<textarea>` or a `border-collapse` `<table>`
always resolve to `BakedFallback` (the same fallback COMP-15 already covers
for form controls and collapsed table borders), and `app-dashboard` resolves
to `BakedFallback` for a reason not fully diagnosed even though it has only
`<img>` content — deprioritized in favor of the 5 confirmed pages above.

Per page: one untimed `build_frame` call absorbs the one-time halo-reservation
reraster from `reserve_effect_animation` (the same ordering fix COMP-15 needed
— running it inside the timed loop would land a spurious upload in the first
scenario's zero-upload assertion), then 4 scenarios x 20 steps each measure
`build_frame` (the compositor path) against
`paint_document_with_text_resources_and_controls` (a full repaint) for the
same step:

- `scroll`: `set_scroll` on the scaffold's scroller.
- `transform`: `set_transform` on the scaffold's moving element.
- `opacity`: `set_opacity` on the scaffold's fade element.
- `filter`: `set_effect_operations` sampling `filter: blur(0)` to `blur(6px)`
  on the same effect node used for `opacity` (`EffectOperations` carries
  filters/blend/clip, not opacity, so the two scenarios don't interact).

Each scenario asserts zero tile uploads and `retained_build_us * 10 <=
full_paint_us` (i.e. the retained rebuild costs at most 10% of a full
repaint), the same bound COMP-15 used.

## Measured

Compositor **93/93** (92 + this new test), 0 doctests changed. Workspace
**3152 passed**, **0 failed**, **14 skipped**, doctests **4/4**.

```
page,scenario,frames,full_paint_us,retained_build_us,uploads
app-kanban,scroll,20,2556976,44053,0
app-kanban,transform,20,2555710,45099,0
app-kanban,opacity,20,2558991,44402,0
app-kanban,filter,20,2558788,45059,0
docs-guide,scroll,20,2410028,44481,0
docs-guide,transform,20,2418592,47066,0
docs-guide,opacity,20,2397176,42826,0
docs-guide,filter,20,2392244,49284,0
news-section,scroll,20,3717110,47815,0
news-section,transform,20,3733375,48506,0
news-section,opacity,20,3712152,46589,0
news-section,filter,20,3716249,46842,0
shop-category,scroll,20,4011254,50677,0
shop-category,transform,20,4017518,48530,0
shop-category,opacity,20,4005419,49838,0
shop-category,filter,20,4008872,48085,0
wiki-portal,scroll,20,3738020,49888,0
wiki-portal,transform,20,3718829,47814,0
wiki-portal,opacity,20,3725179,47677,0
wiki-portal,filter,20,3721119,47176,0
```

All 20 page/scenario combinations report **zero tile uploads** and a
`build_frame` cost between about 0.9% (app-kanban) and 2.0% (shop-category) of
a full repaint, well inside the 10% bound. Per-frame average across the 5
pages (`retained_build_us` / `full_paint_us`, divided by 20 frames each):

| page          | full paint us/frame | retained build us/frame | ratio |
|---------------|---------------------:|-------------------------:|------:|
| app-kanban    | 127,849              | 2,230                    | 1.7%  |
| docs-guide    | 120,360              | 2,298                    | 1.9%  |
| news-section  | 185,974              | 2,369                    | 1.3%  |
| shop-category | 200,584              | 2,466                    | 1.2%  |
| wiki-portal   | 186,262              | 2,413                    | 1.3%  |

This is a `build_frame`-assembly measurement, matching COMP-7/COMP-15's
scope — it does not include `Presenter::present`'s per-pixel filter kernel
cost (only run at present time), GPUI upload/presentation, network, or
JavaScript.

## Still open

- `app-dashboard`'s `BakedFallback` reason (only `<img>` content, no
  tables/forms) is undiagnosed; worth a follow-up if more real-page fixtures
  are needed later.
- Pages with a `<table border-collapse>` or form controls are excluded from
  this trace entirely (they always fall back to `BakedFallback`, which has no
  retained tiled path to measure); their frame-time story is "full repaint,
  every frame," already covered qualitatively by COMP-15's fallback tests.
