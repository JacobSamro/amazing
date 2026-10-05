# Captured native proof archival

Independent tooling on recordeda5a56a90e1fa507fa9babb56e6f64636b496f93f.
No engine gate, live producer, old stopped proof, branch or lifecycle is changed.

Boat documents `/srv` changes as captured, while `/tmp` persistence is not
explicitly guaranteed and the local skill says it is wiped on resume. A stop
also does not run ordinary processes' cleanup. Old CSS/tmp raw and1000 measured
ELFs therefore remain unproven after stop, despite a completed snapshot. This
is not a claim that those bytes were lost or recovered.
https://docs.boat.dev/snapshots

`native-proof-archive.py` owns only a fresh `/srv/amazing-proof/lane/source/job`
archive. Root provisions owner-private mode0700 `/srv/amazing-proof`. Admission
checks actual mountinfo (native disk types, never FUSE/tmpfs), canonical paths,
symlinks and potentially applicable snapshot exclusion files. It rejects
ambiguous filters rather than silently omitting proof. Only declared native
terminal raw/retained directory inputs are copied. It requires a real terminal
source/job/exit receipt; failed engine outcomes can be archived without being
reclassified as passing. Source files and previous archives are never removed,
modified or hardlinked; independent copies are bounded by explicit free-space,
copy and time budgets. Unsupported storage is a genuine failure, no fallback.

Every source file is inventoried with actual bytes/SHA/mode/ELF identity, copied
exclusively, read back, and checked against a second whole-source inventory.
The complete destination tree is then rechecked. File and directory fsyncs
precede completed publication. Failed copies preserve originals, partials and
true failure facts. Manifest/hash and original archival command exit0 feed the
public `verify` command before root-controlled idle stop; changed or incomplete
raw/ELFs reject stop-safety verification. This is resource retention, not a new
engine merge requirement or a PID census. A new caller must not archive a live
producer simply because its files appear momentarily stable.

Future producers should write unique raw and measured binary proof into this
captured native root from outset, keeping `/tmp` only for recoverable cache.
Measured binaries need archived copies before execution so expiry does not
rely on a later finally. Current tool archives actual completed producer roots;
it does not rewrite all producers or unpause the reaper. Root alone controls
stop/resume. Native archival readback is distinct from actual restored snapshot
member verification; the latter requires root's canary lifecycle/API check.
No FUSE raw storage or local raw ferry is introduced.

Prepared whole native controls cover actual disk admission and a measured real
ELF, all original file/window bytes, wrong/live producer identities, explicit
capacity, collisions, symlinks/FIFO/exclusion/FUSE/ramfs, whole mutation cluster,
fsync failures, stop verification, and64MiB production CLI archival. Preserve
native outputs under/srv; return compact counts/exit/hash only. Linux tooling
qualification uses the supported wrapper default4vCPU/8GiB finite7200, with an
exclusive alias and actual capacity guards. No local tests/builds. Native job64c6ae296d29494a9e2a6c1573dbafae and its supported outer owner
both exited0 on default4vCPU/8GiB baremetal; all9 controls passed with no
failures/skips. Actual /srv mount was ext4 on /dev/vda1, non-FUSE. The64MiB
CLI archive completed with exact independent readback; raw control output is
/srv/amazing-proof/native-tooling-fd90/controls.log. Compact counts and hashes
are in native-proof-archive.compact.json. Snapshot restore is still pending
root lifecycle and is not claimed. Root's bounded exact CSS11d archival plan is
recorded separately; source731 live inputs and old stoppedbx_pqzyud8e excluded.
