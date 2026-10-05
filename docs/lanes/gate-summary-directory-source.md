# Unsharded Test262 compact admission

Source-only tooling successor on recorded origin/dev
570b0718a22e6b5bc34a05ff0f13a6955e873973. No engine changes or native
qualification claimed. Preserve the original UTF16 baseline failures.

`gate-summary.py test262` accepts the runner's actual empty shard identity for
unsharded directory selections. It retains `shard: ""`; it does not invent a
numbered shard that could pass the 16-shard merger. Explicit numbered shard
fixture ownership and `test262-merge.py` remain unchanged. Abnormal runner exits,
malformed shard identities, duplicate paths/variants and invalid classifications
are rejected. A normal runner exit 1 is recorded, not converted into success.

The UTF16 workspace caller must add `--status-level skip` to nextest so its
strict native identity collector sees genuine SKIP rows. This ticket does not
change the collector or waive missing identities.

`gate-summary.py bundle INDEX.json OUT.bundle.json` accepts an index of objects
with `role` (before/after), `selector`, `mode` (default/vm), and `compact` path.
All selection identities must be unique. Every original receipt, classification,
variant, source identity and raw hash/path is retained. Identical classification
sets share one lossless table; compressed storage is chunked into <=512 KiB
JSON files with byte lengths and SHA256s in a manifest. The complete manifest
and chunks must fit 2 MiB, or packaging rejects before writing anything. This is
storage, not a new qualification or comparison algorithm. Count the bundle and
other ticket evidence together against the ticket limit; do not also commit its
redundant individual input compacts. Actual 50-report admission is required at
native qualification; synthetic controls establish behavior, not engine passes.

## Prepared bulk native command

Use the supported public Linux launcher on a fresh tooling alias, default
4 vCPU/8 GiB, finite 7200 seconds, only after root grants execution. No Cargo
build or engine execution is needed for these Python-only tooling controls:

```sh
python3 scripts/gate-summary-tests.py > "$OUT/gate-summary-controls.log" 2>&1
```

The controls invoke the actual public Test262 runner against clearly labeled
protocol fixtures: all 25 selectors in default and VM environments, real 0/1
and 16 shards, fixtures, normal runner 1, lossless 50-report and full-size
60,000-file/two-variant bundles, malformed identities/roles/classifications,
existing-destination rejection and file/ticket limits. Raw fixture reports and
logs remain native. Collect only command/source hashes, actual exit and compact
control counts. Before allocation reserve at least 1 GiB native free storage;
no measured capacity or native result is claimed by this source plan.
