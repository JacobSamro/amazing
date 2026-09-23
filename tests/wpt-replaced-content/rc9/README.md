# RC-9: bounded static SVG polyline fill

Fixed dev base: `03b26ec7b4b2186bd27ed1455d045f47b1d2f0b4` (includes RC-8).
Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
All builds, tests, and renders use the replaced-content tree on win-001.

`../rc9-fixtures/` is an owned 64×64 HTML `<img>` corpus, not WPT. Its four
polyline actuals cover simple, concave, transformed/alpha and endpoint-closure
geometry. Literal shape-only SVG references and a shared blank control prevent
blank matches from counting as filled pixels. The fixed-base predecessor
executable rendered all four actuals exactly like blank: **0/4 non-inert**.
The implemented decoder renders **4/4 non-inert twice**; actuals match their
references and differ from blank by 1,024, 1,024, 576, and 528 pixels. The
two after projections agree in classification and all per-page PPM hashes.
`owned-before.json` and `owned-after-*.json` record the evidence.

`../rc9-svg-manifest.json` retains the four pinned CSS Images SVG resource
controls. These contain no applicable polyline resource. They are a regression
control with **zero new WPT polyline credit**. Their fixed-base result was
**4/4 raw passes** in `wpt-before.json`, unchanged in both after runs
including classifications, pixel differences and actual/reference/diff hashes.

The decoder uses the RC-8 finite, bounded points parser and RC-7 nonzero winding
sampler. It treats the open polyline as closed only for fill. All stroke,
fill-rule, malformed points, unsupported attributes, references, and paint
servers reject the whole image. The parser and raster work caps are unchanged.

Final Windows image nextest passed **30/30** with **0** doctests. The
workspace nextest passed **1,551** with **3 skipped**; separate workspace
doctests passed **2/2**. The workspace gate and paired full render sweeps used
shared build slots. All **31** tracked inputs matched Mac and Windows before
and after the final gates, with identical hashes across the two snapshots.
These checks are recorded in `validation.json` and the lane report. `input-hashes-before.json` and `input-hashes-after.json` track source,
runner, manifests, lockfiles, and fixtures around the final gates.
