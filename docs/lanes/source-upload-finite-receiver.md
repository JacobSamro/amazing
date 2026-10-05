# Finite source upload receiver

Qualified native tooling successor on recorded68b1e6363be41f3867c684040326d54bf07af3af.
The authentic GC clean upload received no final receiver row and no diagnostics
before its 900-second sender cutoff: 44,213,462 bytes, actual124/SSH255. That
receipt establishes a transport timeout, not a capacity failure or a proven
physical cause. NET's separate 440-second upload success remains valid. No old
receipt, packet, archive content, retention rule or engine source is changed.

The concrete lifecycle defect is independent: only the local sender had a
cutoff. Killing local SSH did not provide a finite receiver read/EOF/fsync/hash
or publication lifetime. A full payload with its input stream left open could
block indefinitely before exact-size validation completed.

Production send now wraps the receiver in native `/usr/bin/timeout` with its
own finite kill-after bound. Receiver-local SIGALRM covers stream, EOF, sync,
readback and publication and records TimeoutError/actual124 rather than ready.
An already-armed timer is rejected instead of overwritten. Byte size, streaming
hash, full readback hash, collision-proof publication and final source identity
checks remain. A failure after link still retains published-unknown semantics;
a timeout is never successful staging even if a path exists.

Receiver diagnostic stages use stderr: startup, native capacity admission,
streaming byte progress, EOF wait, sync, hash readback and publication. Final
stdout remains the sole authenticated identity receipt. This distinguishes a
future network/receiver wait from native hashing/sync without reconstructing
old missing observations. It does not claim the unexplained GC physical stall
has been reproduced or fixed. Buffered read1 permits progress on available
chunks, without altering the immutable packet byte stream.

All original public source-upload controls remain. Five added native groups
cover a real64MiB packet, three concurrent immutable packets, native deadlines
before any byte and after exact bytes while EOF remains open, and owned timer
restoration. Existing controls retain truncation/extra bytes/hash/ref mismatch,
capacity, filesystem failures, collisions, mutation and truthful255. These
are native protocol/byte controls, not engine tests or network throughput claims.

Prepare one supported public Linux command on a root-granted exclusive alias,
default4vCPU/8GiB finite7200. The outer public launcher must actually upload the
full frozen source packet to its owned endpoint using this helper; attach its
source-transfer receipt (source/archive bytes/hash/real exit), then run:

```sh
python3 scripts/source-upload-tests.py > "$OUT/source-upload-controls.log" 2>&1
```

For actual concurrent SSH qualification, use public source-upload send on the
same root-provided endpoint in three fresh native directories, with the exact
immutable64MiB packet/hash/ref and finite900-second production receiver deadline, retaining
all actual receipts. Native pipe concurrency controls alone do not establish
remote SSH concurrency. Root supplied exclusive endpoint/profile/ownership for this
qualification; no direct Boat calls, old-GC third invocation or hand recovery.
Require1GiB measured native free capacity for packets and controls; no Cargo
build or engine execution. Native raw stays HOME; only compact actual control
counts, source/command/helper hashes and transfer receipts return locally.
Actual qualification: all20 native controls passed; supported public outer
source upload47,127,822 bytes completed with exact source/hash identity in
376.189 seconds. Three concurrent64MiB realSSH packets passed at production900
seconds, with exact native byte/hash readback; heldEOF actual124 never published.
The initial120-second control budget failed with continuing byte progress and
partial packets, preserved unchanged. ONEclean kept all bytes/controls and used
production900. No oldGC third invocation or physical-cause claim. See compact
actual receipts. Shipping helper/test bytes are identical to testedf661.
