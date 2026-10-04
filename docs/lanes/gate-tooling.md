# Supported gate tooling

Base: `4f916d3478518a8296a4b788aca0cd4690f491a5` (`origin/dev`).

`linux-gate.sh REF` and `primary-gate.sh REF` run the exact-ref workspace nextest
and doctest gate on Boat. Both phases run; the returned status is the actual
remote job status. The local receipt contains raw-log paths, sizes and hashes.

`test262-boat.sh REF TAG --baseline BASE.json` defaults to 16 shards. Every
background wrapper is waited for and every failure propagates before merge.
A runner exit of 1 means completed conformance failures, not a tooling success
claim. Exits outside 0/1 reject the shard. Compact receipts retain exact file
and variant classifications in a compressed lossless field; the merge rejects
missing or duplicate execution ownership and checks both passing files and
passing variants against the baseline. Existing unrelated failures may remain.
The merged receipt references its compact shard files; retain those together.

`lane-fetch.sh` fetches explicit compact, summary or manifest JSON only, checking
remote size before transfer and SHA after transfer. Raw results, journals,
logs and pixels stay on Boat. Each evidence file is capped at 512 KiB and a
Test262 receipt set at 2 MiB. Lane corpus commands retain their existing
zero-loss comparison, then produce compact evidence using their runner or
`gate-summary.py manifest DEST EXIT RAW...`.

There is no automatic heal or sandbox stop in these gate paths. A tooling
failure allows one clean rerun by the owner; a second is recorded in
`questions.md`, then the lane moves on. No manual raw recovery protocol.

Validation so far: shell syntax, Python AST parsing and diff whitespace checks.
No engine tests or Boat mission were run for this source preparation. The first
supported integration gate supplies the runtime verification. The integrator
owns packaging preflight hooks; this patch does not implement them.

First supported runtime observation (History `17f1d043`): workspace 4,384 passed,
17 skipped; doctests 4 passed; remote phase exits 0/0. Local wrapper exited 3
because `lane-fetch.sh` had mode 0644. The correction records executable mode
and invokes it through Bash from both supported callers, making invocation
independent of checkout mode handling. Nextest counts accept the optional slow
annotation. This observation does not qualify the failed gate; the owner will
perform one clean rerun. The running Test262 job was not changed or restarted.

## Phase failure diagnosis and source sync

Follow-up base: `2f80eb4d224c2a2d5236987df104154e73b83730`.
Test262's remote EXIT trap publishes a small failure manifest naming the setup,
build, runner or packaging phase and its actual exit. It includes hashes and
remote paths for available raw operands, and explicitly marks absent operands.
The supported wrapper waits for the job, then fetches that manifest on failure;
missing diagnostics do not erase the failed wrapper status. Runner exits 0/1
remain measured conformance results; abnormal exits fail the job. No original
live job or raw artifact is recovered or changed.

Exact-ref source transport now excludes only historical evidence JSONL and
oversized evidence output formats. Source, fixture/testdata/test directories,
TSV files and literal Rust include targets are preserved. Git archive excludes
those objects before extraction and transfer. A short sync summary records
omitted bytes/count/path-list hash. There is no new gate protocol or retry loop.

Shell syntax, Python AST and whitespace checks passed. Runtime verification
belongs to the single supported clean rerun after the original gate terminates.

## Activity reporting

Reporting follow-up base: `fd9183a86c85f1dbcb3d6b2a62676354e9357052`.
`pool-report` skips malformed/interleaved rows with an explicit ignored-row count,
accepts legacy four-field records, and rejects nonfinite/reversed times and
invalid lane/pool/exit metadata. `linux-run` writes through local `record-run`,
which validates then locks and appends a complete row. A partial legacy last
line is separated; an activity-write error cannot replace the remote job exit.

The requested metadata-only checks passed: malformed row and overlap metrics,
160 concurrent locked records after a partial historical row, and an exact-ref
temporary Git fixture preserving literal includes, fixture/testdata/test
inputs, compact evidence, source inputs and TSV bytes while omitting only three
historical raw outputs. Command: `python3 scripts/gate-tooling-tests.py` (3
checks passed). No engine builds, Boat calls, raw pulls or live gate changes.

## Pre-job transport diagnosis

Base: `923938ad1cd00df6203fd51206af6c4ca15af6a1`. Shard 0 of the live
Super-read gate failed during rsync, before its remote shard command installed
the phase trap. The old wrapper exited 3 but did not record rsync's own exit;
that value remains unknown. The compact incident file records the local
688-byte diagnostic's hash and exact observed cause.

`linux-run --setup-summary PATH` now records an actual remote-mkdir or rsync
failure separately from wrapper exit 3. Test262 supplies a run-specific local
manifest path and uses this receipt after the failed job status, avoiding an
irrelevant remote phase fetch when no job started. This does not retry or
recover a run. Four metadata checks pass, including distinct rsync/wrapper
exit fields. The live run and Boat endpoints were untouched.

## Capacity incident boundary

The compact `gate-capacity-diagnosis.json` records two successful metadata-only
commands on the assigned shard-0 box. The original Cargo exit 101 is preserved;
its unchanged log ends in warning/help text without a failure diagnostic.
Observed disk/memory capacity is available and the kernel OOM counter is zero.
HOME is FUSE while the native filesystem is ext4. These facts do not establish
Cargo's failure cause, so no speculative capacity or engine change was made.

Python documents exit 120 for cleanup failures, including buffered stream
flush errors ([sys.exit](https://docs.python.org/3/library/sys.html#sys.exit)).
That narrows the repeated runner failures toward cleanup/output I/O; it does
not prove their exact storage error. No third engine rerun, raw pull, lifecycle
command or other active-gate intervention occurred. The unresolved cause is
filed in lane questions for a separate justified repair.

## Native command diagnostics

Diagnostics ticket base: `00e6045c9d506762781392037132880869a9f58f`.
`gate-command.py` waits for the actual command exit, capturing stdout/stderr
in a fresh native `/tmp` directory. After terminal exit it copies output to
HOME and checks size/hash equality. Command/signal exit, output-copy failure
and receipt-publication failure are distinct fields; a copy failure cannot
rewrite the original engine exit. Native raw output and a compact receipt
remain available when the HOME copy fails. The compact receipt includes a
bounded diagnostic tail; raw output is never fetched to the Mac.

Workspace nextest/doctests and Test262 build/runner/packaging use this helper.
Shard failure manifests embed its phase receipt. Test262 result copies are
checked separately from the measured runner exit; output-copy failure blocks
qualification even when the runner returned the usual conformance code 1.
No selectors, timeout policy, baseline classifications or retry budget changed.

Nine metadata-only checks passed, including failing-command exit/tail,
missing/truncated output, output-copy failure, signal status and literal CLI
exit. No engine build/test, Boat allocation or exhausted-run retry occurred.
This improves future diagnostics; it does not establish a cause for the old
warning-only Cargo101 log or Python120 failures.
