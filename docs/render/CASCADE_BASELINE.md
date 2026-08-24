# Cascade and computed-value baseline

Measured 2026-08-24 on `win-001` against upstream WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` at an 800×600 CSS-pixel
viewport, device scale 1, using Amazing's fallback metrics and
`RectInkProvider`.

This is the pre-ticket-#2a baseline. The computed-value implementation and
post-change evidence are recorded in `tests/wpt-render/computed-values-results.json`
and `tests/wpt-render/cascade-results-ticket2a.json`; the original measurements
below remain unchanged for before/after auditability.

Ticket #3 later proved that 13 selected tests or references are XHTML/XML and
had been parsed incorrectly as HTML. Those historical pixel classifications
are superseded by `tests/wpt-render/cascade-results-xhtml-safe.json`: corrected
baseline 23 pass / 36 mismatch / 13 unsupported, corrected post-#2a 35/24/13,
and current 33/26/13. The Variables 0/12 → 12/12 result is unaffected.

## Method

`scripts/inventory-render-wpt.ps1` recursively inventories CSS Syntax,
Cascade, Selectors, Values, Color, Fonts, and Variables. Within each category
it considers local-reference reftests whose test and reference are script-free,
flag-free, and free of external or embedded resources. It ranks normalized
paths by SHA-256 and selects the lowest 12 hashes per category. This produces a
repeatable stratified corpus without hand-picking tests that suit Amazing.

The pixel harness follows each unmodified test's upstream `rel=match` or
`rel=mismatch` link. This follows the upstream [WPT reftest
model](https://web-platform-tests.org/writing-tests/reftests.html); local
metadata selects tests but cannot redefine their assertion.

| Category | Documents | Testharness | Reftests | Eligible | Selected |
|---|---:|---:|---:|---:|---:|
| CSS Syntax | 49 | 44 | 1 | 0 | 0 |
| Cascade | 158 | 83 | 50 | 31 | 12 |
| Selectors | 684 | 278 | 226 | 83 | 12 |
| Values | 603 | 271 | 211 | 88 | 12 |
| Color | 517 | 62 | 307 | 278 | 12 |
| Fonts | 875 | 163 | 374 | 55 | 12 |
| Variables | 268 | 61 | 182 | 160 | 12 |

CSS Syntax has no self-contained static shard under this policy. Its 44
testharness files require a future DOM assertion runner; no pixel pass rate is
claimed for that category.

## Amazing results

The two full runs produced identical classifications, references, tolerances,
pixel counts, and actual/reference/diff hashes. The stable normalized
projection SHA-256 is
`cc5fc30241a93637dfd7eb8416ad808ce24127be27227f2fdc5d63770a4aed32`.

| Category | Pass | Mismatch | Pass rate |
|---|---:|---:|---:|
| Cascade | 4 | 8 | 33.3% |
| Selectors | 4 | 8 | 33.3% |
| Values | 11 | 1 | 91.7% |
| Color | 5 | 7 | 41.7% |
| Fonts | 6 | 6 | 50.0% |
| Variables | 0 | 12 | 0.0% |
| **Total** | **30** | **42** | **41.7%** |

All 72 cases executed: timeout 0, unsupported 0, crash 0. A pixel pass only
means the upstream test and reference compare as specified in Amazing. It does
not prove support for every feature named by that test. This matters for the
Values shard: Amazing does not parse `calc()`, `ch`, viewport units, or
`attr()`, so several green comparisons are downstream-inert passes rather than
evidence for those value features.

## Blink and WebKit differential oracle

wpt.fyi has exact-revision runs for Blink
[Chrome 154](https://wpt.fyi/api/search?run_ids=5182906107691008&q=/css/css-variables/variable-reference-01.html)
and WebKit
[Safari 26.6](https://wpt.fyi/api/search?run_ids=5141363439173632&q=/css/css-variables/variable-reference-01.html).
Across the selected corpus, both report pass for 64/72 cases. Amazing passes
26 and mismatches 38 inside that cross-engine consensus set:

| Consensus-pass category | Cases | Amazing pass | Amazing mismatch |
|---|---:|---:|---:|
| Cascade | 12 | 4 | 8 |
| Selectors | 12 | 4 | 8 |
| Values | 10 | 9 | 1 |
| Color | 12 | 5 | 7 |
| Fonts | 6 | 4 | 2 |
| Variables | 12 | 0 | 12 |

The eight non-consensus cases are six Fonts cases and two Values cases. Four
legacy font-name tests fail in both Blink and WebKit at this revision, while
four other cases differ between them. They remain in the raw Amazing report
but are excluded when choosing the first convergence primitive. Representative
exact-revision checks confirm Blink and WebKit both pass
[`all-prop-001`](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/css-cascade/all-prop-001.html),
[`lang-016`](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/selectors/selectors-4/lang-016.html),
and [`hsl-003`](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/css-color/hsl-003.html),
where Amazing mismatches.

## Failure clusters

### 1. Computed-value resolution and custom properties

This is the largest coherent first target. Variables mismatch 12/12 while
Blink and WebKit pass 12/12. Six selected cases exercise direct declaration or
reference behavior; six exercise variable-aware `@supports` conditions.
At this baseline, Amazing lowercased every declaration name, although custom-property
names are case-sensitive, preserves most specified values as raw strings, and
has no `var()` substitution, fallback, cycle detection, or invalid-at-computed-
value-time handling. The normative model is defined by [CSS Custom
Properties](https://drafts.csswg.org/css-variables-1/#defining-variables) and
its [substitution algorithm](https://drafts.csswg.org/css-variables-1/#using-variables).

The same missing stage explains much of Cascade's 8/12 mismatch count:
`initial`, `revert`, `revert-layer`, and the `all` shorthand require
property-aware defaulting rather than retaining literal strings. Cascade layers
and scope are additionally discarded because all at-rules are skipped. See
[CSS-wide keywords](https://drafts.csswg.org/css-cascade-5/#defaulting-keywords),
the [`all` shorthand](https://drafts.csswg.org/css-cascade-5/#all-shorthand),
and [cascade layers](https://drafts.csswg.org/css-cascade-5/#layering).

### 2. Selector grammar and matching

Selectors mismatch 8/12. Representative failures require `:lang()`, `:dir()`,
`:nth-child(... of S)`, complex functional-selector arguments, or
pseudo-elements, all documented gaps in the current selector module. These are
specified by [Selectors Level 4](https://drafts.csswg.org/selectors-4/).

### 3. Typed color and length values

Color mismatches 7/12 because paint accepts only hex, a named-color subset,
and `rgb()`/`rgba()`. HSL/HWB, Lab/LCH/OKLab/OKLCH, color spaces, relative
colors, and `color-mix()` require typed parsing and conversion per [CSS Color
4](https://drafts.csswg.org/css-color-4/). Values' raw 11/12 pass must not be
used to prioritize it below Color: code inspection proves the named modern
units/functions are not implemented, and several selected tests exercise
paint/layout features that are inert in both sides under Amazing.

### 4. Fonts are not yet a clean cascade shard

The selection filter cannot infer installed-font prerequisites from prose.
Several selected tests require legacy `CSSTest` fonts; six of twelve are not
Blink/WebKit consensus passes in the exact-revision wpt.fyi runs. Fonts remain
a typography-provider question under ticket #4, not evidence for the first
cascade primitive.

## Decision

Build one explicit specified-to-computed-value stage before adding isolated
property patches. Its first gate is the 12-case Variables shard and the
Cascade cases for CSS-wide keywords. The interface must preserve
case-sensitive custom-property names, inherit custom declarations, substitute
`var()` with fallback and cycle handling, represent invalid-at-computed-value
time, and resolve `initial`/`inherit`/`unset` plus `all` against property
metadata. `revert-layer`, `@layer`, `@scope`, selector gaps, and broader typed
Color/Values remain measured follow-ons rather than being hidden inside this
first change.

## Assets

- `tests/wpt-render/cascade-inventory.json` — full inventory and selected paths
- `tests/wpt-render/cascade-manifest.json` — executable 72-case manifest
- `tests/wpt-render/cascade-results.json` — normalized Amazing pixel evidence
- `tests/wpt-render/cascade-oracles.json` — exact-revision Blink/WebKit evidence
- `scripts/inventory-render-wpt.ps1` — parameterized deterministic selector
