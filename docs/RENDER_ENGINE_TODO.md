# Render-engine conformance decision map

Canonical sequence for moving Amazing's style → layout → paint pipeline toward
standards conformance. Resolve one ticket, verify its exit criteria on
`win-001`, record measured results, then stop at the phase boundary for review.

Frontier: **#1 — reproducible upstream WPT render harness**

## Evidence contract (resolved)

- **Standard conformance** means behavior required by the applicable WHATWG or
  W3C specification and measured with a pinned, unmodified upstream WPT case.
- **Blink/WebKit agreement** is a differential oracle used to triage ambiguous
  failures. It is not a replacement for the specification or WPT reference.
- Feature unit tests and curated WPT-inspired fixtures remain useful regression
  gates, but their counts must never be reported as upstream WPT pass rates.
- Every conformance report names the WPT revision, viewport, font set, device
  scale, test selection, pass/fail/timeout/crash counts, and artifact directory.
- Builds, tests, and reference captures run on `win-001`; local macOS remains
  edit-only. Probes accept parameters rather than embedding one fixture.

### Current measured baseline

- Automated, unmodified upstream WPT render cases: **0**.
- Existing curated CSS2 geometry subset: **17/17**.
- Existing feature regressions: **13 flex**, **16 grid**, **74 layout unit
  tests**, and **51 paint unit tests**.
- These numbers establish regression coverage only; they do not establish
  compatibility with Blink, WebKit, or the broader web platform.

## #1: Reproducible upstream WPT render harness

Blocked by: none
Type: Prototype
Status: ⬜ Pending

### Question

How will Amazing run a pinned upstream CSS reftest, render both test and
reference through the real engine, compare deterministic pixels, retain diffs,
and emit machine-readable measured results on `win-001`?

### Exit criteria

- A revision-pinned WPT source/manifest policy is recorded without copying or
  rewriting test assertions.
- A parameterized Windows probe selects tests, viewport, output directory, and
  timeout; it has no fixture-specific constants.
- The harness classifies pass, mismatch, timeout, unsupported, and crash, and
  saves actual/reference/diff images for every mismatch.
- At least one known-pass and one intentionally known-fail upstream reftest
  prove the harness rather than the engine feature.
- A repeat run produces the same classification and pixel hashes.

### Answer

Pending.

## #2: Cascade and computed-value conformance baseline

Blocked by: #1
Type: Research
Status: 🌫️ Fog

### Question

Which upstream CSS Syntax, Cascade, Selectors, Values, Color, and Fonts tests
reach the existing engine, and which missing primitive creates the largest
failure cluster?

### Answer

Pending until #1 exposes measured clusters.

## #3: Block, inline, sizing, and positioning convergence

Blocked by: #2
Type: Prototype
Status: 🌫️ Fog

### Question

What formatting-context and intrinsic-sizing changes close the first coherent
CSS2/CSS Sizing WPT cluster without encoding test-specific behavior?

### Answer

Pending until #2 identifies the first cluster.

## #4: Standards-grade text pipeline

Blocked by: #1, #3
Type: Research
Status: 🌫️ Fog

### Question

How will shaping, fallback, kerning, Unicode line breaking, bidi, vertical
metrics, and deterministic test fonts cross the std-only engine boundary?

### Answer

Pending. This may require a reviewed provider exception and therefore an
architecture decision; ASCII advance-per-character is not a parity path.

## #5: Flexbox and Grid convergence

Blocked by: #2, #3, #4
Type: Prototype
Status: 🌫️ Fog

### Question

Which flex/grid WPT clusters fail after shared sizing and text primitives are
correct, and what spec algorithms replace the current documented subsets?

### Answer

Pending until shared primitives stop dominating the results.

## #6: Paint order, clipping, stacking, and compositing

Blocked by: #3, #4
Type: Prototype
Status: 🌫️ Fog

### Question

Which display-list model is deep enough for backgrounds, border radii,
shadows, overflow clips, opacity groups, transforms, and stacking contexts
without coupling rasterization back into layout?

### Answer

Pending until geometry and glyph output are deterministic.

## #7: Replaced content and additional rendering surfaces

Blocked by: #3, #4, #6
Type: Discuss
Status: 🌫️ Fog

### Question

Which of form controls, image object fitting, SVG, Canvas 2D, and embedded
content belongs in this convergence program, and which requires a separate
engine surface and conformance map?

### Answer

Pending; scope will be chosen from measured WPT dependency clusters.

## #8: Continuous convergence gate

Blocked by: #2, #3, #4, #5, #6, #7
Type: Discuss
Status: 🌫️ Fog

### Question

What pinned WPT shards and non-regression budgets must every render change pass,
and what measured threshold defines the next compatibility milestone?

### Answer

Pending until representative costs and failure distributions are known.
