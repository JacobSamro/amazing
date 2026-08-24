# Positioned and out-of-flow layout — ticket #3a

Measured 2026-08-24 on `win-001` against upstream WPT revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800×600 CSS pixels,
device scale 1, using Amazing fallback metrics and `RectInkProvider`.

## Gate

`position-manifest.json` is the fixed 12-case HTML-only Position projection
from ticket #3's deterministic geometry manifest. It includes absolute,
relative, fixed, sticky, block, Flex, Grid, table-role, and semi-replaced
scenarios without adding tests chosen after implementation.

| Result | Before | After |
|---|---:|---:|
| Pass | 5 | 9 |
| Mismatch | 7 | 3 |
| Unsupported / timeout / crash | 0 | 0 |

Four tests changed mismatch → pass, each reaching zero differing pixels:

- relative positioning of a table caption;
- relative table-header-group offset with an absolute child;
- a very large negative absolute inset;
- relative table-footer-group offset with an absolute child.

Two after-runs produced identical classifications and image hashes. Stable
projection SHA-256:
`c45488778a032ad886550a74ef673c8dd8b4ae0f93bbdfebb6338b05c7c6bd9d`.

Exact-revision wpt.fyi data for Blink Chrome 154 and WebKit Safari 26.6 agrees
on five cases ([combined run query](https://wpt.fyi/api/search?run_ids=5182906107691008,5141363439173632&q=/css/css-position/)).
Amazing moved from 3/5 to 5/5 in that consensus subset. The remaining three
Amazing mismatches are table-row/footer relative-offset semantics which WebKit
also reports failing at this revision; they remain measured rather than being
treated as agreement.

The broad HTML-only geometry corpus moved 35/26 → 38/23. Four Position cases
became passes; one former inert `display:contents` out-of-flow pass now exposes
the missing unboxing behavior. The block-collapse family remains 9/9.

## Interface decision

The throwaway terminal prototype validated a two-pass tree and was deleted
after absorption. `LayoutBox` now owns normal `flow` children and a separate
`positioned` list. Box construction removes absolute/fixed descendants before
block, inline, Flex, or Grid item layout. `layout_root` performs normal flow
first, then one positioning module:

- relative boxes shift their complete painted subtree without changing sibling
  slots;
- absolute boxes use the nearest positioned ancestor's padding box;
- fixed boxes use a definite viewport supplied by WPT, GPUI, or Win32;
- percentages resolve against the correct containing-block axis;
- dual insets stretch auto width/height after padding, border, and margins;
- negative insets remain negative and LTR/RTL over-constraints select the
  stronger side;
- auto insets use a static anchor after the preceding normal-flow block.

Inline-block/table roles are currently blockified so they retain boxes and can
establish containing blocks. This is more faithful than flattening them but is
not full atomic-inline or table formatting.

Positioned descendants participate in document-order paint traversal, dumps,
Win32 hit testing, GPUI link/control regions, and scrollable overflow. Fixed
boxes do not extend document height. A positioned-link GPUI regression proves
painted absolute content remains interactive.

## Remaining positioning work

- exact static-position rectangles for inline, Flex, and Grid alignment;
- shrink-to-fit and intrinsic absolute sizes, replaced-element sizing, and
  `box-sizing:border-box` used-value integration;
- sticky scroll constraints;
- full table formatting and the remaining relative row/footer semantics;
- transforms/containment as containing-block creators;
- z-index and stacking contexts (ticket #6).

## Assets

- `tests/wpt-render/position-manifest.json`
- `tests/wpt-render/position-results.json`
- `tests/wpt-render/position-oracles.json`
- `tests/wpt-render/geometry-results-positioned.json`
