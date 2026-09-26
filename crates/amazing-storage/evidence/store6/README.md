# STORE-6 — Storage Buckets (`navigator.storageBuckets`)

Pinned WPT revision: `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`.

Selection: the `storage/buckets/*.any.js` sources that the installed `wpt-harness`
runner can execute directly. `idlharness-worker.https.any.js` is excluded: it
needs `/resources/WebIDLParser.js` and `idlharness.js`, which are outside this
partial pinned checkout, so it reports a file-level resource error on both the
before and after runs.

| File | Before | After |
|------|--------|-------|
| `storage/buckets/bucket_names.tentative.https.any.js` | 0 pass / 29 fail | 29 pass / 0 fail |
| `storage/buckets/buckets_basic.tentative.https.any.js` | 0 pass / 3 fail | 3 pass / 0 fail |
| `storage/buckets/bucket-storage-policy.tentative.https.any.js` | 0 pass / 1 fail | 1 pass / 0 fail |
| `storage/buckets/buckets_storage_policy.tentative.https.any.js` | 0 pass / 1 fail | 1 pass / 0 fail |
| `storage/buckets/storage_bucket_object.tentative.https.any.js` | 0 pass / 9 fail | 5 pass / 4 fail |
| `storage/buckets/bucket-quota-indexeddb.tentative.https.any.js` | 0 pass / 1 fail | 0 pass / 1 fail |

Before baseline: **0 pass / 44 fail** (binary from the STORE-5 tree, which has no
`navigator.storageBuckets`). After: **39 pass / 5 fail**, zero previously-passing
subtests lost. Raw JSONL: `before/` and `after/`.

Runs execute the built `amazing.exe wpt-harness <wpt-root> <test> 800 600 30000`
with a fresh `AMAZING_STORAGE_DIR` profile per file. The WPT checkout must be
pinned at the revision above and checked out with LF line endings, because the
runner verifies each executed resource byte-for-byte against `git show REV:path`.

Known gaps after this ticket: `StorageBucket.caches` is still the Cache API stub
(parked on NET-26 Request/Response wrappers), and per-bucket quota is reported
but not enforced, so `bucket-quota-indexeddb` and the cache-usage estimate
subtest remain failing. Four `storage_bucket_object` subtests (`expires()` and
`setExpires()` round trips) fail on the JS side because `Date.now` is not
installed — routed to js-runtime; they should pass once `Date.now` lands, which
would take the suite to 43 pass / 1 fail.
