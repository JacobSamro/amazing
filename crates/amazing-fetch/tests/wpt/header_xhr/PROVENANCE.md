# NET-24 pinned Fetch Headers and XHR header expectations

WPT revision: `c48d58747e1f211527fb695fd60548a997fae617`.
The 19 exact upstream files are in `source/`, with SHA-256 values in
`source-hashes.json`. They came from `web-platform-tests/wpt` at the pinned
revision. The upstream license is in `../upstream-LICENSE.md`.

`../../wpt_headers_xhr.rs` ports 332 native expectations from those tests:
case-insensitive lookup and mutation, duplicate combining and sorted entries,
no-cors guard behavior, header value normalization for all selected control
bytes, request wire echo, and XHR response header lookup/combining/cookie
exclusion. The `headers.py` response and seven raw `.asis` responses are served
by `../../../../amazing-net/tests/support/wpt_server.rs` on loopback. Raw
fixtures retain their upstream bytes; the shim only converts LF framing to
HTTP CRLF framing when serving them.

The selected expectations are not full JS-level WPT file results. Constructor
WebIDL conversion, JS iterator descriptors, worker variants, the dynamic
`not-cors-safelisted.json` resource, and all other assertions outside the
listed native matrix await binding/runner adoption. The source files remain
available for that next step.

Fixed base `a8b9386` baseline: **128/332 pass, 204 fail**. The failures were
header value normalization and consequent rejected control-byte wire cases.
After `normalize_value` follows Fetch's HTTP whitespace edge trimming and
rejects only NUL/internal CR/LF/non-ByteString code points: **332/332 pass**.
