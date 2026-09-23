# DOM-5 full `dom/` triage, 2026-09-23 checkpoint

This ranks investigation opportunities across the exact 627 tracked sources in
`wpt-dom5-before.json`. The 2 pass / 241 fail / 30 timeout / 221 error /
133 unsupported and 83 passing / 2584 failing subtests are **historical,
pre-outage** results on engine base `11dd8a8`, not a current gate. Pinned WPT
revision is `c2a2fce69e5c2509d9dc28440ec8b42532e5c293`; the runner measures
source files, not expanded WPT URLs. A refreshed pair is pending. Ranking uses
measured breadth and the first observed dependency, not predicted pass uplift.

| Priority | Cluster | Historical coverage | First dependency and ownership |
|---|---|---:|---|
| 1 | Shared `dom/common.js` setup in Range, traversal and node tests | 30 error sources, zero Amazing subtests; 49,514 reference registrations across the same pages | All 30 stop at missing `body.insertBefore`; harness exception formatting masks the first error. The native pre-insert algorithm and its binding are implemented on DOM-5, but JS runtime and same-arena driver adoption are required before WPT can reach it. Later shared fixture steps need CDATA, DOMImplementation and other factories. |
| 2 | Already-running failed sources | 241 files, 1,711 recorded subtests (1,638 fail, 73 pass) | 823 failures report `value is not callable`; this text is a broad symptom, not one root cause. Other leading messages: 170 string expected/undefined, 161 expected true, 50 missing Event, 44 missing MutationObserver, 39 missing NodeFilter. Binding/JS/runtime seams dominate; some algorithm defects may remain after registration. |
| 3 | JS parse barriers | 118 error sources, zero registered subtests | 76 shared `testdriver.js` async object method; 33 await expressions, 6 trailing call commas, 3 surrogate escapes. JS-owned. Downstream subtest counts unknown. |
| 4 | Duplicate-name/formatting diagnostics | 7 error sources, 744 already recorded subtests | 596 `Document-createElementNS`, 100 passive events, 48 shadow Range. Test names and array conversion, frames, events and ShadowRoot are mixed; not a namespace-core-only cluster. JS/runtime and other hosts. |
| 5 | Missing callable outside `common.js` | 35 error sources, 129 recorded subtests | Trace identifies first or nearby operations in 29 sources. Three ChildNode files first stop at `createComment`; three ParentNode files reach `createDocumentFragment` after their first family; six NodeList helper files stop at JS-owned `Math.random` before page code. Other calls span selectors, collection methods, mutation observers, events and document factories. See `callsite-trace-results.json`; cleanup calls sometimes follow a failure, so its last call is not always the cause. |
| 6 | Deadlines | 30 timeout sources, 78 recorded subtests | 26 sources or their support scripts explicitly use frames; four other sources cover two click tests, `EventListener-handleEvent` and SVG `window.onload`. Frame use is a dependency, not proof of one timeout root. |
| 7 | Remaining setup/runtime/resource errors | 31 error sources, 1 recorded subtest in missing-global group | 14 missing globals/interfaces, 11 conversion diagnostics outside `common.js`, 3 missing resources and 3 other runtime errors. The 11 conversion diagnostics resolve to 3 missing globals and 8 callable setup operations; `case.js` is pure JS. |
| 8 | Unsupported source formats/driver capabilities | 133 files, zero subtests | 64 generated `.any/.window/.worker.js`, 55 XML/SVG/non-HTML, 8 URL variants, 3 server templates, 2 HTTPS origins, 1 module/async/defer script. These are runner exclusions, not engine failures. |

The 221 errors partition into 118 parser, 41 conversion (30 shared fixture +
11 other), 35 callable, 14 missing global, 7 duplicate name, 3 missing
resource and 3 other runtime. Together with the 30 deadlines, the diagnostic
inventory accounts for all 251 error/timeout sources. The 241 fail, 133
unsupported and 2 pass sources complete the 627-source selection. No source is
silently omitted from this classification.

## Shared fixture evidence

`setup-exceptions.json` records the original exception just before the
unchanged harness `String(e)` masked it. Each of the 30 `common.js` sources
reaches `document.body.insertBefore` and throws `TypeError: value is not
callable`. The reference-only Edge census in `reference-results.json` measured
49,514 registrations: 34,505 Range boundaries/queries, 5,626 Range mutation,
4,179 Range content, 3,652 node relationship/property and 1,552 traversal.
All 30 reference harness statuses were 0; one reference subtest failed. The
counts measure potential registration in a different browser, **zero Amazing
passes** and no promised uplift from one binding.

The executed diagnostic call tracer covered 46 historical callable/conversion
error sources using disposable copied resources and the saved pre-outage
executable. None changed from a non-parser error to a parser error; 9 emitted
no member-call trace. It is a diagnostic transformation, not WPT. It confirms
the eight masked non-global setup cases first encounter `createComment`
(CharacterData-remove and the Element tag-name pair),
`document.implementation.createDocument` (Document-createAttribute and
Node-textContent), `createDocumentType` (DocumentType-remove),
`createElementNS` (Element-children), or pure-JS `case.js` array/function
operations. In the Element tag-name pair, the last traced call is
`createComment`; the preceding fixture calls ran. See the full trace and
original recovered errors in `callsite-trace-results.json`. Its lightweight
rewriter is not a complete JS lexer, so individual trace entries remain
corroborating evidence, not conformance results.

## Implementable boundary

The DOM-5 branch implements the native checked ParentNode/ChildNode mutations,
node factories, base properties, and a runtime-independent `BindingHost`
contract. The refreshed Windows crate gate passed 157/157 nextest tests and
separate doctests (0 tests in each crate). The first missing APIs in the largest
clusters are **JS-visible bindings**, so these native changes cannot improve
the current WPT runner until js-runtime and the integrator-owned driver install
the contract on the same parsed arena. JS parser, builtins, frames, driver
format support, and resources are outside this lane's owned paths. No WPT
AFTER improvement is claimed here.
