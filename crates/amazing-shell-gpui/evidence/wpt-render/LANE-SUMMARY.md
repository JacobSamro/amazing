# Linux render baseline map

SHELL-L8: fleet-wide summary of every pinned-WPT `css/*` subdir (plus the `svg/*` and `html-rendering` categories SHELL-L7 added) that the shell lane's `scripts/linux-render-wpt.sh` has attempted, with the owning lane and eligible/pass/mismatch counts. "attempted, 0 eligible" means the subdir was run through the discovery pipeline but contains no static, self-contained reftest this headless-render tool can score (a testharness.js-only API surface, or a reftest whose state change is driven by `<script>`, e.g. css-transitions/css-animations/css-motion-path) -- no evidence directory is committed for those, since there is nothing to baseline.

| Subdir | Owning lane | Eligible | Pass | Mismatch | Status |
|---|---|---|---|---|---|
| css-compositing | compositor | 34 | 18 | 16 | baselined |
| css-css-animations | compositor | 14 | 5 | 9 | baselined |
| css-css-easing | compositor | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-transforms | compositor | 329 | 145 | 184 | baselined |
| css-css-transitions | compositor | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-view-transitions | compositor | 3 | 2 | 1 | baselined |
| css-css-will-change | compositor | 37 | 4 | 33 | baselined |
| css-CSS1 | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-CSS2 | css | 175 | 121 | 54 | baselined |
| css-css-cascade | css | 20 | 16 | 4 | baselined |
| css-css-conditional | css | 64 | 26 | 38 | baselined |
| css-css-env | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-link-params | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-mixins | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-namespaces | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-nesting | css | 12 | 5 | 7 | baselined |
| css-css-parser-api | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-position | css | 54 | 35 | 19 | baselined |
| css-css-properties-values-api | css | 6 | 3 | 3 | baselined |
| css-css-pseudo | css | 89 | 37 | 52 | baselined |
| css-css-style-attr | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-syntax | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-typed-om | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-ui | css | 84 | 56 | 28 | baselined |
| css-css-values | css | 108 | 63 | 45 | baselined |
| css-css-variables | css | 161 | 124 | 37 | baselined |
| css-cssom | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-cssom-view | css | 1 | 0 | 0 | baselined |
| css-fetching | css | 0 | 0 | 0 | attempted, 0 eligible |
| css-mediaqueries | css | 3 | 1 | 2 | baselined |
| css-motion | css | 76 | 0 | 76 | baselined |
| css-selectors | css | 89 | 79 | 9 | baselined |
| css-css-align | layout | 34 | 6 | 28 | baselined |
| css-css-anchor-position | layout | 67 | 3 | 64 | baselined |
| css-css-box | layout | 38 | 3 | 35 | baselined |
| css-css-break | layout | 226 | 57 | 169 | baselined |
| css-css-contain | layout | 235 | 111 | 124 | baselined |
| css-css-content | layout | 40 | 4 | 36 | baselined |
| css-css-counter-styles | layout | 177 | 153 | 24 | baselined |
| css-css-device-adapt | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-display | layout | 34 | 28 | 6 | baselined |
| css-css-exclusions | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-forms | layout | 12 | 2 | 10 | baselined |
| css-css-gcpm | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-inline | layout | 7 | 1 | 6 | baselined |
| css-css-layout-api | layout | 1 | 0 | 1 | baselined |
| css-css-lists | layout | 135 | 91 | 44 | baselined |
| css-css-logical | layout | 5 | 1 | 4 | baselined |
| css-css-multicol | layout | 103 | 44 | 59 | baselined |
| css-css-navigation | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-overflow | layout | 335 | 79 | 256 | baselined |
| css-css-overscroll-behavior | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-page | layout | 189 | 86 | 103 | baselined |
| css-css-rhythm | layout | 10 | 1 | 9 | baselined |
| css-css-round-display | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-scroll-anchoring | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-scroll-snap | layout | 7 | 1 | 6 | baselined |
| css-css-scrollbars | layout | 9 | 9 | 0 | baselined |
| css-css-sizing | layout | 129 | 49 | 80 | baselined |
| css-css-tables | layout | 82 | 49 | 33 | baselined |
| css-css-viewport | layout | 79 | 52 | 27 | baselined |
| css-css-zoom | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-geometry | layout | 0 | 0 | 0 | attempted, 0 eligible |
| css-printing | layout | 20 | 14 | 6 | baselined |
| css-css-flexbox | layout-flexgrid | 520 | 340 | 180 | baselined |
| css-css-gaps | layout-flexgrid | 357 | 10 | 347 | baselined |
| css-css-grid | layout-flexgrid | 943 | 346 | 597 | baselined |
| css-css-backgrounds | replaced-content | 133 | 72 | 61 | baselined |
| css-css-borders | replaced-content | 54 | 16 | 38 | baselined |
| css-css-color | replaced-content | 236 | 226 | 10 | baselined |
| css-css-color-adjust | replaced-content | 2 | 0 | 2 | baselined |
| css-css-color-hdr | replaced-content | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-forced-color-adjust | replaced-content | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-image-animation | replaced-content | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-images | replaced-content | 141 | 85 | 56 | baselined |
| css-css-masking | replaced-content | 103 | 40 | 63 | baselined |
| css-css-motion-path | replaced-content | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-paint-api | replaced-content | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-shadow | replaced-content | 35 | 0 | 35 | baselined |
| css-css-shapes | replaced-content | 101 | 7 | 94 | baselined |
| css-fill-stroke | replaced-content | 1 | 0 | 1 | baselined |
| css-filter-effects | replaced-content | 121 | 65 | 56 | baselined |
| html-rendering | replaced-content | 198 | 130 | 68 | baselined |
| svg-animations | replaced-content | 2 | 0 | 2 | baselined |
| svg-coordinate-systems | replaced-content | 7 | 6 | 1 | baselined |
| svg-extensibility | replaced-content | 13 | 4 | 9 | baselined |
| svg-geometry | replaced-content | 21 | 21 | 0 | baselined |
| svg-layout | replaced-content | 8 | 0 | 8 | baselined |
| svg-linking | replaced-content | 8 | 3 | 5 | baselined |
| svg-painting | replaced-content | 16 | 12 | 4 | baselined |
| svg-path | replaced-content | 19 | 19 | 0 | baselined |
| svg-print | replaced-content | 1 | 1 | 0 | baselined |
| svg-render | replaced-content | 7 | 6 | 1 | baselined |
| svg-shapes | replaced-content | 22 | 18 | 4 | baselined |
| svg-struct | replaced-content | 37 | 27 | 10 | baselined |
| svg-styling | replaced-content | 32 | 26 | 6 | baselined |
| svg-text | replaced-content | 22 | 20 | 2 | baselined |
| css-css-font-loading | text | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-fonts | text | 32 | 25 | 7 | baselined |
| css-css-highlight-api | text | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-ruby | text | 67 | 23 | 44 | baselined |
| css-css-size-adjust | text | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-speech | text | 0 | 0 | 0 | attempted, 0 eligible |
| css-css-text | text | 446 | 218 | 227 | baselined |
| css-css-text-decor | text | 192 | 65 | 127 | baselined |
| css-css-writing-modes | text | 153 | 50 | 103 | baselined |

## Totals by owning lane

| Lane | Subdirs | Eligible | Pass | Mismatch |
|---|---|---|---|---|
| compositor | 7 | 417 | 174 | 243 |
| css | 25 | 942 | 566 | 374 |
| layout | 32 | 1974 | 844 | 1130 |
| layout-flexgrid | 3 | 1820 | 696 | 1124 |
| replaced-content | 30 | 1340 | 804 | 536 |
| text | 9 | 890 | 381 | 508 |

**Grand total:** 106 subdirs attempted, 7383 eligible reftests, 3465 pass, 3915 mismatch.

