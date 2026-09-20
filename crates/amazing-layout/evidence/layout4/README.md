# LAYOUT-4 partial evidence — checkpointed, not certified

`ranking.json` and `ranking-geometry-{1,2}.json` retain the pre-implementation
measurement at `bb79669`. The interrupted certification attempt used a freshly rebuilt baseline
at `b5d1b99` and implementation `15f9572`. Do not attribute the earlier dev gains
(geometry 34 → 35, tables 8 → 9) to LAYOUT-4.

All engine builds and tests ran on win-001. The source manifests record every
Rust source and Cargo manifest/lockfile, including the new contents tests.
Build metadata records fresh first-party compilation, executable hashes, and
log hashes. Nextest and doctests have separate counts. `summary.json` is produced
only after both runs of every selection match per-case classifications and
actual/reference/diff hashes, and every baseline pass is retained.

To reproduce, sync the complete baseline commit from the Mac into the layout
Windows tree, then run `../build-layout4.ps1 -Phase dev-baseline
-SourceManifestPath C:\Users\server\target\layout\layout4-dev-sources.json
-BuildImages`. Restore and sync the implementation's full source manifest, then
run the same helper with `-Phase final -FullWorkspace -BuildImages` and its
`layout4-final-sources.json`. The JSON files here provide those exact manifests.
The helper refuses source drift and clears first-party artifact caches.

Run `../parallel-layout4.ps1` in Windows against the pinned WPT checkout
`C:\Users\server\target\layout\wpt`; it runs ten selections twice for each
artifact. It resumes completed reports and launches at most four rendering
jobs, without building. Repeat `cargo nextest run --workspace --locked` and
`cargo test --workspace --locked --doc` separately, retaining the repeat logs
and exit files expected by `../collect-layout4.ps1`. Run that collector after
all gates. Copy its `layout4-collected.json` back and analyze it with
`python ../summarize-layout4.py <collected.json> .` from this directory.

Fresh-epoch `review/review.json` and `summary.json` are deliberately absent:
report collection, hash stability, final source/artifact audit and fresh pixel
review could not be completed during the SSH outage. The older pixel review was
moved to ignored orchestration scratch so it cannot be mistaken for fresh proof.
The summary script requires new review metadata and verifies it against reports.

An intermediate epoch at `11dd8a8` was superseded after an external refresh at
2026-09-20 20:15:43 UTC changed workspace, HTML, and layout files. The pinned
executables remained unchanged, but the final source audit failed. Its builds,
logs and 40 reports are archived on win-001 under
`C:\Users\server\target\layout\layout4-11dd8a8-builds`; none are credited by the
final summary. The final epoch uses `layout4-recovery-evidence` after rebasing,
self-refreshing, restoring the exact baseline, and repeating builds and gates.

The existing rendering session completed all 40 runs during the outage; see
`checkpoint.json` for observed counts and explicit unproven items. No gates were
run on the post-run rebase `985e5ef` / `9bf1224`. `post-gate-rebase.json` verifies
unchanged layout source. `implementation.patch` reconstructs the measured layout
changes on `b5d1b99` without requiring the old unpublished commit to remain in
Git. The summarizer checks tracked gate inputs against that measured base, not
the newer working tree. Do not refresh before preserving retained results, and
do not open SSH/SCP until the integrator broadcasts the all-clear.
