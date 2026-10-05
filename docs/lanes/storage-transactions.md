# Native durable storage transactions

Base: `ccbe84327b1e177cdf05bc4d9b6ce27b433e4b5f` (recorded origin/dev).

The missing area was atomic multi-entry DiskStore updates. Staging isolates writes
and unwinding, the shared quota ledger remains locked from final validation through
log sync and publication, and one CRC batch record supplies the recovery boundary.
Legacy put/delete records and all existing tests are retained. No external code or
new dependencies are used. Browser bindings and transaction scheduling are separate.

Controls: every byte boundary of a mixed put/delete batch across two partitions,
reappend after torn recovery, error/panic/noop rollback, filesystem failure rollback,
shared quota rejection with no partial partition charge, bucket/namespace deletion,
and repeated checkpoints mixed with legacy records. Crash controls simulate disk
images, not physical power cuts. Directory sync on non-Unix remains limited by std.

Linux fixed-ref before/after workspace nextest, doctests and the unchanged 54-file
STORE9 WPT selection run on alias storage-transactions-20261005. Compact evidence
and exact raw paths/hashes are recorded alongside this note after job completion.

Measured Linux gate: workspace **4651 → 4655 passing, zero failed**, 17 skipped
unchanged; doctests **4 → 4 passing**. STORE9: **360 passing, 37 failing, zero
timeout/error/notrun** before and after; exact classifications unchanged, zero
lost passing identities. Source gate commit `d520d81a66f7f02367afe4f73fa29c76b2036860`; the ready commit adds
evidence only, preserving these engine input hashes. Evidence is in
`storage-transactions-evidence/` (raw files remain `/home/user/amazing/out/` on
`bx_g82mzfag`). Final integration/16-shard Test262 belongs to the integrator.
