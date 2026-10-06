# Grid row baseline sharing

First and last baselines now share real row groups in ordinary Grid and Row
Lanes. Intrinsic sizing includes measured baseline shims, final placement uses
actual chosen rows, requested baseline items retain used heights, and auto block
margins keep priority. Original 30 groups / 44 document fixtures are unchanged;
six new groups add 20 document fixtures (36 groups / 64 fixtures total).

Qualified source: `77b312c685256c303ee639a8b607f333827fd106` on fixed a5a56a90.
Ready squash base: `973bdec5345bbbf768acd1fa3ef8220d0274b153`. Shipping layout
bytes are identical to the qualified source. The authentic source570 BEFORE
classifications were carried unchanged through published 6bf/dbce and e8 AFTER;
all actual source/role identities are retained. This is not a fresh a5 BEFORE.

Linux/Boat: **5,042 nextest passed, 0 failed, 17 skipped; 36 doctests passed**.
Whole original eight directories / 2,421 static cases: **1,216 → 1,217 pass,
+1 gained, zero lost**. The gain is grid-align-baseline-explicit-placement.
Initial e3's three Row Lanes reference-side regressions are resolved by the real
shared producer correction; its failed branch/raw/ELFs remain preserved.
Actual native, runner and authenticated fetch exits are all zero. Native target,
raw and 998 captured ELFs remain protected; only compact proof is committed.

First-group sizing/participation is ported from Taffy at
`04965fc5ac0c0e9423f5aab8ca29b48231738c7a`, MIT; headers and an appended notice
record attribution. Last sharing/safe fallback and Row Lanes adaptation follow
[CSS Align](https://www.w3.org/TR/css-align-3/#baseline-sharing-group) and
[Grid3](https://www.w3.org/TR/css-grid-3/#baseline-alignment). Horizontal block-axis
only; orthogonal fallback, legacy numeric minmax sizing and nested-grid export
limits are explicit. No Cargo/CSS/JS changes or held source imports.

Integrator current merged-head workspace/full16 qualification is still required
before publication. No optional census or repeated audit is a merge blocker.
