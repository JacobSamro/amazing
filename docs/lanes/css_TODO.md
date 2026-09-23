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
