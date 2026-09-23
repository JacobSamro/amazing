# RC-11: upstream SVG shape and painting inventory

Fixed dev base: `592e54266dfc310e204da95803b8045cd76eb478` (RC-10 merge).
Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

`../rc11-svg-manifest.json` freezes **all 35** non-reference, script-free HTML
`rel=match` reftests under `svg/shapes` and `svg/painting`, sorted by WPT path.
The manifest SHA-256 is
`2fdbe12184173e8808ad2c09a6dc2ed41254affaa4bd1d84054e1880e110cfb0`.
The selection was written before any RC-11 source edit. `../rc11-inventory.json`
retains all 45 HTML reftest candidates, including nine script-dependent match
cases and one mismatch-relation case excluded by the rule, with the reason and
source hash for every case.

The independent wpt.fyi runs are Chrome 154 `5182906107691008` and Safari
26.6 `5141363439173632`; both report the exact pinned full revision hash.
Their frozen outcomes for the 35 selected tests are 20 both-pass, 10
Chrome-pass/Safari-fail, and 5 both-fail. Per-case statuses are in the inventory.

The current HTML image reftest artifact decodes `<img>` and CSS background
resources; it does not paint inline SVG DOM. Of the 35 selected cases, 32
contain inline `<svg>`, one uses an SVG `<iframe>`, one uses a CSS data URI
background, and one uses an `<img>` data URI. The data URI paths are not
resolved by the current artifact. The `<img>` case additionally requires line
stroke, vector-effect and CSS zoom. These blockers remain in the selection.

The fixed-base predecessor executable SHA-256 is
`1caed13624f221e495191e27c1450ef009ec63e89d671f1a5420035615414cba`.
Both Windows sweeps are identical: **22 pass, 13 mismatch, 0 error, 0 timeout,
0 unsupported, 0 crash**. In the 20-test both-browser-pass subset Amazing has
**11 raw passes and 9 mismatches**. Sixteen raw passes and nine consensus raw
passes are all-white actual/reference matches; one inspected PPM has 480,000
white pixels and no nonwhite components. The exact per-case classifications,
pixel differences, hashes, first mismatch metrics and source-based dependency
inferences are in `baseline-analysis.json`. All 45 tracked source, runner,
manifest, lock and selected WPT page hashes matched Mac and Windows before
and after the sweeps. The two hash snapshots are identical. C: remained above
222 GB free throughout the successful baseline.

No selected both-browser-pass mismatch can be moved by a change restricted to
`amazing-image`, `amazing-media` or `amazing-paint`: seven need inline SVG
presentation, and the other two need data URI resource resolution outside
the lane. The `<img>` data URI case also needs CSS zoom plus line stroke and
vector-effect. These are source-based inferences, not conformance gains.
RC-11 therefore stops at the evidence boundary: **no implementation, no after
sweep, no gains or losses claimed**. Cross-lane requests are in the lane
`questions.md`.

Oracle metadata: [Chrome run](https://wpt.fyi/api/runs/5182906107691008), [Safari run](https://wpt.fyi/api/runs/5141363439173632).
