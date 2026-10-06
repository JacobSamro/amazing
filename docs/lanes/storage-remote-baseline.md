# Remote baseline references

Independent tooling on recorded973bdec5345bbbf768acd1fa3ef8220d0274b153.
A fresh AFTER sandbox may use an authenticated BEFORE metadata reference;
BEFORE source, raw and retained executables remain on the original sandbox.
This does not claim those executables were read on the receiving sandbox or
survived a stop. Existing same-device full blob readback is unchanged.

Public export, against the original captured completed endpoint:

```
python3 scripts/completed-job.py export-reference --receipt before-endpoint.json \
  --lane LANE --source BEFORE_SHA \
  --storage-summary amazing/out/LANE/BEFORE/storage.summary.json \
  --destination before-reference.compact.json
```

The command requires original completed command/runner0, exact job/source,
and successful catalog-authenticated storage summary. On that endpoint it
checks retention manifest bytes/hash, source/storage-job/profile/counts and
actual device against original capacity admission. It does not open, copy or
move retained ELF blobs. The compact includes original completed catalog,
summary bytes, verified manifest metadata and original endpoint/device/paths.
Reference export is bounded60s and refuses overwrite; authenticity derives
from the captured SSH endpoint, completed-job identity and pinned export hash,
not a signature or fabricated portable proof.

A schema1 AFTER storage plan uses `baseline_source`, `baseline_reference`
(the parsed export object) and `baseline_reference_sha256` (SHA256 of sorted
compact JSON plus newline as emitted by the exporter). `baseline_manifest`
and reference are mutually exclusive. BEFORE accepts neither. Native admission
and execution verify the pinned reference, completed/source/catalog/summary,
manifest metadata and matching Cargo profile. Receipts preserve remote original
box/device/path identity and `local_elf_readback:false`. No remote path is
reopened on the receiver; no original ELF/raw ferry or capacity reduction is
introduced. Capacity still measures the receiving device with unchanged caller
budgets; insufficient space fails before execution. Reference invalidity is a
tooling failure, never a baseline pass or authorization to drop original proof.

Whole native controls include an authentic local completed-job fixture with
measured native /usr/bin/true and retained generated source, all remote metadata
rejection clusters, guarded non-access to original ELF blobs, real AFTER CLI,
local-readback mutation refusal, profile and capacity refusal, and all original
native storage lifecycle controls. The fixture is tooling only, not an engine
baseline or historical original sandbox proof. Linux qualification is through
the supported wrapper, default4vCPU/8GiB finite7200. No local tests/builds.

HTML's original a5a56/b7dc22 BEFORE onbx_zbzscxub remains terminal0 with1003
retained ELFs; observed26.56GB free is below its unchanged32.35GB AFTER budget.
This API enables a fresh receiver without relabeling BEFORE as local. Original
/tmp proof remains subject to its independent archival/lifecycle policy.

Qualification: measured source18efd32c, native completed job4d3ad704d8a848c583486982c5071fdc
and supported outer both exited0. All43 original+8 new groups pass, no skips.
Raw logs and real measured tool ELF remain under/srv/amazing-proof/storage-remote-18efd32c.
First source56f ended1 due test module exception-class mismatch; original43
passed and remote7 passed/1 error. One clean full rerun fixed only the class
reference, retaining all rejection contracts. Original failures are preserved.
Actual public export85919 against HTML bx_zbzscxub/b7dc22 exited0, authenticating
original1003ELF manifest002f metadata without ferrying its payload. The compact
reference is7463bytes/SHA7adc05bb1b0063df62ec1860e402417a29cb737205ff9752030eac5313b15129.
Neither original ELF availability after stop nor receiver-side original ELF
readback is claimed. Full counts/source/native/command/raw hashes are in the
compact evidence; no engine qualification or extra merge barrier is introduced.
