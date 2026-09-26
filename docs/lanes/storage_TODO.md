# Storage lane

## STORE-1

Partitioned Web Storage, IndexedDB, and Cache engines with disk recovery are
merged on `dev`.

## STORE-2: Web Storage and IndexedDB bindings

The driver installs Storage and IndexedDB interfaces over the persistent
storage engines. Web Storage has ordered named properties, methods, local
persistence, session isolation, and quota errors. IndexedDB has factory,
database, transaction, store, index, key range, cursor, request, and event
delivery bindings. The structured value codec distinguishes `undefined` and
`null`. The pinned Windows WPT selection and its raw before/after reports are
under `crates/amazing-storage/evidence/store2/`.

Remaining integration work:

- IA-STORAGE-EVENTS: deliver `storage` events to other same-origin documents.
- JS runtime: fix the `dbname` lexical redeclaration failure that stops the
  selected IndexedDB add/get WPT sources before storage code executes, and the
  `TypeError` subtype assertion affecting the no-argument `cmp()` case.
- The installed WPT runner does not run `webstorage/*.window.js`; add that
  runner mode before claiming their WPT results.
- Cache API JS wrappers need NET-26 Request/Response record accessors. The
  storage engine is ready; the exposed `caches` bridge remains a stub.
- Extend structured cloning to object graphs with cycles and shared references
  when the engine's tree-shaped `StructuredValue` can represent them.

## STORE-5: IndexedDB, StorageManager, and Web Locks

The STORE-5 branch is based on local `dev` merge `34049fcc`, which includes
JSS-SC. IndexedDB now persists the realm-independent structured clone graph,
including cycles, shared references, sparse arrays, typed binary data, Blob,
and File. The storage engine keeps a key-path view of that graph and updates
both representations when it injects an auto-generated key. The graph survives
the IDB disk codec. `navigator.storage` reports profile-wide usage across Web
Storage, IndexedDB, and Cache, enforces a shared quota, and persists grants.
Web Locks exposes request/query and shared/exclusive arbitration. The
`LockCoordinator` API in `amazing-storage::locks` is the IPC seam for
cross-process coordination; same-process documents already share it.

The pinned 32-file IndexedDB WPT selection at revision
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293` went from 52 pass / 76 fail
at the base, to 76 pass / 70 fail after IDB changes, to 79 pass / 63 fail
after JSS-SC adoption and host-value support. The final run has five file-level
harness errors, including three `dbname` lexical redeclaration errors owned by
js-runtime. Raw reports, the fixed selection, and the runner are under
`crates/amazing-storage/evidence/store5/`.

STORE-4 Cache API bindings remain parked on NET-26 Request/Response record
accessors. IPC owns wiring the coordinator across tabs and processes.

## STORE-6: Storage Buckets

`navigator.storageBuckets` implements `StorageBucketManager` (`open`, `delete`,
`keys`) and `StorageBucket` (`name`, `persisted`, `persist`, `estimate`,
`durability`, `expires`, `setExpires`, `indexedDB`, `caches`). Bucket names use
the `[a-z0-9][a-z0-9_-]{0,63}` charset; `open` accepts `{quota, durability,
expires}` with a `[1, 2^53-1]` quota range. Handles are invalidated by an epoch
counter, so every method on a deleted (or deleted-and-recreated) bucket rejects
with `UnknownError`.

`amazing-storage::buckets::BucketRegistry` persists each bucket's policy on disk,
and the IndexedDB engine is now keyed by `BucketKey` so `bucket.indexedDB` gives
each bucket its own database namespace. `StorageBucket.caches` returns a
bucket-scoped `CacheStorage`, but the Cache API surface itself still waits on
NET-26. Per-bucket quota is reported through `bucket.estimate()` and validated on
`open`, but writes are not yet evicted per bucket.

The six runnable pinned `storage/buckets/*.any.js` files are the gate
(`idlharness-worker.https.any.js` needs `idlharness.js` resources absent from the
pinned checkout). Raw reports are under
`crates/amazing-storage/evidence/store6/`.

## STORE-8: fleet-wide event-loop regression, fixed

A cross-lane bisection blamed the IA-FRAME-PHASES render hook
(`crates/amazing/src/js_scheduler.rs`) for the pinned IndexedDB selection
falling from 0 timeouts to 441. Both that path and `amazing-dom`'s event loop
were read and instrumented and are sound. The real bug was this lane's own
STORE-6 commit: `IDBDatabase` member access re-derived its `BucketKey` via a
live `idb::Connection` lookup that threw `TypeError: IDBDatabase is closed` on
every access after first close — including the idempotent second `close()`
call and the read-only getters WPT's support helpers call during cleanup.
Fixed by caching `BucketKey` on `StorageObject::Database` at open time. Raw
reports and full writeup: `crates/amazing-storage/evidence/store8/`.

## STORE-7: "Object store not found" cluster, fixed

`IDBOpenDBRequest`'s "success" event could fire before its versionchange
transaction had actually committed (`TransactionComplete`'s self-reschedule,
triggered whenever a request placed inside the transaction was still
undelivered, could land behind the open request's already-queued
`RequestSuccess`), so a handler reading `db.transaction(...)` right after
`onsuccess` saw a pre-upgrade schema and threw `NotFoundError`. Fixed by having
`TransactionComplete`'s commit branch schedule `RequestSuccess` itself, instead
of `OpenUpgrade` scheduling it eagerly — mirroring the abort branch's existing
`RequestError` scheduling. Pinned 32-file selection: 115 pass / 75 fail → 164
pass / 26 fail, 0 timeouts, zero lost passes. Residual clusters (Array
join/toString cycle protection routed to js-runtime; partial `idbindex_get(Key)`/
`idbcursor_{update,delete}_objectstore` improvements; a few unchanged
pre-existing fails) are listed in `crates/amazing-storage/evidence/store7/README.md`.

## IA-REAL-2: storage opened eagerly at realm init, corrupt store failed every page load

`install_storage_interfaces()` eagerly allocated `indexedDB`/`caches`/
`navigator.storage`/`.locks`/`.storageBuckets` for every realm, and
`storage_allocate` unconditionally opened the on-disk store as its first step
— so a stale/corrupt on-disk store failed page load for every single page,
before any script ran. Fixed by (1) converting those five singletons to lazy
getter accessors (`storage_singleton()`, dispatched through a `"!singleton"`
sentinel member, mirroring the existing `localStorage`/`sessionStorage` lazy
pattern) so storage opens only on first use, and (2) making
`amazing-storage`'s `DiskStore::open` quarantine an unreadable checkpoint/log
aside (timestamped suffix, logged once) and start fresh instead of returning
`Error::Corrupt`, plus a new on-disk format version stamp (`format` file,
`AMSTFMT1`) so a store from an unrecognized future/old version is quarantined
the same way. Workspace nextest 3289 pass / 0 fail / 15 skipped, doctests 4/0,
including `amazing::real_pages::real_pages_corpus_renders_without_crashes` —
the exact 52-real-page repro, now passing.

## STORE-9: widened IndexedDB selection to 54 files, largest residual clusters fixed

Widened the pinned selection with `idbobjectstore_*`/`idbtransaction_*`/
`idbkeyrange*` files. Fixed `getAllRecords()` (new `IDBRecord` interface —
its `key`/`primaryKey`/`value` had to be real inherited getters, not own
properties copied down by `materialize_accessors`, to satisfy WPT's
`assert_idl_attribute`; also unblocks `getAll`/`getAllKeys`'s
`IDBGetAllOptions` dictionary argument), a `count: 0` semantics bug (means "no
limit", not "zero results"), two SIGABRT crashes from cyclic-array IndexedDB
keys, deleted-store/deleted-index wrong-exception-type mapping, an
`IDBKeyRange` open-flag bug, and `delete(range)` doing nothing. 54-file
selection: 258 pass / 123 fail → 359 pass / 38 fail, 0 timeouts, zero lost
passes (verified file-by-file). Residual clusters (an `amazing-js`
structured-clone/transfer bug filed for js-runtime/dom-bindings; cursor
stale-source checks; a few unchanged pre-existing fails) are listed in
`crates/amazing-storage/evidence/store9/README.md`.
