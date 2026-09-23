# IA-STREAM fixed-base driver evidence

The driver implementation revision is `2fb0208adb7cd889798f2a8614b89f6c3abe82db` on green dev base `133db582307bf14fd9cd8ece92a47c9620869f0d`. Tests ran only on win-001 in `C:\Users\server\src\wt\integrator-assist` with `CARGO_TARGET_DIR=C:\Users\server\target\integrator-assist`. This is a driver integration gate, not WPT credit.

`document::tests::stream_consumes_before_final_read_and_matches_buffered_decoding` serves a controlled HTTP response. The server withholds all but the first segment until the driver callback records that segment as consumed. It therefore fails if the driver tries to read the last body bytes before feeding the parser. The buffered `parse_document_bytes` result is the content-equivalent before path. The test compares exact DOM dumps and document kind. It splits a UTF-8 BOM, a meta declaration, a multibyte UTF-8 scalar, and a UTF-16 BOM; the large case checks input retention after 100,007 body bytes. The fixture reports the caller-owned chunk plus the HTML byte parser's retained byte input, excluding the DOM and any incomplete HTML token. The table records one fixed-base Windows run; small chunk counts can vary with TCP coalescing.

| HTML case | Body bytes | Chunks | Peak driver byte buffer | Buffered/streamed DOM and mode |
| --- | ---: | ---: | ---: | --- |
| UTF-8 BOM over conflicting charset | 39 | 7 | 43 | exact |
| Split windows-1252 meta | 37 | 5 | 43 | exact |
| Split UTF-8 scalar | 12 | 3 | 18 | exact |
| UTF-16LE BOM | 18 | 4 | 22 | exact |
| Large HTML text | 100,007 | 14 | 8,192 | exact |

`streamed_xml_keeps_buffered_mode_dump_and_fatal_errors` compares valid XML's exact DOM and `Xml` mode, then compares malformed XML's error string and exit code 2 with the buffered path. Existing `xml_dispatch` tests cover XML MIME selection and unsupported exit 3. `http_error_is_rejected_from_headers_without_reading_body` holds back a declared 100,000-byte 404 body and confirms `fetch-parse` exits 1 on the response head without waiting for it. The native GUI local-file decoding test passes through the new streaming path and confirms synthetic file Content-Type does not force UTF-8.

The driver still executes scripts after the document reaches EOF. This ticket makes no claim of streaming script execution or progressive paint. HTML retains no full response body in the driver; XML still collects its byte source for the existing whole-source XML parser. HTML byte decoding remains HTML-2's four-decoder subset.

`source-manifest.json` records SHA-256 of tracked Cargo, crates, scripts, and non-evidence test inputs at the implementation revision. Historical JSON evidence files are excluded because Git text conversion gives them different local and Windows worktree bytes; those files are not build inputs. The final source verification and gate counts are in `gate-summary.json`.
