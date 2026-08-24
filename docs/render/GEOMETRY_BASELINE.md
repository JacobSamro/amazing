# Block geometry convergence — ticket #3

Measured 2026-08-24 on `win-001` against upstream WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800×600 CSS pixels,
device scale 1, using Amazing fallback metrics and `RectInkProvider`.

## Measurement correction: XHTML is not HTML

Ticket #3 found that `.xht`/`.xhtml` WPT files were being fed to Amazing's
HTML parser even though WPT serves them as XML. CDATA styles made the mismatch
observable, but every prior XHTML pixel result was invalid regardless of
whether it happened to compare green. The runner now classifies an XHTML test
or reference as `unsupported` until Amazing has an XML parser mode. Inventory
excludes those files from executable static shards.

The corrected 72-case cascade corpus is 33 pass, 26 mismatch, and 13
unsupported. For historical comparison, the XHTML-safe reconstruction is
23/36/13 at the original baseline and 35/24/13 after ticket #2a. The Variables
result remains 12/12 because every selected Variables test/reference is HTML.
`tests/wpt-render/cascade-results-xhtml-safe.json` is authoritative for current
cross-ticket totals; older JSON is retained as audit history.

## Broad HTML-only geometry inventory

The existing deterministic inventory mechanism was extended to accept
case-sensitive category names and reject XML parser modes. It scanned CSS2,
Box, Display, Inline, Position, and Sizing:

| Category | Documents | Reftests | Eligible HTML | Selected | Current pass/mismatch |
|---|---:|---:|---:|---:|---:|
| CSS2 | 11,323 | 6,268 | 120 | 12 | 9 / 3 |
| Box | 179 | 70 | 20 | 12 | 7 / 5 |
| Display | 390 | 213 | 34 | 12 | 6 / 6 |
| Inline | 393 | 179 | 1 | 1 | 1 / 0 |
| Position | 485 | 251 | 32 | 12 | 5 / 7 |
| Sizing | 885 | 562 | 82 | 12 | 7 / 5 |
| **Total** | **13,655** | **7,543** | **289** | **61** | **35 / 26** |

The broad hash-ranked corpus was 36/25 before the change and 35/26 after. The
single change is a former inert pass for inline-table overflow which now
exposes unsupported table-wrapper clipping. None of the focused margin tests
were selected by the broad hash projection, so that count is diagnostic debt,
not the ticket exit gate.

## Focused block-collapse family

Five initially chosen min/max-height comparisons were rejected as a progress
gate because they passed while those properties were absent. The final family
keeps those regression cases but adds three failures that directly exercise
one shared algorithm: signed adjoining margins, negative trailing margins in a
BFC, and collapse through zero-height blocks.

| Result | Before | After |
|---|---:|---:|
| Pass | 6 | 9 |
| Mismatch | 3 | 0 |
| Unsupported / timeout / crash | 0 | 0 |

The three fixed tests each moved from 40,000 differing pixels to zero. Two
after-runs produced identical classifications and image hashes; stable
projection SHA-256:
`f50b778ff7b69d4e5d29b00527303eb3864f0a4f207bd6eb09375272380f0307`.

Exact-revision wpt.fyi data for Blink Chrome 154 and WebKit Safari 26.6 agrees
on five of the nine cases ([combined run query](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/CSS2/normal-flow/)).
Amazing passes all five consensus cases. Safari
reports failure on four recent min/max-height margin tests; they remain in the
raw family but are not used to claim cross-engine agreement. See
`tests/wpt-render/block-collapse-oracles.json`.

## Implementation decision

The throwaway terminal prototype demonstrated that one
`CollapsedMargin { positive, negative }` plus a trailing-margin result can
represent all target states. The shell was deleted after absorption.

`stack_blocks` now returns content extent and trailing collapsed state instead
of a single height. Signed margins combine as the largest positive plus the
most-negative component. Direct empty blocks can preserve a pending adjoining
group. The owning block decides whether the trailing group escapes, is trapped
when min/max-height changes tentative auto height, or contributes inside a BFC.
Simple min/max-height constraints and non-visible-overflow BFC separation use
the same decision.

The last WPT mismatch proved to be paint rather than layout: a correctly placed
red descendant escaped an `overflow:hidden` parent. Paint traversal now carries
intersected padding-box clips through descendant backgrounds, borders, text,
and images. The CSS `background` shorthand also maps a single color into
`background-color`, allowing the upstream red/green failure design to remain
observable.

## Remaining measured geometry work

- out-of-flow absolute/fixed/sticky positioning and relative offsets;
- floats and line interaction;
- `display:contents`, run-in, tables, and margin-trim;
- min/max-width, intrinsic width keywords, `box-sizing`, and containment;
- full recursive parent/child collapse-through beyond direct empty boxes.

## Assets

- `tests/wpt-render/geometry-inventory.json`
- `tests/wpt-render/geometry-manifest.json`
- `tests/wpt-render/geometry-results.json`
- `tests/wpt-render/block-collapse-manifest.json`
- `tests/wpt-render/block-collapse-results.json`
- `tests/wpt-render/block-collapse-oracles.json`
- `tests/wpt-render/cascade-results-xhtml-safe.json`
