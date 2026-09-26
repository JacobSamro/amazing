# LAYOUT-20: anonymous table repair and inline table flow

Baseline: LAYOUT-19 commit `415fa342`. The before reports are its
`evidence/layout19/final-after` results: 272/386 static tables and 280/449
retained. There is no intervening engine change. Same pinned WPT c2a2fce69,
800×600 viewport, manifests and script exclusions.

Changes group orphan table siblings inside inline parents before flattening,
preserve inline-table wrappers and first-row baselines, keep atomic inline
content in its generated cell, and remove irrelevant anonymous whitespace.
Table cells isolate descendant margins; nested empty blocks collapse through.
Intrinsic column sizing includes nested inline tables. Shaped line fitting
accounts for spaces before atomic boxes. Atomic text starts at the containing
block origin, avoiding redundant translation of caption glyphs. CSS table boxes
on ordinary elements honor content/border-box widths; HTML tables retain their
CSS2 legacy width convention.

Rules: [anonymous table generation](https://www.w3.org/TR/CSS22/tables.html#anonymous-boxes),
[row baselines](https://www.w3.org/TR/CSS22/tables.html#height-layout),
[CSS/HTML table width distinction](https://www.w3.org/TR/CSS22/tables.html#separated-borders).

Candidate4: **327 layout tests passed, one skipped; zero layout doctests**.
Full static tables: **272 → 310 /386, 38 gains and zero losses**. All 71 cases
in the anonymous-table filename family pass. All renderer failure counters
are zero; remaining mismatches are reported. **Final Linux gate: 2,780 workspace nextest passed /10 skipped in58.518s;
four doctests passed. Retained449: 280→280, zero gains/losses.**
The final full table sweep repeats310/386 (+38/-0).

All candidate results remain visible: candidate1 failed two new tests and did
not reach the corpus; candidate2 had three losses, candidate3 one, candidate4
zero. These are repaired regressions, not reclassified or excluded cases.

`batch.sh <tag> <checkpoint>` builds, tests the whole layout crate, runs its
doctests and all 386 table reftests. `gate.sh <checkpoint>` runs the final
workspace/docs and table/retained before-after comparisons. Invoke both only
through `scripts/linux-run.sh layout -- 'bash <script> ...'`.
Provenance records all changed source hashes and the lane-exclusive target.
