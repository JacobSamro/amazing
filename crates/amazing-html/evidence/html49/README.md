# HTML49 codec EOF/error recovery and encodeInto

Fixed corpus baseline HTML48 5021c0e5; source also contains the separate
explicit-mode shadow-fragment wrapper 1e8cbad9. Pinned WPT
c2a2fce69e5c2509d9dc28440ec8b42532e5c293. No new upstream port.

UTF16 end-of-input now clears a pending high surrogate and partial byte in
one error. Fatal errors consume malformed input before returning, retaining
restored bytes and ISO2022JP mode for subsequent streaming calls. The existing
TextDecoder adapter already preserved stream state; stale pending bytes in
the codec caused the two fatal-stream failures. TextDecoder now rejects the
replacement encoding while the core retains it for HTML decoding.

TextEncoder.encodeInto converts its source to USVString, validates a genuine
Uint8Array, writes complete UTF8 scalars into its existing backing store, and
returns consumed UTF16 units and written bytes. Two parent-granted public JS
helpers validate the internal view kind and write respecting view offsets,
bounds, detachment, and immutable backing. No shared VM/value-model changes.
Grouped tests cover invalid kinds, empty/boundary/overflow writes, detached
views, foreign-interpreter views, shared backing, resizable out-of-bounds
views, surrogate replacement, partial scalar capacity, and bytes outside
destination views. Every UTF16 input truncation/split and repeated EOF is
also tested. One stale native expectation of two replacement characters for
high-surrogate+partial-byte input was corrected to the standard one.

Native Boat gate: 1050 owned passed/7 skipped (including amazing-js), 3556
workspace passed/17 skipped, four doctests. All62 tree files retain1894/1894
CLI passes; expanded1930pass/6unsupported records are identical to HTML48.
The72-page syntax selection is byte-for-byte unchanged at2650pass/58fail.
Full194-file encoding sweep improves12216/406 to12268/354, with52 gained
assertions and zero lost passes:42 encodeInto,6 replacement constructors,
and4 UTF16/fatal-stream cases. Files27pass/17fail/1timeout/8error/141unsupported.
Exact name-byte SHA256/status records retain multiplicity and tolerate async
subtest order; encoding-status.json and encoding-comparison.json record
before/after and gains.

The whole existing backing-store Test262 selections (ArrayBuffer,
SharedArrayBuffer, TypedArray including TypedArrayConstructors, DataView,
Atomics) preserve every file/variant classification and input hash:6486
passing,158 failing,230 unsupported variants before and after, zero losses.
binary-comparison.json records all selected paths/variants and classifications.

The first owned attempt caught the stale UTF16 expectation; the corrected
source passed all checks. A malformed run.py CLI invocation stopped only
the remaining corpus sweep; it was completed in one continuation against
unchanged compiled source with binary hashes verified before/after. The
shared explicit-mode shadow-fragment API was then committed separately.

Reproduce through MAIN linux-run.sh html --host boat, native /tmp source and
target: build release amazing/test262_host/html5lib_host; owned/workspace
nextest; workspace doctests; ensure-wpt.sh; scripts/html5lib.py all62 files;
HTML45 run.py CORPUS --binary BIN --output JSON; HTML46 encoding_sweep.py;
HTML48 syntax_sweep.py; scripts/test262.py --only built-ins/AREA for the five
areas above. Corpus conformance failures are recorded, and every prior pass
is compared rather than treated as an infrastructure failure.

Sources: https://encoding.spec.whatwg.org/#shared-utf-16-decoder,
https://encoding.spec.whatwg.org/#dom-textdecoder,
https://encoding.spec.whatwg.org/#dom-textencoder-encodeinto.
