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
