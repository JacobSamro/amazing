# LAYOUT-10 fixed-base evidence

The lane began from `origin/dev` `18fefffe06186af30581cef97d763427af839058` and rebased LAYOUT-9 onto it as `6dee58805c371c8843c54ba3b96341c119f5fb2f`. The actual CSS-9 merge in this history is `d2f6d73` (source `4f51acd`); the brief's `e0adf15` is a subsequent NET-18 merge. Only the layout Windows tree was refreshed. The WPT revision is `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, at 800×600. The frozen 53-case calc and 200-case broad manifests are unchanged from LAYOUT-8 and LAYOUT-5. Each baseline and candidate selection was rendered twice from immutable executables, with identical per-case classifications and actual/reference/diff SHA256 hashes within each pair.

- CSS-9-only baseline executable SHA256: `d4d95b3e1c42a546321a5f78b41eb974e0b5b12dffcee5420c3e73a5746fc393`.
- LAYOUT-10 candidate executable SHA256: `48cbe35387ad961719f08a235f95a0b2925141bd02a3b9ae37d74e038251007e`.
- The 33-file `final-sources.json` manifest matched the Windows source tree. The retained image adapter remained `484f3d713e2d3734ed6719ac18f2534a1c0b772f2cc4b6513fed20371a227587`.

`summary.json`, produced by `summarize.py`, records all case-level movement and proves the repeat stability:

| Comparison | Raw pass count | Raw gains/losses | Browser-consensus gains/losses |
| --- | ---: | ---: | ---: |
| CSS-9 alone, calc vs LAYOUT-8 | 16/53 → 31/53 | +15/−0 | +14/−0 |
| CSS-9 alone, broad vs LAYOUT-9 | 83/200 → 84/200 | +1/−0 | +1/−0 |
| LAYOUT-10, calc | 31/53 → 31/53 | +0/−0 | +0/−0 |
| LAYOUT-10, broad | 84/200 → 86/200 | +2/−0 | +2/−0 |

CSS-9 restores `css-flexbox__gap-006-rtl.html`. LAYOUT-10 restores `css-flexbox__gap-006-lr.html` and `css-flexbox__flex-flow-008.html` by collecting definite-height column flex lines before placing them across the cross axis. Both new passes are consensus cases. The standalone two-case `gap-diagnostic-candidate.json` also passes 2/2 exactly.

| Named case | CSS-9-only baseline → LAYOUT-10 |
| --- | --- |
| `calc-max-height-block-1` | pass → pass; CSS-9 changed the prior 1,760-pixel mismatch to exact |
| `calc-min-width-block-intrinsic-1` | pass → pass; CSS-9 changed the prior 720-pixel mismatch to exact |
| `calc-rounding-003` | pass → pass; the computed style now retains `width:calc(50% - 2 * 4.009px)` after `var()` substitution |
| `calc-border-radius-1` | mismatch → mismatch; 84 differing pixels, unchanged; `amazing-paint/src/paint.rs::resolve_corner_radii` must adopt layout's calc-aware radius helper (already routed) |
| `css-flexbox__gap-006-rtl.html` | pass → pass |
| `css-flexbox__gap-006-lr.html` | mismatch (28 pixels at viewport bottom) → exact pass |

Ten retained selections ran twice on both executables. Every selection has zero gains and zero losses from CSS-9-only baseline to candidate. One geometry mismatch, `css-box__margin-trim__flex-column-inline-multiline.html`, changes its test and reference images and improves from 4,000 to 3,000 differing pixels; the other nine retained selections have zero changed image hashes: focused flex/grid 24/24, geometry 41/61, tables 10/12, floats 6/8, block collapse 9/9, position 12/12, text flow 14/16, contents 6/6, paint compositing 24/24, images 12/12. The calc selection retains one page-script crash, `calc-zero-percent-height`, unrelated to this layout change.

The layout crate gate passed 234/234 nextest tests and 0/0 doctests. The source-verified workspace gate passed **1660/1660** nextest tests with 3 skipped and **2/2** workspace doctests. Its log and parsed counts are in `workspace-gate.log` and `workspace-gate.json`. No build or test ran on macOS.
