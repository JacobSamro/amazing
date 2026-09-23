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

## Preflight winding correction

The initial RC-7 renderer used crossing parity. SVG's default `fill-rule` is
`nonzero`: see the [SVG 2 property index](https://svgwg.org/svg2-draft/propidx.html)
and [SVG 1.1 painting rule](https://www.w3.org/TR/SVG11/painting.html).
A literal one-subpath `M/H/V/Z` outline loops around the same square twice in
the same direction. The added decoder regression failed on win-001 before the
correction (`[0,0,0,0]` instead of `[255,0,0,255]` at `(2,2)`) and passed
after changing containment to signed winding. Explicit `fill-rule` remains
unsupported by the SVG subset.

The owned HTML image corpus now includes `double-wound.html`, a shape-only red
reference and the shared blank. `owned-review-before.json` records the RC-7
pre-correction executable: **4/5**, with double-wound equal to blank and
1,024 pixels different from its reference. `owned-review-after-1.json` and
`owned-review-after-2.json` record **5/5 non-inert matches**, identical per-case
classifications and PPM hashes; double-wound matches its reference and differs
from blank at 1,024 pixels. Executable SHA-256 changed from
`e705c9c62d4e29e016d2e586274442bcaf7644a382c4750753656b9bc8e30aab`
to `0b4b0924a06ca307d7451140c22935a953a654dd2246901c6383298641f1d9c8`.

The review correction's Windows image package gate is **24/24 nextest passed**
and **0 doctests**. `input-hashes-review-before.json` records the 28 matched
Mac/Windows inputs before the source correction;
`input-hashes-review-final-before.json` and
`input-hashes-review-final-after.json` record the corrected inputs, equal on
both machines and unchanged across the final image and owned render gates.
The previous WPT control and workspace gate belong to the initial RC-7 commit;
they were not rerun for this requested image-package correction.
