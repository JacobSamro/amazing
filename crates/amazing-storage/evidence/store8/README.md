# STORE-8 — fix the cross-lane event-loop regression

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Selection: the
same fixed 32-file pinned IndexedDB list used for STORE-7
(`crates/amazing-storage/evidence/store7/selection.json`), run through
`crates/amazing-storage/evidence/store7/run_selection.py`.

## Root cause

Not the suspected IA-FRAME-PHASES render hook (`crates/amazing/src/js_scheduler.rs`,
commits `939942bb`/`2e80541c`/`3cafe76a`) or `crates/amazing-dom/src/event_loop.rs`.
Both were read and instrumented; `EventLoop::rendering_opportunity` only queues one
`TaskSource::Rendering` task that round-robins with every other task source in
`next_task`, and `pump_at_with_render` only fires it once `step()` goes idle, then
keeps draining further tasks — no starvation path found there, and a `dom/events`
sweep (below) shows the render/task-scheduling path is healthy.

The actual regression is a bug in this lane's own STORE-6 commit `c1e29741`
(`crates/amazing/src/runtime_host_storage.rs`): `IDBDatabase` member access
(`"name"`, `"objectStoreNames"`, `close()`, event-handler getters/setters, etc.)
derived the connection's `BucketKey` by looking up the live `idb::Connection`:

```rust
let key = self.state.borrow().storage.as_ref().unwrap().idb.borrow()
    .connection(connection).map(|connection| connection.key.clone())
    .ok_or_else(|| Throw::type_error("IDBDatabase is closed"))?;
```

Once a connection was closed, every later member access — including the
spec-idempotent second `db.close()` call and read-only getters WPT's IndexedDB
support helpers call during per-subtest cleanup (`db.close()`, `db.name`) — threw
an uncaught `TypeError`. WPT's cleanup step never signaled completion, so the
first subtest in a file passed and every later subtest in that file (and every
later file) piled up as a harness timeout. `IA-FRAME-PHASES` and STORE-6 landed
in the same window on `dev`, which is why the render hook looked like the prime
suspect.

## Fix

Cache the `BucketKey` on `StorageObject::Database` at connection-open time
(`crates/amazing/src/runtime_host_storage.rs`) instead of re-deriving it from a
live `idb::Connection` lookup on every member access. `key` is added to the
`Database` variant and to `StorageJob::OpenUpgrade`, threaded through both open
paths (`idb.open` success and the blocked/upgrade-needed path) and the
`OpenUpgrade` job handler. All existing `match`/`if let` sites on
`StorageObject::Database` already used `..`, so this is additive.

Added `indexeddb_database_members_survive_close` regression test
(`runtime_host_storage.rs`, `storage_binding_tests`): opens a database, and after
`onsuccess` calls `db.close()` twice with a `db.name`/`objectStoreNames` read in
between, asserting all three survive without throwing.

## Measured (Boat sandbox `bx_cxad649k`, release build, `amazing` binary)

Bisected directly (no ref-by-ref history walk needed once the repro was
reproducible on current `dev`): reverting only the uncommitted fix on the exact
same `dev` tip (`70009676`) reproduces the reported regression almost exactly,
confirming attribution.

| | pass | fail | timeout | error |
|---|---|---|---|---|
| current `dev` (`70009676`), fix reverted | 19 | 90 | **441** | 0 |
| current `dev` (`70009676`) + fix | **115** | 75 | **0** | 0 |

(`before-summary.json`/`after-summary.json` are the raw `run_selection.py`
summaries. Per-file `pass`/`fail` mix differs from the ticket's ~72/74 baseline
because the pinned selection has moved on with STORE-6/STORE-7 fixes since
`d0d6af6b`; the signature that matters — 441/437-ish timeouts collapsing to 0 —
is reproduced and fixed.)

Cross-lane check: `dom/events/*.any.js` (9 files, 41 subtests) through the same
binary — **0 timeouts**, confirming the general task/render scheduling path
(owned by other lanes) was never the problem and is unaffected by this fix.

Gates on the same tree (Boat sandbox):
- `amazing-storage` + `amazing` crate nextest: **304 passed, 6 skipped, 0 failed**
  (includes the new regression test).
- Workspace nextest: **3,132 passed, 14 skipped, 0 failed**.
- Workspace doctests: **5 passed, 0 failed** (`amazing_a11y`, `amazing_devtools`,
  `amazing_html`, `amazing_js`, 1 each).

Zero lost passes anywhere.
