# STORE-9 — widened IndexedDB selection, largest residual clusters

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Selection: the
widened 54-file pinned IndexedDB list (`selection.json`), run through
`run_selection.py` (its `--detail` flag prints every non-pass subtest for
cluster triage). Base: merged dev `47355c3c` (after STORE-7).

Before -> after, whole selection: **258 pass / 123 fail / 0 timeout -> 359
pass / 38 fail / 0 timeout**. Per-file statuses: 22 pass / 24 fail / 2 timeout
/ 6 error -> 29 pass / 19 fail / 2 timeout / 4 error. Verified zero lost passes
file-by-file against the saved before-detail listing: every file's fail set
either shrank or stayed byte-identical; none grew or gained a new failing
subtest.

## Clusters fixed

1. **`getAllRecords()` missing** (largest cluster, ~58 subtests across
   `idbobjectstore_getAllRecords.any.js`, `idbobjectstore_getAll(Keys)-options.any.js`,
   plus `getAll`/`getAllKeys`'s own dictionary-argument tests, which WPT's
   `resources/support-get-all.js` gates behind `'getAllRecords' in queryTarget`).
   Added the `IDBRecord` interface, a `storage_get_all_args` helper that
   accepts both the legacy `(query, count)` and the new `IDBGetAllOptions`
   dictionary form, and dedicated `getAll`/`getAllKeys`/`getAllRecords`
   handling in both `storage_store_invoke` and `storage_index_invoke`, reusing
   a new `idb::Cursor::all(limit)`.
   - A first pass created each `IDBRecord` as a plain object with own
     `key`/`primaryKey`/`value` data properties. That satisfied
     `assert_class_string` (needs `Symbol.toStringTag`) but failed WPT's
     `assert_idl_attribute`, whose message is explicit: *"property "key" found
     on object expected in prototype chain"* -- WebIDL readonly attributes
     must be inherited getters, not own properties. Root cause:
     `storage_allocate` calls `materialize_accessors`, which copies every
     prototype accessor down onto the instance as an *own* property -- needed
     for other storage wrappers' host-object property lookup, but exactly what
     breaks this assertion for `IDBRecord`. Fixed by (a) giving `IDBRecord`
     real getter members (`storage_interfaces.rs`) dispatched through a new
     `SI::IDBRecord` arm in `storage_idb_invoke` backed by a new
     `StorageObject::Record{key,primary_key,value}`, and (b) skipping
     `materialize_accessors` specifically for `kind==SI::IDBRecord` in
     `storage_allocate`.
   - Per spec, a `count` of `0` (explicit or from an omitted/`undefined`
     dictionary field) means "no limit", not "zero results" -- fixed in
     `storage_get_all_args` (`if count==Some(0) {count=None}`), which also
     fixed the pre-existing "zero maxCount"/"Zero count" failures.
2. **Two SIGABRT crashes** (`idbkeyrange-includes.any.js`,
   `idbobjectstore_openKeyCursor.any.js`; each aborted the whole worker,
   losing every remaining subtest in the file): a self-referential array used
   as an IndexedDB key (`var a = []; a.push(a);`) recursed `storage_to_key`
   infinitely. Fixed with ancestor-chain cycle detection (identity-based via
   `Rc::ptr_eq`, not structural, so distinct-but-equal sibling arrays are not
   false-flagged) that throws `DataError` instead of recursing.
3. **Deleted-store/deleted-index wrong exception type** (~10 subtests across
   `idbobjectstore_{add,clear,count,delete,deleteIndex}.any.js`,
   `idbindex_{get,getKey,openCursor,openKeyCursor}.any.js`): operating on a
   handle whose underlying store/index had been deleted threw `NotFoundError`
   (from the engine's fresh by-name lookup) instead of spec-required
   `InvalidStateError` (a stale-handle condition). Fixed with
   `storage_idb_error_for_handle`, and for `IDBObjectStore.index`/
   `createIndex`/`deleteIndex` specifically, splitting the **stale-handle**
   check on the containing object store (must map to `InvalidStateError`) from
   the **fresh by-name lookup** of the index name itself (must still map to
   `NotFoundError`, e.g. `idbobjectstore_deleteIndex.any.js`'s "removes the
   index" case) via a `(bool, idb::Error)` tag, rather than a uniform redirect.
4. **`IDBKeyRange.lowerBound()`/`.upperBound()` open-flag bug** (2 subtests):
   both constructors set the *wrong* bound's open flag. Fixed in
   `storage_range_invoke`.
5. **`delete(range)` on an object store silently did nothing** (1 subtest,
   `idbobjectstore_delete.any.js`): fixed the range-delete path in
   `storage_store_invoke`.

## Residual clusters (not fixed, filed or deferred)

- **`unreachable transfer entry`** (5 files, one subtest each --
  `idb{index,objectstore}_getAll(Keys)(-options|Records).any.js`'s "invalid
  query keys" tests): a genuine `amazing-js` bug, not storage's. WPT's
  `createDetachedArrayBuffer()` helper transfers a buffer via
  `postMessage('', [buffer])` without the buffer being reachable from the
  posted value, which is spec-valid, but
  `crates/amazing-js/src/structured_clone.rs` incorrectly requires every
  transferable to be reachable from the serialized value graph. Filed in
  `.agents/storage/questions.md` for js-runtime/dom-bindings.
- **`idbcursor_{delete,update}_objectstore.any.js`** (9 subtests, unchanged):
  cursor operations don't yet re-check whether the cursor's *source* (as
  opposed to the object store handle checked above) was deleted or its
  transaction is inactive; `DataCloneError`'s `code` property is also missing
  on a thrown error object. Needs the same stale-handle treatment cursors
  don't have yet -- left for a future pass, not regressed by this ticket.
- **`idbindex_get.any.js`/`getKey`/`openCursor`/`openKeyCursor`'s "deleted by
  an aborted upgrade" case** (4 subtests, unchanged): still throws
  `TransactionInactiveError` instead of `InvalidStateError` -- a different
  code path (an aborted, not merely deleted, index) from the one this ticket
  fixed.
- **`idbobjectstore_createIndex.any.js`** (5 subtests, improved from 6):
  object-identity mismatch on the returned `IDBIndex.objectStore`, and
  `Constraint`/`SyntaxError` not yet mapped to their DOM exception names in a
  couple of createIndex-specific paths.
- **`idbtransaction_abort.any.js`**, **`idbtransaction_objectStoreNames.any.js`**,
  **`keypath.any.js`** (the js-runtime-owned `Array` cycle-to-string bug, see
  STORE-7's README), **`keypath_invalid.any.js`**, **`idbobjectstore_getKey.any.js`**
  (still times out), **`idbobjectstore_put.any.js`** (2 subtests),
  **`transaction-scheduling-ordering.any.js`**,
  **`upgrade-transaction-lifecycle-{committed,user-aborted}.any.js`**:
  unchanged from before this ticket, not investigated further here.

## Regression coverage

`idb.rs`: `Cursor::all(limit)` (reuses `Cursor::new`'s existing sort/dedup/
direction-reversal logic; no new engine-level test needed, it's exercised
end-to-end by the getAllRecords/getAll/getAllKeys driver tests and the WPT
selection itself).
