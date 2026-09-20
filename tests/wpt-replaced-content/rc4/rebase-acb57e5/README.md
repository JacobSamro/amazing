# Superseded partial gate — do not use as the final RC-4 gate

The fleet refresh to dev `11dd8a8` arrived during this run. Its owned runner was
stopped; completed reports are retained here as historical evidence. The full
paired WPT sequence did not finish. The subsequent `rebase-11dd8a8` evidence
uses cargo-nextest plus separately measured doctests and replaces this gate.
Original before evidence remains untouched.

---

# RC-4 final gate after the standalone lock fix

Dev base: `acb57e596422b454850779e8070f34ee656afa36`.
Compiled lane HEAD: `d8967dd0bf1e9aac1b1e8c2df3ade8fb96cf2251`;
implementation rebased to `694f539`. No RC-4 source edits were needed.
The primary integrator supplied the standalone shell lock update and refreshed
this lane tree. The lane re-synced all owned files afterward. All 189 checked
tracked Rust/Cargo inputs match the Mac checkout. The root graph was refreshed
with artifact invalidation; isolated image first-party artifacts were invalidated
again before its build. All builds/tests are on win-001 with --locked and
CARGO_INCREMENTAL=0. Both root and image-workspace locked metadata checks pass.
No lane lockfile regeneration/bypass or shell source grant was used.

This directory supplements the parent RC-4 evidence; all original before and
after reports, proof images and credit limitations remain untouched. Results
here are repeated **after** measurements on the newly integrated dependency
graph. They do not reassign existing raw WPT matches to RC-4.

The 48-case scope and 24-case controls selections use the root driver; PNG and
SVG image selections use the isolated static-image example. Focused paint also
uses the root driver. WPT remains pinned to
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` with its recorded viewport/font
providers. The nine owned 64x64 proof cases are explicitly not WPT; the SVG
counterpart selection still earns zero conformance credit for CSS-bearing
resources rejected by the decoder. Production control state/labels remain
outside this ticket.

Reproduction: use the parent's gate and fixture scripts with a separate output
prefix `target/rc4-acb57e5-` and newly built after executables under the same
prefix. `../../collect-rc4-rebase.ps1` collects reports using `-Tag acb57e5`.
`../../analyze-rc4-rebase.py` summarizes saved reports only.

Fresh Windows validation: root workspace **1,185 passed / zero failed / three
ignored across 59 targets**, including **87 paint tests**. Isolated image
adapter **17 passed** (4 unit, 3 presentation, 10 SVG), with its --locked example
build green. The owned diagnostic renders are **9/9 in each fresh after run**;
the original **0/9 before** is preserved in the parent evidence directory.
`build-inputs.json` records all 189 inputs whose hashes matched the server.
