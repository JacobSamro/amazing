# CSS lane decision map

## CSS-12 — font features and broader cascade gate (fixed base `582a6af`)

CSS Fonts 4 `font-feature-settings` and `font-variant-ligatures` are inherited,
initial `normal`, and validated before cascade ranking. Their computed text is
available through `ComputedStyle::get()`. `font` resets both, including when
its value comes from `var()`, and also resets the already represented
`font-size-adjust` longhand. CSS Fonts 4 specification pin:
CSSWG drafts `main` revision `d3e4fc5eced2be0f70c1ba9cb7d23d3c54e2dec6`.

The broader gate fixes WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` and selects up to 80
static, self-contained, local-reference HTML reftests per category by the
lowest SHA-256 path hashes from `css/selectors`, `css/css-color`,
`css/css-values`, and `css/css-cascade`. Eligible totals are 83, 232, 107,
and 19; the frozen selection has 80, 80, 80, and 19 cases (259 total).
The manifest and inventory are in `crates/amazing-css/evidence/`. Exact-revision
Chrome 154.0.8016.0 and Safari 26.6 run IDs are 5182906107691008 and
5141363439173632. All 259 oracle cases were found: 215 pass/pass,
36 Chrome pass/Safari fail, 4 fail/fail, and 4 Chrome pass/Safari unreported.

The first consensus clusters selected from this corpus are Selectors 4
`:lang()` extended language ranges and CSS Color 4 `oklab()`, `oklch()`,
`lab()`, and predefined `color()` spaces. CSS normalizes supported colors at
computed style so paint receives a usable RGBA value. A same-space relative
`color(from currentColor <rgb-space> r g b)` retains its source through
explicit inheritance and rebinds to the inheriting element's `color`. The
unimplemented wider CSS Color 5 relative-color grammar remains a separate
cluster; this ticket covers the channel-copy identity form exercised by the
pinned corpus. Color conversion follows CSS Color 4's published matrices and
transfer functions; language-range matching follows Selectors 4 and RFC 4647
extended filtering.

Win-001 final gate: CSS nextest **209/209**, CSS doctests **0/0**;
workspace nextest **1,726/1,726** with 3 skipped, workspace doctests **2/2**.
The 259-case broad corpus ran twice on each build, with stable per-case
classification, differing pixels, and actual/reference/diff image hashes:
**160 pass / 97 mismatch / 2 crash → 184 pass / 73 mismatch / 2 crash**.
There are 24 raw pass gains, 21 Chrome/Safari pass/pass gains, and zero pass
losses. Consensus pass/pass counts move **153 → 174**. The two unchanged
crashes are the driver's `data:text/css` stylesheet path handling, routed in
`.agents/css/questions.md`. The old 72-case cascade corpus is **59/11/2 →
59/11/2**, with zero pass losses; its three changed image hashes remain passes.
All eight downstream selections have identical per-case projections. Full
per-case results and selection projection hashes are in
`crates/amazing-css/evidence/css12-case-results.jsonl` and
`css12-results.json`. Baseline and final executable SHA256 values are
`51c63e2bdc4e4db2090cb44cbc9d49011e4ef93618e6ae8ad6c3d93470858348`
and `192c550c43826af344b9ecad4d407d31cdaa927941544335030e6056967babaf`.
The gate used WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`;
`origin/dev` advanced to `f8bdb33` without changing CSS-owned paths since
the fixed base.


## CSS-11 — standardized CSS zoom and cascade retriage (fixed base `cb3064b`)

`zoom` accepts a nonnegative number or percentage, computes percentages to
numbers, and defaults to 1. It is not inherited as a computed property.
`ComputedStyle::effective_zoom()` multiplies this element's used zoom with
its ancestors'; zero has used zoom 1 for compatibility. Computed lengths
remain unscaled. `scale_used_length(px)` provides the used-value boundary
that layout and paint must adopt for absolute lengths and replaced natural
sizes. Percentages and `auto` do not scale. The pinned specification is
CSS Viewport Module Level 1 §4, CSSWG drafts `main` revision
`d3e4fc5eced2be0f70c1ba9cb7d23d3c54e2dec6`.

Win-001 fixed-base evidence: CSS nextest 201/201, doctests 0/0;
workspace nextest 1,706/1,706 with 3 skipped, doctests 2/2. The
72-case cascade corpus and eight downstream render selections ran twice
before and after with identical per-case classifications, differing-pixel
counts, and image hashes: zero changed cases or pass losses. Cascade stayed
59 pass / 11 mismatch / 2 unsupported. The additional pinned two-case
CSS Viewport zoom selection (`basic.html`, `inherited.html`) was 0 pass /
2 mismatch on both baseline and candidate, each with 30,000 differing
pixels; layout has not adopted the CSS used-zoom API. All 40 render runs
had no errors, timeouts, or crashes. WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

Baseline and candidate executable SHA256 values:
`8c9bfae737f84eb752c89c7edeb79911435deef467c8c2c7702df4a50f0a4c8c`
and `200c971f38f784e27846c8875d3eeff75cef40e68bf9246265893e13e426d711`.
The cascade projection SHA256 was
`616fe54f74d75858bab70845becd504f6689b58418de1543acb69cb68990df05`
on both builds. No CSS-owned files changed between the fixed base and
`origin/dev` at `8f2695d`.

The prior 59/11/2 cascade evidence contains no remaining CSS-owned first
blocker: eight `css-fonts` mismatches are text provider/font matching, one
`revert-layer-011` needs sampled registered-property animation, one
`all-prop-002` needs nested inline/block box layout, and
`calc-border-radius-1` needs layout/paint used-value radius resolution after
CSS already retains the typed math. These are routed in `questions.md` or
the integrator backlog. CSS-11 adds no speculative cross-lane fix.


## CSS-10 — SVG presentation attributes (fixed base `4f51acd`)

SVG 2 §6.6 presentation attributes on SVG namespace elements enter the
ordinary document cascade at the presentational-hint origin. The attribute
collector validates CSS property values and SVG unitless length and transform
syntax before constructing declarations. `patternTransform` and
`gradientTransform` map to `transform` only on their respective elements.
The standalone paint API and document cascade share the candidate resolver,
and `StyledDom::source` exposes the winning origin or inherited/initial source.
SVG-specific longhands have typed computed values and inherited metadata.

Specification pin: SVG Working Group `svgwg` revision
`513eb752b75f4250ce68edf28e7817d66726c840`, SVG 2
[`styling.html` §6.6](https://svgwg.org/svg2-draft/styling.html) and
[`propidx.html`](https://svgwg.org/svg2-draft/propidx.html). WPT render
selection pin: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

Fixed-base evidence on win-001: CSS nextest 198/198 and CSS doctests 0/0;
workspace nextest 1,640/1,640 with 3 skipped and workspace doctests
2/2. All nine pinned render selections ran twice on both baseline and
candidate. Cascade was 59 pass / 11 mismatch / 2 unsupported on both;
the eight downstream selections had zero pass losses. All 36 reports were
stable by per-case classification, differing pixels, and
actual/reference/diff image hashes, with no render errors or changes.
The fixed base is `4f51acd3ebe04c5941f31f1e479042f16b1f3c0b`; the
baseline and candidate executable SHA256 values were
`f482b1eeae3456be2e72012bd10ab0765e28e85cdb184e82540a9a9992a787d1`
and `ed755155c625fbb9109377043f6b36fda5c2c6fb006cb3bff925c7382c1f399e`.
The cascade per-case projection SHA256 was
`616fe54f74d75858bab70845becd504f6689b58418de1543acb69cb68990df05`
on both builds. The 59/11/2 fixed-base count supersedes the brief's
56/14/2 snapshot after sibling work advanced; no CSS-owned files changed
between this base and `origin/dev` at `6de1d934`.


## CSS-9 — logical sizing and unresolved CSS math (fixed base `bc8aa455`)

The cascade maps logical sizing (`inline-size`, `block-size`, and their
min/max variants) to width/height using the element's computed writing mode.
Logical margin, padding, and inset sides map using writing mode and direction.
Logical and physical declarations compete with their original origin, layer,
specificity, importance, and source order; both names expose the mapped
computed value. The source of truth for layout remains the physical longhand.

CSS Values math expressions on width/height, min/max sizing, margin, padding,
and inset are type checked in CSS and retained as unresolved
`ComputedValue::LengthPercentageExpression` plus canonical `get()` text.
Percentages retain their containing-block basis. Border-radius shorthand and
longhands accept validated length-percentage math without resolving it early.
The old simple negative-pixel radius clamp remains for compatibility.
Variable substitution precedes computed-time type validation. The math parser
supports nested `calc`, `min`, `max`, and `clamp`, sums, products, division by
a number, and CSS-required whitespace around binary plus/minus. It rejects
incompatible units and products of two dimensions.

Pinned WPT declaration reach: `calc-max-height-block-1`,
`calc-min-width-block-intrinsic-1`, `calc-border-radius-1`, and
`calc-rounding-003`. The CSS reach test uses the exact declarations from
WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Layout and paint own used-value evaluation and render movement; the fixed
base did not yet contain the `parse_length_percentage` and
`resolve_corner_radii` consumers mentioned in the brief. Requests are in
`.agents/css/questions.md`.

Evidence on win-001: CSS nextest 193/193 and doctests 0/0; workspace
nextest 1,611/1,611 with 3 skipped and workspace doctests 2/2. The pinned
nine-selection sweep ran twice for fixed-base baseline and CSS-9 candidate.
Cascade remained 56 pass / 14 mismatch / 2 unsupported. Every sampled case
in all nine selections retained the same classification, differing-pixel
count, and actual/reference/diff image hashes; zero pass losses or render
errors. The baseline and candidate executable SHA256 values were
`41767d974d4f360ac729af13158aea611e8e7ad5e8ed8e15912914ab9852ec08`
and `2a066579f3a2a3e7d17c6d08cb66a2a770e8c9c30bdc04d32712e5016dd7ef34`.
Their per-case cascade projection SHA256 was identical:
`9cfddc3ea3693acdee8aae6055ed99d8d3f2e9b19dc4c66eea02bdb4512dcf37`.
Fixed base `bc8aa45523d156f7e2f92128df7f585539509993`; pinned WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. A later `origin/dev`
`0f1afbd` had no changes under `crates/amazing-css` since that base. The
integrator's merged-head gate remains authoritative for the combination.

## CSS-8 — cascade corpus triage (final gate base `6fcb990`)

The 32 original mismatches were triaged on base `945ca38`: 21 CSS-owned
and 11 routed elsewhere. After the grid-line and DOM merges, the fixed-base
72-case cascade selection is 39 pass / 31 mismatch / 2 unsupported on
`6fcb990`, and 56 pass / 14 mismatch / 2 unsupported after CSS-8. The table
retains each original first blocker and records the final rebased transition.
Chrome 154.0.8016.0 and Safari 26.6 statuses come from the
[exact-revision wpt.fyi search API](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/);
both runs report WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
"pass / fail" means Chrome / Safari. A CSS first blocker can still need
layout or paint adoption after it is removed; the table does not promise a
render pass from parser work alone.

| Original mismatch case | First feature/root cause | Owner | Chrome / Safari | Rebased base → CSS-8 |
|---|---|---|---|---|
| `css-cascade__revert-layer-011.html` | @property + paused @keyframes custom-property interpolation; animation stage absent | integrator (animation) | pass / pass | mismatch → mismatch |
| `css-cascade__all-prop-002.html` | `all: inherit` creates inline containers; nested inline/block box layout | layout | pass / pass | mismatch → mismatch |
| `css-cascade__revert-layer-005.html` | important `revert-layer` leaves later important layers eligible | css | pass / pass | mismatch → pass |
| `selectors__has-style-sharing-002.html` | `:has(> span)` relational selector unsupported | css | pass / pass | mismatch → pass |
| `selectors__dir-style-03a.html` | `:dir(ltr)` directionality selector unsupported | css | pass / pass | mismatch → pass |
| `selectors__nth-child-of-no-space-after-of.html` | `:nth-child(3 of S)` syntax unsupported | css | pass / pass | mismatch → pass |
| `selectors__selectors-4__lang-016.html` | `:lang(fr)` ancestor/BCP47 prefix matching unsupported | css | pass / pass | mismatch → pass |
| `selectors__selectors-4__lang-022.html` | `:lang("i-navajo")` string argument unsupported | css | pass / pass | mismatch → pass |
| `selectors__nth-child-of-tagname.html` | `:nth-child(odd of webkit, fast)` selector-list filtering unsupported | css | pass / pass | mismatch → pass |
| `selectors__selectors-4__lang-002.html` | `:lang("FR")` case-insensitive language matching unsupported | css | pass / pass | mismatch → pass |
| `selectors__nth-last-child-of-complex-selector.html` | `:nth-last-child(4n of complex S)` filtering unsupported | css | pass / pass | mismatch → pass |
| `selectors__selectors-4__lang-000.html` | `:lang(en-US)` inherited language matching unsupported | css | pass / pass | mismatch → pass |
| `css-values__calc-border-radius-1.html` | `border-radius: calc(...) / calc(...)` shorthand rejects math values | css | pass / pass | mismatch → mismatch |
| `css-values__calc-max-height-block-1.html` | `max-height: calc(length + percentage)` rejected before layout | css | pass / pass | mismatch → mismatch |
| `css-values__calc-min-width-block-intrinsic-1.html` | `min-width: calc(...)` rejected before intrinsic sizing | css | pass / fail | mismatch → mismatch |
| `css-values__ch-unit-008.html` | `5ch` accepted as typed unit but used width lacks zero-glyph metric | layout | pass / pass | mismatch → mismatch |
| `css-values__calc-invalid-range-clamping.html` | `border-radius: calc(-10px)` rejected instead of computed clamping | css | pass / pass | mismatch → pass |
| `css-values__calc-rounding-003.html` | original base lacked `margin-inline` and width `calc()`; upstream base already renders this case | css | pass / pass | pass → pass |
| `css-color__deprecated-sameas-017.html` | `@supports` system color `ThreeDFace` and ButtonFace palette unsupported | css | pass / pass | mismatch → pass |
| `css-color__hwb-005.html` | `hwb()` color conversion unsupported | css | pass / pass | mismatch → pass |
| `css-color__a98rgb-001.html` | `color(a98-rgb ...)` color-space conversion unsupported | css | pass / pass | mismatch → pass |
| `css-color__deprecated-sameas-019.html` | `@supports` system color `ThreeDLightShadow` and ButtonBorder palette unsupported | css | pass / pass | mismatch → pass |
| `css-color__hsl-003.html` | modern `hsl()` color conversion unsupported | css | pass / pass | mismatch → pass |
| `css-color__lch-001.html` | `lch()` Lab conversion unsupported | css | pass / pass | mismatch → pass |
| `css-fonts__font-family-name-000.xht` | CSSTest Verify/Fallback installed-font fixture and family fallback differ | integrator (text provider) | fail / fail | mismatch → mismatch |
| `css-fonts__font-family-name-022.xht` | multiword family names including `x-large`/`caption` need matching installed fonts | integrator (text provider) | pass / pass | mismatch → mismatch |
| `css-fonts__test-synthetic-italic.html` | synthetic italic glyph shaping/raster missing | integrator (text provider) | pass / pass | mismatch → mismatch |
| `css-fonts__font-family-name-018.xht` | unknown named family must fall back to inherited/available face | integrator (text provider) | pass / pass | mismatch → mismatch |
| `css-fonts__font-variant-ligatures-11.optional.html` | `font-variant-ligatures` / `font-feature-settings` shaping | integrator (text provider) | pass / fail | mismatch → mismatch |
| `css-fonts__matching__font-unicode-PUA-primary-font.html` | PUA glyph fallback restriction/font matching | integrator (text provider) | pass / pass | mismatch → mismatch |
| `css-fonts__font-family-name-024.xht` | system-named family fallback against CSSTest font fixture | integrator (text provider) | fail / fail | mismatch → mismatch |
| `css-fonts__font-family-name-019.xht` | unknown named family fallback against CSSTest font fixture | integrator (text provider) | pass / pass | mismatch → mismatch |

Seventeen cases moved from mismatch to pass: important `revert-layer`, nine
Selectors 4 cases (`:has`, `:dir`, four `:lang`, and three filtered `:nth`),
computed clamping of a negative `calc()` radius, and six CSS Color 4 cases
(system color aliases, HWB, A98 RGB, modern HSL, and LCH). No pass was lost.
The remaining 14 mismatches comprise three CSS math/used-value cases and
the 11 routed non-CSS blockers listed in `questions.md`. `margin-inline` now
expands in CSS computation; `calc-rounding-003` already passed on the rebased
base. The three remaining math cases need layout's mixed length/percentage
and intrinsic sizing consumption. Their adoption requests are in
`questions.md`.

Evidence: final fixed base `6fcb990`, WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, two stable runs on each
side. Baseline and candidate per-case projection SHA256 values are
`72e11773d593ab7e927c2a0c317daa76f496f96d3ae1fdb5cd9ca4e443377a86`
and `749e0f97c7f8fe10d3e6eab13da495d0340090b39d1e2bbcdf5042c5a36b23a8`.
Their immutable executable SHA256 values are
`963f2eef5caffe559c75717b2725ab5f080688a1568dd63f3ca5a46ba6e237be`
and `ef5355c7f809a02012076b8aab850724cf41931f2578afe13b74177fb30cce54`.
The final per-case reports and all nine selections are summarized in
`.agents/css/css8-rebase-final-summary.json`; the eight other selections have
identical classifications, pixel counts, and image hashes before/after.
No pass was lost. On win-001, CSS nextest passed 187/187 and doctests 0/0;
workspace nextest passed 1,562/1,562 with 3 skipped and workspace doctests
passed 2/2. The fixed-base source hash and both executable hashes remained
stable through the render sweep. `origin/dev` later reached `55d3fc1` with no
changes under `crates/amazing-css` since `6fcb990`.

## CSS-7 — consumer adoption

Replaced-content's SVG adapter can call `compute_element_paint_style(parent,
presentation_hints, author, inline)` for each SVG element, passing the returned
`computed` style to descendants. Convert supported `color` and `fill` RGBA8,
`fill-opacity`, and `opacity` to the adapter's paint values; preserve `None`
fill. Handle `ElementStyleValue::Unsupported` explicitly (for example SVG paint
servers and CSS color functions outside the current parser), rather than
reparsing the CSS text or guessing a fallback. Presentation attributes must be
passed as `Declaration`s in `presentation_hints`, not mixed with inline style;
the API keeps their lower cascade origin. The optional `author` slice carries
matched stylesheet declarations when the consumer has them. CSS-wide keywords,
variables, important declarations, and origin rollback stay inside CSS.

The same ticket canonicalises supported CSS Display 3 two-keyword forms to
existing computed `Display` variants, including `inline flow-root` to
`inline-block`, for LAYOUT-6 consumption. The `grid` shorthand now carries
explicit subgrid track lists into both template longhands and resets its
implicit track longhands. The pinned reach probe reports 19/19 subgrid cases
twice; this is CSS typed reach, not a layout render pass.

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

## Cross-lane follow-up — remove transitional paint color parser

Originally labeled CSS-2 in the CSS-1 handoff; this is a replaced-content
follow-up, separate from the CSS-2 cascade ticket.

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

## CSS-2 — cascade layers, origin/layer rollback, and scope

Implementation stays inside `amazing-css`; `compute(dom, sheets)` and typed
computed values remain the consumer seam. No caller/layout/paint files changed.

### Decisions and behavior

- Parse layer statements and blocks, dotted/nested names, and anonymous layers.
  Keep empty ordering declarations, including layers without matching rules.
  Names are case-sensitive and shared across author sheets; anonymous segments
  are private to each occurrence and stylesheet. Preserve token adjacency in
  dotted names; `default` is a valid identifier, CSS-wide keywords are not.
- Compile hierarchical first-appearance order across all sheets of an origin.
  A parent's direct declarations occupy its last implicit sublayer; unlayered
  rules occupy the outermost last layer. Important declarations reverse that
  entire order; inline style keeps its separate precedence.
- Compare importance, origin (reversed for important), element-attached style,
  layer (reversed for important), matched-selector specificity, scope proximity,
  then declaration order. Scope-start specificity never enters the rule's key.
- Retain the property candidate lists until rollback finishes. `revert` removes
  the author origin at both importance levels; UA revert defaults as unset.
  `revert-layer` removes the current layer and the important-to-normal interval,
  with the element-attached important exception. Rollback can chain through
  several layers. Logical margins join physical candidates before final ranking.
- Custom properties roll back before their dependency graph is resolved.
  Longhand and supported deferred-shorthand rollback keywords produced by
  variable substitution use the same retained candidates. Invalid substituted
  values still default; they do not revive a lower candidate. `all` also accepts
  variable substitution while retaining its CSS-wide-only grammar and excluding
  direction, unicode-bidi, and custom properties.
- Parse explicit scope roots/limits and nested scopes. Consider every applicable
  ancestor root, exclude limits and descendants, and use innermost-root distance
  only after specificity. `:scope` resolves through compound selector functions
  and combinators; outside an explicit scope it matches the document element.
  Implicit relative scope anchors contribute zero specificity.
- Group nesting and combined layer-name depth are capped at 128. Oversized
  constructs are dropped, not partially flattened into unlayered declarations.

References: [Cascade 5 sorting/layers/rollback](https://www.w3.org/TR/css-cascade-5/),
[Cascade 6 scoped rules and proximity](https://drafts.csswg.org/css-cascade-6/).

### Approved integration boundary and limits

`Stylesheet::scope_root: Option<NodeIdx>` stores the stylesheet owner's parent
for implicit scope; `None` is the ownerless document default. Explicit
`@scope (<selector>)` does not require it. The user approved this seam and owns
integrator work to preserve **separate stylesheets and owner nodes** through the
driver and shell, which currently concatenate CSS. This is also prerequisite
provenance for CSS-4 URL resolution (originally queued as CSS-3).

**Implicit @scope cannot be measured end to end until that driver/shell wiring
lands.** Public-seam tests assigning `scope_root` prove only CSS behavior, not
end-to-end browser support. No caller code is changed by this ticket.

Existing selector limitations remain (pseudo-elements, complex functional
selector arguments, CSS nesting `&`, and shadow-tree behavior). Layer-qualified
imports and media evaluation remain outside the parser's supported rule set.
User-origin sheets, animation origins, and transitions have no input seam;
origin rollback here covers the existing author and UA origins. This ticket
makes no full Cascade 5/6 conformance claim.

### Validation

Clean-artifact Windows workspace: **943 passed / 0 failed / 3 ignored**, across
52 test/doc-test targets. CSS: **88 unit + 27 cascade-context + 17 color-parity
+ 5 typed-compute tests**, all passing. The 27 public-seam tests plus one added
UA-origin unit test are regression tests, not added WPT coverage. Rendering
measurements are recorded below. Evidence uses base 80c6636
(133292f plus the refresh-script commit). The first refresh invalidated earlier
CSS-2 runs. A subsequent full-workspace attempt found stale Cargo library
artifacts even though source hashes matched: NET-3 tests could not find methods
present in `ParsedUrl`. Tar extraction restored older mtimes, so source-hash
verification alone was insufficient. All 18 first-party workspace packages were
cleaned from the isolated CSS target, and both measurements restarted as `r3`.
Only `css2-r3-*` artifacts qualify as final evidence. Build inputs are checked
before and after compilation, and immutable executable hashes before/after
rendering; the final source tree is also checked after all gates.


WPT revision: **c2a2fce69e5c2509d9dc28440ec8b42532e5c293**. Unchanged
`tests/wpt-render/<selection>-manifest.json` selections; lane-local WPT checkout,
800x600 viewport, scale 1, same fonts/resources. Counts are
**pass / mismatch / timeout / unsupported / crash**.

| Selection | Before | After |
|---|---|---|
| cascade (two runs each) | 32/27/0/13/0 | 35/24/0/13/0 |
| Variables subset (two runs each) | 12/0/0/0/0 | 12/0/0/0/0 |
| block-collapse | 9/0/0/0/0 | 9/0/0/0/0 |
| position | 9/3/0/0/0 | 9/3/0/0/0 |
| text-flow | 14/2/0/0/0 | 14/2/0/0/0 |
| flex-grid | 24/0/0/0/0 | 24/0/0/0/0 |
| paint-compositing | 24/0/0/0/0 | 24/0/0/0/0 |
| text-shaping | 12/0/0/0/0 | 12/0/0/0/0 |
| vertical-ruby | 11/9/0/0/0 | 11/9/0/0/0 |

Both baseline normalized hashes:
`f722c4dda0d3324bfa245734c6dc4d7688b2b819a0a46a9f8ca003f099ed8183`.
Both final normalized hashes:
`a259e9d395326e4c39b890dd494b159dcacb28a834ccc497e6ee94a7ec608026`.
Normalization is the same sorted per-case classification/difference/image-SHA256
projection used for CSS-1, excluding timestamps and filesystem paths. Every
one of the seven downstream selections is identical before/after, including
actual/reference/diff image hashes, not just counts.

Exactly three cascade cases change mismatch → pass:
`css-cascade__scope-featureless.html`,
`css-cascade__all-prop-revert-color.html`, and
`css-cascade__scope-pseudo-element.html`. The last remains **partially inert**:
pseudo-element/generated-content selectors are dropped in both test and reference;
this is a measured pixel pass after the root background starts matching, not a
claim of pseudo-element conformance. The three selected `revert-layer` cases
remain unsupported because their references require XHTML/XML parsing. Layer
semantics are covered by focused Rust tests, not added WPT passes.

The refreshed baseline is 32/27/13, not CSS-1's historical 33/26/13:
`css-values__calc-max-height-block-1.html` already mismatches on the refreshed
siblings before CSS-2. That drift is not attributed to this change.

Artifacts on win-001, all under `C:/Users/server/target/css/`:

- Baseline executable `css2-r3-before.exe`, SHA256
  `b16ce505d6334bf8ea33051d1bea99c5843a631764713b3553f06eeba340e230`.
- Final executable `css2-r3-final.exe`, SHA256
  `60442ba140ed95eb21bfdc240387469ed610b3956c87ccffdd4446d8f0648430`.
- `css2-r3-{before,final}-<index>-<selection>/run-*/report.json` and PNGs.
  Indices 1/2 are cascade; 3–9 follow the downstream order in the table.
- `css2-r3-workspace.log`, source/executable build proofs, and per-run logs.

Local orchestration evidence in the main checkout's `.agents/css/`:
`css2-r3-evidence.json`, `css2-r3-summary.json`, `css2-r3-workspace.log`,
`css2-baseline-source.json` (196 inputs), `css2-final-source.json` (199 inputs),
`css2-r3-{before,final}-build-proof.json`, and source-proof text files. The final
199 source/input hashes still matched after all gates. No Mac build or test ran.

Reproduce on Windows after syncing the complete chosen base, then the lane CSS
sources. Invalidate first-party Cargo artifacts after archive-based refreshes;
otherwise restored old mtimes can leave the wrong libraries cached. Use two jobs:

```powershell
Set-Location C:\Users\server\src\wt\css
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing-css --locked
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css2-r3-final.exe `
  -OutputDirectory C:\Users\server\target\css\css2-review `
  -SkipFetch -SkipBuild
```

## CSS-3 — stylesheet ownership and implicit scope through the driver

The integrator reassigned CSS-3 to close the CSS-2 caller boundary. Background
shorthand and URL-base propagation are now CSS-4. Final base: origin/dev `0e298c1`.

- Driver `<style>` and fetched `<link rel=stylesheet>` sources are parsed as
  separate sheets in document order, with `scope_root` set to the owner node's
  parent. `style` and `layout` preserve their existing inline-then-extra-file
  policy; render preserves interleaved style/link document order after scripts.
- `compute` receives the ordered sheet slice. The layout command uses existing
  public box-tree and fallback layout functions with that computed result.
- A narrowly granted additive layout entry point,
  `layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport`,
  performs the existing fallback layout, float-aware shaping, replaced content,
  and optional fixed viewport pipeline. The existing string entry point is
  unchanged, per the integrator's explicit restriction during the tables ticket.
- Raw source text remains available for legacy font discovery. This does not
  yet resolve font URLs relative to their external stylesheet.
- Driver CLI regressions exercise split versus concatenated cascade order,
  shared named layers and important reversal, variables, extra-file order,
  interleaved fetched sheets, implicit scope for style/link owners, nested scope
  proximity, layout output, and parser recovery at a stylesheet boundary.
- GPUI is not changed. The integrator must collect separate sheets after DOM
  changes, set owner-parent scope roots, and use the new sheet-aware layout API
  (or pass the slice to `compute` at its existing direct call sites).

### Final CSS-3 evidence

All builds and tests ran on win-001, source `C:/Users/server/src/wt/css`,
target `C:/Users/server/target/css`, two Cargo jobs. Final base `0e298c1`
received an integrator artifact-invalidating refresh, followed by rebase and
re-sync of the three granted source/test files. Only `css3-r3-*` evidence is
final; earlier attempts were superseded by refreshes. No new dependencies.

- Full locked workspace: **1,023 passed / 0 failed / 3 ignored**, 53 targets.
- New actual-driver CLI regressions: **6 passed / 0 failed** (included above).
- Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`;
  unchanged `tests/wpt-render/cascade-manifest.json`, 800x600, scale 1,
  lane-local WPT checkout and the same fonts/resources.
- Two baseline and two candidate runs each: **35 pass / 24 mismatch /
  13 unsupported / 0 timeout / 0 crash**. Variables: **12/12** in all four.
  Every per-case classification, differing-pixel count, maximum difference,
  and actual/reference/diff image SHA256 is unchanged across all four runs.
- All four normalized evidence hashes:
  `a259e9d395326e4c39b890dd494b159dcacb28a834ccc497e6ee94a7ec608026`.
  Normalization is the sorted per-case projection documented for CSS-2 above.
  This preserves the pinned corpus; it does not add WPT conformance passes.

The dedicated runnable **80x60 implicit-scope fixture** has three 20px bands.
A style owned by the first section scopes it green, an external link owned by
its sibling scopes that section blue, and the unscoped third section stays red.
Compared with the explicit-selector reference, the baseline differs at
**3,200 pixels**, while the candidate differs at **0 pixels**. Candidate and
reference PPM SHA256 both equal
`e38c7b3cde94c88dbc92af0c36dcfb0e197bbce0a040ea303bfee19683897ec7`.
The final PPM was copied to the Mac, converted to PNG, and visually inspected.
This is driver software-raster evidence, not a GPUI window-capture claim.
The durable equivalent fixture and assertions live in
`crates/amazing/tests/css3_stylesheets.rs`.

Source manifests cover first-party Rust/manifests, root lock/manifest, the
render runner and selected cascade manifest. **163 baseline / 164 candidate**
inputs match before/after compilation; all 164 candidate inputs match again
after the gates and match the Mac source tree. Immutable executable hashes
were checked throughout both gate runs and at final collection.

Artifacts under `C:/Users/server/target/css/`:

- Baseline: `css3-r3-before.exe`, SHA256
  `d0ef0834ed903f409ff5423bc5a30de990bc775019290a8389a2f7df9ada818d`.
- Candidate: `css3-r3-final.exe`, SHA256
  `a7ba29b18321020c61b0b2f2c8214970939b130cb3ba6aa4f83ca8f19a30c11e`.
- Cascade reports/images: `css3-r3-{before,final}-{1,2}/run-*/`.
- Scope fixture: `css3-scope.html`, `css3-scoped.css`,
  `css3-scope-reference.html`; rendered `css3-r3-{before,final}-scope.ppm`,
  `css3-r3-reference.ppm`, and `css3-r3-scope-proof.json`.
- Workspace log: `css3-r3-workspace.log`.

Local summaries, build/source proofs, fixture/PPM/PNG copies, and reproduction
helpers are in the main checkout's `.agents/css/`: `css3-r3-evidence.json`,
`css3-r3-summary.json`, `css3-r3-*-source.json`, `css3-r3-*-build-proof.json`,
`css3-r3-final-source-proof.txt`, and `summarize_css3.py`.

Reproduce after the approved tree refresh and re-sync on Windows:

```powershell
Set-Location C:\Users\server\src\wt\css
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing --test css3_stylesheets --locked
cargo test --workspace --locked
& C:\Users\server\target\css\css3-r3-final.exe render-wpt `
  C:\Users\server\target\css\css3-scope.html `
  C:\Users\server\target\css\css3-scope-review.ppm 80 60
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css3-r3-final.exe `
  -OutputDirectory C:\Users\server\target\css\css3-review `
  -SkipFetch -SkipBuild
```

The existing `&str` layout API is unchanged. Exactly five driver functions
changed: `cmd_style`, `collect_style_text`, `collect_author_css`, `cmd_layout`,
and `render_surface`. All authored files remain below 4,000 lines (driver:
3,519; layout lib: 1,549; CLI regressions: 197). CSS-3 is complete; stop here.

## CSS-4 — side-border shorthands and author-specified provenance

The integrator reassigned CSS-4 to two CSS-owned requests while shell stylesheet
provenance remains outstanding. Background shorthand was provisionally CSS-5 (subsequently reassigned below). Base:
`156cc13`. No layout, paint, driver, shell, or root-manifest changes.

### Side-border shorthands

`border-top`, `border-right`, `border-bottom`, and `border-left` expand to the
side's width/style/color longhands before ranking. Each accepts one optional
component of each category, in any order; omitted components reset to medium,
none, and currentcolor. Duplicate, unsupported-width, or invalid components
reject the whole declaration without emitting partial longhands. Validation
uses the existing token component splitter and typed longhand validators.
Known unsupported color spaces retain the existing Unparsed compatibility
policy; this is not a new full CSS Color or math grammar.

Each var-bearing side shorthand retains a deferred whole value per longhand.
After substitution, the same atomic grammar extracts each component; an invalid
winner defaults as unset without reviving earlier candidates. Token boundaries
remain intact: var(--n)px cannot manufacture a dimension. CSS-wide values work
before and after substitution, including origin/layer rollback. Expanded
longhands retain source order, importance, specificity and layer metadata.
Ordinary non-variable border grammar is unchanged; the CSS-4 follow-up below
corrects whole-border CSS-wide and deferred-var expansion.

### Additive consumer surface

`ComputedStyle::is_author_specified(property: &str) -> bool` reports whether
this element's final cascaded winner is a stylesheet or inline author
declaration **after rollback**. It does not infer origin from the computed value.
Query longhands (`background-color`, `border-top-style`, etc.), not shorthand
names. Normal names are ASCII-insensitive; custom-property names preserve case.

| Situation | Result |
|---|---|
| Explicit author value, including transparent/none/initial values | true |
| Explicit initial, inherit, unset, or color:currentcolor | true |
| Expanded author shorthand or all longhand | true |
| Author winner invalid after var substitution, defaulting as unset | true |
| No local declaration; inherited value from an author-styled parent | false |
| UA rule, materialized initial value, or dir presentational hint | false |
| revert / revert-layer, including substituted rollback | origin of restored winner; false if none/UA |
| Invalid at parse time | ignored; provenance of surviving winner |
| Synthetic FromIterator / Default | false |
| Synthetic set | clears markers only for replaced longhands |

The marker describes the winning declaration, not whether its value was valid
at computed-value time and not historical presence of any losing author rule.
Inheritance does not copy provenance to descendants. Explicit custom-property
and unknown-property declarations may also be queried, without exposing custom
values. Logical-to-physical margin mapping carries the winning origin.

Both requesting lanes have adoption instructions in `.agents/css/questions.md`:
layout can adopt side shorthands; replaced-content can query author-specified
longhands before choosing control defaults. Existing consumers need no edits.
CSS does not claim that control painting or the table tests have adopted it yet.

### Validation

Final evidence uses base **156cc13**, after the lane ran
`bash scripts/refresh-lane-trees.sh css` with artifact invalidation and re-synced
its own source files (lock prefix 3FEAF813). All execution was on win-001,
source `C:/Users/server/src/wt/css`, isolated target `C:/Users/server/target/css`,
two Cargo build jobs. No Mac builds/tests and no consumer changes.

- Full locked workspace: **1,038 passed / 0 failed / 3 ignored**, 55 targets.
- Focused CSS: **152 passed / 0 failed**, including **15 new public-compute
  regressions** (8 provenance, 7 side-border tests exercising all four sides).
- The initial focused failure caught Number/Ident token fusion after var
  substitution; using boundary-preserving serialization fixed it before gates.
- WPT revision **c2a2fce69e5c2509d9dc28440ec8b42532e5c293**; unchanged selections
  in `tests/wpt-render/`, lane-local WPT checkout, 800x600 / scale 1 / same fonts.
- **32 rendering runs:** two baseline and two candidate runs for every selection.
  Every per-case classification, differing-pixel count, maximum difference,
  and actual/reference/diff SHA256 is identical across all four runs for each
  selection. Variables remain **12/12** in all four cascade runs.

Counts are pass / mismatch / timeout / unsupported / crash, each run:

| Selection | Before | After |
|---|---|---|
| cascade | 35/24/0/13/0 | 35/24/0/13/0 |
| block-collapse | 9/0/0/0/0 | 9/0/0/0/0 |
| position | 9/3/0/0/0 | 9/3/0/0/0 |
| text-flow | 14/2/0/0/0 | 14/2/0/0/0 |
| flex-grid | 24/0/0/0/0 | 24/0/0/0/0 |
| paint-compositing | 24/0/0/0/0 | 24/0/0/0/0 |
| text-shaping | 12/0/0/0/0 | 12/0/0/0/0 |
| vertical-ruby | 11/9/0/0/0 | 11/9/0/0/0 |

Normalized SHA256, identical in all four runs per selection (same sorted
per-case projection and JSON normalization documented for CSS-2):

- cascade: `a259e9d395326e4c39b890dd494b159dcacb28a834ccc497e6ee94a7ec608026`.
- block-collapse: `86e7829838e559e7ad23e5556610044630d928f6e143b86476c28f95bbfa035e`.
- position: `4fe678b080d9f157b5fb1692c37331048a556478bc9785b2ddc1a0082bfb407a`.
- text-flow: `c5a0a7d3d4d164921b00d80168db276be7e8c286335c6c4c3a7d46113885bda3`.
- flex-grid: `36ab53bedb24655ca108bc25d6d06105400f10ad48f99a36414a673ed46d04c5`.
- paint-compositing: `56a70b2097c2ce5e22a0ff05f70551f71b0d4d84022b8596edf1a66855bb2e2d`.
- text-shaping: `1effab2e65d081d3b2e1a2d1b6e46a2ae853d66c7bfcfcfb7ff606bb24a898f0`.
- vertical-ruby: `30fabfc95c6d77d8796155179ee310c9a2b6be8ed1009c806bf42de482e8fb71`.

**171 baseline / 173 candidate** source/build/gate inputs matched before and
after compilation. All 173 final inputs matched again after every gate and
against the committed Mac sources. Immutable executable hashes were checked
through both rendering passes and at final collection. Under
`C:/Users/server/target/css/`:

- Baseline `css4-before.exe`, SHA256
  `3a74f33b1497114b6e0d6ba59e1b8be58dca4e9e82b59cd6e5133ff753a54a00`.
- Candidate `css4-final.exe`, SHA256
  `d2b27fa8f5d24c5683ea81a1e529e9fd8c4d195854b56265bd55fbf628e8c6cc`.
- Reports/PNGs: `css4-{before,final}-<selection>-{1,2}/run-*/`.
- Logs: `css4-workspace.log` and `css4-focused.log`.

Local evidence beside the main checkout's `.agents/css/report.md`:
`css4-evidence.json`, `css4-summary.json`, `css4-*-source.json`,
`css4-*-build-proof.json`, `css4-final-source-proof.txt`, the logs, and build,
render, collection and normalization helpers. No new WPT passes or consumer
adoption are claimed. The runnable driver executable above uses the new side
shorthands; the API regressions exercise provenance through public compute.

Reproduction after the lane refresh and re-sync, on Windows:

```powershell
Set-Location C:\Users\server\src\wt\css
$env:CARGO_TARGET_DIR = 'C:\Users\server\target\css'
$env:CARGO_BUILD_JOBS = '2'
cargo test -p amazing-css --locked
cargo test --workspace --locked
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-render-wpt.ps1 `
  -ManifestPath tests\wpt-render\cascade-manifest.json `
  -WptRoot C:\Users\server\target\css\wpt `
  -ExecutablePath C:\Users\server\target\css\css4-final.exe `
  -OutputDirectory C:\Users\server\target\css\css4-review `
  -SkipFetch -SkipBuild
```

Repeat the renderer for each unchanged selection above and a second output
folder per selection. All authored files remain below 4,000 lines; largest
changed source is computed.rs at 3,170. CSS-4 is complete; stop here.


### CSS-4 follow-up — whole-border rollback provenance

Downstream replaced-content reproduced `button{background:revert;border:revert}`
with background and border colors unmarked, but border widths/styles incorrectly
author-specified. The legacy whole-border parser treated the CSS-wide token as
color and emitted literal medium/none values for the other components. Those
were genuine author candidates from the API's perspective; provenance storage
and layout propagation were not the cause.

An eight-line guard now expands whole-border CSS-wide and var-bearing values
through the existing side-border helper for all twelve longhands. This covers
initial/inherit/unset/revert/revert-layer before and after substitution, and
keeps ordinary non-variable border parsing unchanged. Four public-compute
regressions compare a button's shorthand against explicit longhands, exercise
all CSS-wide defaults/inheritance, restore every component from a lower author
layer, and test valid/invalid deferred whole-border values. This extends the
original side-border coverage to the legacy whole-border path that had been
left out of CSS-4's initial scope.

Measured after rebase/self-refresh to **784dc59**, lock **23CEF873**, on win-001:

- Red command: `cargo test -p amazing-css --test author_provenance whole_border_rollback_matches_explicit_longhands_for_control_chrome --locked`.
  **1 failed**: `revert: border-top-width incorrectly suppresses control chrome`.
- Corrected CSS suite: **156 passed / 0 failed**, including the four additions.
- Full locked workspace: **1,138 passed / 0 failed / 3 ignored**, 58 targets.
- All **180** Rust/manifest/lock input hashes matched the Mac after the tests.
- Runnable regression artifact: `C:/Users/server/target/css/css4-rollback-author-tests.exe`,
  SHA256 `1407ca6664f9026f4355845f337b4a5d48387fc1b54c59d1c3c874a2ea65cf07`. Run with
  `whole_border_rollback_matches_explicit_longhands_for_control_chrome --exact`.
- Logs and proofs: `.agents/css/css4-rollback-{red,green,workspace}.log`,
  `css4-rollback-source.json`, `css4-rollback-source-proof.txt`, and
  `css4-rollback-artifact-proof.txt`; server copies in the CSS target directory.

No layout/paint/driver files changed and no debug instrumentation was added.
The original 32 rendering runs above precede this follow-up; they were not rerun
and are not claimed as post-fix evidence. This focused follow-up proves the
canonical CSS API regression and the refreshed full workspace.

## Later ticket — background shorthand and stylesheet URL provenance (queued)

User originally queued this during CSS-2; **do not start without a new brief**. Replaced-content has implemented background longhands in paint.
CSS owns expansion of the full multi-layer `background` shorthand grammar;
the current single-colour arm makes shorthand values containing `url()` unusable.
CSS must also preserve the winning declaration's stylesheet-relative URL
provenance through computed values, resolving against the originating stylesheet.
Keep this work separate from CSS-2 cascade layers, revert, and scope.


The deferred background ticket still needs an explicit stylesheet base-URL seam: preserve the fetched
stylesheet's final URL, inline/ownerless sheet document base, and the winning
declaration's provenance through cascade/variable/shorthand computation. CSS-3
preserves sheet identity and scope roots, not URL bases. Font discovery still
uses joined source text and the page URL; the render command's extra CSS string
also retains its existing combined ownerless input. Coordinate those caller
seams with the integrator before claiming relative-resource conformance.


## Routed request — HTML width/height presentational hints (now CSS-5)

Routed by integrator-assist after correcting the intrinsic_size contract.
Width/height attributes were accidentally applied as intrinsic dimensions by
the old driver adapter; that behavior has correctly been removed. CSS currently
omits the corresponding presentational hints. Implement low-priority HTML
width/height hints in CSS, with applicable-element/value parsing rules and
ordinary author CSS overriding them. Keep author-declaration provenance distinct
from these hints, as it already is for dir. The existing StyledDom and
ReplacedElementProvider boundaries suffice; do not restore the driver workaround.

This was recorded separately from the CSS-4 border rollback follow-up and is
now assigned CSS-5 below. Background shorthand remains deferred pending shell
stylesheet provenance.


## CSS-5 — HTML presentational hints (complete on fixed base 2021707)

Assigned 2026-09-21 after CSS-4 and its border rollback follow-up merged.
The previously queued background shorthand ticket remains deferred; CSS-5 now
owns the HTML attribute regression exposed by the corrected intrinsic-size contract.

A distinct `Hint` origin sits between UA and author origins, before layer and
specificity comparisons. Every author declaration (including a universal rule in
the first layer) wins. Hints never set `is_author_specified`. Explicit author
initial/inherit/unset remain author-specified. Per Cascade 5 section 6.5, author
`revert` removes hints, whereas `revert-layer` can restore them. The same applies
after variable substitution. Existing public compute/StyledDom signatures stay intact.

DOM-local mappings implemented for review:

- HTML img/embed/iframe/object/video and input[type=image] width/height;
  img/video/image-input aspect ratio; canvas aspect ratio only (its attributes
  remain intrinsic dimensions, not CSS width/height). HTML decimal-prefix and
  nonnegative-integer grammars are distinct, including element-specific zero rules.
- Replaced-element hspace/vspace, border and iframe frameborder; legacy float
  and vertical alignment mappings that have direct CSS equivalents.
- Table/group/row/column/cell dimensions; cellspacing, nearest-table cellpadding;
  align/valign/nowrap; table border/frame/rules and cell/group border hints,
  with logical sides resolved against computed writing mode.
- Legacy body/table/group/row/cell/marquee background colors, font/hr colors,
  table bordercolor, body text and unvisited link colors. HTML legacy color
  recovery is separate from CSS color parsing, with all named-color keywords.
- Body local margin attributes; font face/size; list type/value/start/reversed;
  block text alignment, br clear, pre wrap, textarea wrap=off; hr size/align;
  marquee width/height and spacing.
- Background attributes serialize as a single escaped document-relative URL,
  never as a stylesheet-relative URL. Actual resource URL resolution remains
  the embedding loader's responsibility.
- Initial/inherited metadata added for aspect-ratio, border-spacing and list
  counter properties so `all`, CSS-wide keywords and inheritance cover hints.

Known context/consumer boundaries, routed in questions.md (not claimed complete):
CSS receives no document URL/encoding/quirks mode, visited or active link state,
selected picture source, or containing frame. Exact background URL serialization,
quirks cell nowrap exception, selected-source image dimensions, body vlink/alink,
and frame margin fallback need those inputs. HTML descendant block alignment,
replaced align=middle's baseline geometry and anonymous color-well backgrounds
need layout/paint support rather than invented host-property mappings. Counter
and aspect-ratio computed values do not by themselves claim downstream support.
Foreign namespaces receive none of these HTML hints. The existing simple background shorthand forms (none, color, CSS-wide and
var-substituted equivalents) now reset omitted longhands so an author shorthand
can override a hinted image. Full layered/image/position/size shorthand and
stylesheet URL provenance remain deferred.

References: [HTML rendering](https://html.spec.whatwg.org/multipage/rendering.html),
[HTML microsyntaxes](https://html.spec.whatwg.org/multipage/common-microsyntaxes.html),
[hint origin and rollback](https://drafts.csswg.org/css-cascade-5/#preshint).

Historical pre-outage baseline bb79669 and lock E4D68B10 are superseded by
the fixed-base 2021707 gate below. Do not present those older results as current.


### Integration boundary discovered by the CLI proof

At final-gate base `11dd8a8`, `crates/amazing/src/main.rs::load_page_media`
still turns integer width/height attributes into `PageImage.used_size`, and
`PageMedia::intrinsic_size` returns that value. The integrator-assist correction
referenced in the assignment is not yet present in this fetched dev tree.
Consequently an integer-only before/after fixture is masked by the old adapter.
The CSS proof uses width=80% with height=32 at an 80x50 viewport and a decoded
2x2 red P6 image: **1024 differing pixels before, 0 after**, versus a CSS 64x32
reference. A low author layer overriding integer attributes to 24x12 is **0**
different. Both full CLI proof runs have identical SHA256 results.

A separate diagnostic deliberately sets author width:auto;height:auto on an
image with width=64,height=32. CSS computes auto, but the current driver reports
64x32 as intrinsic dimensions: **2044 differing pixels before and after** versus
the true 2x2 reference. This is routed to integrator-assist and is explicitly
not claimed fixed. No driver/provider workaround was added to CSS.


### CSS-5 day-end checkpoint — 2026-09-21

Stopped by user instruction during the fleet SSH outage. Source commits are
`a3953bc` and `736d4e7`; no new ticket or remote publication started. Recorded
**pre-outage** Windows results: CSS 168/0 including explicit cellspacing;
workspace nextest 1241/0/3 skipped; explicit doctests 2/0. These were not rerun
during stand-down and are not evidence for a later rebased/refreshed tree.

The saved bundle contains 36 completed pinned rendering reports (nine selections,
two runs on each side); both repetitions have stable per-case/image hashes.
The original eight selections are unchanged; cascade remains 35 pass / 24
mismatch / 13 unsupported, including Variables 12/12. Raw pinned cellspacing
improves 478 -> 371 differing pixels and remains a mismatch. Layout needs the
`table cellspacing` nonnegative-integer -> `border-spacing` Hint-origin mapping,
including zero, combined with its display:contents work. CLI percentage-size
proof remains 1024 -> 0; the driver author-auto diagnostic remains 2044 -> 2044.

Final remote process and post-render source/executable integrity checks were
not obtained before SSH became unavailable. FIRST resume action: read the
main checkout `.agents/css/report.md` and wait for the integrator recovery
broadcast before any SSH/SCP; then verify preserved artifacts and source
identity before deciding which gates need rerunning. Full background shorthand,
URL provenance, driver intrinsic-size integration, and context-dependent HTML
mappings remain queued. No CSS-owned REFRESH-HOLD was present at stand-down.

## CSS-6 — grid grammar and typed values (queued after CSS-5 lands)

Do not start in parallel with CSS-5. Layout owns formatting algorithms; CSS owns
only specified grammar acceptance and typed computed values. Layout measured
fixed-base `bebca0f` cases against exact-revision Chrome 154 and Safari 26.6.

1. **Subgrid first:** `valid_grid_track_list` rejects
   `grid-template-columns: subgrid` and `grid-template-rows: subgrid`, including
   optional line-name lists, before layout can see them. Nineteen broad cases
   are blocked; **8 have Chrome/Safari pass consensus**. Accept only the
   specified grammar and preserve a typed subgrid value for layout.
2. **Grid Lanes second:** `values.rs` rejects `display: grid-lanes` and
   `display: inline grid-lanes`, leaving layout with the initial inline value.
   Of 61 failing cases, **17 have Chrome/Safari pass consensus** and 41 are
   Chrome-only. The 17 consensus cases make this the largest consensus cluster,
   so its grammar is worth implementing after subgrid. Grid Lanes is draft CSS
   Grid Level 3 (formerly masonry). Accept only the specified grammar and keep
   the computed display value typed.

These are layout-lane measurements, not CSS-6 results. CSS-5 must land and
stop at its ticket boundary before CSS-6 begins.

### CSS-6 layout adoption (implemented on `lane/css`)

The public `ComputedStyle::typed("grid-template-columns")` and `typed("grid-template-rows")`
return `Some(ComputedValue::Subgrid(Subgrid { line_names }))` for a valid
`subgrid` axis. `line_names: Vec<SubgridLineNames>` is source ordered:
`Names(Vec<String>)` holds one bracket group (including empty `[]`), while
`Repeat { count: SubgridRepeatCount, names: Vec<Vec<String>> }` holds the
bracket groups inside one name-only `repeat()`. The count is `Integer(u32)` or
`AutoFill`. Layout resolves `AutoFill` against the used subgrid span; CSS
does not expand it. `ComputedStyle::get(...)` also retains the computed CSS
text. A standalone track list remains `ComputedValue::Unparsed` pending a
separate typed track-size model.

`ComputedStyle::typed("display")` returns
`Some(ComputedValue::Display(Display::GridLanes))` for `grid-lanes` and
`Some(ComputedValue::Display(Display::InlineGridLanes))` for both
`inline grid-lanes` and the specified `inline-grid-lanes` alias. This follows
the [21 January 2026 Grid Level 3 Working Draft](https://www.w3.org/TR/2026/WD-css-grid-3-20260121/#valdef-display-grid-lanes).
That draft reuses `grid-template-*` and `grid-auto-*` for track specification;
the orientation property in §2.3 remains TBD, so CSS-6 adds no guessed
orientation or new track property. Its `flow-tolerance` property governs
placement precision and is outside this grammar ticket.

The subgrid grammar follows [Grid Level 2 §7.2 and §7.2.3.1](https://www.w3.org/TR/css-grid-2/#track-sizing), including the one-`auto-fill`
restriction. Formatting, parent-track adoption, grid-lanes placement, and
used-value resolution remain layout work.

CSS-6 fixed-base evidence on `9b52dfb` (WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`): the layout lane's 200-case
broad sample contains 19 failing subgrid signatures (8 Chrome/Safari consensus)
and 61 failing Grid Lanes signatures (17 consensus). Extracting the authored
grid/display declarations and running each through public CSS computation on
win-001 twice gives typed reach **0/19 → 12/19** for subgrid (consensus **0/8
→ 4/8**) and **0/61 → 61/61** for Grid Lanes (consensus **0/17 → 17/17**).
Seven subgrid cases use the separate unsupported `grid` shorthand, which this
longhand grammar ticket does not expand. The reach probe counts CSS values
available to layout; it is not a rendering pass claim.

CSS nextest: 171 passed, 0 skipped; CSS doctests: 0. Workspace nextest:
1,360 passed, 3 skipped; workspace doctests: 2 passed. Nine pinned downstream
selections (including cascade **38/32/2** and raw cellspacing **1/0/0**) were
rendered twice on both fixed-base executables. All 36 reports have identical
per-case classifications, differing pixels, and actual/reference/diff hashes
within and across baseline/candidate. Thus the expected CSS-only render delta
is zero. Full evidence is in the CSS lane's `.agents/css/css6-summary.json`,
`css6-reach.tsv`, and `report.md` in the main checkout.


### CSS-5 final fixed-base gate — 2026-09-23

**Gate base:** `origin/dev` at `2021707` (IA-XML driver dispatch merge),
rebased lane branch `lane/css`. The CSS Windows tree was refreshed to that
commit with `tar -m` and first-party Cargo artifact invalidation. The five
CSS-owned changed files were then re-synced with `tar -m`. The refreshed
baseline and candidate were built from **238** and **240** verified tracked
Rust/manifest/selected-render inputs, respectively. Candidate inputs matched
again after the full workspace, all rendering, and the CLI repetitions.

Fresh Windows results (no Mac build or test):

- `cargo nextest run -p amazing-css --locked`: **168 passed / 0 failed /
  0 skipped**, including the explicit cellspacing origin/rollback regression.
- `cargo nextest run --workspace --locked`: **1,329 passed / 0 failed /
  3 skipped**.
- `cargo test --workspace --locked --doc`: **2 passed / 0 failed /
  0 ignored** across 19 targets. Nextest does not include these doctests.
- Two baseline and two candidate runs of each of nine pinned selections:
  **36 reports**, WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
  Each before and candidate repeat agrees exactly on classification, pixel
  count, and actual/reference/diff image SHA256. Every original eight
  selection has identical before/candidate per-case image hashes.

Counts below are pass / mismatch / unsupported; errors, timeouts, and crashes
are zero in every run.

| Selection | Baseline | Candidate |
|---|---:|---:|
| cascade | 38/32/2 | 38/32/2 |
| Variables subset | 12/0/0 | 12/0/0 |
| block-collapse | 9/0/0 | 9/0/0 |
| position | 12/0/0 | 12/0/0 |
| text-flow | 14/2/0 | 14/2/0 |
| flex-grid | 24/0/0 | 24/0/0 |
| paint-compositing | 24/0/0 | 24/0/0 |
| text-shaping | 12/0/0 | 12/0/0 |
| vertical-ruby | 11/9/0 | 11/9/0 |
| raw cellspacing | 0/1/0 | **1/0/0** |

Cascade differs from the pre-outage 35/24/13 because the current base's XML
driver dispatch classifies former unsupported XHTML cases; the baseline and
candidate both read 38/32/2. The cascade per-case normalized SHA256 is
`2aa504f219f1fbbd68a0e55622d12cd22633b1bdc92b8266d3434c8bb91e60a1`
for all four runs. The pinned unmodified
`css-display__display-contents-td-001.html` raw case improves **399
differing pixels / mismatch -> 0 / pass**, stable in both repeats. This is
observed with the base's integrated layout; CSS supplies the HTML
`cellspacing` -> computed `border-spacing` Hint-origin mapping, including 0.

Real CLI proof, repeated twice at 80×50, uses a decoded 32×32 red P6 image,
CSS `height:32px`, and HTML `width=80%` versus a CSS 64×32 reference.
Baseline: **1,024 differing pixels**; candidate: **0**. An author-layer
definite-size override and author `width:auto;height:auto` diagnostics are
0 before/after with the corrected driver. The repeated proof JSON files are
identical, SHA256
`7b3b28cdfaca38cbb2f6a614b8d20dafa924124eb37a7f331cef47fbdd3a9a86`.
The corrected driver now supplies the image's natural dimensions; the prior
2044-pixel author-auto diagnostic was historical and is no longer a defect.

Baseline executable SHA256:
`2735149475AB6D67D3C09AC48932C86693F9C442CFD9D8335189A2581FD91337`.
Candidate executable SHA256:
`0CE5458B21F4E021D1D942E6718F121B909C83D533C895B86F2E764C8198142C`.
Both immutable hashes were verified after rendering. The full evidence,
per-case hashes, source manifests, logs, and scripts live in the primary
checkout's `.agents/css/css5-r3-*`; Windows artifacts are in
`C:/Users/server/target/css/`. The shared-slot wrapper printed an acquired
slot for each full workspace/render gate.

Two resumed intermediate runs are excluded from final claims: first, a
474251e run predated later merged input changes; second, its candidate CSS
was extracted without `tar -m`, leaving Cargo to reuse stale downstream
artifacts despite byte-correct source. That run had CLI 2044→2044 and
cellspacing 478→478 and correctly failed the CLI assertion. The final
2021707 run refreshed and invalidated first-party artifacts, compiled CSS,
layout, and driver together, and passed the end-to-end proof.

Under the fleet's fixed-base rule, later HTML-only merges do not require
CSS-5 to chase `dev`: `git diff --name-only 2021707 origin/dev --
crates/amazing-css` was empty at reporting. The integrator's authoritative
merged-head gate validates the combined tree. Context-dependent mappings
listed above remain separate work; CSS-6 is queued below.

## CSS-13 generated content (2026-09-24)

The selector parser now accepts the Selectors 4 pseudo-elements `::before`,
`::after`, `::marker`, `::placeholder`, `::selection`, `::first-line`,
`::first-letter`, `::backdrop`, and `::file-selector-button`, including the four
legacy single-colon forms. Pseudo selectors match an `(element, pseudo)` target
and add type specificity. `StyledDom::get_pseudo` returns the computed style
inheriting from its originating element, while `generated_content` returns
resolved text/image runs and optional alternative text for layout. Before and
after are omitted when `content` does not create a box. Marker is limited to
list items that create a marker box. First-line and first-letter styles are
computed only for elements whose pseudo selector matches. `pseudo_matched`
exposes that selector result separately from box creation, so layout can
propagate first formatted line styling without splitting unrelated text.

The `content`, `quotes`, `counter-reset`, `counter-increment`, and `counter-set`
properties now have parsed grammars. Evaluation walks document order, handles
reset/increment/set and reversed/list-item counters, tracks quote depth, and
resolves strings, `attr()` (including fallback), `counter()`/`counters()`,
quote keywords, and `url()` images. Layout adoption is LAYOUT-13; CSS-owned
unit tests prove the style/output seam. `css13-manifest.json` freezes 78
script-free local-reference cases from WPT `css-pseudo/` and `css-content/`
at revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
Each stylesheet can now carry its final fetched URL (or the document URL for
inline CSS) in `Stylesheet::base_url`. The winning declaration's URL follows
`content` and inherited `list-style-image` through the cascade. Generated
image runs resolve relative URLs against that URL; a default `::marker` uses
`list-style-image` before falling back to `list-style-type` text. TEXT-6's
driver pipeline now populates `base_url` for fetched and inline stylesheets,
including the final response URL after a redirect.
Counter scope stacks can remain empty after an explicit reset's scope ends.
Increment, set, and counter reads instantiate a new zero-valued counter in
that case; the regression covers `display: contents` descendants and a
reversed list followed by another list item. The pinned 54-case css-lists
subset is recorded in `css13-lists-manifest.json`.
Quoted `list-style-type` values now produce literal marker text after CSS
escape decoding. An omitted `reversed()` reset value uses the later increments
in its scope to derive a descending start; explicit values are preserved.
The subsequent page follow-up expands `list-style` into its inherited type,
image and position longhands, including `none`, image URLs, CSS-wide values and
post-substitution `var()`. The selector parser now accepts `:nth-of-type()` and
`:nth-last-of-type()` so a structural compound immediately before `::marker`
can match its originating list item. CSS crate regressions cover both paths.

## CSS-14 animation and transition parsing (2026-09-24)

The CSS crate retains `@keyframes` across ordinary stylesheets and supported
`@layer`/`@scope`/`@supports` groups. `keyframes_rules(&[Stylesheet])` returns
last-named definitions with per-frame offsets, timing functions, and validated
non-important declarations for the compositor. Animation and transition
longhands have initial values and canonical comma-list computed values;
shorthands reset omitted components and defer `var()` until substitution.
`crates/amazing-css/evidence/css14-parsing-manifest.json` freezes 51 upstream
parsing paths at WPT revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
The Rust parser tests use selected cases from that set; upstream CSSOM
animation runtime conformance remains a downstream compositor/DOM gate.

## CSS-14 follow-up — corpus consensus and image markers (base `c167fb3`)

Commit `53e37b5` sends `list-style-image` gradient and other CSS `<image>`
functions through `ContentRun::Image`; `url()` still resolves against the
stylesheet URL. The `list-style` shorthand accepts the same image functions.
A public-API regression checks direct and shorthand gradient markers and an
invalid image fallback. Paint rasterization of generated gradients is RC-18.

The pinned WPT broad selection (`css12-broad-manifest.json`, 259 cases,
revision `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`) moved from
**188 pass / 71 mismatch** to **198 pass / 61 mismatch**, with ten
Chrome/Safari pass-consensus gains and zero losses, errors, timeouts or crashes.
The gains cover `:required`, `:open`, no-namespace `[|attr]`, four `nth-child`
`of` cases, comment-separated `rgb()`/`hsl()` channels, and the `InfoText`
system-color alias. The 72-case cascade selection stayed **68/2/0/0/2**
(pass/mismatch/error/timeout/unsupported), with identical per-case classes.
Its two mismatches still need sampled animation of important declarations and
private-use font matching, respectively. Twenty-seven broad consensus
mismatches remain; the color-function, layout, text and dynamic-DOM clusters
need separate ownership and gates.

Windows final gate on `53e37b5`: CSS **239/239**, workspace **2000/2000**
plus five skipped, doctests **2/2**. The base and final executable SHA256 values
were `2fc0125276088103c127c86f2e4f3b6733d59e3b0f17fb538141e6a184a058a0`
and `ad959094b81da24a6fb9e05407b2cbb5d44d86da10e9a1737c4278a15b78b559`.

## CSS Values 4 font metric unit handoff (LAYOUT-14 blocker)

Commit `50d7b41` registers `ic`, `cap`, `lh`, `rch`, `ric`, `rex`, `rcap`, and
`rlh` as typed relative lengths. The CSS math type validator now accepts them
inside `calc()`, and all length-bearing computed properties retain valid math
expressions as `LengthPercentageExpression` until layout supplies used-value
font metrics. Public API tests verify `get()` and typed values for direct and
math forms. Linux: CSS 241/241, workspace 2000/2000 (+5 skipped), doctests
2/2. Windows: CSS 241/241; an unrelated `amazing-process` renderer-crash
case failed once with `Remote("UnknownInterface")`, then passed in isolation;
the full rerun passed 2002/2002 (+5 skipped) and 2/2 doctests. The LAYOUT-14
used-value and WPT gates remain with layout.

## CSS Fonts 4 longhand handoff (TEXT-12 and LAYOUT-14)

Commit `f558b9e` registers inherited `font-kerning`, the requested
`font-variant-*`, `font-synthesis-*`, optical sizing, palette, size adjustment,
language override and variation settings longhands. It validates their grammars
before cascade, exposes canonical computed values, expands `font-synthesis`,
recognizes them in `@supports`, and applies CSS Fonts 4 `font` resets while
preserving `font-palette` and `font-synthesis-*`. Linux CSS 244/244; Windows CSS
244/244, workspace 2005/2005 (+5 skipped), doctests passed. The Values 4
font-relative units in `50d7b41` remain available to layout.

## CSS-15 style invalidation (ADR 0010 step 2)

Commit `be7c0dd` adds `StyleInvalidator`, `RenderMutation`, `RecalcRoots`,
`recompute_roots` and `StyleDelta`. Its stylesheet-bound feature index tracks
class, id, attribute and dynamic state dependencies for element, descendant
and following sibling invalidation. Attribute old/new values seed the same
coalesced work; tree, shadow-slot, text, relational selector, recycled node and
stylesheet epoch changes take a full-recompute fallback. Incremental work
reuses retained styles outside affected roots, resolves generated content,
and offers `compare_to_full` with the first differing node/property. The
synthetic mutation tests include inheritance, siblings, old/new tokens,
pseudo content, root font sizing and conservative fallbacks. DOM-21 will
supply the journal and generation mapping; the driver has not adopted this
opt-in seam yet. Linux CSS 250/250; Windows CSS and workspace nextest plus
doctests passed on the exact source commit.

## CSS-16 animation origin (IA-ANIMATIONS handoff)

Commit `3a8351c` exposes `compute_with_animations(dom, sheets, &AnimatedValues)`.
`AnimatedValues` holds validated, typed longhand samples per node and retains
animation and transition values independently. Animation samples override
normal author declarations, including inline styles, while important author
values override animations. Transitions override all other origins. Author
`revert` can expose an animation sample. `ComputedStyle::get()` and `typed()`
show the winning value; `StyledDom::source()` reports its origin. The compositor
or driver must serialize its `AnimationValue` samples into the CSS insertion
API and route main-thread property updates at the rendering-update boundary.

## CSS-17 color consensus cluster

Commit `1e99299` adds two-color `color-mix()` in sRGB, linear sRGB and LCH,
`contrast-color()` choosing black or white, relative HSL identity from
`currentColor`, and the `InactiveBorder` system alias. A mix using
`currentColor` resolves against the element's computed color; `color` uses its
parent's color, while an inherited background mix retains its source and
resolves again on the child. The selected WPT `color-mix-currentcolor-002` and
`-003` cases verify this inheritance path. Other color spaces and general
relative-color expressions remain separate work.

The pinned 259-case broad selection moved from **198 pass / 61 mismatch** to
**202 pass / 57 mismatch**, with four Chrome/Safari consensus gains and zero
pass losses. The 72-case cascade selection remained **68 pass / 2 mismatch /
2 unsupported**, with no classification changes. Both use WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; per-case hashes and
classifications are in `crates/amazing-css/evidence/css17-*-case-results.jsonl`
and the summary in `css17-color-results.json`. Windows exact-source gate:
CSS **266/266**, workspace **2027/2027** plus five skipped, doctests **2/2**.

## CSS-18 selector serialization and complex logical lists

Commit `3172976` re-escapes decoded identifiers when declaration and selector
tokens are serialized, so `:lang(\*-FR)` retains its wildcard language range
through stylesheet parsing. `:is()`, `:where()`, and `:not()` accept complex
selector-list arguments with combinators. The first two drop invalid list
items and match nothing when none remain. Complex arguments use their full
selector specificity, while `:where()` stays zero. Style invalidation takes
the conservative relational fallback for these complex arguments.

The pinned 259-case broad WPT selection moved from **202 pass / 57 mismatch**
to **204 pass / 55 mismatch**, gaining `is-where-pseudo-elements` and
`lang-015` with zero losses. Both are Chrome/Safari pass-consensus cases.
The 72-case cascade selection remained **68 pass / 2 mismatch / 2
unsupported** with identical classifications. Per-case classifications and
pixel hashes are in `crates/amazing-css/evidence/css18-*-case-results.jsonl`;
`css18-selector-results.json` records counts and report hashes. Linux CSS
**268/268** and workspace **2027/2027** plus five skipped passed.
The exact-source Windows gate passed CSS **268/268**, workspace **2029/2029**
plus five skipped, and doctests **2/2**.

## CSS Ruby 1 property handoff for LAYOUT-16

The ruby display values were already accepted by the typed `display` parser and the UA sheet already mapped `ruby`, `rb`, `rt`, `rtc`, and `rp`. Add `rbc` mapping and complete the UA defaults (annotation size only once within `rtc`, nonwrapping bases/annotations, ruby font variant, and isolation). Register the inherited `ruby-align`, `ruby-position`, `ruby-overhang`, and `ruby-merge` properties with their CSS Ruby 1 initials and value grammars. The `ruby-position` unordered two-keyword form canonicalizes to `alternate over/under`; legacy `ruby-overhang: none` computes to `spaces`. See [CSS Ruby 1](https://drafts.csswg.org/css-ruby-1/).

Linux CSS nextest: 270/270. Linux workspace nextest: 2029/2029 plus five skipped. Windows exact-source gate and doctests are recorded in `.agents/css/report.md`.

## CSS-13 style-change severity and compositor handoff (ADR 0010)

`StyleChangeKind` orders `None`, `CompositorOnly`, `PaintOnly`, `Layout`, and `Rebuild`. The explicit table covers all 193 registered longhands; its registry test fails when a new property lacks a classification. `StyleDelta::change_kind(node)` reports the most severe computed change from incremental recalculation. Changes to pseudo boxes or unknown computed state require rebuild. The normal recomputation path escalates transform changes to layout and opacity changes to rebuild because it cannot prove a retained property-tree-only update.

`StyleInvalidator::update_attribute` handles a single class/style mutation and returns `AttributeStyleUpdate`. Its `CompositorOnly` variant is a value-only path: it parses only the old/new inline declarations and existing cached style, never calls CSS `compute`, and carries `opacity: Option<f32>` plus a final row-major local `transform_matrix`. The matrix includes the default 50%/50% transform origin; `Matrix4(matrix)` is the compositor conversion, with no second origin application. `update_attribute_with_box` accepts `CachedBoxSize` from unchanged retained layout for percent translations and rotation/scale origin composition. `CompositorStyleDelta::apply_to_styles` patches the retained computed style for repeated updates. The fast path rejects selector dependencies on `style`, `:has`, `attr()` dependencies, invalid or important declarations, non-inline winners, a transform that appears/disappears, child elements under a changed transform, unsupported relative lengths, explicit transform-origin, and opacity crossing 1. All rejected cases use incremental recomputation and a conservative severity.
## CSS-13b class-edit compositor classification (ADR 0010)

- `StyleInvalidator::update_attribute_with_box` compares old/new class selector matches without CSS compute, rejects descendant and sibling invalidation sets, relational and pseudo dependencies, and non-compositor declaration changes.
- Normal unlayered opacity and transform winners produce the existing resolved `CompositorStyleDelta`; retained style source, stable effect node, and cached transform geometry remain required. Layered or important competitors recompute.
- `css13b_class_delta.rs` covers the IA opacity class fixture, unchanged shared declarations, cached transform geometry, combinators, `:has()`, inherited declarations, pseudo content, class attributes, and layered important competition.
- The follow-up admits fixed implicit `<style>` scopes and stable explicit `@scope` roots. It evaluates scope start/end limits before comparing the old/new subject selector match and rejects scopes whose selectors depend on the edited class.


## CSS-14 compositor animation handoff (COMP-12)

`animation::CompositorAnimation` is the CSS-owned handoff: node/generation, property, resolved keyframes, timing, start time, and animation identity. `CompositorAnimation::from_keyframes` rejects mixed main-thread properties, missing endpoints, incompatible transform function lists, matrix functions requiring decomposition, and unresolved box geometry; `for_transition` resolves old/new values. `AnimationTiming::from_computed_style` reads the selected animation or transition list item. Transform frames include a row-major local 4x4 matrix plus resolved translation/rotation/scale/skew functions and default transform origin. `animation::sample(&handoff, t_ms)` is pure and is the single sampler for compositor frames and main-thread reads. The compositor constructs its `Matrix4` from the sampled matrix field.

`AnimatedValues::handoff` marks the node/property compositor-owned and suppresses per-frame insertion. `sample_owned` and `read_owned_style` call the same sampler on demand. `commit_property_update` validates id/generation and completion, commits forward fill at the animation/transition cascade origin once, then calls a main-thread event callback for `animationend` or `transitionend`; a two-property animation emits one end event after both updates. `cancel_for_mutation` cancels all owned properties for that node/generation, returns the handoffs for main-thread resumption, retains the last sampled value, and emits cancel events. Driver integration must invoke these APIs at the rendering-update boundary; no compositor dependency enters amazing-css. See [CSS Animations 1](https://drafts.csswg.org/css-animations/) and [Web Animations 1](https://drafts.csswg.org/web-animations-1/) for fill and iteration timing.

## RC-21 CSS handoff: image and media selection

`image-orientation` now computes to inherited `from-image` initially, accepts `none` as the opt-out, and retains legacy angle/flip forms for parsing. `object-fit` and `object-position` are registered non-inherited values for all elements, including replaced contexts. The image loader reads all three through `ComputedStyle::get`; invalid declarations leave the appropriate initial value. `@supports` accepts the new image properties.

`media_queries::MediaEnvironment` supplies viewport size, DPR, media type, and user preferences. `MediaQueryList::parse` with `evaluate` matches screen/print, width/height/resolution, orientation, and common `prefers-*` features. `parse_sizes_attribute` selects the first matching source size, resolves viewport/absolute/font-relative lengths and common CSS math, and accepts an optional auto width; invalid entries fall through to 100vw.

## CSS-15b — predefined counter styles and focus pseudo-classes (2026-09-26)

The CSS crate now implements the full normative predefined counter-style set from
CSS Counter Styles 3 (§6 simple and §7 complex). Per-style suffixes are honored
(`、` for `cjk-decimal`, the fixed CJK styles, hiragana/katakana/iroha and the
Japanese/Chinese longhands; `", "` for the Korean longhands; `". "` otherwise).
`cjk-earthly-branch` and `cjk-heavenly-stem` are fixed, not cyclic, with a
`cjk-decimal` fallback. Added the simple additive styles (`armenian`,
`upper-armenian`, `lower-armenian`, `georgian`, `hebrew`), the limited-range CJK
longhand additive styles (`japanese-informal`, `japanese-formal`,
`korean-hangul-formal`, `korean-hanja-informal`, `korean-hanja-formal`), and the
Chinese limited-range specialized algorithm (`simp-chinese-informal/-formal`,
`trad-chinese-informal/-formal`, `cjk-ideographic`). Custom `@counter-style`
rules now support `system: additive`. Out-of-range values fall back per style
(`cjk-decimal` or `decimal`).

Selectors 4 `:focus`, `:focus-visible`, and `:focus-within` now match from the
DOM's programmatic focus record (`Dom::is_focused`, `is_focus_visible`,
`is_focus_within`). `:focus-visible` follows the keyboard-visibility flag with
the text-control exception (text-ish `input`, `textarea`, `contenteditable`).
Added `Pseudo::FocusVisible` parse/match/specificity and the `focus-visible`
invalidation token. This depends on the dom-bindings focus API (`9a79112`,
merged to dev as `8cb9e5cb`/`9d2e5c27`); the selector code had previously been
cherry-picked into this branch by mistake and is now committed lane-clean.

Linux (venus-001) `cargo nextest run -p amazing-css --locked`: **310/310**,
CSS doctests 0. Commits `1d4b4d11` (counter styles) and `13e1d722` (focus).

**Render result (counter-styles selection).** `css/css-counter-styles` was
restored at the pinned revision via a blob-filtered sparse checkout on the Mac
and transferred to win-001. Running `run-render-wpt.ps1` with the 80-case
`crates/amazing-css/evidence/css15b-counter-manifest.json` (a counter-styles-only
subset of the css15b manifest) against the candidate executable gives
**73 pass / 7 mismatch / 0 error / 0 timeout / 0 unsupported / 0 crash**, up from
the committed baseline's **4 pass / 76 mismatch**: 69 gains, zero regressions.
Evidence: `crates/amazing-css/evidence/css15b-counter-after.json`. The 7
remaining are 3 Hebrew cases (RTL/bidi and `counter-hebrew-nested`), 1
`korean-hanja-informal` case, and 3 Chinese `counter-*` `counter()` cases
(not `::marker`), which are followed up separately.

The full 219-case css15b corpus (css-pseudo/counter-styles/conditional) still
needs `css/css-conditional` restored at the manifest revision for a comparable
all-category after-run; the counter-styles directory and selection above are the
part this ticket owns and has now measured.

## Next CSS-15 bulk css/ WPT sweep

Pinned WPT revision `53bc812da2ed81179ebd1ba76b2d197cc4747d12` has the largest document counts in `css-grid` (3410), `css-flexbox` (1974), `css-backgrounds` (1355), `css-transforms` (1245), `css-images` (751), `selectors` (684), `css-values` (603), `css-color` (517), `css-variables` (268), and `css-cascade` (158). Grid/flexbox are layout-owned and backgrounds/images paint-owned. The selected CSS-owned directories are `selectors`, `css-values`, and `css-color`: 80 path-stratified static reftests from each, in `crates/amazing-css/evidence/css15-manifest.json`.

The fixed-base baseline is **190 pass / 50 mismatch / 0 error / 0 unsupported**. Directory splits are selectors **74/6**, values **43/37**, and color **73/7**. CSS fixes cover `an+b` whitespace sign tokenization in nested `:nth-last-child`, typed `attr()` substitution against the originating element with fallbacks and case-insensitive HTML attribute names, deprecated system-color aliases, Lab/OKLab lightness endpoints, and root font-size math using the initial `em`/`rem` basis. Layout-owned sizing, relative offsets, viewport scrollbar units, and first-line/first-letter cases were routed to `.agents/layout/questions.md` with the pinned IDs. Final after counts and exact-source gates are recorded in `.agents/css/report.md`.

## Viewport computed values and resize invalidation

`compute_with_environment(dom, sheets, &MediaEnvironment)` is the driver entry point. `compute()` supplies a default 800×600 CSS-pixel environment for existing callers. `vw`/`vh`/`vmin`/`vmax` and their `sv`/`lv`/`dv` variants become computed px before inheritance; the small, large, and dynamic sizes default to the layout viewport when absent. This resolves root `font-size: 100vw` before child `rem` values. `StyleInvalidator::new_with_environment` captures the prior environment; `collect_viewport_resize` selects nodes with matching viewport-dependent rules or viewport units in inline styles, and `recompute_roots` uses the new environment for those subtrees. The driver creates a fresh invalidator with the new environment after applying the delta. Container units remain future work.

## COMP-15 — filter, blend, mask, and clip-path (2026-09-26)

CSS now registers `filter`, `backdrop-filter`, `mix-blend-mode`, `isolation`,
`mask-image`, `mask-mode`, `mask-repeat`, `mask-position`, `mask-clip`,
`mask-origin`, `mask-size`, `mask-composite`, and `mask-type`, and parses
`clip-path` shapes. `crates/amazing-css/src/effects.rs` holds the typed values:

- `FilterList` resolves a `<filter-value-list>` to functions
  (blur/brightness/contrast/drop-shadow/grayscale/hue-rotate/invert/opacity/
  saturate/sepia/url). Number/percentage arguments are clamped to the property
  range; `calc()` arguments are retained as `FilterFunction::Expression` text for
  the compositor to fold. `drop-shadow` offsets/blur are lengths, not
  percentages.
- `BlendMode` covers the full Compositing and Blending 2 keyword set including
  `plus-lighter`; `isolation` is a bool (`isolate` true).
- `clip-path` parses `none`, `url()`, `<geometry-box>`,
  `inset()`/`circle()`/`ellipse()`/`polygon()` (with optional `round <length>`
  and fill-rule) and `path()` (with optional fill-rule), in either
  `<shape> <box>` order.
- `mask-image` is a comma list of `none`/`url()`/`<image>`; `mask-mode`,
  `mask-type`, `mask-composite`, `mask-clip`/`mask-origin` are typed keyword
  enums; `mask-repeat`/`mask-position`/`mask-size` are grammar-validated.
- `clip-path` now routes through the effects parser instead of the SVG
  presentation path (which only accepted `none`/`url()`); SVG consumers only
  require a non-`Unparsed` typed value.

Effect-only longhands classify as `StyleChangeKind::CompositorOnly` and
`CompositorStyleDelta` carries them as `effects: Vec<(property, css)>`
value-only updates through the CSS-13 delta path (`update_attribute`,
`apply_to_styles`), so the compositor can apply filter/blend/mask/clip changes
without a CSS recompute.

Boundaries (not claimed): `xywh()` and `rect()` shapes (CSS Shapes 2) and
`ray()` are unimplemented; `polygon(round ...)` is validated but not retained
typed; GPU rasterization of filters/backdrop-filters/blend/masks and the
compositor adoption of the `effects` field are the compositor lane's work. The
engine's computed serialization places no space after commas in shape functions,
which differs from the WPT canonical form but is the existing serializer policy.

Evidence: Linux (Boat) CSS nextest **322/322**; workspace nextest **2489/2489**
(10 skipped). WPT parsing selections pinned at revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` in
`crates/amazing-css/evidence/css19-effects-manifest.json`; the valid/invalid
declaration sets are encoded in `effects.rs` unit tests and
`tests/css19_effects.rs`, because the DOM CSSOM/`getComputedStyle` path those
harness files need is not implemented. Linux final gate (Boat, lane head
`4c331292`): CSS crate **322/322**, workspace nextest **2539/2539** (10
skipped), workspace doctests **2/2**. WPT render sweeps are deferred under the
Linux-first rule (2026-09-26).

## CSS-15c — i64 counter values and the extended CJK longhand algorithm

Commit `6c90b5e5` (on the lane head `c8bb65e7` after the dev merge). Counter
values (`counter-reset`, `counter-increment`, `counter-set`, a `reversed()`
initial value, and the `counters()` history) are now `i64`, so author-specified
values beyond 32 bits survive to the marker instead of clamping. The Chinese
longhand styles (`simp-chinese-informal`/`-formal`, `trad-chinese-informal`/
`-formal`, and the deprecated `cjk-ideographic`) use the CSS Counter Styles 3
[§7.1.2 extended algorithm](https://drafts.csswg.org/css-counter-styles-3/#extended-range-optional):
four-digit groups with `万`/`亿`/`万亿` (simplified) or `萬`/`億`/`兆`
(traditional) markers, a single `零` collapsing runs of zero groups, and the
informal tens-digit drop, defined over |value| < 10^16. The `japanese-*` and
`korean-*` longhands stay the §7.1.1 limited-range (-9999..=9999) forms and keep
their spec `fallback: cjk-decimal`. Values at or beyond 10^16 fall back per the
counter style (`cjk-decimal` for the CJK longhands, `decimal` otherwise). The
`counter-trad-chinese-informal-ref.html` reference values through 10^15 and the
out-of-range fallback are unit-tested in `counter_styles.rs`.

This is the CSS-owned part of the seven remaining counter-styles render
mismatches: the three Chinese `counter()` cases used values past the old
limited range. Linux final gate (Boat, lane head `4c331292`): CSS crate
**322/322**, workspace nextest **2539/2539** (10 skipped), workspace doctests
**2/2**. The counter-styles WPT render after-run uses the Windows-only
PowerShell runner, so it is deferred under the Linux-first rule; the formatting
itself is proven by the spec-reference unit tests above.

## CSS-20 — CSS Values 5 calc-size() preservation (LAYOUT-18)

Commit `9bb410cd`. CSS accepts `calc-size( <calc-size-basis>, <calc-sum> )`
([CSS Values 5 §11](https://drafts.csswg.org/css-values-5/#calc-size)) on the
size longhands: `width`, `height`, `min-width`/`min-height`,
`max-width`/`max-height`, `flex-basis`, and the logical `inline-size`/
`block-size` family (mapped to the physical longhands before validation). The
basis is a sizing keyword (`auto | min-content | max-content | fit-content |
stretch | contain`), the `fit-content()` function variation, a `<calc-sum>`, a
nested `calc-size()`, or `any`. The calculation is a `<calc-sum>` in which the
`size` keyword names the basis; `size` is rejected when the basis is `any`.

`crates/amazing-css/src/math.rs::calc_size` validates the grammar and the
expression is retained unmodified as
`ComputedValue::LengthPercentageExpression`, so `ComputedStyle::get` returns the
canonical `calc-size(...)` text and `typed()` exposes the expression without
resolving the intrinsic keyword at computed-value time. Layout owns the
used-value evaluation (its own `calc-size()` parser and intrinsic basis
handling). Malformed arithmetic, trailing tokens, a non-sizing property
context, an unknown basis, and `size` under an `any` basis are rejected before
cascade, so the declaration behaves as invalid rather than manufacturing a
value.

Boundary (not claimed): `calc-size()` nested inside another math function
(`min(calc-size(...), 50px)`) is not accepted; only the top-level function is.
Layout's other LAYOUT-18 seam, typed `attr()` inside math (`width:
max(attr(data-test type(<length>)))`), was already implemented by CSS-15.

Linux final gate (Boat sandbox, lane head `9bb410cd`): CSS crate **323/323**
(adds `values::tests::calc_size_is_preserved_for_layout`), workspace nextest
**2540/2540** (10 skipped), workspace doctests **2/2**, zero failures.

## CSS-21 — CSS Color 5 relative color syntax

Commit `1cf7076c`. `rgb()`/`rgba()`, `hsl()`/`hsla()`, `hwb()`, `lab()`,
`lch()`, `oklab()`, and `oklch()` accept
`fn(from <color> <channel> <channel> <channel> [ / <alpha> ])`. Channel tokens
are the function's keywords (`r g b`, `h s l`, `h w b`, `l a b`, `l c h`),
plain numbers or percentages on the channel's scale, `none`, and `calc()`
arithmetic over keywords and numbers. `alpha` is the alpha keyword; an omitted
alpha keeps the source color's alpha. Sources are any parseable CSS color.
`color_advanced.rs` gains reverse source-channel conversions (HSL, HWB, Lab,
OKLab and the LCH builders) plus a shared `hwb_rgb`; `ExprEval` evaluates
channel `calc()` with keywords bound and handles the tokenizer's bare
sign-ident spellings.

A `currentcolor` source is retained by the computed-value seam:
`ComputedStyle` stores the relative source and re-resolves it against the
element's own `color`, so an explicitly inherited relative color re-binds on
the child. `ComputedValue::Unparsed` preserves `rgb(from currentcolor ...)`
until that seam; `color()` relative keeps the previous identity handling there.

Boundaries (not claimed): `none` channels resolve to zero rather than
propagating a missing component. General `color(from <color> <space> ...)`
per-space channel references are CSS-22 below.

Linux final gate (Boat sandbox `bx_cp5jbn5z`, lane head `fd96655b` containing
`1cf7076c`, clean worktree): CSS crate **325/325**, workspace nextest
**2736/2736** (10 skipped), workspace doctests **4/4**, zero failures.
Tests: `color::tests::relative_color_syntax_resolves_channel_keywords` and
`values::tests::relative_currentcolor_rebinds_on_inheritance`.

## CSS-22 — `color(from <color> <space> ...)` relative forms across spaces

Commit `92a647fe`. Extends CSS-21's relative-color seam to `color()`:
`color_advanced.rs::relative_color_space` converts the source into the
target predefined RGB or XYZ space (`srgb`, `srgb-linear`, `display-p3`,
`display-p3-linear`, `a98-rgb`, `prophoto-rgb`, `rec2020`, `xyz`/`xyz-d65`,
`xyz-d50`) via new reverse conversions (`rgba_to_color_space`), then the
channel keywords (`r g b`, or `x y z` for XYZ) and `calc()` resolve in that
space through the existing `color_space_rgba` builder shared with the
absolute-form parser. A `currentcolor` source resolves through the same
generic textual-substitution seam as the other relative-color functions
(`resolve_currentcolor_relative` in computed.rs is function-name agnostic),
so `color(from currentcolor <space> ...)` now covers every space and
arbitrary channel math, not just the identity `r g b` copy CSS-21 special-
cased (`relative_currentcolor_identity` stays as an unreachable-in-practice
fallback for that one shape).

Bug found and fixed while wiring the reverse path: `XYZ_TO_A98` was a stray
Bradford-like matrix (not `A98_TO_XYZ`'s inverse), so `color(from <color>
a98-rgb r g b)` round-trips diverged by dozens of levels per channel. Fixed
by computing and hardcoding the correct inverse.

Boundary: `unsupported_color_spaces_preserve_the_existing_cascade_winner` in
`tests/typed_compute.rs` now exercises a custom-ident space
(`--custom-space`), since `xyz` moved from unsupported to supported.

Linux final gate (Boat sandbox `bx_cp5jbn5z`, lane head `852a342d`): CSS
crate **326/326**, workspace nextest **2737/2737** (10 skipped), doctests
**4/4**, zero failures. Tests: `color::tests::relative_color_syntax_resolves_channel_keywords`
(extended) and `values::tests::relative_currentcolor_color_space_rebinds_on_inheritance`.

## CSS-23 — `light-dark()`/`color-scheme`, and hover/pointer/aspect-ratio media features

Commit `316c413d`. Prioritized ahead of the previously queued item because
dark-mode CSS and touch/mouse-responsive media queries are what real pages
actually use.

- `color-scheme` (CSS Color Adjustment 1 §2): registered as an inherited
  property, initial `normal`, grammar `normal | [light | dark]{1,2} &&
  only?` validated in `values.rs`.
- `light-dark(<color>, <color>)` (CSS Color 5 §4.1) is accepted on
  `color`/`background-color`/border-`*`-color (the same properties CSS-21's
  relative-color seam covers) when each branch is `currentcolor` or an
  already-parseable `<color>`. `ComputedStyle::resolve_light_dark` runs
  after inheritance and after `resolve_relative_colors`: the used scheme is
  the element's own computed `color-scheme` when it names exactly one of
  `light`/`dark`; otherwise (`normal`, or both listed) it falls back to
  `MediaEnvironment::prefers_color_scheme`. A `currentcolor` branch
  substitutes the element's own resolved `color`, resolving `color` itself
  first so other properties can reference it.
- Invalid `light-dark()`/`color-scheme` values are dropped before cascade
  like any other invalid declaration on a known property
  (`computed::expand_declaration`'s existing early-validity filter) — the
  earlier valid declaration for that property wins outright (its literal
  text; plain named colors are not normalized to hex, only the functional
  seams write hex). This differs from the *guaranteed-invalid-after-`var()`*
  policy (`compute_normal_value`'s later check), which falls back to
  initial/inherited instead — the two existing tests
  (`typed_compute::invalid_supported_color_after_var_defaults_without_reviving_prior_winner`
  and the new `css23_light_dark::invalid_light_dark_does_not_displace_the_cascade_winner`)
  cover both policies.
- Media Queries 4 `hover`/`any-hover`/`pointer`/`any-pointer` (new
  `HoverCapability`/`PointerCapability` enums on `MediaEnvironment`,
  desktop-mouse defaults `hover`/`fine`) and `aspect-ratio`/`min-`/`max-
  aspect-ratio`, evaluated through the existing public
  `evaluate_media_query`/`MediaQueryList` API — the same seam
  `.agents/css/questions.md` already told IA-REAL-PAGES is ready for
  `window.matchMedia`.
- `change_classification.rs` classifies `color-scheme` `PaintOnly`
  alongside `color`/`cursor`/`visibility`.

Boundary (not claimed): `light-dark()` is only recognized as the direct
value of the four color properties above (matching CSS-21's scope), not
inside gradients, `box-shadow`, or SVG `fill`/`stroke`. `color-scheme`'s
`only` restricts to the engine's own choice among the listed schemes but
does not yet feed a UA form-control/scrollbar palette (no such rendering
exists yet). Custom-ident color-scheme names (forward-compat per spec) are
not accepted, only `light`/`dark`.

Linux final gate (Boat sandbox `bx_cp5jbn5z`, lane head `316c413d`): CSS
crate **335/335**, workspace nextest **2746/2746** (10 skipped), doctests
**4/4**, zero failures. WPT CSS reftest sweeps and all render gates remain
deferred under Linux-first (2026-09-26).

## CSS-24 — Color 5 `none`-propagation, color-mix() expansion, calc-size() nesting (base `5b923bf7`)

Commit `2ef1d3b2`. The queued Color 5 work from CSS-23's report, done as one
area: `none`-channel missing-component propagation through absolute colors
and `color-mix()` interpolation, expanded `color-mix()` space/hue-method
coverage, and `calc-size()` nesting.

- **`none` in absolute color functions** (CSS Color 4 §4.4): previously
  supported only in the relative-color forms (`rgb(from ... )` etc, CSS-21).
  Now also accepted directly in `rgb()`/`rgba()` (modern space syntax only —
  the legacy comma syntax predates `none` and does not accept it), `hsl()`,
  `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`, and the predefined
  `color()` function, via shared helpers (`channel_or_none`/`alpha_or_none`
  in `color.rs`; `none_component`/`is_none` plus the existing `hue`/`alpha`
  in `color_advanced.rs`).
- **`color-mix()` missing-component propagation** (CSS Color 4 §13): a
  channel missing (`none`) in exactly one operand now takes the other
  operand's value directly rather than being averaged in as a literal 0.
  Alpha follows the same substitution rule for its own value, but rgb/polar
  channels that are *not* themselves missing still mix by the normal
  premultiplied weighted average.
  - This needed `none_mask` (which detects whether an operand is written
    literally in the mix's own space, so a `none` there means *that*
    channel is missing) to recognize `rgb()`/`rgba()` (as `srgb`) and
    `color(<space> ...)` (as `srgb`/`srgb-linear`/`xyz`/`xyz-d65`/`xyz-d50`,
    with `xyz`/`xyz-d65` treated as aliases) — it previously matched only
    the six polar/lab-family function names (`hsl`/`hwb`/`lab`/`lch`/
    `oklab`/`oklch`) and silently gave every other space's `none` the
    established none-is-0 treatment instead of propagating it.
  - It also needed the premultiplication weights (`w1`/`w2`) to use each
    operand's *effective* alpha — substituted from the other operand's
    alpha when its own is missing — rather than the literal 0 the parser
    stores for a `none` alpha; using the raw 0 zeroed out that operand's
    contribution to every other channel instead of leaving it unaffected
    by the missing alpha specifically.
- **`color-mix()` expanded interpolation spaces**: `lab`, `oklab`, `xyz`,
  `xyz-d65`, `xyz-d50`, `hsl`, `hwb`, `oklch` (previously only `srgb`,
  `srgb-linear`, `lch`), plus the `shorter | longer | increasing |
  decreasing hue` interpolation methods (§12.2, `shorter` default) via a new
  `HueMethod` enum and `mix_hue`/`parse_mix_method`. The achromatic/moot
  test for a polar operand's hue is space-specific: `hwb`'s is
  `white + black >= 100%`, not "chroma channel near zero" — `hwb(_, 0%,
  0%)` is fully chromatic, not achromatic.
- **`calc-size()` nesting** (CSS Values 5 §11): now parses inside
  `min()`/`max()`/`clamp()`, not just at the top level or as its own basis
  argument. `math.rs` extracts `calc_size_body` (the shared
  basis-comma-calc-sum-closeparen logic) out of `calc_size_function`, and
  `atom()`'s generic `Token::Function` branch special-cases `calc-size`
  before the shared `self.expression()` path, since a `<calc-size-basis>`
  can start with a bare keyword (`auto`, `any`, ...) that `expression()`
  cannot parse.
- New test file `tests/css24_color5_gaps.rs` (12 tests): absolute-`none`
  coverage per function family, missing-component propagation (rgb
  channels, alpha, hue), the expanded-space round-trip invariant (mixing a
  color with itself in every newly supported space reproduces it exactly),
  the hsl circular-hue-path check (red/blue mixes through magenta, not a
  desaturated middle), all four hue-interpolation methods, the hwb
  achromatic distinction, and `calc-size()` nesting through the cascade. Two
  channel-level comparisons use a ±1 byte tolerance (`assert_close`) since
  they compare two independently-rounded computation paths (a full
  `color-mix()` against a directly-parsed literal reference), not the same
  code path twice.

Boundary (not claimed): missing-component propagation for `color-mix()`
only recognizes an operand as "written in the mix's own space" when it is a
*literal* same-space color function (not `from`-relative, not a different
space, not a custom property reference) — cross-space operands and any
operand that isn't a direct literal keep the established none-is-0
treatment, matching the scope `none_mask`'s doc comment states. `display-p3`
and the other gamma-encoded predefined spaces are not added to
`color-mix()`'s interpolation-space list (CSS Color 4 doesn't require them
for mixing — only the listed rectangular/polar set — and no page-real-world
priority was found for adding them speculatively).

Linux final gate (Boat sandbox `bx_cp5jbn5z`, lane head `2ef1d3b2`): CSS
crate **335 → 347**, workspace nextest **2906/2906** (10 skipped), doctests
**ok** across all 31 doc-tested crates, zero failures. Sandbox disk hit 0 GB
free mid-gate (other lanes sharing the pool's caches); resolved by purging
this lane's own `debug/incremental` plus the pool-shared
`target/debug/incremental` — both are cargo-rebuildable caches, not
committed work, per the same disk-hygiene principle AGENTS.md states for
win-001. WPT CSS reftest sweeps and all render gates remain deferred under
Linux-first (2026-09-26).

## CSS-25 — Linux WPT parsing/computed-value runner (base `5b923bf7`)

Commit `e58765a8`. WPT on Linux without render sweeps: a runner for the
pinned WPT parse/computed-value testharness selections, driving
`amazing-css` directly (no browser, no JS engine), plus the biggest
failure clusters it surfaced.

- **The runner**: `crates/amazing-css/examples/wpt_conformance.rs`. WPT's
  `css/support/parsing-testcommon.js` and `computed-testcommon.js` give
  every `*-valid.html`/`*-invalid.html`/`*-computed.html` file a small,
  regular shape — top-level calls to `test_valid_value(property, value[,
  serialized])`, `test_invalid_value(property, value)`, and
  `test_computed_value(property, specified, computed)`, almost always with
  plain quoted-string arguments. The runner statically extracts those call
  sites (a small parenthesis/quote-aware scanner, not a JS parser — see its
  own doc comment for the exact grammar it walks) and checks them against:
  - valid/invalid: the new `pub fn values::parses(property, value) ->
    bool` (a thin wrapper over the crate-internal `parse`, exposed
    read-only for this runner; cascade code keeps using `parse` directly).
  - computed: a lone `div { property: specified }` through `compute()`,
    comparing `ComputedStyle::get(property)` textually against the
    expected serialization.
  Usage: `WPT_ROOT=~/corpora/wpt cargo run -p amazing-css --example
  wpt_conformance [selection...]`; defaults to css-color, css-values,
  css-cascade, css-conditional.
- **Selections and file-shape reality**: the ticket named css-color,
  css-values, css-cascade, mediaqueries and css-conditional. Only the first,
  second, third and fifth have files in the `test_valid_value`/
  `test_invalid_value`/`test_computed_value` shape (57 files, 360 extracted
  assertions total at the pinned `c2a2fce` WPT revision). `css/mediaqueries`
  has none — its parsing tests use a per-file `test_parsing(query, expected)`
  pattern against `matchMedia(query).media`'s CSSOM canonical serialization
  (e.g. `"(color"` normalizes to `"(color)"`, an unparseable query to
  `"not all"`), which `MediaQueryList` does not implement (it stores each
  comma-separated query as trimmed source text, not a canonical
  re-serialization) — a distinct, real gap, but a different shape of work
  than this runner covers; not attempted here.
- **Known measurement limits** (see the runner's own doc comment): a
  property this crate has no grammar for falls through to
  `ComputedValue::Unparsed`, which accepts anything, so `values::parses`
  reports it valid regardless of content — an honest "cannot yet reject
  this" signal, not a scoring bug. Computed-value comparison is exact
  string equality; this crate mostly retains the cascade winner's literal
  (lightly canonicalized) text rather than always re-serializing to the CSS
  Color 4/Values 5 computed forms WPT expects, so a computed failure can be
  either a real bug or this known model gap (the fixes below narrow that
  gap; the remainder in "Deferred" is the gap). Assertions made through a
  per-file local JS wrapper function (not a direct call to the three
  helpers) are not extracted, so counts are a lower bound per file.
- **Measured**: **160/360 before any fix → 203/360 after**, zero
  regressions on the amazing-css test suite (335 → 347 → still 347 across
  this ticket's fixes).

Three real bugs found and fixed, biggest cluster first:

- **`syntax::serialize()` never inserted a space after a comma** inside a
  function's argument list: `min(calc-size(auto,size + 20px),300px)`
  instead of the CSS-canonical `min(calc-size(auto, size + 20px), 300px)`.
  This is why nearly every multi-argument computed value across css-values
  and css-color was failing on purely cosmetic grounds — `clamp()`,
  `color-mix()`, `container`, `random-item()`, `rgba()`, `translate()`, all
  of them. Fixed engine-wide: `Token::Comma` now always serializes as `", "`
  (comma, one space, regardless of the source's own spacing), matching
  every browser's canonical serialization. Updated the handful of existing
  tests that had hardcoded the old (wrong) spacing in `cascade.rs`,
  `computed.rs`, `parser.rs`, `syntax.rs`'s own tests, and two integration
  tests. This also fixed one test in `crates/amazing/` (outside this lane's
  paths) that depended on the old spacing; filed for integrator-assist in
  `.agents/css/questions.md` with the exact one-line fix.
- **`opacity`/`fill-opacity` only accepted a bare `<number>`**, silently
  dropping every percentage, `calc()`, `min()`, `max()`, `clamp()` value to
  the initial value `1` (invalid declarations don't override the cascade
  winner, and there was none, so it fell back to initial) — CSS Color 4
  §4.5's `<alpha-value>` is `<number> | <percentage>`, with the usual
  `calc()` math (CSS Values 4 §10). Added `values::alpha_value`, a small
  immediate recursive-descent evaluator (`+ - * /`, nested `calc()`/`min()`/
  `max()`/`clamp()`): the percentage basis for an alpha value is always
  exactly 1 (100% = 1), a *fixed* basis unlike a length's containing-block-
  dependent one, so unlike `crate::math` (grammar validation only, deferred
  resolution) this can and does evaluate to a concrete `f64` immediately.
  `clamp(MIN, VAL, MAX)` follows the CSS Values 4 §10.6 formula exactly
  (`max(MIN, min(VAL, MAX))`, not a naive `VAL.clamp(MIN, MAX)` — when
  `MIN > MAX` the result is `MIN`, and Rust's own `f64::clamp` panics on
  that input). One tokenizer quirk worth remembering elsewhere: a
  standalone `-` or `+` lexes as a one-character `Token::Ident`, not
  `Token::Delim`, per CSS Syntax 3's "would start an identifier" rule
  (`calc(1 - 1)`'s middle token is `Ident("-")`) — `is_op()` matches both
  forms. Fixed 20+ opacity valid/computed cases.
- **`all` (CSS Cascade 3 §7.4) had no grammar at all** — any value "parsed"
  via the generic accept-anything `Unparsed` fallback every property this
  crate doesn't otherwise model falls into, so every css-cascade
  invalid-value test failed (`all: auto`/`none`/`filter`/two keywords all
  wrongly "parsed"). Given its own arm: only the CSS-wide keywords
  (`initial | inherit | unset | revert | revert-layer`). Fixed css-cascade
  from 3/10 to 10/10.
- **`container`/`container-name`/`container-type`** (CSS Containment 3):
  not evaluated (no size-container-query matching exists in this engine),
  but now validated (`container-name`: `none | <custom-ident>+`;
  `container-type`: `normal | [ [ size | inline-size ] || scroll-state ]`;
  `container` shorthand: `<container-name> [ / <container-type> ]?`) and
  `container-name`/`container-type` given `PROPERTIES` metadata (initial
  `none`/`normal`, not inherited) so `initial`/`inherit`/`unset` resolve
  instead of dropping the declaration entirely (previously `style.get()`
  returned `None` for those three cases). Classified `Rebuild` in
  `change_classification.rs` (a real change establishes/removes a query
  container and containment, conservatively the broadest kind). The
  `container` shorthand itself is stored as its own pass-through
  declaration (this crate's shorthands other than the CSS-13-era ones
  aren't independently re-derived from their longhands on read; that's a
  broader existing architecture point, not new to this ticket) so its own
  `initial`/`inherit`/`unset` cases are unchanged. Fixed 4 of
  css-conditional's 12 container-related computed failures.

Boundary (deliberately not fixed here, with the reason):

- **`alpha()`** (CSS Color 5's relative-alpha function, e.g. `alpha(from red
  / 0.5)`): entirely unimplemented (falls through to the generic
  cascade-preserved-text path, so it "parses" as inert text but never
  computes a color). All 3 `alpha-color-*.html` files (roughly 70
  assertions) fail on this. Bleeding-edge, effectively unused on real pages
  today; deferred rather than building a whole new relative-color function
  for a corpus-measurement ticket.
- **`random-item()`** (CSS Values 5, similarly brand-new/experimental):
  unimplemented; `random-item-invalid.html`/`random-item-computed.html`
  account for roughly 27 of the remaining css-values failures. Same
  rationale as `alpha()`.
- **`calc()` `infinity`/`NaN` arithmetic** for `width` et al.
  (`calc-infinity-nan-computed.html`, ~15 failures): this crate's general
  `crate::math` calc validator doesn't carry IEEE-754 infinity/NaN through
  arithmetic to a used-value clamp; a real but narrower gap than the three
  fixed above.
- **Relative-color channel keywords (`r`/`g`/`b`/...) used inside a
  `calc()`/`clamp()`/`min()`/`max()` that is itself an argument to
  `rgb(from ... clamp(r, 30, 300) g b)`**: the relative-color grammar
  doesn't thread channel-keyword substitution through nested math the way
  `alpha()`'s calc-with-`alpha`-keyword case already does (`clamp-color-
  computed.html`, ~15 failures, mostly overlapping with the comma-spacing
  fix for the ones that don't need channel substitution).
- **`container` shorthand serialization nuances**: slash-spacing
  normalization (`foo /inline-size` → `foo / inline-size`) and
  `container-type`'s canonical keyword order (`scroll-state size` → `size
  scroll-state`) both need dedicated shorthand-serialization logic (this
  crate generally serializes the cascade winner's tokenized text, not a
  per-shorthand canonical re-derivation), disconnected from any actual
  container-query *evaluation* capability. ~6 failures.

Linux gate (Boat sandbox `bx_cp5jbn5z`, lane head `e58765a8`): CSS crate
**347/347** (no regressions from any of the above), workspace nextest
**2905/2906** (10 skipped) — the one failure,
`runtime_binding_host::compositor::property_scene_tests::css13_attribute_delta_updates_bound_nodes_without_recompute`
in `crates/amazing/src/runtime_host_compositor.rs` (outside this lane's
paths), is a hardcoded string literal that depended on the old (wrong)
comma-serialization convention; filed for integrator-assist with the exact
one-line fix. Doctests **ok** across all 31 doc-tested crates. WPT
mediaqueries `matchMedia` canonical serialization, and every item under
"Boundary" above, are candidates for the next CSS-25-style pass.

## CSS-26 — `unicode-range` tokenization (base `dev` at `a1e1f36c`)

Root cause (from integrator-assist's IA-LIVE-PAGES report in
`.agents/css/questions.md`): `@font-face { unicode-range: U+0131; }` reached
`FontFaceRule.unicode_range` as `"u131"`. `syntax.rs`'s tokenizer is
generic/context-free and has no `<urange>` production, so `U+0131` lexed as
`Ident("u")` followed by a signed numeric token; `Token::Number`/
`Token::Dimension` don't retain raw lexeme text (unlike `Token::Url`, which
keeps both a decoded `value` and a `raw` spelling), so the `+` sign and the
leading zero were lost *at tokenization*, before serialization ever ran — a
post-hoc string patch on the already-serialized descriptor value could not
have recovered them.

Fix: implemented CSS Syntax 3 §4.3.9 "consume a unicode-range token" as a
dedicated tokenizer rule, matching real UAs — unconditional at the tokenizer
level (not gated on "are we inside a `unicode-range` descriptor"), since `u`/
`U` immediately followed by `+` and a hex digit or `?` is not otherwise
meaningful CSS.

- Added `Token::UnicodeRange(String)` (`syntax.rs`), storing the **verbatim**
  matched source text (case, digit count, wildcards as written) rather than
  a renormalized form, so `U+0131` stays `U+0131` (not `U+131`) and `U+4??`/
  `u+41-5a` keep their original spelling. `Lexer::unicode_range()` implements
  the spec algorithm: up to 6 hex digits, then up to 6 total hex-or-`?`
  wildcard characters, then an optional `-<hex>` explicit end (only when no
  wildcards were consumed for the start).
- `tokenize()` detects the form at the point it would otherwise start
  consuming `u`/`U` as an identifier: next two code points are `+` then a hex
  digit or `?`.
- `serialize()` emits `Token::UnicodeRange`'s stored text as-is.
- `parser.rs`'s `parse_font_faces` `unicode_range` extraction simplified: the
  old post-hoc patch (`"U" not followed by "+"` → insert `"+"`) only ever
  fixed one narrow malformation of the *already-lossy* string and is now
  dead weight, since `descriptor("unicode-range")`'s re-serialized text is
  correct by construction. It now just trims and rejoins each comma-
  separated part with a bare comma (this field's established no-space
  convention, distinct from `syntax::serialize()`'s canonical `", "`).

Tests: tokenizer-level coverage in `syntax.rs` for the exact bug-report case
(`U+0131`), lowercase `u`/hex, hyphenated ranges, wildcards up to `U+??????`,
a comma list, and non-hijacking of unrelated `u`-idents (`um`, `14u`,
`url(...)`, `u+x`); `parser.rs` gained two `@font-face`-rule tests, one
reproducing the exact reported shape (`unicode-range: U+0131;`) and one
covering a single low value, a leading-zero value, an explicit range and a
wildcard together in one real descriptor list.

Gate: crate 352/352 (+5 from CSS-25's 347), workspace nextest and doctests
green (see CSS-27's combined gate below — both tickets landed in the same
session before the final gate). No regressions; the pre-existing
`unicode-range: U+41-5A, U+4??` test (uppercase, no leading zeros) was
already correct and needed no change.

## CSS-27 — `alpha()`, opacity math, and widening the WPT corpus (base `dev` at `a1e1f36c`)

Continued the WPT parsing/computed-value corpus from CSS-25's 203/360
baseline by fixing the two biggest failure clusters, then widened the
runner's selection.

**`alpha()` (CSS Color 5), previously deferred in CSS-25 as a boundary
item** — the single biggest cluster (47 of ~86 `css-color` failures:
`alpha-color-computed.html` + `alpha-color-parsing-invalid.html`):

- `color.rs`/`color_advanced.rs`: `alpha( [from <color>] / [<alpha-value> |
  none] )`. Unlike `rgb()`/`hsl()`/etc.'s relative form, `alpha()` takes no
  channel overrides (nothing may sit between the source and `/`), `from
  <color>` and `/ <alpha>` are both mandatory, and its only substitutable
  keyword is `alpha` itself (never the source's `r`/`g`/`b`/...).
- Introduced `color_advanced::AlphaOutcome` (`Invalid` / `Deferred` /
  `Resolved`) rather than reusing the other relative-color helpers' plain
  `Option<[u8; 4]>`. Collapsing "invalid" and "deferred" to `None` (as
  `relative()` does for the numeric-channel functions) would make
  `alpha(from currentcolor)` (invalid: missing its own `/ <alpha>`)
  indistinguishable from `alpha(from currentcolor / 0.5)` (valid, deferred
  to the computed-value seam like every other relative-color function's
  `currentcolor` handling). `Deferred` also covers a `var()` reference
  inside the `/ <alpha>` slot, which needs substitution before it can be
  evaluated.
  - `classify_source()` gives nested `alpha(...)` sources (e.g. `alpha(from
    alpha(from currentcolor / 0.5) / alpha)`) the same precise
    three-way classification recursively, so a genuinely malformed inner
    call (missing *its own* `/ <alpha>`) still reports invalid instead of
    both cases collapsing into "deferred, so valid". Other nested relative-
    color functions don't expose this distinction, so they fall back to
    the existing shallow `from`-prefix heuristic (values.rs's
    `tokens.get(2) == Ident("from")` check) other functions already rely
    on.
  - The alpha slot's own grammar (number/percentage/`none`/`alpha`/
    `calc(alpha ...)`) is checked with a placeholder `alpha` keyword value
    so a `currentcolor` source can still be judged syntactically valid
    without resolving it.
- `values.rs`: gave `alpha(...)` its own match arm ahead of the generic
  "unrecognized color function → preserved as `Unparsed`" fallback, so a
  genuinely malformed call (e.g. missing `/ <alpha>`, extra tokens, a
  channel keyword used in the alpha slot) is rejected instead of silently
  parsing as valid-but-inert. `AlphaClassification::{Invalid, Deferred,
  Resolved}` maps to `None`/`Some(V::Unparsed)`/`Some(V::Color(...))`.
- Fixed 23/23 in `alpha-color-parsing-invalid.html` and 5/8 newly-exposed
  cases in `alpha-color-parsing-valid.html` (the `var()`-reference and
  nested-`currentcolor` cases above). `alpha-color-computed.html`'s 24
  failures are unchanged — they need the same computed-value
  canonicalization gap described below (`color`'s computed serialization
  doesn't re-derive `color(srgb ...)` text from the resolved RGBA payload;
  it retains the cascade winner's lightly-canonicalized literal text),
  which is out of scope here.

**`opacity`/`fill-opacity` `calc()`/`min()`/`max()`/`clamp()` computing to
the initial value `1`** — the second-biggest cluster (20 of the 86,
`opacity-computed.html`): CSS-25 already taught `values::alpha_value()` this
grammar, but two *other*, independent, narrower implementations in
`computed.rs` never learned it and silently discarded that work:

- `valid_new_paint_property`'s `valid_opacity()` gate (checked *before* the
  value is even considered for computation) only matched a bare
  `Token::Number`/`Token::Percentage`, so `min(50%, 0%)` etc. failed this
  gate and the computed value fell all the way back to `opacity`'s initial
  `1` — the property's *own* `computed_opacity()` function was never even
  reached. Now delegates to `values::alpha_value(value).is_some()`.
- `computed_opacity()` itself (reached once the gate above passes)
  separately re-implemented a bare number/percentage parse rather than
  calling `values::alpha_value()`. Now delegates to it directly (made
  `pub(crate)`), keeping the f32 precision/clamping the original had.
- Fixed all 20/20 in `opacity-computed.html`.

**Widened `wpt_conformance.rs`'s selection** from the original four
(css-color, css-values, css-cascade, css-conditional) to ten, adding
css-fonts, css-text, css-backgrounds, css-transforms, css-flexbox and
css-grid — all confirmed (via `find -iname '*-valid/-invalid/-computed
.html'`) to have files in the runner's supported shape before adding them.
This is a **measured baseline for future tickets**, not fixed here.

Measured (Linux, Boat sandbox `bx_cp5jbn5z`, pool-layout):

| Selection | Before (CSS-25) | After alpha()+opacity | 
|---|---|---|
| css-color | 111/197 | 151/197 |
| css-values | 58/121 | 58/121 |
| css-cascade | 10/10 | 10/10 |
| css-conditional | 24/32 | 24/32 |
| **Original 4 total** | **203/360** | **243/360** |

Widened baseline (new selections, unfixed — candidates for the next
ticket, biggest clusters first): css-fonts 477/636, css-text 421/674,
css-backgrounds 385/672, css-transforms 186/362, css-flexbox 117/166,
css-grid 454/953. **Combined 10-selection total: 2283/3823.**

Boundary (deliberately not fixed here, with the reason):

- **`color`'s computed-value canonicalization** (`color-computed.html`, 15
  failures, plus `alpha-color-computed.html`'s remaining 24 and a chunk of
  the widened selections' failures too): this crate's computed-value text
  for color properties retains the cascade winner's lightly-canonicalized
  *literal* text (`"red"` stays `"red"`, not `"rgb(255, 0, 0)"`; a resolved
  `alpha()`/relative-color call stays its own source spelling) rather than
  re-serializing the resolved `[u8; 4]`/color-space payload to the CSS
  Color 4/Values 5 canonical computed form. This is CSS-25's already-
  documented "known serialization-model gap" (see `wpt_conformance.rs`'s
  own top-of-file doc comment) — fixing it means threading a typed,
  space-aware color representation through the computed-value seam
  wherever `color`/`background-color`/etc. resolve, a materially bigger
  architecture change than a corpus-measurement ticket's scope.
- **Legacy comma-form `rgb()`/`hsla()` strictness** (`color-invalid.html`,
  4 failures): mixed number/percentage channels in the same legacy comma
  call (`rgb(10%, 20, 30%)`), too many comma-separated args
  (`hsla(1,2,3,4,5)`), etc. are accepted when they should be rejected.
  Small, contained, but a different area from alpha()/opacity; left for
  the next ticket.
- **`sibling-index()`/`sibling-count()`** as an `<alpha-value>` inside
  `alpha()` (3 residual `alpha-color-parsing-valid.html` failures): CSS
  Values 5 tree-counting functions need live DOM/layout-order context at
  compute time, not just string-level grammar validation — a distinct,
  substantial feature, not part of implementing `alpha()` itself.
- **The six widened selections' own failures** (1540 of 3823): not
  triaged into clusters yet beyond the raw counts above; `css-grid`'s
  `*-invalid.html` files are the single largest visible mass (roughly 300+
  across `grid-template-shorthand-invalid`, `grid-template-rows/columns-
  invalid`, `grid-shorthand-invalid`, `grid-row/column/area-invalid` —
  grid track-list/shorthand grammar is currently permissive, likely via the
  same "unrecognized-but-preserved" pattern seen elsewhere), with
  `box-shadow-invalid.html` (40), several `*-computed.html` transform-origin/
  scale/rotate files, and font-variant/font-synthesis grammar close behind.

Gate (Boat sandbox `bx_cp5jbn5z`, pool-layout, both CSS-26 and CSS-27 on
this same head): CSS crate **352/352** (0 regressions), workspace nextest
**3048/3048 passed, 12 skipped, 0 failed**, doctests **ok** across every
doc-tested crate (`amazing-a11y`, `amazing-devtools`, `amazing-html`,
`amazing-js` each ran real doctests; the rest ran 0/0 ok). Disk on the
shared sandbox ran low (`~/amazing/targets/*/debug/incremental` regrows
fast under six lanes sharing one host); purged per the established
disk-hygiene precedent (rebuildable, safe).

## CSS-28 — `place-*` shorthand expansion + the css-grid `*-invalid.html` cluster (base `dev` at `d1d41ec3`)

Two parts, one commit series.

**Part 1: `place-items`/`place-self`/`place-content` reach the full cascade.**
`expand_declaration` already had real arms for these three shorthands, but
nothing had exercised them through the full `compute()` pipeline (as opposed
to `expand_declaration`'s own unit tests) — layout-flexgrid's grid abspos
static-position code reads `ComputedStyle::get("align-items")`/etc. after a
real cascade, not via a synthetic call. Added
`place_shorthands_reach_computed_style_through_the_full_cascade` in
`crates/amazing-css/tests/css6_grid_values.rs` covering all three shorthands
plus the ordinary "a longhand declared after the shorthand wins for that
longhand only" cascade-order case; no production-code gap was found (already
correct), so this is a regression-proofing test, not a fix.

**Part 2: css-grid's `*-invalid.html` shorthand-strictness cluster.**
Root cause: `values::parse()` — the function
`wpt_conformance.rs`'s `test_valid_value`/`test_invalid_value` calls through
`values::parses()` — had **no match arm at all** for
`grid-template-columns`/`-rows` (fell through to an unconditional `Some(...)`),
and no arm whatsoever for `grid-row-start`/`-end`, `grid-column-start`/`-end`,
`grid-row`/`grid-column`, `grid-area`, `grid-auto-columns`/`-rows`,
`grid-auto-flow`, `grid-template-areas`, `grid-template`, or `grid` (all fell
through to the generic bottom-of-function fallback, which accepts anything).
Every `test_invalid_value` for these properties was therefore unwinnable:
validity was never actually checked.

Fixed by wiring all of the above into real validators in `computed.rs`
(`valid_grid_track_list`/`valid_grid_track_tokens` rewritten for
`<line-names>?` sequencing and fixed-size enforcement alongside an
auto-repeat; new `valid_track_size` adding `fit-content()`; `valid_grid_line`
rewritten from a token-order-independent flag check to a word-based,
position-tracking implementation enforcing "`span` may lead or trail an
integer+name pair but not sit between them" and rejecting lexical
`<number>`s like `1.0` where `<integer>` is required; new
`valid_grid_line_pair`, `valid_grid_area`, `valid_grid_auto_track_list`,
`valid_grid_auto_flow`, `valid_grid_template_areas`, `valid_grid_template`,
`valid_grid_shorthand`), then calling them from new `values::parse` match
arms for every property above. Also added `grid-area`'s cascade expansion in
`expand_declaration` (it previously had no arm at all and fell to the
generic passthrough, so it never split into its four longhand quadrants for
layout).

**Regression found and fixed before this could ship (zero-lost-passes
check):** giving `grid-area` a real `expand_declaration` arm stopped it from
being stored under its own shorthand name (matching the crate's established
"shorthand values are not directly retrievable via `get()`" pattern, per the
existing `public_set_uses_the_same_shorthand_expansion_seam` test) — but six
`grid-area-computed.html` cases were passing *only* because the old
passthrough happened to echo the specified text back verbatim, which is what
`getComputedStyle().gridArea` needs. Restored that round-trip deliberately:
`expand_declaration`'s `"grid-area"` arms now emit a fifth `("grid-area",
value)` pair alongside the four longhands, so `get("grid-area")` still
works. This is a literal echo, not a re-derived shortest serialization, and
can go stale if a later, higher-priority `grid-row-end`/etc. declaration
overrides only one quadrant of the same cascade slot — see Boundary below.
`grid-row`/`grid-column`'s own equivalent computed-value gap predates this
ticket (they never had this passthrough) and is not a regression; left
as-is (see Boundary).

Two more regressions surfaced by the same before/after diff, both inside the
new `valid_grid_line` rewrite itself (not `grid-area`): (a) it pre-split the
value on ASCII whitespace *before* tokenizing, which cut a CSS escape's own
mandatory single trailing whitespace (`\31 st` decodes to the one ident
`1st`; naive splitting cut it into `\31` and `st`, each unparseable) — fixed
with a new `split_grid_line_words` that walks the string once, treating an
escape sequence (and, for the calc case below, anything inside balanced
parens) as non-splitting; (b) the old value-based (not token-order) rewrite
had no path for CSS Values 4's "`<integer>` may be a math function"
(`span calc(-2)`, `span min(-1, 6)`) — added an `integer_calc_function` check
recognizing a balanced `calc()`/`min()`/`max()`/`clamp()` in the integer slot,
accepted for grammar purposes with an *unknown* value (exempt from the sign
checks, since a UA cannot statically evaluate `sibling-index()` or a
container-query-dependent `sign()`).

**Verification methodology:** measured pass/fail by *test identity*
(`property: value`, stripping the `-> actual, expected` suffix before
diffing), not raw line text or net counts — a naive line-diff flags a test
that fails at both baseline and current-but-with-a-different-actual-value
text as a "new regression" when it never passed either way. Baseline was
captured by `git stash`-ing all CSS-28 changes locally (the Boat sandbox tree
has no `.git`, so `git stash` had to run on the Mac worktree before each
`scripts/linux-run.sh` sync), running the full `wpt_conformance` corpus, then
popping the stash and re-running on the same commit (`d1d41ec3`) plus
uncommitted changes.

Measured (Linux, Boat sandbox `bx_cp5jbn5z`, pool-layout):

| Selection | Baseline (`d1d41ec3`, no CSS-28 changes) | After CSS-28 |
|---|---|---|
| css-grid | 454/953 | 792/953 |
| **All 10 selections total** | **2283/3823** | **2621/3823** |

Zero regressions by test identity across all 10 selections (`comm -13` on the
sorted, actual-value-stripped FAIL-key sets: empty); 338 tests newly passing,
all in css-grid. Crate tests: **354/354** (0 doctests, unchanged). Workspace
gate: `cargo nextest run --workspace --locked --no-fail-fast` **3104/3104
passed, 13 skipped, 0 failed** (up from baseline's 3102 — the two new
`place-*` regression tests); `cargo test --workspace --locked --doc` **ok**
across every doc-tested crate, both before and after.

Boundary (deliberately not fixed here, with the reason):

- **`grid-row`/`grid-column`'s own `*-computed.html` round-trip** (8 of
  `grid-area-computed.html`'s remaining failures): these two shorthands have
  never stored a reconstructible value under their own name (no passthrough
  was ever added for them, unlike `grid-area`'s restored one above), so
  `get("grid-row")`/`get("grid-column")` return `None` after a full cascade —
  a pre-existing gap, not a regression from this ticket. A real fix needs
  actual CSS shortest-serialization (`grid-row-shortest-serialization.html`
  covers this directly, but that file is never run: `wpt_conformance.rs`'s
  `collect_files` only picks up `*-valid.html`/`*-invalid.html`/
  `*-computed.html`, and this file ends in `-serialization.html`).
- **`calc()`-typed `<integer>` resolving to an actual value** (5 remaining
  `grid-area-computed.html` failures: `calc(1.1)` rounding, `calc(10 + (sign(
  2cqw - 10px) * 5))` needing real container-query evaluation against the
  test's `100px` container, `span calc(-1)`/`span calc(sibling-index() - 2)`
  needing the spec's "negative or unresolvable → clamp to 1" rule): accepted
  for *validity* (fixed above, unblocking three `grid-area-valid.html`
  failures), but computing the actual resolved integer — especially the
  container-query-dependent case — needs real layout/query evaluation this
  validator-level fix does not attempt.
- **`grid-area`'s round-trip staleness** (no current test exercises this,
  documented as a known edge): the restored `("grid-area", value)` passthrough
  is the shorthand's own raw text, cascaded as its own independent candidate.
  If a later, higher-specificity `grid-row-end` (etc.) declaration overrides
  only one quadrant, `get("grid-area")` still returns the original shorthand
  text, not a value reflecting the override. Real shortest-serialization
  (same fix as the bullet above, generalized to all three shorthands) would
  close this properly.
- **`grid-lanes` tentative WPT files** (~51 failures per the earlier widened
  baseline): not touched this ticket, out of the `*-invalid.html` cluster's
  scope.
- **`computed.rs` file size**: was already over this repo's 4,000-line
  budget before this ticket (4,081 lines at `d1d41ec3`); the validators added
  here bring it to 4,806. Flagged in `.agents/css/questions.md` as a split
  candidate for a dedicated follow-up ticket (e.g. hiving off the
  `valid_grid_*` family into a new `grid_values.rs` submodule) rather than
  attempted under this ticket's gate.

## CSS-29 (based on 120600f9)

Three parts, one commit series: a code-health split, the selector cluster
routed from dom-bindings, and the box-shadow-invalid.html cluster (the
largest remaining one across the 10 WPT selections).

**Part 1 -- split `computed.rs`.** Moved the css-grid track-list/`<grid-
line>`/shorthand grammar (30 functions, unchanged bodies) into a new
`grid_values.rs` module, re-exported with `pub(crate) use grid_values::{...}`
so both `crate::computed::name` (from `values.rs`) and bare-name internal
calls keep working. `computed.rs` 4,806 -> 3,960 lines; `cargo check`/`cargo
nextest run -p amazing-css` clean; css-grid WPT selection unchanged at
792/953, proving zero behavior change. Commit `12deb44a`.

**Part 2 -- the selector cluster.**

- **querySelector CSS-escapes** (`dom/nodes/ParentNode-querySelector-
  escapes.html`, 40 subtests, routed earlier): CSS Syntax 3 SS4.3.7's
  "consume an escaped code point" had two bugs. The hex-digit branch
  returned `None` (meaning "not an escape at all") for a NUL/surrogate/
  out-of-range value instead of U+FFFD -- but by then it had already
  consumed the whole escape sequence, so the caller's `None`-handling code
  (which assumes the `\` is still unconsumed) advanced position AGAIN,
  desyncing the tokenizer. A lone backslash at EOF decoded as a literal `\`
  instead of U+FFFD (a parse error per spec, but still a code point, not
  "not an escape"). Fixed both branches to always return a code point except
  for the one true "not a valid escape" case (`\` directly followed by a
  newline). Also added CSS Syntax 3 SS3.3 input preprocessing (CRLF/lone-CR/
  FF -> LF, NUL -> U+FFFD) before tokenizing, needed because a hex escape's
  trailing-whitespace consumption only ever sees an already-normalized LF.
  Root-causing this surfaced a THIRD, separate bug: the tokenizer's hash-
  token (`#`) and at-keyword (`@`) dispatch only checked for a literal ident
  char right after the sigil, never for a valid-escape lookahead (backslash
  not followed by newline) -- so `#\30 foo`-style escaped id selectors
  tokenized as a lone `Delim('#')` and never reached `ident()`/`escape()` at
  all. Fixed by adding the escape-lookahead arm to both dispatches.
- **`:scope`/`:has(> :scope)` for `Element.closest()`** (`dom/nodes/Element-
  closest.html`, part of its 4 subtests): root-caused to
  `crates/amazing/src/dom_host.rs`'s `matches_selector`, which always calls
  `amazing_css::selectors::matches(...)` (scope always `None`) -- `closest()`
  must bind `:scope` to the ORIGINAL context element throughout its whole
  inclusive-ancestor walk. `amazing-css` already had the right scope-
  threading machinery (`matches_in_scope`/`matches_pseudo_in_scope`, and
  `Simple::Has`'s `compound_in_scope` call already forwards `scope`
  correctly), it was just `pub(crate)`-only. Made both `pub` so a caller
  outside this crate can supply a scope root; the actual `closest()`/
  `DomHost` wiring is outside `amazing-css` (routed in
  `.agents/css/questions.md` to devtools/integrator-assist).
- **`:invalid`/`:valid`** (the other half of `Element-closest.html`'s 4
  subtests, previously a SyntaxError since an unrecognized pseudo-class
  invalidates the whole selector): added as a Phase 2, attribute-only
  subset -- no live form/widget state in this DOM. `input`/`select`/
  `textarea` are invalid when `required` and their effective value is empty
  (select reads its selected/first `<option>`'s `value` attribute or text;
  textarea reads its text content). `fieldset`/`form` match `:invalid` when
  they have any non-disabled `input`/`select`/`textarea` descendant that is
  itself invalid -- per the HTML Standard's own `:valid`/`:invalid` rule for
  those two elements, since neither has a value of its own. `pattern`/
  type-format/min-max mismatches are out of scope. New unit test
  `invalid_and_valid_pseudo_classes_read_required_and_empty_value` mirrors
  the routed WPT fixture's shape (`do_test(":invalid", "test11", "test2")`:
  a form and a fieldset both matching `:invalid` via a descendant, while the
  select in between is validly filled).

amazing-css: 356/356 nextest (incl. new
`id_selector_escapes_decode_per_css_syntax_4_3_7` and the `:invalid`/`:valid`
test above), 0 doctests. WPT parsing-conformance runner (10 selections)
unaffected by this part: 2621/3823, matching the CSS-28 baseline exactly (the
runner only exercises `values::parse`/`compute()`, not selector matching).
Verification for the actual routed DOM/selector WPT files is by unit tests
mirroring their assertions plus the full crate regression above, since this
lane does not own the JS-engine/DOM-bindings harness that executes real
`testharness.js` files end to end. Commit `92372a58`.

**Part 3 -- box-shadow-invalid.html cluster (40 failures, the largest
remaining across the 10 selections once Part 2 was measured with no effect
on this runner).** `box-shadow`'s `<shadow>` grammar
(`<color>? && <length>{2,4} && inset?`) was checked by counting each kind of
component independently and never checking adjacency: `-4px red 4px`
(a color splitting the two required lengths into two separated runs),
`4px inset -4px`, a color or `inset` alone with no lengths, too many/too few
lengths, two colors, two insets -- all of it parsed as valid. Rewrote as
`valid_single_box_shadow`: classify each component, then require the
lengths to form exactly one contiguous run (a color/inset interrupting it
directly produces two runs, caught immediately) plus at most one color and
one `inset`.

Wiring this into `values::parse`'s `"box-shadow"` arm (there wasn't one --
box-shadow fell through to the generic `Unparsed` catch-all, so the WPT
runner reported every string valid regardless of content, masking the bug
above from measurement) surfaced two more gaps once real values started
being checked: `shadow_length` only recognized the `px` unit, and rejected
`calc()`-based lengths outright including percentage-free ones CSS Values 4
allows for `box-shadow` (`calc(1em - 2px)`, `calc(-1px)` -- the latter valid
because a `calc()` result outside a property's allowed range is clamped at
computed-value time, not rejected as a parse error, unlike a literal
out-of-range token, so a calc()-derived blur radius's sign is deliberately
left unchecked). Also added the missing comma-separated shadow LIST support
(multiple `<shadow>`s), previously rejected outright regardless of content.
Fixing these three together, rather than only the ordering bug, was needed
to avoid regressing four `box-shadow-valid.html` cases that had been passing
vacuously (no real check existed) and would otherwise have started failing
once real validation was wired in.

Split box-shadow's whole grammar into `shadow_values.rs` (same pattern as
Part 1's `grid_values.rs`) to keep `computed.rs` under the 4,000-line policy
-- adding the fixes above brought it back up to 4,079 lines; after the split,
3,797.

amazing-css: 356/356 nextest, 0 doctests, both before and after the split.
WPT parsing-conformance runner (10 selections): css-backgrounds 385 -> 427
(+42), combined total 2621 -> 2663/3823. Verified **zero lost by test
identity**: diffed the full sorted FAIL-key sets (test file + assertion,
suffix after `->` stripped) before and after with `comm`; the four
box-shadow-valid.html cases mentioned above were the only ones that would
have regressed, and are covered by the calc()/units/comma-list fixes.
Commit `accfba0d`.

Boundary / not fixed here:

- **Full-workspace gate blocked by a pre-existing, unrelated compile break**:
  `cargo nextest run --workspace --locked` fails to compile at
  `crates/amazing/src/runtime_host_devtools.rs:974`
  (`missing field 'node' in initializer of 'PageScript'`) -- HTML-29 added
  that field to `PageScript` in `crates/amazing/src/cmd/js.rs`, but
  devtools's own file wasn't updated. Confirmed via `git status`/`git log`
  that this is pre-existing in the synced tree, not introduced by CSS-29.
  Routed in `.agents/css/questions.md` with the exact one-line fix; not
  edited here since it's outside `LANE_PATHS["css"]`. `amazing-css`'s own
  crate gate (nextest + doctests) is unaffected and reported above.
- **`:scope`/`:has(> :scope)` end-to-end wiring**: the `amazing-css`-side
  primitive is public now; the `DomHost`/`closest()` consumer change is
  cross-lane (routed, see Part 2 above).

## CSS-30: background-clip/-origin/-attachment/-repeat/-size/-position grammars

Rebased on local `dev` (`7cfe7780`) first; the rebase was a clean fast-forward
(CSS-29 was already merged there). Per the coordinator's routing, item 1
(`Element.closest()`'s `:scope` wiring) stays with dom-bindings: re-confirmed
on the rebased head that the only call sites
(`crates/amazing/src/dom_host.rs::BrowserDomHost::matches_selector`,
`crates/amazing-js/src/interp_builtins.rs::native_dom_closest`) are outside
`amazing-css`; the crate-side primitive (`matches_in_scope`/
`matches_pseudo_in_scope`, made `pub` in CSS-29) needs no further change here.

Item 2: took the next-largest tractable grammar-gap cluster in
css-backgrounds (245 failing after CSS-29), triaged into two families.
css-text (253, the single largest) is almost entirely unimplemented
properties from scratch (`text-transform`, `text-wrap`, `hyphens`, `word-break`,
20-odd more) -- a much bigger "implement N properties" undertaking, set aside
in favor of the grammar-gap pattern the coordinator pointed at.

**Root cause, `background-clip`** (same shape as CSS-29's box-shadow bug):
`valid_background_clip` only ever matched a single plain keyword
(`[Token::Ident(_)]`), so `expand_declaration`'s guard rejected any comma
list (`"border-box, padding-box"`) or the box+text combination
(`"border-area text"`, both real, WPT-tested `background-clip` values) and
fell through to the shared fallback arm that returns an empty `Vec` --
dropping the declaration entirely, so the computed value defaulted to
`PropertyMetadata`'s initial `"border-box"` regardless of what was
specified. `values::parse` had no dedicated arm at all, so the "valid"/
"invalid" WPT dimension separately over-accepted (the generic
`margin`/`inset`/`size`/`border`/`spacing`/`gap`/`padding` fallback doesn't
match `"background-clip"`, so it always returned `Some(Unparsed)`).
`background-origin`/`-attachment`/`-repeat`/`-size` had no grammar in either
place: `values::parse` over-accepted the same way, and
`expand_declaration` had no arm so every value (valid or not) passed straight
through as literal text with no computed-value canonicalization.

**The grammars**, all in a new `background_values.rs`:
- `background-clip`: `[ <visual-box> | border-area | text | (<visual-box> |
  border-area) && text ]#`. Canonical order is box-keyword before `text`
  regardless of specified order (`"text border-area"` -> `"border-area
  text"`).
- `background-origin`/`-attachment`: plain `<visual-box>#`/`<attachment>#`
  keyword lists, no reordering.
- `background-repeat`: `<repeat-style>#` (`repeat-x`/`repeat-y`, or 1-2 of
  `repeat|space|round|no-repeat`); two identical keywords collapse to one at
  the computed-value stage (`"repeat repeat"` -> `"repeat"`).
- `background-size`: `<bg-size>#`; a single component's missing second
  defaults to `auto` (`"1px"` -> `"1px auto"`), but two literal `auto`s
  collapse back to the single keyword `auto` (`"auto auto"` -> `"auto"`);
  negative lengths/percentages rejected, calc()-derived values left
  sign-unchecked at parse time (same policy as CSS-29's box-shadow).
- `background-position`/`-position-x`/`-position-y` (the last two newly
  registered as longhands -- they did not exist as properties before this):
  CSS Backgrounds 3 SS3.9's resolution algorithm. 1-token (keyword or bare
  length-percentage); 2-token strict pair, swapped only when a keyword's own
  axis identity requires it (`"top left"` -> `"0% 0%"`, since `top` is a
  Y-keyword forcing `left` into the X slot); 3-4-token `&&`-combined
  edge+offset chunks assigned to X/Y by keyword identity, with a lone
  `center` chunk taking whichever axis the other chunk didn't claim
  (`"top 17px right -18px"` -> `"calc(100% + 18px) 17px"` -- a
  right-anchored negative length offset becomes a sign-flipped
  `calc(100% - <length>)`; a percentage offset resolves as plain arithmetic,
  e.g. `"right 11% bottom"` -> `"89% 100%"`). The writing-mode-relative
  `x-start`/`x-end`/`y-start`/`y-end` keywords (CSS Backgrounds 4) have no
  resolution possible without direction/writing-mode context this crate
  doesn't carry here, so they stay opaque passthrough -- matching their
  current (accidentally correct, pre-CSS-30) passing status rather than
  regressing them.

Every property's validator is wired into both `values::parse` (the
"valid"/"invalid" WPT dimension) and `expand_declaration`/
`valid_new_paint_property` (the cascade and var()-substitution "computed"
dimension) -- the same two-gate discipline CSS-29 established for
box-shadow, now applied consistently to this whole property family.

Boundary, not fixable here: `em`/`rem`-relative offsets and calc()
expressions mixing a percentage-free length with a relative unit need the
div's cascaded `font-size` to resolve to a px number (e.g. `"0.5em"` ->
`"20px"` at `font-size: 40px`); `wpt_conformance.rs`'s harness casts a lone
`div { property: specified }` with no other declaration, so these keep
their literal specified text. Confirmed these were *already* failing before
CSS-30 (just with different displayed text, since the property had no
grammar at all previously) -- not a regression, the same documented
harness limitation as other properties' `em`-dependent computed tests.
`background-image`'s own invalid-argument cases (`radial-gradient`'s
negative radii, `cross-fade()`'s percentage range) are a distinct,
separate gradient-argument-validation task, not touched here.

Moved this cluster into `crates/amazing-css/src/background_values.rs` (same
reason as CSS-29's `grid_values.rs`/`shadow_values.rs`): `computed.rs` was at
3,797 lines; the new wiring alone (without the split) would have pushed it
over 4,000.

Measured (pinned 10-selection WPT parsing-conformance runner, before/after
on the same rebased base, `7cfe7780`): css-backgrounds 427/672 -> 540/672
(+113); every other selection unchanged (css-color 151/197, css-values
58/121, css-cascade 10/10, css-conditional 24/32, css-fonts 477/636,
css-text 421/674, css-transforms 186/362, css-flexbox 117/166, css-grid
792/953); TOTAL 2663/3823 -> 2776/3823. Verified **zero lost by test
identity**: diffed the full sorted FAIL-key sets for css-backgrounds with
`comm` -- 117 lines resolved, 0 regressed, 4 lines changed displayed text
only (the `em`-boundary cases above, `FAIL` both before and after).

amazing-css: 356/356 nextest (two tests added for the new grammars; one
pre-existing property-count assertion in `change_classification.rs` updated
for the two newly-registered longhands), 4 doctests, both green. Full
workspace gate (win-001-equivalent, `linux-run.sh`): `cargo nextest run
--workspace --locked` 3305/3305, `cargo test --workspace --locked --doc`
4/4. Commit `55657df3`.

## Cross-lane: bare `stretch`/`-webkit-fill-available` sizing keyword (2026-09-27, authorized by integrator, css lane idle)

`values.rs`'s `parse()` accepted `min-content`/`max-content`/`fit-content`
as bare keywords for `width`/`height`/`min-width`/`min-height`/`max-width`/
`max-height`/`flex-basis` but had no arm for CSS Sizing 4's `stretch`
(née `-webkit-fill-available`), so `expand_declaration` dropped every such
declaration as invalid before `amazing-layout` ever saw it -- root-caused by
the layout lane (LAYOUT-32) via `css/css-sizing/stretch/min-width-1.html`'s
exact 5000px/50x100 shortfall. Added a `Keyword::Stretch` variant and one new
match arm, `"stretch" | "-webkit-fill-available" if size => Some(Keyword::Stretch)`,
mirroring the three existing size keywords exactly (`crates/amazing-css/src/values.rs`,
~9 lines). `math.rs`'s separate `calc_size_basis` (the `calc-size()` basis
grammar) already accepted `stretch`; this only closes the bare-keyword path.
No exhaustive match over `Keyword` exists outside `values.rs`, so the new
variant is additive. Applied and gated by the layout lane as part of
LAYOUT-33 (amazing-css + amazing-layout crate tests, workspace gate); see
`docs/lanes/layout_TODO.md`'s LAYOUT-33 section for the corpus numbers.

## Next

Biggest clusters first (post-CSS-30): css-text (253 failing, unimplemented
properties from scratch -- the largest but not a grammar-gap fix), css-grid
(161, mostly `repeat()`/`fit-content()`/track-list computed-value expansion
and `grid-row`/`grid-column`'s `<custom-ident>` support), css-transforms
(176), css-fonts (159), css-values (63), css-flexbox (49), css-color (46),
css-conditional (8). Within css-backgrounds, the remaining 132 failures are
mostly the documented `em`/font-context boundary (background-position/-size)
and `background-image`'s gradient-argument validation (negative
`radial-gradient` radii, `cross-fade()`'s 0-100% range) -- a distinct task
from this ticket's list-grammar work. Also still open from CSS-25: `calc()`
infinity/NaN, relative-color channel keywords inside nested
`calc()`/`clamp()`, `container` shorthand serialization, `random-item()`,
and `mediaqueries`' `matchMedia().media` canonical serialization.

## CSS-31: logical border axis and edge shorthands

Fixed base: `9d0b42d0`. CSS Logical Properties Level 1 §4.5 specifies
`border-inline`/`border-block`, four edge shorthands, and the axis
width/style/color one-or-two-value shorthands. Previously none expanded
into the logical border longhands already mapped by cascade.rs; declarations
remained inert diagnostic text. This blocked the routed table-intrinsic-size
cluster despite correct layout intrinsic sizing.

`logical_border.rs` uses the existing atomic side-border parser and existing
writing-mode mapping. It handles omitted component resets, CSS-wide keywords,
functional colors, paired values, variable substitution, and invalid-at-computed
values. Invalid logical longhands are rejected before cascade ranking.
Axis computed serialization is reconstructed from final longhand winners;
logical longhands mirror their physical winners, including RTL/vertical modes.
CSS.supports recognizes the complete family. No upstream code was ported.
Moved computed.rs's existing tests into computed_tests.rs to keep every changed
source below 4,000 lines.

Linux gates on Boat pool-layout `bx_shgjh66q`, native `/tmp/amazing-target-css`:
crate 358/358 (before 356), crate doctests 0; workspace 3517/3517, 17 skipped,
workspace doctests 4/4. Eleven pinned WPT parsing selections 3076/4292 ->
3132/4292, css-logical 300/469 -> 356/469; exact failure-key comparison gained 56,
lost 0, all ten other selections unchanged. CSS tables render 54/82 -> 59/82,
gained 5/lost 0: table-intrinsic-size001..004 and rules-groups. Remaining 23
mismatches unchanged. Durable gate summaries and compact render classifications
are under `crates/amazing-css/evidence/css31/`. No Windows gate required by the
Linux-first override. No local build/test or direct Boat lifecycle commands.

## CSS-32: background-image argument validation

Fixed base `03e13e3f`. CSS Images 3 §3.2 and Images 4 §2.6 require
nonnegative explicit radial-gradient radii, length-only circle radii, and
one optional 0–100% percentage with one image/color per cross-fade item.
Mathematical percentages are typechecked independently of lengths; circles
reject percentage-valued math, and logical/container length units remain valid.
Previously background-image had no validator, accepting every string.

`image_values.rs` validates balanced image-list entries, handles light-dark
image pairs and recursive cross-fade arguments, rejects invalid radial sizes
before the position clause, and leaves negative color-stop positions valid.
The math parser now exposes percentage-only type checking, so cross-fade
rejects length or mixed length/percentage expressions. Balanced linear/conic
gradients, image-set/image/element/paint and legacy image functions retain
the existing downstream parser representation; their separate argument
grammars remain future work. Both declaration-time and post-var-substitution
gates use the new validator, with tests distinguishing invalid declarations
from invalid winning substituted values. No port/dependency added.

Boat/Linux final: 360/360 crate tests, 0 crate doctests; workspace 3519/3519,
17 skipped, 4/4 doctests. Twelve parsing selections 3235/4506 ->3248/4506,
gained 13 exact assertions/lost 0 (backgrounds +12, images +1; all other
selections unchanged). Static rendering preserves all classifications:
backgrounds 75/134, images 92/141, gained 0/lost 0 for each. Evidence and a
bulk reproduction script: `crates/amazing-css/evidence/css32/`.

## CSS-33: contain-intrinsic-size family

Fixed base `7c9e3652`. Routed by LAYOUT-38: raw contain-intrinsic-size
and unmapped logical longhands lost declaration order. Implemented CSS
Sizing 4 §5.2 grouped `[auto? [none | nonnegative length]]{1,2}`, atomic
expansion into physical width/height, CSS-wide values, deferred variables,
four registered physical/logical longhands, Layout invalidation classification,
and logical mapping before ranking with provenance and computed mirroring.
Font/absolute lengths compute to px; CSS owns values, while layout owns remembered
sizes and the effect of size containment. The compatibility shorthand getter
is reconstructed from final physical winners.

Found and fixed a measurement bug in wpt_conformance.rs: an explicitly present
non-literal expected argument (such as length_ref("2vw")) was replaced with the
specified string, manufacturing false passes for unresolved viewport lengths.
Unsupported dynamic expected values now count as skipped, consistent with the
runner's documented static extraction boundary. Both before/after parsing
measurements use the same corrected harness; the base engine remains 7c9e3652.
Full workspace 3521/3521, 17 skipped; doctests 4/4. Crate 362/362, docs 0.
Full static sizing sweep 90/138 ->91/138, gained 1/lost 0: logical-001 now passes.
Corrected-harness parsing 3440/4805 ->3482/4805, gained 42/lost 0 by exact
assertion identity, 169 nonliteral cases skipped. Evidence: css33/gate.json
and before/after compact classifications under amazing-css/evidence.

## CSS-34: individual transform properties

Implemented CSS Transforms 2 §5 as one CSS-owned area: registered inherited-false
`translate`, `rotate`, and `scale` longhands with `none` initial values; dimensional
grammar checks before cascade ranking and after variable substitution; computed
axis/angle/percentage/length serialization; distinct `none` versus identity values.
Scale percentages become numbers without opacity clamping, redundant Y/Z defaults
are omitted, reverse axis rotations negate the angle, and general/zero vectors
retain their axis. Translation retains percentages and mixed linear calc values
until box geometry is available, while resolving absolute/font/viewport lengths.
Number/percentage/angle math uses the existing typed math parser; font-dependent
scale `sign()` expressions resolve only when their metric is available. EOF
function closure follows CSS Syntax recovery. Relative font metrics not provided
by the CSS environment and piecewise percentage math remain symbolic.

The fixed base is `a717bb9749a867628f92811b45abb435efde7769`. The corrected
CSS-33 parsing runner is identical before and after. The full 14-selection parsing
corpus gains 83 assertions with zero losses: 3482 → 3565 / 4805, across 498 files
(169 nonliteral assertions excluded equally). Linux workspace: 3524 passes,
17 skips, plus four doctests. Full css-transforms static reftests: 145 / 348
before and after; all 348 classifications unchanged, zero losses. See
`crates/amazing-css/evidence/css34/` for compact before/after render maps and exact
gained assertions. Builds and tests use native `/tmp/amazing-source-css` and
`/tmp/amazing-target-css` on Boat; retained source and outputs remain under HOME.

Consumer boundary: layout/paint/compositor do not yet compose these individual
properties into the rendered transformation matrix. Their owners must compose
translate → rotate → scale before the existing transform list, and preserve the
stacking/containing-block distinction between `none` and identity. Changes are
conservatively classified Rebuild until that consumer path is adopted. Rendering
parity here is a zero-loss gate, not a claim of newly rendered transforms.
Spec: https://drafts.csswg.org/css-transforms-2/#individual-transforms.

## CSS-35: font-synthesis family

Completed the CSS-owned synthetic-face controls as one area: four inherited
longhands plus shorthand grammar, deferred `var()` expansion, invalid-at-computed
unset behavior, CSS-wide inheritance, and canonical shorthand reconstruction
from the winning axes. `oblique-only` is accepted on the style longhand and as
the style alternative in the pinned WPT shorthand grammar; it conflicts with
`style`, including after variable substitution. Unspecified shorthand axes reset
to none. The independent `font` shorthand does not reset synthesis controls.
Layout-time CSS style mutation also rebuilds the derived shorthand summary.

Base `0e59c664cdbbd9cdccf064ba66c557a23c79cace`; same CSS-33 corrected runner
and full 14-selection parsing corpus: 3565 → 3599 / 4805, +34 exact assertions,
zero losses. Linux workspace 3527 passes, 17 skips, four doctests; CSS crate 368
passes. Full css-fonts static render selection remains 24 / 32, all 32
classifications unchanged, zero losses. Native Boat source/target; durable
before/after classifications and gained assertion identities in
`crates/amazing-css/evidence/css35/`.

Consumer follow-up routed to layout: `box_tree::font_synthesis` currently treats
all non-none policies as enabled. For oblique-only, the style synthesis bit must
be disabled when the requested font-style is italic; oblique requests retain
synthesis permission. CSS supplies the distinct canonical policy string.
References: CSS Fonts 4 §2.8 and pinned WPT
`css/css-fonts/parsing/font-synthesis-{valid,invalid,computed}.html`.

## CSS-36: font-size-adjust metric grammar and typed consumer seam

Implemented CSS Fonts 5 §3.4's whole metric/value grammar: none, optional
ex-height/cap-height/ch-width/ic-width/ic-height and number/from-font, scalar
math, nonnegative computed clamping, default ex-height serialization omission,
variable/inheritance/unset processing, and font shorthand reset. CSS declarations
and SVG presentation attributes now share that grammar; an older SVG numeric
parser previously intercepted font-size-adjust before CSS Fonts validation.

Public `font_size_adjust::{FontSizeAdjust,FontSizeAdjustMetric,FontSizeAdjustAspect}`
is carried in `values::ComputedValue::FontSizeAdjust`. `resolve_from_font` accepts
a ratio from the actual first available font (missing font → none); the provider
supplies standard fallback metrics before calling it. `resolve_math` accepts a
length-to-px callback for font/viewport/container geometry. Pure and available
font-length math computes at the existing CSS boundary; from-font and unavailable
context stay typed. Resolve font-derived ratios before inheritance so children
inherit the parent's computed numeric aspect rather than selecting another ratio.

Base f865b7b637796e10dae6e491c34558d9bf07223c. Identical corrected CSS-33
14-selection parsing corpus: 3599 → 3626 / 4805 (+27 exact, lost0); crate371
passes, Linux workspace3530passes/17skips+4doctests. Full static fonts24/32,
all32 classifications unchanged/lost0, using CSS35's artifact-bound final map as
the exact-base before evidence. Durable evidence in `crates/amazing-css/evidence/css36/`.
Native source/target on Boat, retained outputs fetched immediately.

Consumer request: box_tree's current helper reads only bare numbers and ex-height.
Adopt all metric variants, obtain first-available-font ratios and geometry for
context math, and publish resolved values through the normal computed-style cache.
The seven remaining font-size-adjust computed assertions require six loaded-font
ratios and one container geometry input; the CSS-only static runner cannot supply
the fixture's @font-face/document setup. No fixture-specific metric is invented.
Spec: https://drafts.csswg.org/css-fonts-5/#font-size-adjust-prop.

## CSS-37 — contextual display blockification

CSS Display 3 §2.7–2.8 normalizes computed display for roots, floats,
absolute/fixed positioning and flex/grid children, including children flattened
through nested display:contents. None/contents suppress boxes; root contents
computes to block. Inner flex/grid/table/ruby modes survive, inline flow-root
loses flow-root as required, and layout-internal types become block flow.
Absolute/fixed positioning computes float:none. Contextual fixups preserve
winning author provenance and synchronize string/typed computed values.

Public blockified_display(&str)->String supports layout synthetic styles;
ComputedStyle::display_before_blockification()->Option<&str> preserves the
pre-fixup value for CSS2 hypothetical static positions. Context-free resolve
and synthetic styles keep this metadata absent. Nearest generated parent display
is carried privately across contents and incremental cached styles.

Static WPT computed targets now use html/body/div rather than making div the
root. Baseline overlays exactly this runner on CSS36 3347187d; both sides use
identical 15-selection fixtures and skipped nonliteral-expected arguments.
Grouped tests cover all display families, contexts, nested contents and
incremental parent display changes. Three pre-existing inline/ruby test fixtures
were corrected to target an actual descendant rather than the document root.
Spec: https://drafts.csswg.org/css-display-3/#transformations

Final CSS37 Boat gate: workspace3534passes/17skips +4docs, CSS375.
Identical corrected 15-selection harness3779→3783/5126 (+4/lost0).
Full display27/35, tables59/82, sizing91/138, contain108/229; all484
classifications unchanged/lost0. Evidence css37. Layout consumer adoption
routed; next area is complete unordered outer/inner/list-item display grammar.

## CSS-38 — complete CSS Display 3 keyword grammar

Parses the unordered outer/inner pair and the optional list-item flag as one
area: both keyword orders and all six permutations of three keywords, default
outer/inner axes, root/position/flex contextual fixups, and duplicate/incompatible
component rejection. Computed text uses legacy shortest aliases, canonical
outer/inner order and preserves the marker flag. Typed display includes run-in
inner variants and all six list-item outer/inner forms. The public
is_list_item_display helper supports generated marker styles for compound
values; normalizing marker parents continues to use ordinary CSS inheritance.

Run-in reparenting/inlinification and compound list-item layout consumers are
routed to layout as a separate box-tree scope. CSS grammar does not pretend
to implement those geometry algorithms. Existing grid-lanes serialization is
retained to avoid breaking its installed consumers; the pinned tentative
Display4 computed fixture wants inline grid-lanes rather than the existing
inline-grid-lanes alias and remains a documented serialization mismatch.
Spec: https://drafts.csswg.org/css-display-3/#the-display-properties

Final CSS38 on base452323a9: CSS378, Boat workspace3537passes/17skips
+4docs; identical15selection3783→3928/5126 (+145 exact/lost0).
Full display27/35,tables59/82,sizing91/138,contain108/229 all484 unchanged/lost0.
Evidence css38. Continue CSS Text policy validation cluster.

## CSS-39 — CSS Text 4 policy grammar and overlapping shorthands

Bulk keyword grammar for text transforms, whitespace collapse/trim, wrapping
mode/style, autospace/spacing trim, breaking/hyphenation, hanging punctuation,
word-space-transform, text justification/group alignment and wrap controls.
White-space, text-wrap and text-spacing expand into independently cascading
axes, including deferred var() values and CSS-wide defaulting. Computed aliases
are reconstructed from the winning inherited longhands; white-space and
text-wrap share only mode, and white-space never resets wrap style. Trim is
non-inherited. Legacy whitespace aliases remain available to existing layout
consumers; string/typed alias synchronization also runs after inheritance and
layout-time synthesis. word-wrap is a true alias of overflow-wrap.

Grammar validates duplicate/conflicting flags, canonical order and explicit
insert/replace exclusivity. New policies carry metadata and layout invalidation
severity. This commit exposes policy values; new autospace, trim, balancing,
run-in wrapping and transform shaping algorithms remain lower consumer work,
not fabricated CSS parsing passes. Spec: https://drafts.csswg.org/css-text-4/

Final CSS39 fixed CSS38 base56703574: CSS382, Boat workspace3541passes/17skips
+4docs; identical15selection3928→4105/5126 (+177 exact/lost0).
Whole css-text235→238/445 (+3/lost0): overflow-wrap-anywhere-span-001,
overflow-wrap-break-word-span-001 and text-wrap-nowrap-001. Renderer SHA256
3637ff6fe6be9b87f004549b3a4e228399d1dca9bf32869b60147db6e9b115d3.
Evidence css39. Completed own PPM scratch removed after maps/reports/logs fetch;
shared native targets and HOME legacy cache left intact. Next font-variant area.

### CSS40 — whole font-variant feature family

Fixed base CSS39 `209bcc867a3fca80d5333e765371277305f2fa86`.
CSS Fonts4 §6.4–6.11, §9.3 and §13: seven inherited axes independently cascade;
normal resets all, none resets all with ligatures disabled; low-level features,
variation and language override survive. Full feature conflicts/functions and
reserved custom-ident validation, deferred variables, font reset and derived
round-trip summary implemented in dedicated bounded modules. Axis serializers
follow grammar order and preserve case-sensitive alternate feature names.
Static WPT runner quoted literal chains fixed and SAME runner overlaid on base
before measurement; never accept a malformed concatenation as CSS.
Provider callback/physical font-size-adjust metrics queued CSS41 with layout;
actual-font FromFont resolution must happen before inheritance.

CSS40 final: crate385+example1, workspace3544+4docs; parsing4105→4126/5126 (+21/lost0), wholecss-fonts24/32 before/after (32 unchanged/lost0). Renderer SHA256 b0b5a218de8d81d423dac523cdee56f5c5a5d9410c194404e25671201e2888be. Evidence `crates/amazing-css/evidence/css40/`.

### CSS41 — actual first-font normalization context

Fixed base CSS40 `da0e41f05f7d5a5e458bb5747db0a3bebf427bca`. Parent grants
narrow `amazing-text-provider` physical metric consumer; provider remains free
of CSS dependencies. `FontMetricContext::aspect_ratio` takes element/pseudo,
computed selection and typed metric; `contextual_length_px` optionally supplies
actual glyph/context units in numeric math. Full/animation/pseudo and partial/
full incremental recompute resolve before descendants inherit. Inherited aspects
never re-select a child's different font, including unresolved symbolic math.
Missing actual first font produces none; no context explicitly retains symbolic
values. Driver must install context before initial style and keep it on every
recompute; root owns actual font loading and all driver/CSSOM adapters.
Provider query `font_size_adjust_ratio(TextStyle, FontSizeAdjustMetric)` measures
physical U+0030 horizontal width and U+6C34 horizontal/vertical advances in fixed
orientations, using standard missing-glyph fallbacks. Existing dynamic
`adjusted_font_size` reuses the same ratios; no text trait extension needed.
Grouped regression fixtures distinguish 0.5em width from 1em height over all
five writing modes/three text orientations, inheritance/pseudos/font reselection,
actual glyph-unit math, no-font defaults, animation and incremental rebuild.

CSS41 final: workspace3550/17skip+4docs; parsing4126/5126unchanged/lost0; fonts24/32unchanged/lost0. Six grouped regressions, including actualanisotropic metrics atzero CSSsize andunresolvedinheritedmathprovenance. Evidence `crates/amazing-css/evidence/css41/`. Driveradapter root-owned pending; no installed FromFont claim.

### CSS42 — whole border-image family

Fixed CSS41 `b90e214731d495100f828e0b52c2dc900566e944`. CSS Backgrounds3 §5
(https://drafts.csswg.org/css-backgrounds-3/#border-images): five noninherited
axes and unordered atomic shorthand groups, strict slash/fill/numeric/image
grammar, independent cascade/defaults/deferred variables and global-border reset.
Side borders preserve image axes. Pure arithmetic, percentages and actual
em/rem/absolute lengths resolve before typed values; contextual units remain
explicit. Cross-sheet variable inheritance retains the parent's image URL base.
No painter consumer exists yet; route to paint owner after CSS commit. Existing
static WPT runner omits fixture fonts/containing geometry; no fabricated contexts.
Evidence `crates/amazing-css/evidence/css42/`; final bulk gate pending.

CSS42 final: CSS393, workspace3553/17skip+4docs; identical15selection4126→4163/5126 (+37/lost0); wholebackgrounds75/134 unchanged/all134stable/lost0. Renderer215feb8660fefe52a17351ba78709d51c2b3957737443d1cbecd1396ba3a6e6c. Own completed PPM scratch removed after durablemaps/logs fetched; sharedtargets preserved. Next positions/origin grammar family.

### CSS43 — position and origin family

Fixed CSS42 `6cd6ffe5168edf069e7a5e7a1a5f8978c1e484cc`. CSS Values4 §8.3
(https://drafts.csswg.org/css-values-4/#position) excludes background-only
three-value syntax and disallows mixed numeric/keyword axis reordering.
CSS Transforms1 §4 (https://drafts.csswg.org/css-transforms-1/#transform-origin-property)
restricts Z to a length and X/Y to one/two-value grammar; Transforms2 §9
(https://drafts.csswg.org/css-transforms-2/#perspective-origin-property) permits
generic four-value edge offsets. Transform-box registers all five references.
Canonical computed axes resolve physical keywords and actual em/rem/absolute
lengths, preserve percentages and unknown metric units until their real context,
and omit zero Z. Defaults transform50%50%, perspective50%50%, boxview-box.
Parent/layout L44 owns actual reference rectangles and origin consumption.
Old painter image.rs explicitly rejects calc; object-position complements
require its precise parser consumer before final zero-loss rendering gate.

Parent precisely approved image.rs::position parser consumer plus grouped tests.
CSS exposes PositionAxis fractions/pixel offsets, affine_position_axis/pair and
resolve(actual_basis); no context guesses and no nonlinear sampling. Painter
retains same actual leftover geometry under contain/cover and legacy edge inputs.
Four CSS grouped regressions plus one painter grouped regression; origin effect
hooks remain L44-owned. InitialCSS396green; parsing4163→4215/5126 (+52/lost0).
Combined final workspace/wholeimages+transforms gate active.

CSS43 final: workspace3558/17skip+4docs; parsing4163→4215/5126 (+52/lost0), wholeimages92/141 andtransforms145/348 unchanged/all489stable/lost0. Rendererabaef3139e14cb76403fff329403f0c535d6130f45de53b31d7fbd2db2dbf7d2. Fivegroupedregressions (fourCSS/onepainter). Next additive CSSOM computed enumeration/serialization seam requested by driver ownerjs_date.

### CSS44 — computed CSSOM/document and used-position seams

Fixed CSS43 `56877eaffa0b8dfc7fb7e184a151959376c17ceb`. Driver owner js_date
requested public CSSOM declaration/value APIs and native document context.
Spec https://drafts.csswg.org/cssom/#dom-window-getcomputedstyle and
https://drafts.csswg.org/cssom/#dom-cssstyledeclaration-getpropertyvalue.
Supported longhands/valid custom enumeration excludes shorthand aliases and
unknown literal passthrough; preserve custom case/valid empty values and omit
invalid/cycles. Independent quad/pair/side-border and existing derived summaries
serialize through lower CSS; unsupported/unrepresentable returnsNone. Dynamic
logical border axes included; combined background cross-axis ordering remains
explicitly unsupported. Empty custom parsing/supports guards fixed without
allowing ordinary empty values. Native-document font-context computation keeps
arena indices, scoped active sheets/body hints/generated counters, inherited
actual aspects, pseudos and invalid-root checks. Driver owns resolved geometry,
color and WebIDL behavior.
Layout L44 found calc-only affine API literal gap: explicit px/%/zero handling
added with finite/negative/overflow cases. used_position_axis(css,actual_basis)
adds true piecewise calc/min/max/clamp; unresolved metric/container units rejected.
Core math reversed clamp bounds use CSS minimum precedence instead of panic.
Six grouped regressions. Final workspace+15directory+489render gate active.
Next CSS45 additive IDL property-name/shorthand-name enumeration, requested
after fixed CSS44 gate began; no driver duplicated CSS grammar.

CSS44 final: CSS403/workspace3564/17skip+4docs; parsing4215/5126 unchanged/lost0; images92/141 andtransforms145/348 unchanged/all489stable/lost0. Renderer8095e96bd006d5df0c4f3f262bc37a7b0624c12dc61c728d04e97a25e383d353. Six grouped regressions; installed CSSOM remains driver-owned, actual origin effect geometry layout-owned.

### CSS45 — CSSOM property and shorthand registry

Fixed CSS44 `d7bd30024de2649457396f5dcd752c95935375fb`. Driver owner js_date
requested IDL names including shorthand accessors. Public cssom_property_names
returns sorted/deduplicated supported ordinary names; cssom_shorthand_names
exposes recognized shorthand names. Projection reuses lower expansion for
font/background/grid/flex/animation reset axes. Serialization stays independent:
unsupported composite getters remain None, never concatenate reset axes.
Existing grouped regression expanded for names/order/projection/boundaries.
Frozen final workspace/docs + identical15parsing + wholeimages/transforms gate.

CSS45 final: workspace3564/17skip+4docs; parsing4215/5126 unchanged/lost0; wholeimages92/141 andtransforms145/348 all489classifications unchanged/lost0. Renderer0db48706510d198e73d827b94e128d6b482e7af522b149653f3e706474b245c4. Installed CSSOM proof remains driver-owned.

### CSS46 — whole font shorthand grammar and computed serialization

Fixed CSS45 d9dbf6b9. CSS Fonts4 https://drafts.csswg.org/css-fonts-4/#font-prop.
Single lower parser validates/expands font optional unordered categories, adjacent
oblique angles, decimal weights, legacy keyword widths, complete math size/height
components, and quoted/identifier comma-separated families. Invalid declarations
are rejected atomically rather than retained as raw shorthand; substitutions still
resolve after cascade and invalid winning substitutions default all reset axes.
System keywords use permitted UA default fallback; no OS preference query exists.
CSSOM serializes actual component axes only with representable reset-only state,
converts representable width percentages to legacy keywords, refuses unknown widths
and unresolved relative weights. Font synthesis/palette remain independent.
Three grouped regressions; old invalid-raw compatibility expectations corrected.
IterationCSS406 green;4231/5126 (+16/lost0), wholefonts24/32 unchanged.
Frozen full workspace/docs + identical15directory + wholefonts final gate.

CSS46 final: CSS406/workspace3567/17skip+4docs;4215→4231/5126 (+16/lost0);wholefonts24/32 unchanged/all32stable/lost0. Renderer9353f73d5723d0609532e188c7eff5c2350b62e259607680772ac308585a2439. Next coherent selection-value normalization (weight/style/width/family), preserving unavailable metric/context values.

### CSS47 — basic font selection grammars and computed values

Fixed CSS46 8c45bd1d. CSS Fonts4 https://drafts.csswg.org/css-fonts-4/.
Validate complete weight/style/stretch/family values atomically; actual parent
relative weights, decimal/pure math numeric computation/range clamping, oblique
angle conversion/pure math computation and safe family serialization. Width
keywords normalize to percentages; font-width aliases stretch cascade/animation
axis and CSSOM. Number/angle/percentage evaluation reuses existing typed math
without inventing geometry. Sign/container expression typing keeps unavailable
context unresolved; Values5 random-item compatibility remains Unparsed.
First same-harness diff identified23 valid losses; whole known-function/type
cluster fixed before final, grouped regression covers these boundaries.
IterationCSS409 green,+25 parsing/lost0 and all32font renders stable.
NextCSS48 native-document/font-context computed pseudo seam requested by js_date;
layout generated/applicability map remains separate.

Parent granted one driver assertion correction: phase2_style.rs font-weight bold→700 records computed normalization, retaining selector/cascade semantics. First workspace had3569pass/1obsolete expectation; full corrected-source gate rerun.

CSS47 final: CSS409/workspace3570/17skip+4docs;4231→4256/5126 (+25/lost0);wholefonts24/32 unchanged/all32stable/lost0. Renderer8eee72f3b7304afd70ba361b5e213b8af37a418076bfd9baa930d9ee4d1ec6a3.

### CSS48 — native-document computed pseudo styles

CSSOM https://drafts.csswg.org/cssom/#dom-window-getcomputedstyle and CSS Pseudo4 https://drafts.csswg.org/css-pseudo-4/. Compute actual recognized pseudo cascade without conditioning on generated boxes; selected native-document sheets, principal inheritance, root units, display context and actual font callback. Generation map unchanged. Backdrop fully styleable/current inheritance with Position4 UA fixed/inset default. Three grouped regressions. Specialized selection parent-highlight inheritance, limited property applicability and pseudo animation samples remain explicit follow-ons; actual used dimensions belong to driver/layout. Fixed CSS47 ea3d491b; identical15-dir and wholefonts zero-loss gate.

CSS48 final: CSS412/workspace3573/17skip+4docs;4256/5126 unchanged/lost0;wholefonts24/32/all32stable/lost0. Renderer6e412187daa5d8fa0b76136f1e8ba5bc0dfdd38f088a05cc03d443506c99afaf. Driver CSSOM adoption/gains separate; next lower gradient computed-value family (largest43-case cluster).

### CSS49 — gradient computed-value families

Fixed CSS48 865ffd50; CSS Images3/4 grammar and serialization. Whole linear/radial/conic and repeating families validate complete stop lists/hints/header, canonicalize positions/defaults and basic typed colors while retaining functional color precision. Computed image contexts resolve actual font/root and absolute lengths; percentages/unavailable contextual units remain symbolic. No paint edits or percentage basis guesses. Largest remaining43-case gradient-position cluster. Three grouped regressions; same15-dir plus entire image/fonts/transforms render gate.

CSS49 final: CSS415/workspace3576/17skip+4docs;4256→4312/5126 (+56/lost0), including all43gradient-position cases. Exact48 before/after wholeimages92/141,fonts24/32,transforms145/348, all521 classifications unchanged/lost0. Rendererea0471b193bef8b4ac05765a4317ad3966c324b515c717441d4908ff8853b941. Next current generic() family correction from installed CSSOM diagnostic, then shadow boundary50.

### CSS47A — current generic-family grammar correction

Installed CSSOM dependency audit found one lost generic(fangsong) subtest after47. Fonts4 https://drafts.csswg.org/css-fonts-4/#generic-family-value defines four script functions fangsong/kai/khmer-mul/nastaliq and current bare emoji/fangsong as named custom-ident families. Whole grammar and canonical computed/shorthand serialization corrected, invalid args/prefix functions rejected, quoted meaningful whitespace retained. Font-only source independent of49; lower fixed49 workspace/docs+15dirs+fonts final, driver191 installed boundary separate. Actual script-specific installed font mapping is a provider policy, not claimed by parsing.

CSS47A final: CSS416/workspace3577/17skip+4docs;4312/5126 unchanged/lost0;wholefonts24/32/all32stable/lost0. Renderer02a32cfff71c698b03ba26b1bbf7eb0eb406d43c8a14a33764323860791c1425. Independent font source dependency supplied to installed191 CSSOM owner.

### CSS50 — native shadow style boundaries

CSS Shadow1 https://drafts.csswg.org/css-shadow-1/ and Cascade4 flat-tree value processing. Additive explicit scoped principal/pseudo entry points preserve native IDs and driver actual font context; document ordinary selectors cannot enter shadow scopes. Attached open/closed/nested host inheritance, named/manual immediate slot inheritance, UA slot display:contents, flattened membership and nested Document isolation. Existing APIs preserved. :host/::slotted, scoped named resources and actual shadow layout/generated counters remain explicit follow-ons.

CSS50 final: CSS419/workspace3580/17skip+4docs;4312/5126 unchanged/lost0;wholeimages92/141,fonts24/32,transforms145/348 all521stable/lost0. Rendererfdda8fb4d48b2a1a1cb669b0db16f3fbd4ce342a685542c1f2e3db89776bfd6b. Driver scoped collection adoption/installed shadow gains remain separate.
