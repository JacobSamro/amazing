# amazing-storage

A std-only, unsafe-free engine for partitioned storage. `StorageKey` combines a
serialized origin and top-level site; callers must provide canonical values.
Buckets carry the storage key and a bucket name. `MemoryStore` applies quota to
the sum of namespace, entry-key and value bytes for an origin partition, offers
transaction rollback, and evicts the least recently accessed non-persistent
bucket. Bindings decide persistence permissions, quota amounts and event dispatch.

## Disk format, version 1

`DiskStore` contains `checkpoint.0`, `checkpoint.1`, and `log` files. All numbers are little endian.
Strings and byte arrays have a `u32` byte-length prefix. Each entry key encodes
origin, top-level site, bucket name, namespace, then raw key bytes. A checkpoint
is `AMSTSN01`, `u64 generation`, `u32 payload length`, payload, then IEEE
CRC-32 of the payload.
The payload is `u32 entry count` followed by entry key and length-prefixed value
pairs. A log starts `AMSTLOG1`. Each record is `u32 payload length`, payload,
CRC-32. Payload opcode 1 is put (entry key plus value); opcode 2 is delete
(entry key). Opcode 3 is an atomic batch: `u32 operation count` followed by
that many opcode-1/opcode-2 operations. Existing single-operation records remain
readable; older implementations cannot read batch records. Records are capped at 64 MiB. A complete checksummed record is the
commit boundary. Reopening ignores a torn final record and truncates that tail before new
writes. Bad checkpoint data is rejected. Compaction writes and syncs an
inactive slot temporary file, atomically renames it into that slot, then
replaces the log with its header. The latest valid generation wins on reopen;
the other slot remains a crash fallback. Replaying a pre-compaction log is
idempotent for puts and deletes.

`DiskStore::transaction` stages a callback against a private `MemoryStore` clone,
checks final per-partition and shared-profile quotas, and publishes the complete
entry delta in one synced batch record. Callback errors, panics and persistence
errors leave committed entries and quota accounting unchanged. The exclusive
mutable borrow serializes transactions; this is a native synchronous primitive,
not an IndexedDB request scheduler. Quota settings and access clocks remain
in-memory metadata. Empty entry deltas require no log record. A failed append
repairs its original log length; failed repair blocks further writes until reopen.
On Unix, new log files and checkpoint renames also sync the storage directory.

`StorageArea` preserves UTF-16 code units and insertion order. `IndexedDb` takes
owned `StructuredValue`s; the caller maps script values, async request events,
and transaction task boundaries. Cache request and response bodies are owned
bytes. These engines do not yet expose browser bindings.

The persistent adapters (`PersistentWebStorage`, `PersistentCacheStorage`, and
`IndexedDb::with_disk`) encode each area, cache or database as one log value.
An IndexedDB readwrite/versionchange transaction publishes its full committed
database only after the log record has synced. Relaxed durability is presently
accepted as a scheduling hint; all disk commits use strict sync semantics.

## Engine-level conformance cases

The engine assertions are derived from WPT `IndexedDB/` at commit
`c39e25f16d8d768ac3f00d441d6a88bfaff81d0d`:

- `idbkeyrange-includes.any.js`: open and closed bound inclusion.
- `idbobjectstore_createIndex.any.js`: unique index conflict and key path extraction.
- `idbcursor-advance.any.js`, `idbcursor-continue.any.js`, and
  `idbcursor-continuePrimaryKey.any.js`: cursor ordering and advancement.
- `idbversionchangeevent.any.js`: upgrade and versionchange version fields.
- `open-request-queue.any.js`: versionchange notification before a blocked
  request, and release after the old connection closes.

The Rust tests exercise these engine-level expectations; they are not WPT
harness results. Browser bindings and event-loop integration remain separate.
