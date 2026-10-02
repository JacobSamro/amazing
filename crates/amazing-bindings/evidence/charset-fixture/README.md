# Pinned character-set fixture server support

Fixed DOM base `20a85c17`; HTML46 dependency `40f6e604` adopted as `342a09d5`.
The two pinned Document-characterSet-normalization tests have **654 failures**
(315+339), all from null contentDocument. The runner served the Python endpoint
as a static application/octet-stream file, so no HTML document loaded.

The backend now answers `dom/nodes/encoding.py` requests according to the
unchanged pinned handler. It uses the first `label` form parameter, decodes its
bytes isomorphically, escapes it in a meta charset declaration, and returns a
UTF-8 HTML response with status200 and **no Content-Type**. The HTML46 adapter
uses actual response signatures to detect HTML, the existing decoder chooses
its encoding, and canonical encoding names populate document metadata.
Meta x-user-defined normalizes to windows-1252 through HTML's decoder. Root
and inert-document metadata also use HTML's canonical-name API.

No expected encoding label is assigned to the engine, no WPT file/assertion is
rewritten, and no upstream code algorithm is copied. Missing label returns
500/application-json; the environment-specific upstream traceback is omitted.
Native regression verifies first-label and escaped Latin1 semantics and reads
the actual socket response to ensure the successful handler omits Content-Type.

`record-protocol.py` runs the pinned upstream endpoint unchanged on Linux,
recording response bodies for all218 labels plus four protocol edge cases.
Raw hashes cover the test/helper/handler and wptserve handler/response framework.
Baseline assertions are in before.json; after.json records all654 passing assertions.

Boat native final gate:3,531 workspace tests passed,17 skipped; four doctests
passed. Complete314 dom/nodes files:12,028→12,682 passing subtests (+654),
951→298 failed subtests, six errors unchanged, **zero lost passes**, comparing
(test,name) multiplicities. One previously timed-out file now reports its
remaining failure; its passing count stayed zero. Both normalization pages pass
315/315 and339/339. The prior451 legacy encoding and14 unsupported-encoding
XHR assertions all remain passing. gate.json records compact full-file
classifications and the frozen binary hash; protocol.json stores upstream
responses; encoding-after.jsonl stores the encoding assertions. A hard-TTL
interrupted attempt is excluded; the complete exact-source gate was rerun.

Protocol references at pinned WPT c2a2fce:
[endpoint](https://github.com/web-platform-tests/wpt/blob/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/dom/nodes/encoding.py),
[FunctionHandler](https://github.com/web-platform-tests/wpt/blob/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/tools/wptserve/wptserve/handlers.py),
[Response](https://github.com/web-platform-tests/wpt/blob/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/tools/wptserve/wptserve/response.py),
[request parameters](https://github.com/web-platform-tests/wpt/blob/c2a2fce69e5c2509d9dc28440ec8b42532e5c293/tools/wptserve/wptserve/request.py).
