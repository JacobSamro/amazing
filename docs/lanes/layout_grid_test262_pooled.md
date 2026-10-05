# Optional pooled Test262 coordinator

Recorded dev base: `fd87ab4c5467af81758ea59cff179863ffdde6f4`.
Qualified source: `9b0e43d662c799d8d599784505a1c362ee63625c`.

`test262-boat.sh <ref> <tag> --boxes 4` uploads and builds once per native box,
then executes four original shard IDs sequentially on each. All sixteen shard IDs,
JOBS 4, the 60-second timeout, pinned runner/corpus and public zero-loss merger remain
unchanged. Omit `--boxes` to retain the existing backend. Raw reports/logs stay native;
captured HOME output contains bounded compact classifications and phase metadata only.

Dedicated native tooling qualification: **45/45 controls, 0 failures/errors/skips**,
actual job/runner 0, authenticated compact fetch 0. Four fixture builders execute once;
the original runner processes all sixteen shard selections of a 64-file protocol
fixture. Actual positive zero-loss and runtime-error/lost-pass merger exit 1 controls
pass. Missing/duplicate shards and mismatched binaries exit 2; a real child build 101
remains 101 in its phase receipt while the pool exits 3. The synthetic protocol host
proves tooling behavior only; these are not browser conformance numbers.

The first caller was rejected locally (exit 2, no allocation). The first native suite
exited 1 with 30/44 executed because its fixture setup returned 3. Both are preserved.
The fixture omitted the public runner's required async boolean; the corrected source
adds it and an actual missing-async exit 2 negative. Old phase cause was not invented,
and no raw evidence was recovered by hand. The corrected run also captures its own
native phase witnesses automatically in compact proof.

Native box `bx_pah7hnkr`, job `c4543bce1eb24c1793dc83c4c94d5fa6`; finite default
4 vCPU/8 GB/7200-second profile. Supported ensure may renew finite TTL. Root holds raw
and fixture custody. Public production files are byte-exact to the qualified source;
no Cargo or engine changes. The integrator owns the next real full sixteen-shard gate.
