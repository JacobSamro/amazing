# LAYOUT-18 measurement inputs

Pinned WPT: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`, 800 × 600.
`flex-grid-manifest.json` contains 1,446 eligible static cases (518 flexbox,
928 grid). The frozen `6db18c89` dev baseline is 614 pass / 832 mismatch,
with no errors, unsupported cases, crashes or timeouts.

Source checkpoints:
- `c2cfe242`: used-size math, flex wrap-reverse and grid-lanes fill-reverse.
- `8ebd1dcb`: auto-fit empty-track collapse, gap/repetition counts, repeated names.
- `83e497a0`: cyclic percentage contributions, padding/border counted once,
  and stretching of minmax tracks whose maximum is auto.

Linux iteration only: `linux-grid-repeat.log` has 295 passing tests;
`linux-intrinsic.log` has 298 passing tests. Both skip one timing diagnostic
and contain zero doctests. Windows final measurements are pending.

Rules: [Grid repetition](https://www.w3.org/TR/css-grid-2/#auto-repeat),
[cyclic percentage contributions](https://www.w3.org/TR/css-sizing-3/#cyclic-percentage-contribution).

`run-fast.ps1` executes the repository render harness with only its per-pixel
comparison loop replaced by equivalent C#. Reference discovery, process
execution, fuzzy limits, PPM parsing and result serialization remain the
original harness. On win-001, the helper matched eight exact image pairs and
four mismatching pairs against the original comparison function: identical
pixel counts, maximum channel differences, threshold decisions and diff-file
SHA-256s. See `comparator-parity-{pass,mismatch}.log`. The interrupted original
candidate sweep is diagnostic scratch, not a completed measurement.

Candidate renderer for source `83e497a0`:
`27E77E7728FFC34FA72A5BC3B4761E6639FFCD288B0205BCC9D1E33F93F4DD22`.


Restart recovery: `flex-grid-after.json`, `retained-after.json`, and
`css-values-after.json` are the completed **2287c1bf** Windows reports.
The paired `*-comparison.json` files report respectively 47 gains/5 losses,
2 gains/1 loss, and 3 gains/0 losses. These are pre-repair measurements.
`loss-manifest.json` isolates all six losses; both `windows-loss-*.log`
reruns reproduced all six mismatches. The workspace gate did not run.

Repair source is `012f017a`; `linux-loss-repair.log` measures 302 passed,
one skipped, zero doctests. `linux-loss-red.log` preserves failing geometry
reproductions. CSS must still accept sizing stretch keywords before the
retained upstream case can exercise the new layout handler.
The first repair Windows upload failed; its log is retained and no Windows
repair gate is claimed. See the lane decision map for remaining work.
