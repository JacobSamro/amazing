# STORE-7 — "Object store not found" cluster

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Selection: the
fixed 32-file pinned IndexedDB list (`selection.json`), run through
`run_selection.py`.

## Root cause

`IDBOpenDBRequest`'s "success" event fired before its versionchange transaction
had actually committed, so a handler that read `db.transaction('test', ...)`
(the shape every `idbcursor-direction`/`idbindex_get(Key)`/
`idbcursor_{update,delete}_objectstore` test uses) saw a pre-upgrade schema and
threw `NotFoundError: Object store not found`. Once that throw escaped a
support.js-wired handler, the harness lost its completion signal for the rest
of the file.

In `crates/amazing/src/runtime_host_storage.rs`, `StorageJob::OpenUpgrade`
scheduled the open request's `RequestSuccess` job unconditionally, right after
scheduling its transaction's `StorageJob::TransactionComplete`:

```rust
self.storage_schedule(StorageJob::TransactionComplete{transaction:transaction_slot})?;
self.storage_schedule(StorageJob::RequestSuccess{request,result:database})?;
```

`TransactionComplete` defers itself whenever a request placed *inside* the
transaction (e.g. the `add()` call a real `onupgradeneeded` handler makes)
hasn't been delivered yet — it reschedules itself to the *back* of the pending
queue (`waiting` check, `push_back`). With two concurrent open+upgrade flows in
flight, that reschedule could land behind the open request's already-queued
`RequestSuccess` job. Traced job order for one such run (two concurrent opens,
`req=7`/`tx=10` for `cd-a`):

```
OpenUpgrade(req=7,tx=10)      -> queues TransactionComplete(10), RequestSuccess(7)
...
TransactionComplete(tx=10)    -> `add()` request (12) still pending -> reschedules (push_back)
RequestSuccess(req=7)         -> fires BEFORE tx=10 actually commits
DispatchListener(7,...)       -> runs onsuccess, which calls db.transaction('test',...)
                                  -- store 'test' not yet committed -> throw
```

## Fix

Stop scheduling the open request's success eagerly from `OpenUpgrade`. Instead,
`TransactionComplete`'s commit (`Ok(())`) branch now looks up the open request
whose `.transaction` is this versionchange transaction (the same lookup its
abort/`Err` branch already used to schedule `RequestError`) and schedules
`RequestSuccess` itself, after dispatching `"complete"` — mirroring the
existing abort-branch pattern. "Success" for an upgrading open request can now
only fire once its transaction has truly committed, matching the IndexedDB
spec's request-processing steps.

Added `indexeddb_two_independent_auto_fail_wired_cursor_tests_both_complete`
(`runtime_host_storage.rs`, `storage_binding_tests`): replicates two concurrent
`createdb`-shaped opens wired the way WPT's `resources/support.js` wires them
(`auto_fail` + `addEventListener` + `Object.defineProperty` on `on<event>`),
each running the same two-`openCursor`-then-done shape as
`idbcursor-direction.any.js`'s `cursor_direction()`. It reproduced the bug
exactly (`NotFoundError: Object store not found`) before the fix and passes
after it. Two earlier triage tests
(`indexeddb_concurrent_independent_opens_each_see_their_own_store`,
`indexeddb_open_request_on_event_setter_honors_own_defined_accessor`) ruled out
two other hypotheses (a concurrent-open engine bug; an own-accessor-bypass bug)
and remain as regression coverage.

## Measured (Boat sandbox, release build of `amazing`)

| | pass | fail | timeout | error |
|---|---|---|---|---|
| STORE-8 baseline (`50c89bb2`, before this fix) | 115 | 75 | 0 | 0 |
| STORE-8 + this fix | **164** | 26 | 0 | 0 |

Zero lost passes: every file's pass count only increased or held steady, and
every file's fail count only decreased or held steady (`before-summary.json` /
`after-summary.json`). Biggest single-file gains: `idbcursor-direction.any.js`
0→5, `keypath.any.js` 0→19 (its one remaining failure is the Array
join/toString cycle-protection bug routed to js-runtime, not this cluster),
`idbindex_count.any.js` 0→4, `idbindex_get.any.js`/`idbindex_getKey.any.js`
1→6 each, `transaction-requestqueue.any.js` 0→1,
`idbcursor-continuePrimaryKey.any.js` 0→2.

Residual failures after this fix (26 fail, 0 timeout, 2 error-status files) are
tracked as the next STORE-7 triage clusters:
- `idbcursor_delete_objectstore.any.js` (2/3), `idbcursor_update_objectstore.any.js`
  (3/6) — partial improvement, same file still has other failing subtests.
- `idbindex_getKey.any.js`/`idbindex_get.any.js` (6/2 each), `idbindex_openCursor.any.js`
  (1/2), `idbindex_openKeyCursor.any.js` (2/2) — partial improvement, residual failures
  likely a different bug (not yet triaged).
- `idbindex-multientry.any.js` — pre-existing, unrelated to STORE-7/STORE-8 (noted
  in the STORE-8 writeup); harness-level `timeout` status with 1 pass/1 fail.
- `transaction-scheduling-ordering.any.js`, `upgrade-transaction-lifecycle-user-aborted.any.js`
  — pre-existing `error`-status files, unchanged by this fix.
- `keypath.any.js`'s one remaining failure — Array join/toString cycle protection,
  routed to js-runtime.
- Unchanged pre-existing fails: `clone-before-keypath-eval.any.js` (5/1),
  `idbindex_getAllKeys.any.js`/`idbindex_getAll.any.js` (17/1, 18/1),
  `idbtransaction_abort.any.js` (2/1), `keypath_invalid.any.js` (23/1),
  `upgrade-transaction-lifecycle-committed.any.js` (1/1).

Gates on the same tree (Boat sandbox):
- `amazing-storage` + `amazing` crate nextest: **307 passed, 6 skipped, 0
  failed** (+1 from this pass: the new regression test).
- Workspace nextest: **3,135 passed, 14 skipped, 0 failed**.
- Workspace doctests: **5 passed, 0 failed**.
