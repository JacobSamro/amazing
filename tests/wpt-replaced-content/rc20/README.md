# RC-20 — gradient background-image layers

Paint now rasterizes CSS `<gradient>` functions supplied by `background-image`
layers through the same `amazing-paint::gradient` raster that RC-18 added for
generated image keys. No CSS parsing or duplicate rasterizer was added.

## Behavior

- `amazing-paint::background::BackgroundLayer` carries an optional serialized
  `<gradient>` beside its existing `url()`. At most one of the two is set;
  unsupported image functions still describe no layer.
- A gradient has no intrinsic dimensions. `auto` resolves to the background
  positioning area, and the tile is rasterized at its used size; `cover`,
  `contain` and explicit sizes scale from the positioning area.
- URL layers are unchanged: provider pixels and natural size.
- Layer order (first listed on top), repeat (`repeat`/`no-repeat`/`repeat-x`/
  `repeat-y`/`space`/`round`), position, `background-origin`/`background-clip`
  and the shared per-frame sample budget all run through the existing loop.
- Unsupported gradient syntax paints no layer; it is never guessed.

Recognized functions: `linear-gradient`, `repeating-linear-gradient`,
`radial-gradient`, `repeating-radial-gradient`, `conic-gradient`,
`repeating-conic-gradient`.

## Tests

Four focused `amazing-paint` unit tests assert pixels: a linear gradient fills
the positioning area, a repeating gradient tiles through `background-size`,
the first listed gradient layer paints above later layers, and unsupported
syntax (bad color, single stop, a direction rejected for radial) paints no
layer.

## Pinned WPT selection — deferred

The ticket's frozen selection would come from `css/css-images/gradient` and
`css/css-backgrounds` gradient reftests at WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. The render reftest runner lives on
win-001 (pinned WPT checkout, fonts, GPU), and the orchestrator deferred Windows
gates under the Linux-first rule, so the before/after sweep and its
pass/mismatch/crash/timeout/unsupported transitions and pixel hashes are not run
here. This directory therefore records the Linux gate only; no WPT conformance
credit is claimed for RC-20.

Retained selections (RC-18 focused 12, SVG 35, gradient/scroll 4) are likewise
unrun under the deferral and are not claimed unchanged.
