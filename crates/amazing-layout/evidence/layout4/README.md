# LAYOUT-4 current gate — contents unboxing

The certified gate uses baseline `ffb143b` and the unchanged layout implementation
at `1419e48` on `lane/layout`. The earlier `b5d1b99` / `15f9572` runs in
`checkpoint.json` are historical and are not credited. The 2026-09-23 gate ran
only on win-001. The pinned WPT revision is
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, viewport 800 × 600.

`dev-baseline-sources.json` and `final-sources.json` pin every Rust/Cargo input
(219 and 220 files). `dev-baseline-build.json` and `final-build.json` pin freshly
compiled renderer and image-adapter executables and all build/test logs. The
Windows collector verified the final exact source set, clean WPT checkout,
artifacts, logs, and successful exits after rendering. The first collector
attempt used a stale prefixed source-manifest filename; `collect-layout4.ps1`
was corrected to read the same `final-sources.json` as the build. It then passed.

`summary.json` verifies both before and both after reports for every selection:
identical per-case classifications and actual/reference/diff hashes within each
pair, the same case IDs, no baseline pass losses, and zero timeout, unsupported,
or crash results. The 40 normalized reports, collection, test summaries and
build records are retained here. `review/review.json` checks fresh PPM hashes
against the geometry reports and lossless PNG hashes against inspected images
for text inheritance, sharing, out-of-flow positioning, anonymous table repair
and first-letter background removal. The raw PPMs remain in the ignored
`.agents/layout/layout4-review-current/` scratch area.

| Selection | Baseline | Implementation |
| --- | ---: | ---: |
| Broad geometry | 35/61 | 39/61 |
| Tables | 9/12 | 9/12 |
| Contents | 2/6 | 6/6 |
| Floats | 6/8 | 6/8 |
| Block collapse | 9/9 | 9/9 |
| Position | 12/12 | 12/12 |
| Text flow | 14/16 | 14/16 |
| Flex/grid | 24/24 | 24/24 |
| Paint compositing | 24/24 | 24/24 |
| CSS Images | 12/12 | 12/12 |

Layout nextest: 189 → 201 passed, with zero skipped. Separate layout doctests:
zero tests in each phase, zero failures. Final workspace nextest: 1,274 passed,
three skipped, repeated twice; separate workspace doctests: two passed, zero
failed, repeated twice. Every full workspace run and the full render sweep
acquired a shared build slot through the primary checkout's coordination script.
An older `474251e` repeat used the worktree-private slot script before the
integrator fix and is excluded from this gate.

The four broad geometry gains are `display-contents-text-inherit`,
`display-contents-sharing-001`, `display-contents-oof-002`, and
`display-contents-first-letter-002`. The focused contents shard gains flex,
float, inline and unusual-HTML-element cases. `td-001` now produces contiguous
PASS but remains a mismatch because HTML `cellspacing="0"` is not mapped to
computed border spacing. Two before/after cases need generated pseudo-content.
The first-letter fixture only proves removal of a red contents background,
not general first-letter support.

The 22 remaining broad geometry mismatches are eleven margin-trim, five
intrinsic sizing, two float/quirks, two generated pseudo-content, one
flow-root/list-item, and one `td-001` presentational-mapping case. Atomic
inline subtree layout, aspect ratio and sticky positioning were not implemented
in this bounded ticket. `ranking.json` records the pre-implementation ranking
and fixture audit.

To reproduce on win-001, use the source manifests with
`../build-layout4.ps1` for `dev-baseline` and `final`, run
`../parallel-layout4.ps1`, repeat workspace nextest and doctests separately,
then run `../collect-layout4.ps1`. Analyze the copied collection with
`python ../summarize-layout4.py collected.json .` from this directory. The
old `post-gate-rebase.json` and `implementation.patch` are retained only to
explain the historical checkpoint.
