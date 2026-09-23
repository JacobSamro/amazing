# NET-25 pinned XHR request-header expectations

WPT revision `c48d58747e1f211527fb695fd60548a997fae617`. The 14 exact
upstream files are in `source/` with SHA-256 values in `source-hashes.json`.
The upstream license is in `../upstream-LICENSE.md`.

`../../wpt_xhr_request_headers.rs` ports 198 native assertions from the 12
`xhr/setrequestheader-*` tests. The std-only loopback server ports
`xhr/resources/inspect-headers.py` and `echo-headers.py`, retaining original
field-name spelling and the final space of a combined empty value. Coverage:
state errors, invalid ByteString/name/value inputs, allowed/forbidden request
headers, method override guards, casing, repeated values, `open()` reset, and
string-body Content-Type behavior.

This is a native network expectation selection, not complete JS-level WPT
results. WebIDL coercion of `null`, `undefined`, and omitted arguments, DOM
Document bodies, Blob body variants, and synchronous document XHR need
embedding bindings/runner adoption. The raw upstream tests remain vendored
for that step.

Fixed base `38350f3`: baseline **164/198 pass, 34 fail**. Net-owned failures:
original request header casing lost, valid token punctuation omitted on the
wire, combined empty values losing their trailing SP, duplicate default Accept,
and automatic Content-Type spelling. After fixes: **198/198 pass** on Linux.
