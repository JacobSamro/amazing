# Faithful legacy single-byte HTTP fixture

Fixed dependency 04ffdf76 (BASE b67b9d94 plus adopted HTML49 d6f1d1ed), so adopted
codec gains are part of the baseline. Whole pinned encoding directory: 194 files,
12,268 passing assertions and 354 failures. Whole 314 DOM node files also gate.
The actual native baseline wpt_harness.rs was fetched and byte-compared with
04ffdf76; the candidate is frozen as --ref 3c39ed7e before its final gate.

Supports exactly encoding/resources/single-byte-raw.py's pinned HTTP protocol:
the first byte-valued label query argument supplies Content-Type
text/plain;charset=<label>, and the response is the 255 actual bytes 0x00..0xFE.
Empty label is valid; missing required label is a handler error (500 JSON,
without an environment-specific traceback). Native socket tests verify first
label/percent decoding, empty and missing labels, exact header and raw body.
The route verifies the pinned resource with the existing source_bytes seam;
corpus bytes and expected results are unchanged. No engine code copied.

Upstream fixture and body hashes are in provenance.json. This changes the
fixture server, leaving actual HTTP charset parsing and decoding to the engine.
Root/module URL and script-execution hunks are untouched.

Native Boat: 3,649 workspace tests passed, 17 skipped; four doctests passed.
Whole encoding: 12,268/354 → 12,396/226 (+128), zero lost passing multiplicity.
Whole 314 DOM node files: 12,692/287/6 unchanged, zero lost passes, including
all passing assertions in the previous accepted BASE report. All 168 single-byte
TextDecoder assertions remain passing. The baseline's 128 XHR raw-Python-body
mismatches (first byte 100 rather than 0) now pass; 40 pre-existing XHR
Network(Transport) errors remain unchanged and are routed to the driver/net owner.
No codec gains are credited. Exact single-byte reports, full-directory compact
classifications/pass-name multiplicities and source/binary hashes are committed.
Pass-name hashes preserve exact UTF-16 code units, including lone surrogates.
