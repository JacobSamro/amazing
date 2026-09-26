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
