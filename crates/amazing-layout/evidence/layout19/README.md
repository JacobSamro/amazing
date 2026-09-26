# LAYOUT-19: table width allocation

Pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800×600.
Inventory: all 386 static first-link reftests from `css/css-tables` and
`css/CSS2/tables`; 151 script-dependent reftests excluded explicitly. This is
static rendering coverage, not dynamic testharness coverage.

The final pair uses engine base `c702f5ef`, with harness-only commit `0bf7475b`.
`final-before/provenance.txt` and `final-after/provenance.txt` record source
hashes, Cargo.lock and the lane-exclusive target. The after source is the
LAYOUT-19 implementation committed with this report. Windows is deferred.

| Measurement | Before | After |
|---|---:|---:|
| Layout nextest | 302 pass / 1 skip | 315 pass / 1 skip |
| Layout doctests | 0 | 0 |
| Full static table corpus | 241 / 386 | 272 / 386 |
| Table gained / lost passes | — | 31 / 0 |

Final Linux workspace: **2,768 passed, 10 skipped**, nextest 60.574 s;
**four doctests passed**. Retained 449: **280 → 280 passed, zero gains/losses**,
with identical actual/reference hashes and classification projections. All measured table corpus runs have zero errors, timeouts,
unsupported cases and crashes. Mismatches remain visible.

Changes cover column and colgroup constraints, first-row spans, absolute and
percentage contributions, collapsed half-border offsets, box sizing, automatic
width guesses and excess allocation, min/max contributions and fixed tracks
narrower than their cell padding. Fixed layout ignores later-row intrinsic
content. CSS2 percentage-cell padding/borders are retained (fixture 026),
despite the offset-ignoring wording in the CSS Tables 3 working draft.

Rules: [CSS2 fixed layout](https://www.w3.org/TR/CSS22/tables.html#fixed-table-layout)
and [CSS Tables 3 width distribution](https://www.w3.org/TR/css-tables-3/#width-distribution).

Candidate1 had two losses; candidate2 repaired both, and candidate3 repaired
five more narrow-track decoration cases. All intermediate results are retained.
`triage.py` groups filenames only, not root causes. The final three fixed-width
mismatches (021/022/023) have blank image-swatch references; their expected table
column positions and widths are covered by geometry tests. Image adoption is
routed to the owning lane, and these cases remain mismatches.

Reproduce through `scripts/linux-run.sh layout -- 'bash
crates/amazing-layout/evidence/layout19/measure-final.sh before <baseline>'`
using `--ref 0bf7475b`, then the same command with `after <source>` on the
implementation tree. Output directories must be fresh. The after command runs
workspace nextest, workspace doctests, all table cases and the retained 449.
PPMs are gzip-compressed after original pixel hashes are captured. Scratch
compression metadata records lossless compaction of earlier LAYOUT-18 images.
