# RC-10: implicit closure for SVG path fill

Fixed dev base: `e6577dc4bb005b1b01523015ebff9aaa85ad6853` (RC-9 merge).
Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.
All builds, tests, and renders use the replaced-content tree on win-001.

`../rc10-fixtures/` is an owned 64×64 HTML `<img>` corpus, not WPT. Open
triangle, concave and transformed/alpha paths have literal shape-only SVG
references; the fourth case compares an open outline with the same path ending
in `Z`. A shared blank control prevents an omitted image from counting as a
fill pass. The fixed-base predecessor executable rendered all four actuals
exactly like blank: **0/4 non-inert**. The implemented decoder renders
**4/4 non-inert twice**. Actuals equal their references and differ from blank
by 528, 1,024, 576 and 1,024 pixels. The two after runs agree in
classification and all PPM hashes. `owned-before.json` and `owned-after-*.json`
record executable and per-page hashes and exact pixel counts.

`../rc10-svg-manifest.json` retains the four pinned CSS Images SVG resource
controls. Their SVG resources have no applicable open path outline. They are a
regression control with **zero new path WPT credit** and were **4/4 raw passes**
on the fixed base and both after runs, with identical pixel projections.

The parser keeps its one-subpath straight-line command subset and numeric and
work limits. Its default nonzero fill sampler already joins the final and
first vertices; accepting end-of-path without `Z` exposes that fill behavior.
No stroke is painted or simulated. Unsupported and malformed resources still
reject as a whole.

Final Windows image nextest passed **34/34** with **0** doctests. Workspace
nextest passed **1,562** with **3 skipped**; separate workspace doctests
passed **2/2**. The workspace gate and paired full render sweeps used shared
build slots. All **31** tracked inputs matched Mac and Windows before and
after final gates, with identical hashes across snapshots. These checks are
recorded in `validation.json`, the paired JSON reports, and the lane report.
