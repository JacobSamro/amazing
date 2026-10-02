CSS-34 registers and computes translate/rotate/scale together. Same corrected
CSS-33 runner and pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293 on both
sides, fixed base a717bb9749a867628f92811b45abb435efde7769.

Parsing: 3482 → 3565 / 4805, +83 exact assertions, lost 0; 498 files,
169 nonliteral assertions excluded equally. Workspace: 3524 passes / 17 skips,
4 doctests. Transform rendering: 145 / 348 on both sides, all classifications
unchanged. Render viewport 800×600, 4 jobs, timeout 20 seconds; run on Boat Linux.

`gate.sh` batches workspace nextest, doctests, parsing corpus, explicit driver
build and zero-loss full transforms rendering. Native source/target directories
avoid lazy snapshot filesystem artifacts. `before/compact.json` and
`after/compact.json` contain compact rendered classifications. `gate.json`
contains the measured summary and exact gained parsing assertions.

Rendering adoption is a cross-lane follow-up: this ticket supplies computed
values and conservative invalidation, not layout/compositor matrix composition.
