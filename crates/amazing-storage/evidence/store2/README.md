# STORE-2 pinned WPT selection

Base: `c5192e6` (contains `origin/dev` `e9ddbcc`). WPT revision:
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`. Runner:
`amazing wpt-harness` on win-001, 800 × 600, 10,000 ms per file, separate
storage profile per file. The adjacent JSONL files are the raw before and
after reports.

| Source | Before pass/fail | After pass/fail | After status |
| --- | ---: | ---: | --- |
| `IndexedDB/idbfactory_cmp.any.js` | 0/12 | 11/1 | fail |
| `IndexedDB/idbfactory-open-request-success.any.js` | 0/1 | 1/0 | pass |
| `IndexedDB/idbobjectstore_add.any.js` | 0/16 | 0/16 | fail |
| `IndexedDB/idbobjectstore_get.any.js` | 0/7 | 0/7 | fail |
| `webstorage/event_basic.html` | 0/1 | 0/0 | timeout |
| `webstorage/event_no_duplicates.html` | 0/0 | 0/0 | timeout |

Selected passing subtests: 0 before, 12 after, 0 lost. The add/get sources
fail before invoking IndexedDB because the JS runtime reports `identifier
'dbname' has already been declared`. The webstorage pages wait for
cross-document events; delivery is tracked by IA-STORAGE-EVENTS. The
installed runner does not support the single-document `webstorage/*.window.js`
sources, so those are covered by focused engine/driver tests without WPT
credit. `IDBFactory.cmp()`'s remaining no-argument case fails at the WPT
harness's `TypeError` subtype assertion.
