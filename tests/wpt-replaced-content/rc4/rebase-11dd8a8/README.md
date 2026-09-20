# RC-4 gate with nextest and explicit doctests

Dev base: `11dd8a8aefd81318fcfe858334e865ed66694cd8`. Compiled lane HEAD: `51ba19fbb4c55386e0822715d27062d4b14b9271`.
The integrator refreshed the Windows tree to this head's dev base; the lane
then re-synced its owned files. RC-4 source is unchanged apart from the clean
rebase. The earlier acb57e5 gate was interrupted by that refresh; its partial
reports are preserved separately and are not claimed as a final paired gate.
All original RC-4 before evidence remains intact.

Tests run only on win-001 with --locked and CARGO_INCREMENTAL=0. Root workspace
uses `cargo nextest run --workspace --locked`, followed by
`cargo test --workspace --locked --doc`. This explicitly includes the JS and
HTML doctests that nextest does not execute. The isolated image workspace uses
`cargo nextest run --manifest-path crates/amazing-image/Cargo.toml -p amazing-image --locked`
and a separate `cargo test --manifest-path crates/amazing-image/Cargo.toml -p amazing-image --locked --doc`.
Nextest and doctest counts are reported separately. The isolated image target's
first-party artifacts are invalidated before building its saved renderer.

Both renderers are rebuilt. Pinned WPT selections are repeated twice at
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`: scope, controls, PNG, SVG and
focused paint. Owned 64x64 fixtures are also repeated twice and are explicitly
not WPT. Original before evidence is reused, not regenerated. SVG raw matches
remain uncredited where CSS-bearing resources are unsupported. Production
control state and label wiring remains outside RC-4.

Use the existing RC-4 run scripts with output/executable prefix
`target/rc4-11dd8a8-`; collect with `../../collect-rc4-rebase.ps1 -Tag 11dd8a8`
and analyze saved reports with `../../analyze-rc4-rebase.py 11dd8a8`.

Measured test counts (kept separate):

| Runner | Passed | Failed | Skipped/ignored |
| --- | ---: | ---: | ---: |
| Root nextest, 41 binaries | 1,209 | 0 | 3 skipped |
| Root doctests, 18 targets | 2 | 0 | 0 ignored |
| Image adapter nextest, 3 binaries | 17 | 0 | 0 skipped |
| Image adapter doctests | 0 present | 0 | 0 ignored |

The two root doctests are one in amazing-html and one in amazing-js. Paint has
87 passing tests within the workspace nextest run. Both locked metadata checks
and renderer builds passed; all 191 checked Rust/Cargo inputs match the Mac
checkout. Owned proof cases are 9/9 in each fresh after run. These are independent
of WPT credit; the original 0/9 diagnostic before result remains preserved.

Final repeated counts: scope **26 pass / 4 mismatch / 1 unsupported / 17 renderer
errors**; controls **21 pass / 2 mismatch / 1 unsupported**; PNG **12/12**;
SVG **12/12 raw, zero credit**; focused paint **24/24**. No timeouts. All five
pairs agree on classifications, differing pixels, maximum differences, and
actual/reference/diff hashes. These fields also match both original before and
after reports. `repeatability.json` records the per-selection projection hashes.
The owned after pair remains **9/9**, identical to the original after proof;
the original repeated **0/9** before evidence is preserved. No new WPT
conformance passes are claimed. `artifacts.json` identifies both fresh renderer
executables. Original before executables were re-hashed and still match the
original artifact record. The following commit records evidence only; the
compiled source HEAD above identifies the code that produced these results.
