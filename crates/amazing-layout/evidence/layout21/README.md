# LAYOUT-21: ruby pairing and placement

Fixed base `6ab9a612` (LAYOUT-20 final); baseline harness `97d79da4`.
Pinned WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, Linux, 800×600.
The inventory includes all118 static css-ruby reftests and explicitly excludes
25 script-dependent reftests. These are first-reference pixel comparisons.

Final `final-repaired/` measures checkpoint97d79da4-layout21-candidate6:

| Gate | Before | After | Gains | Losses |
|---|---:|---:|---:|---:|
| Full static ruby |33/118|68/118|35|0|
| Full static tables |310/386|310/386|0|0|
| Retained union |280/449|299/449|19|0|

Workspace nextest: **2798 passed,10 skipped**,59.415s. **Four doctests passed**.
Layout crate candidate5:345 passed,one skipped,zero doctests. Every final renderer
error/timeout/unsupported/crash counter is zero. Source hashes in provenance.txt
match the committed implementation. Hash changes are recorded in comparisons;
zero losses does not mean unchanged images. Windows is deferred.

The implementation preserves segments, annotation levels and independent spans,
nested ruby, orphan/generated ruby parts, whitespace pairing and auto-hiding.
The shaper supports alignment, over/under/alternate levels, first-line position,
vertical placement, contextual space overhang, nested intrinsic contributions,
and typed tab stops. Parsing and nested reshaping have explicit resource budgets.
Parser and shaper are separate owned modules, with18 new geometry/model tests.

`final-after/` is the first failed gate, kept for diagnosis: it lost three retained
passes because a broad intrinsic keyword override used incomplete non-ruby token
measurements. `final-repaired/` restricts that new path to ruby's shaped widths;
all three losses are repaired. Earlier candidate reports preserve bulk iterations.
The gate was repeated because the first gate exposed these regressions.

This is core ruby pairing/placement, not complete CSS Ruby conformance:50 static
mismatches remain. Punctuation ink/fullwidth-space font metrics and inherited
tab-size are routed to text/CSS owners; flex/grid shaped ruby intrinsic adoption
is routed to layout-flexgrid. General text alignment, ruby box edges/out-of-flow
children and intra-base fragmentation remain follow-up work. No fixtures are
rewritten or classifications suppressed.

Specification: https://www.w3.org/TR/css-ruby-1/ (sections2.2–2.5,3.1,4.1–4.3,5.1).
Run through `scripts/linux-run.sh layout -- 'bash crates/amazing-layout/evidence/layout21/gate.sh <source> <unique-output-tag>'`.
