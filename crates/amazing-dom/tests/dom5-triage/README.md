# DOM-5 triage checkpoint — not a conformance result

All browser/engine executions recorded here occurred on win-001 **before the
2026-09-20 UTC SSH outage**. No post-outage gate ran. These diagnostic copies
and reference-browser counts contribute **zero Amazing WPT passes**.

The actual Amazing BEFORE gate is `../wpt-dom5-before.json`: 627 sources,
2 pass / 241 fail / 30 timeout / 221 error / 133 unsupported; 83 PASS / 2584 FAIL,
two identical runs. WPT revision c2a2fce69e5c2509d9dc28440ec8b42532e5c293.

## Proven disambiguation

`probe-setup.py` ran against the saved Amazing BEFORE executable SHA-256
9e6ddc78f3c3e75d849bb8e31b80f754a0dfaaa05ed6769323ede70499a96e4f.
It copied resources, emitted the original setup exception immediately before
the original harness `String(e)`, and traced selected common.js statements.
It did not replace the failing API or remove the error-formatting expression.
The driver explicitly labels this mode `scaffolding-only`.

`setup-exceptions.json` records all 41 conversion-diagnostic sources:

- All **30 dom/common.js** sources reached `document.body.insertBefore(...)`
  and threw `TypeError: value is not callable` before the next statement.
  The earlier presence probe found that method absent. The missing binding is
  the confirmed first blocker; this is **not** evidence of a bad native Range
  or pre-insert algorithm. The harness then masks it with its own String(e)
  failure. Fixing formatting alone does not fix setup.
- Three other sources reveal missing globals: Event, Node, NodeFilter.
- Eight reveal a non-callable setup operation. Their exact call-site tracing
  is unfinished. In particular case.js setup is pure JavaScript: do not assign
  its failure to native DOM from the generic error message.

## Ranked measured opportunity behind the shared setup

`reference-census.py` ran once in **Microsoft Edge 153.0.4234.48 on win-001**.
The 30 pinned pages, common.js and testharness.js remained unchanged; only the
visual reporter was replaced by a count callback. Served over loopback HTTP,
not the full WPT server. All 30 completed with harness status 0. These are
reference registration counts, **not predicted Amazing passes or unblocks**.
One reference subtest failed (NodeIterator-removal), which does not alter the
registration count. Logs were captured locally before SSH failed; the server's
full reference results.json still needs retrieval.

| Rank within the shared fixture | Sources | Reference subtests |
|---|---:|---:|
| Range boundaries and queries | 9 | 34,505 |
| Range mutation response | 10 | 5,626 |
| Range content operations | 5 | 4,179 |
| Node relationships/properties | 3 | 3,652 |
| Traversal | 3 | 1,552 |
| **Total, disjoint source groups** | **30** | **49,514** |

This puts **complete shared fixture setup + Range bindings** ahead of isolated
native Range rewrites as the next investigation. insertBefore is only the first
missing step. Setup also needs Document/CDATA, DOMImplementation document and
doctype factories, comments, processing instructions and fragments. A fixed
first call does not demonstrate all 49,514 become reachable. Existing native
DOM-2/3 algorithms can only be assessed after these binding prerequisites work.
The synchronous factory/ParentNode/ChildNode checkpoint remains a useful first
part of that route; it is not a substitute for the complete fixture.

## Confirmed parser clusters

`parser-source-evidence.json` matches each of the 42 non-testdriver parser
errors to its reported source line. The 76 shared testdriver failures point at
`async subscribe()` in an object literal at pinned testdriver.js:2451.

| JS-owned first parse blocker | Sources |
|---|---:|
| Async object method in shared testdriver.js | 76 |
| await expression | 33 |
| Trailing comma in call arguments | 6 |
| Surrogate escape rejected by lexer | 3 |
| **Total** | **118** |

These sources record no Amazing subtests before failure. Their total downstream
registration counts are **unknown**, so they cannot be numerically compared to
49,514 as though unknown meant zero. Driver automation, frames, animation, and
scheduling can remain further blockers after parsing is fixed.

## Remaining work, explicitly unfinished

`diagnostic-clusters.json` accounts for all 251 error/timeout sources by primary
diagnostic, but is not a completed causal ranking of the whole 627-source set.
The seven duplicate-name errors cover 744 recorded subtests and need JS name/
stringification and undefined-data dependency separation. The 35 callable errors
need per-call attribution (six NodeList support scripts already independently
identified as missing Math.random). The eight remaining masked setup operations
need that tracing too. Timeout source inspection establishes frame dependencies
in 26 of 30 sources; it does not prove that one frame implementation fixes all
26. Four other deadlines involve click activation, listener exception handling
and SVG window.onload. Failed and unsupported sources still need their complete
ranked inventory integrated into the final report.

`trace-calls-DRAFT.py` is **unexecuted**: upload/SSH failed during the outage.
It must be reviewed before use. It rewrites member-call keys on disposable
copies while retaining the receiver, skips strings/templates/comments, but is
not a full JS lexer; regex-literal handling and changed parse failures must be
checked. Its output must never be counted as WPT. Existing setup result JSONL
is append-only in the diagnostic helper: use a new output directory or archive
old results before rerunning.

Resume only after the integrator announces SSH restoration. Retrieve reference
results and remove task-owned `dom-5-reference-census/profile-*` directories and
any task-owned leftover headless processes, without touching user browser
profiles. No cleanup connection was attempted during stand-down.

## 2026-09-23 resume addendum

SSH restoration was explicitly announced. The full reference metadata is now
preserved in `reference-results.json`; all 30 task-owned `profile-*` directories
were removed after checking that no process used their command-line paths. The
former draft tracer was run on win-001 as `trace-calls.py` using a separate
disposable copy of the diagnostic resources. Its 46 records are in
`callsite-trace-results.json`; no non-parser diagnostic changed to a parser
error, but the rewriter is not a complete JS lexer. Its output remains
non-conformance diagnostic data. `full-selection-ranking.md` accounts for all
627 historical sources and states the implementable boundary. Those historical
counts still need replacement by a fresh current-base gate.
