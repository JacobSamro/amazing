# CSS lane decision map

## CSS-1 — typed computed values (prototype)

Typed values precede `@layer` (integrator decision, 2026-09-20). Locked decision
4a remains unchanged: `compute(dom, sheets) -> StyledDom` is the only style
computation seam. Custom-property dependency graphs, token substitution, and
CSS-wide defaulting remain private. No layout or paint source is changed.

### Approved interface and scope

`ComputedStyle::typed(name) -> Option<&ComputedValue>` adds a CSS-owned typed
view. Existing `get`, `iter`, and `set` signatures remain compatible. The
computed style stores a typed map alongside legacy serialization; existing
consumers can migrate separately without learning custom-property internals.
Unimplemented compound grammars are explicitly `ComputedValue::Unparsed`.

- Length/percentage values distinguish CSS pixels, percentages, and relative
  bases. All seven absolute units resolve at 96 CSS px/in, including Q.
  Computation resolves em/rem from computed element/root font sizes. ex/ch and
  viewport units retain explicit bases; the CSS seam does not invent metrics.
- Numbers and integers cover opacity, flex factors, order, z-index, orphans,
  widows, and unitless line-height. Property ranges are checked. Percent
  line-height computes against the element font; inherited computed lengths
  are copied rather than recalculated against a descendant's font.
- Angles retain radians without modulo reduction. The supported 2D transform
  list stores typed translate/scale/rotate/matrix operations.
- RGBA8 and currentColor reuse the existing paint color parser's supported
  grammar and precision. `color: currentcolor` inherits the parent's color;
  other color properties retain the currentColor keyword.
- Typed enums cover display, position, overflow, direction, box sizing,
  flex direction/wrap, border style, white space, visibility, text alignment,
  writing mode, float/clear, ruby roles, and the supported alignment keywords.
- Invalid ordinary longhands are rejected before ranking; invalid values
  after var substitution default as unset without reviving an earlier winner.
  Token boundaries are preserved: `var(--n)px` cannot create a dimension.
- Constructors and mutation update the typed map; mutations of one property
  preserve already computed inherited values of other properties.

This is a staged representation, not full CSS Values support: calc/min/max,
full color spaces/named-color coverage, all compound grammars, and layer/origin
rollback remain outside this ticket. Legacy consumers still parse serialized
values until their own migration. Typed ex/ch and viewport units require
used-value context. RGBA8 retains the existing parser's quantization and
permissive mixed legacy RGB channels; no new color-conformance claim is made.
Valid color keywords outside the RGBA8 table and unsupported color functions retain their serialized winning declaration and an
Unparsed marker. Treating those as invalid incorrectly revives earlier fallback
colors; the pinned corpus exposed this during development (30/29/13), and public
compute regression tests now cover the distinction. The same compatibility
rule preserves layout's existing calc(infinity), calc(-infinity), and calc(nan)
flex-factor forms as Unparsed until a typed math grammar lands.

References: [CSS Values 4](https://www.w3.org/TR/css-values-4/),
[CSS Variables 1](https://www.w3.org/TR/css-variables-1/).

### Evidence

Baseline source: lane/css c50ee3f before CSS-1. Lane rebased to origin/dev e9c1e21.
Windows-only execution at `C:\Users\server\src\wt\css`, target directory
`C:\Users\server\target\css`, two build jobs. No Mac build or test.
WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Selection: unchanged `tests/wpt-render/cascade-manifest.json`, 72 cases,
800x600 viewport; XHTML/XML tests and references remain unsupported.
An isolated WPT checkout avoids other lanes changing the sparse selection.
The earlier all-unsupported attempt against the shared sparse checkout is
invalid evidence and excluded.

- CSS unit tests before: 62 passed, 0 failed.
- CSS unit tests after: 87 passed, 0 failed.
- Color parity integration target: 17 passed, 0 failed (16 original paint
  parser tests plus parity over edge cases and deterministic generated input).
- Public compute regression target: 5 tests cover unsupported colors, invalid
  substituted colors, pre-existing flex calc constants, and all ruby display roles.
- Final locked workspace: **772 passed / 0 failed / 3 ignored**, across 48
  test/doc-test targets. Earlier failed attempts found missing named-color,
  special flex calc, and ruby-display compatibility cases; all were fixed
  within CSS and covered by tests before this successful run.
- Two baseline cascade runs: each 33 pass / 26 mismatch / 0 timeout /
  13 unsupported / 0 crash. Two corrected candidate runs reproduce these counts.
  All four normalized evidence hashes are identical. Variables is 12 pass /
  0 mismatch / 0 timeout / 0 unsupported / 0 crash in each of those runs.


### Final rendering evidence (2026-09-20)

Counts below are **pass / mismatch / timeout / unsupported / crash**. Each
selection uses its unchanged `tests/wpt-render/<selection>-manifest.json` at
the WPT revision above. Every before/after case has identical classification,
differing-pixel count, maximum difference, and actual/reference/diff SHA256.

| Selection | Before | After |
|---|---|---|
| cascade (two runs each) | 33/26/0/13/0 | 33/26/0/13/0 |
| Variables subset (two runs each) | 12/0/0/0/0 | 12/0/0/0/0 |
| block-collapse | 9/0/0/0/0 | 9/0/0/0/0 |
| position | 9/3/0/0/0 | 9/3/0/0/0 |
| text-flow | 14/2/0/0/0 | 14/2/0/0/0 |
| flex-grid | 24/0/0/0/0 | 24/0/0/0/0 |
| paint-compositing | 24/0/0/0/0 | 24/0/0/0/0 |
| text-shaping | 12/0/0/0/0 | 12/0/0/0/0 |
| vertical-ruby | 11/9/0/0/0 | 11/9/0/0/0 |

Normalized SHA256 (both baseline runs and both final runs):
`3cea34605e5ca14ffeb0255179366d0ead786d744df8ba5d461045c4368d534d`.
Normalization sorts tests by id, retains id/classification/relation,
differing_pixels/max_difference and all three image SHA256 fields when present,
then encodes JSON with sorted keys and compact separators before hashing.

Artifacts on win-001:

- Runnable baseline: `C:\Users\server\target\css\css1-before.exe`, SHA256
  `c77c5dd302b8e8d0d1ec1caff58ce76c2b80128d353fb45382cb2bf9a252fad1`.
- Runnable final: `C:\Users\server\target\css\css1-verified.exe`, SHA256
  `2387536cbadddb791a451e571c9fa191fa385f7aeefba907b703615499f5388e`.
- Per-case reports/images: `C:\Users\server\target\css\css1-baseline-{1,2}`,
  `css1-before-<selection>`, and `css1-verified-{1,2,<selection>}`.
- Successful workspace log: `C:\Users\server\target\css\css1-workspace-complete.log`.
- Local orchestration evidence: `.agents/css/evidence.json`, `summary.json`,
  `source-hashes.json`, and `workspace.log` in the integrator's main tree.
  All six changed Rust files match the tested Windows files byte-for-byte.

Reproduce from the lane's Windows checkout, using PowerShell:

```powershell
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing-css --locked
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css1-verified.exe `
  -OutputDirectory C:\Users\server\target\css\css1-review `
  -SkipFetch -SkipBuild
```

## CSS-2 — remove transitional paint color parser

**Owner:** replaced-content, coordinated with css. **State:** relayed and acknowledged by
replaced-content via the integrator; do not implement as part of CSS-1.

CSS's `amazing_css::color::parse_css_color` is canonical. The existing paint
parser is deprecated-in-place and transitional, not an independent fork.
`crates/amazing-css/tests/color_parity.rs` compiles the actual paint parser
source as a test-only module and compares its behavior with CSS's parser.
This avoids a reversed runtime dependency or a new workspace lockfile entry.

Replace paint's duplicate implementation with a compatibility re-export of
the CSS parser (preserving paint's public API). Move any remaining tests to CSS
and retire the source-linked transitional parity module. Before removal, keep
parity green for malformed syntax, Unicode, numeric extremes, alpha, named
colors, and hex forms. Rerun paint's focused 24-case gate and workspace tests.

## Later tickets

- Layout/paint adopt `ComputedStyle::typed` for their supported properties.
  Keep percentages and missing metrics explicit at the used-value boundary.
- Independent `@layer` cascade ordering, after CSS-1 review.
