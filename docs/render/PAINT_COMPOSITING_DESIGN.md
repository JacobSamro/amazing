# Paint order, clipping, stacking, and compositing

Phase #6 replaces recursive layout-tree rasterization with a bounded,
provider-neutral display-list pipeline. GPUI remains the sole production
presentation layer; the engine emits pixels and never imports GPUI or platform
types.

## Evidence selection

- WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`
- Viewport: 800×600 at device scale 1
- Fixed point: `c42df4f`
- Clean behavior-stratified gate: 24 static HTML reftests
- Fixed-point baseline: 12 pass / 12 mismatch / 0 unsupported / 0 crash
- Final: 24 pass / 0 mismatch / 0 unsupported / 0 crash
- Blink/WebKit consensus subset: 23 cases, 11/23 → 23/23
- Stable final runs: `run-20260831T161159556Z-17000` and
  `run-20260831T161228867Z-3468`
- Final normalized projection SHA-256:
  `1867a4a90c806501230058892f6df59bad5455da399ee2d974c32d0963801dd0`

Chrome 154 run `5182906107691008` and Safari 26.6 run
`5141363439173632` are exact-revision browser oracles. Both pass 23 selected
cases. Chrome passes and Safari fails `root-element-opacity.html`, so that case
is retained as useful Amazing coverage but excluded from the cross-engine
consensus count.

The original six-category deterministic inventory measured 90/266 at the
fixed point. A bounded operational projection takes the first ten SHA-ranked
eligible cases per category. It moves 20/60 → 23/60 while retaining advanced
3D, blend/filter, line-clamp, table, and shadow failures. The focused gate is
authoritative for the implemented slice.

## Prototype decision

The deleted Rust terminal prototype exercised three difficult relationships:

- one stacking context divides descendants into negative, in-flow,
  positioned-auto/zero, and positive phases;
- a nested context is atomic, so a child with `z-index:999` cannot escape a
  parent below a sibling context;
- clip, transform, and opacity commands can scope a subtree without making
  the rasterizer traverse layout.

The resulting production seam is `display_list`: it is the only paint stage
that walks `LayoutBox`. Rasterization consumes a flat command stream containing
box decorations/content, overflow clips, and bounded effect groups.

## Implemented slice

- Negative, in-flow, positioned-auto/zero, and positive stacking phases with
  atomic nested contexts and hoisted descendant participants. Fixed and sticky
  boxes form atomic contexts even when `z-index` computes to `auto`.
- Full-subtree opacity groups with source-over RGBA compositing.
- 2D `matrix`, translate, scale, and rotate parsing with center-origin affine
  composition and inverse-mapped layer sampling.
- Rectangular and rounded overflow clips, including correct nesting with
  transformed groups.
- Border-radius normalization, rounded backgrounds, rounded border rings, and
  content/padding/border background-clip geometry.
- WPT-credited zero-blur outer shadows and currentcolor. Basic zero-blur inset
  shadows, spread, and double-border thirds are unit-covered but not counted
  as upstream conformance.
- Transparent-layer glyph compositing that preserves coverage until the group
  is composited, rather than baking antialiasing against black.
- Hard limits: one million display commands, layout depth 256, effect depth 16,
  and 16 million simultaneously allocated layer pixels. Exceeding a limit
  fails closed to a white frame.

Twenty-two public tests exercise the HTML → cascade → layout → display-list →
raster path, including hostile nested opacity groups.

The 12 capability-credited upstream gains are the simple outer shadow and 11
2D transform/transform-context cases listed in the result artifact. Direct
z-index escape, group opacity, inset/spread shadows, double borders, and
resource-limit behavior are public regression coverage, not claimed as new
upstream-conformance gains. Rounded/background cases remain pinned WPT
non-regressions that already passed at the fixed point.

## Deliberate frontier

This phase establishes the deep model, not the entire CSS painting corpus.
Remaining dependencies are explicit: floats and inline stacking fragments,
scrollbar UI and root overflow propagation, blurred/multiple shadows, full
WPT-conformant inset/spread shadow geometry, gradients/background images, transform-origin grammar,
3D/perspective/preserve-3d, blend modes/isolation, filters, outlines, and
anti-aliased curved edges. The 60-case diagnostic retains these failures.
Replaced and embedded presentation surfaces remain Phase #7 work.

The Phase #6 Test262 non-regression measurement remains 641 discovered, 196
supported classic scripts passed, 0 failed, and 445 explicitly skipped.

## GPUI proof

The locked ThinLTO release
`crates/amazing-shell-gpui/target/release/amazing-shell.exe` (built
2026-08-31T16:31:29.8277988Z) is open as PID 18120 in active RDP session 2. It renders
`examples/phase6-paint-proof.html` through the production engine inside GPUI.
`PrintWindow` captured the unobscured 1096×768 shell and its 960×570 browsing
frame at `target/diagnostics/phase6-paint-gpui.png`; the inspected image shows
the transformed red layer below the rounded/opaque green context, rounded
overflow, and both shadows. This visual proof complements, not replaces, WPT.
