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
