# RC-7: closed straight-line SVG path fill

Fixed base: `1a8c222` (RC-6 merge). Pinned WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

The owned 64×64 corpus in `../rc7-fixtures/` renders SVG resources through
`<img>` and the existing HTML image path. Four path actuals cover absolute
commands, relative commands and implicit lineto pairs, a concave outline,
and a group transform with fill alpha. Shape-only SVG references and a blank
control make raw blank-image matches ineligible for credit. The saved
fixed-base before executable has all four actuals identical to blank and
different from their references: **0/4**. The path implementation's first
after run has **4/4 non-inert reference matches**, differing from blank at
1,024, 1,024, 624 and 576 pixels respectively. `owned-before.json` and
`owned-after-*.json` record executable and per-page PPM SHA-256 hashes and
classifications. The same input corpus is used in all runs.

The pinned upstream `css/css-images` sparse checkout contains no `<path>` SVG
resources. `../rc7-svg-manifest.json` therefore retains the four prior SVG
resource controls from RC-6. `wpt-before.json` is the saved RC-6 final
fixed-base projection, before RC-7 code changed. These four cases were already
non-inert passes, so their **4/4 raw** result earns **zero new WPT path credit**.
`wpt-after-*.json` records classifications, pixel differences and
actual/reference/diff hashes from the existing pinned runner.

The decoder accepts one explicitly closed polygon with only `M/m`, `L/l`,
`H/h`, `V/v` and `Z/z`. It applies the existing RC-6 computed paint and SVG
transform path. Unsupported commands/features, open or multiple outlines,
malformed numbers, excessive tokens/points and existing image/work limits
reject the entire image. Windows `amazing-image` nextest and separate doctests,
then fixed-base workspace nextest and separate doctests, are recorded in the
lane report. `input-hashes-before.json` and `input-hashes-after.json` compare
all 24 owned source, manifest and fixture files on Mac and win-001.

The second after run exactly repeats all four owned classifications and
actual/reference/blank hashes. The pinned WPT projections are identical
before and in both after runs, including classification, differing pixels,
and actual/reference/diff hashes. Final fixed-base Windows gates passed:
image nextest **23 passed, 0 skipped**, image doctests **0 present**;
workspace nextest **1,464 passed, 3 skipped**, workspace doctests **2 passed**.
The 24 Mac/Windows input hashes matched before and after the gates and did
not change. The per-run machine-readable counts are in `validation.json`.
