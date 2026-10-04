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
