# RC-6: CSS-7 computed style in static SVG resources

Fixed source base: CSS-7 merge `945ca38`. WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. The frozen
`../rc6-svg-manifest.json` selects four previously non-inert CSS Images SVG
resource reftests. Its SHA-256 is
`d794102ba14ea1258c71d099f7beb012cffd05f9a7b4de168c003d8d5435f2a3`.
`prior-wpt-projections.json` preserves the RC-5 prior classifications and
actual/reference/diff hashes. `wpt-before.json` measures the RC-5 adapter
rebuilt on the CSS-7 base; its per-case projection is identical to that prior
packet. Both `wpt-after-*.json` runs are identical to before and each other:
4 pass, zero mismatch, error, timeout, unsupported or renderer error. Raw WPT
pass gain is **zero**; none of the four existing passes is credited again.

`../rc6-fixtures/` is an owned 64×64 proof, not WPT. Four cases cover inline
`inherit`, inline `initial`, an inherited custom property consumed by `var()`,
and an inline variable overriding a red presentation fill. Each has an
equivalent presentation-only colored SVG and a shared blank control. The
fixed-base before executable rendered all four actuals exactly like blank:
0/4 reference matches, with 4,096 differing pixels per reference. The CSS-7
adapter renders 4/4 reference matches in each of two after runs. Every after
actual differs from blank at all 4,096 pixels. Per-case hashes are in
`owned-before.json` and `owned-after-*.json`; measured pixel differences are
in `owned-pixel-differences.json`. The before and after executable SHA-256s
are recorded in the lane report.

The adapter passes each element's parent `ComputedStyle`, SVG presentation
declarations, and inline declarations to
`amazing_css::compute_element_paint_style`. It consumes typed `color`, `fill`,
`fill-opacity`, and `opacity`; every `Unsupported` outcome rejects the whole
image. Its author stylesheet slice is empty because this strict static image
subset has no stylesheet or selector processing. CSS-wide keywords, custom
properties, importance, and origin priority now stay in the CSS crate. This
does not claim general SVG CSS conformance. Scripts, external references,
animation, paths, text, strokes, filters, paint servers, unsupported colors,
nontrivial root/group opacity, and prior size/work limits remain strict.

All builds and renders ran on win-001. The final lane-only refresh used
`945ca38`, then owned files were re-synced. The shared REFRESH-HOLD protected
the final gate. `input-hashes-before.json` and `input-hashes-after.json` verify
Mac/Windows source and fixture equality and unchanged inputs across the gate.
`validation.json` records crate/workspace nextest and separate doctest counts.
The fixed-base workspace gate passed 1,441 nextest tests with 3 skipped, and
its separate doctest run passed 2. The isolated image crate passed 21 nextest
tests and has no doctests.
