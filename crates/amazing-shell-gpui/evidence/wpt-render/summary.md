# SHELL-L4 fleet-wide Linux render baseline

Pinned WPT revision, viewport and font set: see each category's own `report.json`. 61 categories with at least one eligible self-contained static reftest.

| Category | Lane | Eligible | Pass | Mismatch | Error | Timeout | Unsupported | Crash |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `css/CSS2` | css | 175 | 121 | 54 | 0 | 0 | 0 | 0 |
| `css/compositing` | replaced-content | 34 | 18 | 16 | 0 | 0 | 0 | 0 |
| `css/css-align` | layout | 34 | 6 | 28 | 0 | 0 | 0 | 0 |
| `css/css-anchor-position` | layout | 67 | 3 | 64 | 0 | 0 | 0 | 0 |
| `css/css-animations` | replaced-content | 14 | 5 | 9 | 0 | 0 | 0 | 0 |
| `css/css-backgrounds` | replaced-content | 133 | 72 | 61 | 0 | 0 | 0 | 0 |
| `css/css-borders` | replaced-content | 54 | 16 | 38 | 0 | 0 | 0 | 0 |
| `css/css-box` | layout | 38 | 3 | 35 | 0 | 0 | 0 | 0 |
| `css/css-break` | layout | 226 | 57 | 169 | 0 | 0 | 0 | 0 |
| `css/css-cascade` | css | 20 | 16 | 4 | 0 | 0 | 0 | 0 |
| `css/css-color` | replaced-content | 236 | 226 | 10 | 0 | 0 | 0 | 0 |
| `css/css-color-adjust` | replaced-content | 2 | 0 | 2 | 0 | 0 | 0 | 0 |
| `css/css-conditional` | css | 64 | 26 | 38 | 0 | 0 | 0 | 0 |
| `css/css-contain` | layout | 235 | 111 | 124 | 0 | 0 | 0 | 0 |
| `css/css-content` | layout | 40 | 4 | 36 | 0 | 0 | 0 | 0 |
| `css/css-counter-styles` | layout | 177 | 153 | 24 | 0 | 0 | 0 | 0 |
| `css/css-display` | layout | 34 | 28 | 6 | 0 | 0 | 0 | 0 |
| `css/css-flexbox` | layout-flexgrid | 520 | 340 | 180 | 0 | 0 | 0 | 0 |
| `css/css-fonts` | text | 32 | 22 | 10 | 0 | 0 | 0 | 0 |
| `css/css-forms` | layout | 12 | 2 | 10 | 0 | 0 | 0 | 0 |
| `css/css-gaps` | layout-flexgrid | 357 | 10 | 347 | 0 | 0 | 0 | 0 |
| `css/css-grid` | layout-flexgrid | 943 | 346 | 597 | 0 | 0 | 0 | 0 |
| `css/css-images` | replaced-content | 141 | 85 | 56 | 0 | 0 | 0 | 0 |
| `css/css-inline` | layout | 7 | 1 | 6 | 0 | 0 | 0 | 0 |
| `css/css-layout-api` | layout | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| `css/css-lists` | layout | 135 | 91 | 44 | 0 | 0 | 0 | 0 |
| `css/css-logical` | layout | 5 | 1 | 4 | 0 | 0 | 0 | 0 |
| `css/css-masking` | replaced-content | 103 | 40 | 63 | 0 | 0 | 0 | 0 |
| `css/css-multicol` | layout | 103 | 44 | 59 | 0 | 0 | 0 | 0 |
| `css/css-nesting` | css | 12 | 5 | 7 | 0 | 0 | 0 | 0 |
| `css/css-overflow` | layout | 335 | 79 | 256 | 0 | 0 | 0 | 0 |
| `css/css-page` | layout | 189 | 86 | 103 | 0 | 0 | 0 | 0 |
| `css/css-position` | css | 54 | 35 | 19 | 0 | 0 | 0 | 0 |
| `css/css-properties-values-api` | css | 6 | 3 | 3 | 0 | 0 | 0 | 0 |
| `css/css-pseudo` | css | 89 | 37 | 52 | 0 | 0 | 0 | 0 |
| `css/css-rhythm` | layout | 10 | 1 | 9 | 0 | 0 | 0 | 0 |
| `css/css-ruby` | text | 67 | 23 | 44 | 0 | 0 | 0 | 0 |
| `css/css-scroll-snap` | layout | 7 | 1 | 6 | 0 | 0 | 0 | 0 |
| `css/css-scrollbars` | layout | 9 | 9 | 0 | 0 | 0 | 0 | 0 |
| `css/css-shadow` | replaced-content | 35 | 0 | 35 | 0 | 0 | 0 | 0 |
| `css/css-shapes` | replaced-content | 101 | 7 | 94 | 0 | 0 | 0 | 0 |
| `css/css-sizing` | layout | 129 | 49 | 80 | 0 | 0 | 0 | 0 |
| `css/css-tables` | layout | 82 | 49 | 33 | 0 | 0 | 0 | 0 |
| `css/css-text` | text | 446 | 218 | 227 | 0 | 0 | 0 | 1 |
| `css/css-text-decor` | text | 192 | 65 | 127 | 0 | 0 | 0 | 0 |
| `css/css-transforms` | replaced-content | 329 | 145 | 184 | 0 | 0 | 0 | 0 |
| `css/css-ui` | css | 84 | 56 | 28 | 0 | 0 | 0 | 0 |
| `css/css-values` | css | 108 | 63 | 45 | 0 | 0 | 0 | 0 |
| `css/css-variables` | css | 161 | 124 | 37 | 0 | 0 | 0 | 0 |
| `css/css-view-transitions` | css | 3 | 2 | 1 | 0 | 0 | 0 | 0 |
| `css/css-viewport` | layout | 79 | 52 | 27 | 0 | 0 | 0 | 0 |
| `css/css-will-change` | css | 37 | 4 | 33 | 0 | 0 | 0 | 0 |
| `css/css-writing-modes` | text | 153 | 50 | 103 | 0 | 0 | 0 | 0 |
| `css/cssom-view` | css | 1 | 0 | 0 | 0 | 0 | 0 | 1 |
| `css/fill-stroke` | replaced-content | 1 | 0 | 1 | 0 | 0 | 0 | 0 |
| `css/filter-effects` | replaced-content | 121 | 65 | 56 | 0 | 0 | 0 | 0 |
| `css/mediaqueries` | css | 3 | 1 | 2 | 0 | 0 | 0 | 0 |
| `css/motion` | css | 76 | 0 | 76 | 0 | 0 | 0 | 0 |
| `css/printing` | layout | 20 | 14 | 6 | 0 | 0 | 0 | 0 |
| `css/selectors` | css | 89 | 79 | 9 | 0 | 0 | 0 | 1 |
| `html/rendering` | replaced-content | 198 | 130 | 68 | 0 | 0 | 0 | 0 |

| **Total** | | **7168** | **3299** | **3866** | **0** | **0** | **0** | **3** |
