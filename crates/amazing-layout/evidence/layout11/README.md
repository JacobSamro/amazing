# LAYOUT-11 focused baseline

The fixed pre-CSS-11 source is lane commit `82d5574e2f38f8ccddfd6a336340ef513ee14467`
on `origin/dev` `8f2695d92b31962982bb9c389f4d3b4a784ba13a`. The archive was
expanded into the layout Windows build tree. `cargo build -p amazing --bin amazing
--locked` ran on win-001; the immutable renderer SHA256 was
`3c9e967b14c58c3e7757d741afb4d7f930e26f4db2177ac0789580f2e55dbde9`.
The pinned WPT revision was `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`,
800×600 at device scale 1. The zoom fixtures were copied from the CSS lane's
checkout of the same revision into the layout lane's WPT root.
The 292 tracked crate source files and Cargo manifests/lockfile in
`baseline-source.json` matched their win-001 copies, 292/292.

The five-case `focused-manifest.json` ran twice under a build slot, producing
`baseline-1.json` and `baseline-2.json`. Both report **0 pass, 5 mismatch**, with
identical per-case classifications, differing pixel counts and
actual/reference/diff SHA256 hashes:

| Case | Differing pixels |
| --- | ---: |
| `css-viewport__zoom__basic.html` | 30,000 |
| `css-viewport__zoom__inherited.html` | 30,000 |
| `css-cascade__all-prop-002.html` | 10,000 |
| `css-values__calc-border-radius-1.html` | 84 |
| `css-fonts__font-variant-ligatures-11.optional.html` | 424 |

This baseline predates CSS-11 and CSS-12. The latter must expose computed font
features before layout can measure the ligature case. The two cascade cases
have existing layout geometry and require the paint changes routed in
`.agents/layout/questions.md`.

## CSS-11 base and layout candidate

The fixed LAYOUT-11 base is `origin/dev` `582a6af0962dd3b7e6c94427c14746915aea14ab`.
CSS-11 exposes `effective_zoom()` and `scale_used_length`, but the unchanged
CSS-11-only renderer still gives **0/5** in the focused selection twice, with
exactly the pre-CSS pixel counts and hashes. Its immutable SHA256 is
`8a8a77f196f724060b572a2da05f37f96f89f1198b03092e03a070262ab00a92`.

Layout source commit `0b41e3001fe01bc3af83300ce5919c551d8f7996`
applies zoom at used-value boundaries to ordinary box geometry, font size,
borders, flex/grid lengths, table spacing, replaced content and SVG dimensions.
`candidate-source.json` records 428 tracked crate source/test and Cargo files;
all **428/428** hashes matched the win-001 build tree before and after its
workspace gate. The immutable candidate renderer SHA256 is
`bcec27654beb3e1bca74d6587f4fcb79c731ff0b318902d4c24f91cf1518f972`.

| Frozen focused case | CSS-11 only, both runs | Layout candidate, both runs |
| --- | ---: | ---: |
| `css-viewport__zoom__basic.html` | mismatch, 30,000 px | **pass, 0 px** |
| `css-viewport__zoom__inherited.html` | mismatch, 30,000 px | **pass, 0 px** |
| `css-cascade__all-prop-002.html` | mismatch, 10,000 px | mismatch, 10,000 px |
| `css-values__calc-border-radius-1.html` | mismatch, 84 px | mismatch, 84 px |
| `css-fonts__font-variant-ligatures-11.optional.html` | mismatch, 424 px | mismatch, 424 px |

Both repeats have identical classifications, pixel counts and actual/reference/
diff hashes. The cascade and radius cases are paint-owned and have exact
requests in `.agents/layout/questions.md`. The ligature case is **blocked on
CSS-12**: computed font-feature properties were absent from dev at the final
gate, so the provisional layout-to-text feature mapper remains unmerged.

Broad flex/grid stayed **86/200 before and after, twice, zero losses**. The
only changed actual hash was
`css-grid__grid-lanes__subgrid__grid-subgridded-to-grid-lanes__alignment__row-subgrid-alignment-in-subgridded-axis-001.html`,
still a mismatch but improving from 30,628 to 26,300 differing pixels. All
ten retained selections have zero pass losses, zero count changes and zero
actual-hash changes, twice:

| Selection | Before and after, both runs |
| --- | ---: |
| flex-grid | 24/24 |
| geometry | 41/61 |
| tables | 10/12 |
| floats | 6/8 |
| block-collapse | 9/9 |
| position | 12/12 |
| text-flow | 14/16 |
| contents | 6/6 |
| paint-compositing | 24/24 |
| images | 12/12 |

The layout crate passed **235/235** nextest tests and **0/0** doctests. The
source-verified workspace gate passed **1719/1719** nextest tests with 3
skipped and **2/2** doctests. Every build and render ran on win-001; none ran
on macOS. Raw reports are `css11-only-{1,2}.json`, `final-focused-{1,2}.json`,
`broad-{before,after}-{1,2}.json` and the 20 JSON files in each retained
directory. `gate-summary.json` records per-case gains, losses and changed
hashes for both repeats.
