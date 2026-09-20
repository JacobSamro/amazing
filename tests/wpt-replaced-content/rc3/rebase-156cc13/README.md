# RC-3 merge gate after rebase

Integrator-requested rebase onto origin/dev `156cc13`; implementation commit
`0991e80`. No implementation edits were needed. Ran
`bash scripts/refresh-lane-trees.sh replaced-content` from the rebased lane;
only that server tree was refreshed, with first-party artifact invalidation.
Re-scp'd all 49 lane-diff files afterward. Lock hash prefix: `3FEAF813`.
Separately invalidated the isolated image target's first-party artifacts.

All builds/tests ran on win-001 with CARGO_INCREMENTAL=0 and --locked.
Fresh full workspace: **1,035 passed, zero failed, three ignored, 53 targets**.
Paint: **86 passed**. Root driver and isolated image example builds passed.

Repeated after gates retain the same counts: controls **21 matches / two
mismatches / one unsupported**, CSS Images **12/12**, focused paint **24/24**.
The original before/after evidence in the parent directory is preserved.
The controls remain raw matches, with no new conformance credit or production
state/label claim. The root-relative reference harness finding is now routed
by the integrator; the unsupported result remains honest pending its fix.

This directory holds six refreshed reports, executable hashes, validation and
repeatability data. Reproduction uses the parent's run-controls-gates.ps1 flow
with freshly built after executables. Source hashes are supplementary to the
explicit artifact invalidation, never a substitute for it.

All three pairs are stable in classifications, pixel-difference counts and
actual/reference/diff hashes. These projections also match the pre-rebase after
runs exactly. All six paint source hashes and both lockfile hashes match the
local checkout used for the rebuild.
