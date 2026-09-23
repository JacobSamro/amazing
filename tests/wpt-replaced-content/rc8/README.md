# RC-8: bounded static SVG polygon fill

Fixed dev base: `52222f8` (RC-7 merge). Pinned WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

`../rc8-fixtures/` is an owned 64×64 HTML `<img>` resource corpus, not WPT.
Four polygon actuals cover simple, concave, transformed/alpha and repeated
same-direction winding geometry. Shape-only SVG references and a shared blank
control make blank matches ineligible for credit. On the fixed-base
predecessor executable, all four actuals equal blank and differ from their
references: **0/4**. After the polygon decoder, all four actuals match their
references and differ from blank at 1,024, 624, 576 and 1,024 pixels:
**4/4 non-inert twice**. `owned-before.json` and `owned-after-*.json` contain
executable hashes, classifications, exact pixel differences, and PPM hashes.
The two after files are identical. The predecessor executable SHA-256 is
`db2f4d998484994924114b40f9676410efc5a1a67cfb30e715befa38613ff2c4`;
the after executable SHA-256 is
`2857d3587495d3c5569dff34c695bec1e9acccf3eab2532259b693e442d2a757`.

`../rc8-svg-manifest.json` retains the four pinned CSS Images SVG resource
controls. The pinned sparse checkout contains no `<polygon>` SVG resource.
`wpt-before.json` and both `wpt-after-*.json` runs report **4/4 raw passes**
with identical classifications, differing pixels, and actual/reference/diff
hashes. These were already non-inert passes: **zero new WPT polygon credit**.

The decoder parses a bounded SVG points-list, requires at least three finite
coordinate pairs, and uses RC-7 nonzero winding, transforms, computed paint
and vertex-weighted raster work accounting. Unsupported and malformed input
rejects the entire SVG image. The Windows image and workspace nextest gates,
separate doctests, and final Mac/Windows source, lock and fixture hashes are
recorded in `validation.json` and the lane report.

Final Windows gates on the fixed base passed: image nextest **26 passed, 0
skipped** and separate image doctests **0 present**; root workspace nextest
**1,529 passed, 3 skipped** and separate workspace doctests **2 passed**.
`input-hashes-before.json` and `input-hashes-after.json` compare 31 source,
manifest, runner, lockfile and fixture files on Mac and win-001; every hash
matches across machines and the two snapshots are identical. The workspace
and both full render sweeps used the shared build-slot wrapper.
