# Public retained gate failure diagnostic

`gate-failure-diagnostic.py` binds a retained native `/tmp` log to the original
hashed terminal observation and gate summary. It requires a genuine nonzero
native exit and a nonzero phase exit on the same exact source and captured job.
It verifies canonical regular-file paths, original byte counts and SHA256 before
selecting failure markers with line-numbered assertion context. It never reruns
engine tests or reconstructs classifications. A fresh diagnostic is at most
64 KiB, with explicit truncation and retained original raw identity.

The public controls cover receipt/source/job/phase mismatches, hashes, native
path custody, missing input, absent failure markers and bounded large assertions.
Qualified36d78 public controls:24 PASS/0FAIL, actual mission/native0 and compact fetch0.
The bounded diagnostic preserves original4bd outer1/nextest100; the retained
SVG assertion expected rotate(30deg), while production emitted
rotate(29.999999999999996deg). No engine rerun or raw transfer occurred.
