# LAYOUT-5 — baseline alignment and outer inline flex/grid

Fixed base: merged `dev` at `bebca0f`; branch `lane/layout`. The final source
manifest contains 218 Rust/Cargo inputs and was checked against both the Mac
worktree and the Windows lane tree. `git diff --name-only bebca0f origin/dev --
crates/amazing-layout` was empty at collection. All compilation, unit tests,
doctests, and WPT rendering ran on win-001. Pinned WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800×600.

`broad-baseline-{1,2}.json` preserves the original 61/200 result.
`slice-flex-grid-broad-{1,2}.json` measures the committed first-baseline
alignment slice: still 61/200, with no classification changes. Four images
changed, all for cases without Chrome/Safari consensus. The focused flex/grid
selection was 24/24 twice. `final-flex-grid-broad-{1,2}.json` measures the
combined atomic outer inline flex/grid implementation: **61/200 twice**.
Every pair has the same test IDs, per-case classifications and
actual/reference/diff hashes. No report has a timeout, unsupported result, or
crash. `summary.json` gives all pass transitions and pixel changes.

| Broad 200-case movement | Raw | Chrome/Safari consensus |
| --- | ---: | ---: |
| Gained passes | 2 | 2 |
| Lost passes | 2 | 2 |
| Net passes | 0 | 0 |

The gains are `css-flexbox__gap-006-rtl.html` and
`css-flexbox__gap-006-lr.html`. The losses are the Grid Lanes/subgrid
`column-subgrid-grid-gap-larger-002.html` and
`column-subgrid-orthogonal-writing-mode-004.html`. Both tests use the unsupported
`display:inline-grid-lanes` and `subgrid` grammar; their references use the
newly working `inline-grid`. The old blockified reference happened to match
the invalid-author-style fallback. These two earlier passes did not prove
Grid Lanes or subgrid capability. They remain counted as raw and consensus
losses in the measured gate; CSS-6 grammar is prerequisite to LAYOUT-6.

The two baseline multi-item flex cases still mismatch. Their shared reference
uses non-replaced `inline-block`, which the engine still blockifies, while the
testcases now use atomic `inline-flex`. The five-case diagnostic manifest
measured 0/5 before outer inline participation and 2/5 after. The full broad
manifest, rather than that diagnostic, is the credited gate.

| Other final selection | Pass/total, twice | Pass losses vs LAYOUT-4 |
| --- | ---: | ---: |
| Focused flex/grid | 24/24 | 0 |
| Broad geometry | 39/61 | 0 |
| Tables | 9/12 | 0 |
| Floats | 6/8 | 0 |
| Block collapse | 9/9 | 0 |
| Position | 12/12 | 0 |
| Text flow | 14/16 | 0 |
| Contents | 6/6 | 0 |
| Paint compositing | 24/24 | 0 |
| CSS Images | 12/12 | 0 |

Windows layout nextest: **205 passed, 0 skipped**; separate layout doctests:
**0 passed, 0 failed**. Workspace nextest: **1,313 passed, 3 skipped**, twice;
separate workspace doctests: **2 passed, 0 failed**, twice. The workspace and
full render sweeps used the shared primary-checkout build slots. `gate.json`
retains artifact and workspace-log hashes; `final-build.json` records fresh
first-party compilation; `provenance.json` pins the final renderer and CSS
Images adapter. A `REFRESH-HOLD` protected the lane tree through collection.

To reproduce on win-001, build with `../build-layout4.ps1` using
`final-sources.json` and `-BuildImages`, run `run-final.ps1` with the two
immutable executables, and use `collect.ps1`. Run `summarize.py` on the copied
collection directory to verify every report pair, pass transition, and
regression selection.
