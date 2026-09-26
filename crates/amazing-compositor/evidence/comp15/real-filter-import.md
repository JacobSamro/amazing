# Real painter filter/backdrop/clip import, and a raw-local raster panic fix

Fixed source base `3d66defe` (merged local `dev`, includes RC-25 filter
metadata, RC-26 blur/drop-shadow/backdrop-filter and RC-27 clip-path
inset/circle/ellipse). Linux Boat pool-paint, compositor's isolated target
`/home/user/amazing/targets/compositor`. Windows deferred under the
Linux-first rule.

## What changed

`PainterFilterSource::from_view` (`paint_filters.rs`) now imports the real
`amazing_paint::display_list_view::EffectNode` fields RC-25/26/27 expose,
instead of the producer-boundary stand-in from the previous ticket:

- `node.filter` / `node.backdrop_filter`: adopted exactly as before, retaining
  the painter's own resolved `FilterPlan` (`painter_filters`) so the retained
  pass runs the painter's pixel kernel rather than an independently-tuned one.
- `node.backdrop_bounds`: unchanged, still the border-box raster rect.
- `node.clip` (new): a resolved `amazing_paint::clip::ClipMask`
  (`inset()`/`circle()`/`ellipse()`), converted by the new `import_clip` into
  the retained `EffectClip::Inset`/`EffectClip::Ellipse`. `circle()`/
  `ellipse()` resolve onto their own exact bounding box with every corner
  sharing one `(rx, ry)` pair; `import_clip` recognizes that case first and
  maps it to `Ellipse` so the unbounded elliptical test matches the source
  shape exactly instead of approximating it with four rounded corners.
  `inset()` (whose parser never splits a corner into separate horizontal/
  vertical radii) maps onto `Inset`.
- The blanket "any clip forces `BakedFallback`" rule is gone. The transform
  restriction is now `(has_filters || has_clip) && any nonidentity transform
  in the view`, since both filters and clip masks execute on the group's
  local, pre-transform surface in the painter's `composite_group` (the same
  stage the retained `DrawQuad::Pass` compositing uses), and the existing
  code has no matrix-aware compositing for that stage.

**Antialiasing was the one real pixel-parity risk**, and it was real:
`amazing_compositor::effect_clip::apply_masks` sampled `EffectClip::coverage`
at 4×4 (16 subsamples per pixel), while the reference painter's
`RoundedRect::contains` mask test in `composite_group` is a hard, single
center-point sample with no antialiasing. Importing painter clip data through
the antialiased path produced genuine edge losses (up to 60/4096 pixels on a
40 px rounded/circular clip; measured before the fix, see "before" numbers
below). `apply_masks` now samples `EffectClip::contains` once at the pixel
center for `clip_path`, matching the painter exactly; `EffectClip::coverage`
(still public) and `EffectMask` (mask-image, already single-sample) are
unchanged. This is a pixel-parity correction, not a visual-quality regression
claim: the retained compositor's job here is matching full-repaint output.

## Unrelated panic fix (routed by the shell lane, folded into this ticket)

`PaintViewSource::new`'s raw-local-raster probe panicked on
`RawRasterError::RasterFailed`, treating it as an unexpected error while
already handling the sibling `UnseparableItem` by falling back to
`BakedFallback`. The shell lane hit this on real pages
(`tests/real-pages/pages/app-mail.html`, `docs-reference.html`) via
`amazing-shell`; evidence of the crash landed in `dev` from the shell lane at
`crates/amazing-shell-gpui/evidence/linux/shell-crash-app-mail.{json,png}`
and `shell-crash-docs-reference.{json,png}`.

Root cause: both pages have a `<table>` under `base.css`'s
`table{border-collapse:collapse}`. `amazing_paint::paint::raster_raw_item`
(the function behind `raster_raw_local`) declines a `PaintDecorations` item
whose box has `suppress_background`, `suppress_border` or a non-empty
`background_holes` list, and declines a `PaintContent` item whose box has
non-empty table `borders` — all border-collapse table decoration painted by a
different, non-per-item code path — returning `false`, which
`raster_raw_local` turns into `RawRasterError::RasterFailed(index)`. This is
the same kind of "not separable this way" outcome as `UnseparableItem`, just
reached through a different check, so `PaintViewSource::new` now treats both
the same way: fall back to the pixel-exact baked range raster instead of
panicking. `InvalidRange`/`InvalidTile`/`TileTooLarge` still panic — those
indicate a caller bug in the fixed 1×1 probe call, not real-page content.

Regression: `paint_view_pages.rs::real_pages_with_border_collapse_tables_use_baked_fallback_without_panicking`
loads both real pages (their HTML plus `tests/real-pages/assets/base.css`,
1024×1024), asserts `PaintViewSource::new` does not panic, asserts the view
selects `BakedFallback`, and checks pixel parity against the full painter.

## Measured gate

Fixed source `3d66defe` plus this ticket's uncommitted changes, one Boat
sandbox (pool-paint), isolated target.

- Compositor **91 passed**, 0 skipped, ~60 s, 0 doctests (was 87 at
  checkpoint; +4: the real-page panic regression, a manual clip-input test,
  and the corpus growing by 8 clip-path cases counted as one directory test).
- Workspace **3019 passed** (1 slow), **11 skipped**, 78.4 s.
- Workspace doctests **4 passed**, exit 0.
- New filter/backdrop/clip-path corpus (`tests/paint_filter_parity.rs` +
  `tests/support/paint_filter_parity.rs`, 32 cases: the original 24
  filter/backdrop cases plus 8 new clip-path cases, alone and combined with a
  filter/backdrop-filter): **32/32 passing, 0 losses** after the antialiasing
  fix above. Before that fix: **29/32 passing**, 3 clip cases losing
  28/60/30 pixels of 4096 each at rounded/circular edges (see git history on
  `tests/support/paint_filter_parity.rs` for the exact before numbers this
  paragraph summarizes; not committed as a separate CSV since the fix landed
  in the same ticket).
- Legacy filter reconstruction (old importer, real painter, clip untouched):
  **13/32 passing** (19 filter/backdrop failures, unchanged from the prior
  ticket's 19/24; the 8 clip-only cases pass in legacy mode too, since
  "legacy" only clears filter fields).
- Existing blend corpus: **83/85**, same **1152 known reference-difference
  pixels** (color-burn rounding, translucent plus-lighter), **0 unexpected**
  — unchanged from the prior ticket.
- Existing four-file CSS value corpus: **37 resolved/14 fallback/0
  failures** — unchanged. `clips-masks.html`'s 12 cases remain fallback: the
  CSS-side `resolve_css_effect_delta` still does not accept `clip-path`/
  `mask-image` as a property (this ticket imports the painter's *resolved*
  clip data on scene construction, not a live CSS clip-path/mask-image
  *delta* converter; that remains open, see below).
- Existing IA oracle pages (reflow/effects-scroller/article): **1/4/1
  layers**, **262144 exact pixels/page**, **0 rounding, 0 losses** —
  unchanged.
- Zero lost parity cases across all of the above relative to the prior
  ticket's gated numbers.

Bulk iteration: build + compositor nextest, then all four corpora
(filter/clip parity normal and `--legacy`, blends, CSS value fixtures,
effects trace), then one full workspace nextest + doctest gate. Logs under
`.agents/compositor/` (scratch, not committed).

## Still open

- `clip-path`/`mask-image` as a live CSS effect *delta* (not just resolved
  painter import at scene construction) needs `resolve_css_effect_delta` to
  accept those properties; routed to CSS previously, still open.
- `mask-image` has no painter reference renderer yet (searched
  `amazing-paint/src`: no `mask-image`/`MaskImage`), so it stays unimported.
- Transformed filter/clip groups still use `BakedFallback`; no retained
  matrix-aware compositing exists for the `DrawQuad::Pass` stage this ticket
  imports into.
- `EffectNode.clip` for `polygon()`/`path()` is `None` from the painter's own
  parser (documented as "skipped rather than guessed"), so those shapes are
  unclipped in both the painter and this import; not a compositor gap.
