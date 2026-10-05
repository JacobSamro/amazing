# Native target storage and executable retention

Recorded origin/dev base `ac72a9687c8a1ff76c3d0eb47db98a321ad263b8`. This is a separate tooling ticket, with no engine gate or retry of unqualified Reporting source6e44/895b.

The actual exclusive Reporting box inventory showed `/tmp` and HOME share device64769, total73,635,184,640 bytes, with zero free space. Native `/tmp` targets prevented lazy snapshot reads but did not provide separate capacity. Three retired exact source targets contained22,838,290,132 logical bytes. Authorized cleanup removed only recoverable Cargo products; every retained ELF and source file was rehashed, while source trees, raw output and HOME evidence were untouched. Compact receipts record actual removed bytes and free space; the full path/content manifest stays on the box.

`scripts/native-storage-retention.py` is an opt-in public helper. It owns no Boat lifecycle, SSH transport, supervisor exit witness or ledger reserve. CSS owns those wrapper/exit repairs. No held or UNKNOWN box is stopped, and no branch is removed.

`check` can run before bootstrap/output-directory writes. It sums build and retention budgets when `/tmp` and OUT share a device, checks actual available bytes, prints a compact admission receipt, and allocates nothing. Callers supply explicit budgets; these are capacity admission estimates, not a guarantee about an arbitrary future build's size. The default additional reserve is64MiB, independent of the supervisor's private128KiB ledger reserve.

`run` wraps the native batch after bootstrap. It checks the exact source against native COMMIT, accepts only a fresh ticket/source/phase/job target beneath `/tmp`, and holds an exclusive lease until the actual command returns. It preserves the CSS profile policy (`CARGO_PROFILE_DEV_DEBUG`/`TEST_DEBUG=line-tables-only`) when those variables are unset and records any caller override; paired profiles must match. An AFTER requires the explicit successful BEFORE executable manifest/source, verifies every retained byte, and creates a fresh target without copying baseline caches.

After actual command exit, all ELF and generated-source artifacts are copied to content-addressed, verified executable files beneath the explicit OUT retention root. The manifest maps their original paths to frozen blobs; baseline executables remain runnable after Cargo cache retirement. No raw output, source checkout or baseline executable is deleted. Only after manifest publication and re-verification is the managed terminal Cargo target removed. Copy/integrity failure preserves the target. A live lease, RUNNING/UNKNOWN state or incomplete retention rejects manual retirement. Failed command exits and native signal exits remain separately recorded; storage success never turns a failed command into a passing gate.

Example native command (budgets must come from the caller's measured workload):

```sh
python3 scripts/native-storage-retention.py run \
  --ticket reporting --source "$COMMIT" --phase before --job before-1 \
  --output-root "$OUT" --retained-root "$OUT/frozen-executables" \
  --build-budget-bytes "$BUILD_BUDGET" --retention-budget-bytes "$ELF_BUDGET" \
  -- bash -c "$NATIVE_BATCH"
```

For AFTER add `--baseline-manifest <before manifest path> --baseline-source <exact before SHA>`, plus a distinct job identity. To preflight before bootstrap use `check` with the same scratch/output/budget arguments. No linux-run file is edited by this ticket; the minimal opt-in preflight and command-wrapper seams are requested from the wrapper owner.

Whole tooling controls use the actual native `/usr/bin/true` ELF and native filesystem. They cover frozen byte/executable/source retention, fresh paired targets, deduplication, actual capacity rejection and shared-device summing, profile/source mismatches, corrupt baselines, failed/signal exits, reused identities, symlink escape refusal, live leases, explicit invalid UNKNOWN ledger fixtures, missing baseline executables and output-owner separation. They do not build or test any engine source and do not infer old job completion.

Qualification: exact source `a3aef6fde7b4eba91162cb81449fecf5b86667c6`, actual supported wrapper0 and compact receiver0, all18 native tooling controls PASS. Initial8ab recorded17PASS/1fixture assertion failure: both phases use the same actual tools launcher source, so the corruption control now asserts absence of the AFTER phase while retaining the BEFORE directory. The native helper bytes are unchanged; every original control identity is retained, gained1/lost0. Raw logs and full cleanup manifest remain on the box under HOME, with paths/hashes in compact receipts. No engine workspace/docs/Test262/WPT qualification is claimed by this tools ticket, and Reporting895b remains source-only. Only documentation/evidence differ in the final ready squash; qualified helper and control bytes remain exact.
