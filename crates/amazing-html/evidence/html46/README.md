# HTML46 document encoding and MIME selection

Fixed base:61f4cb86. Pinned WPT:c2a2fce69e5c2509d9dc28440ec8b42532e5c293.

Document metadata exposes canonical Encoding Standard names through
amazing_html::encoding::canonical_name; TextDecoder keeps lowercase names.
HTML meta x-user-defined normalizes to windows-1252, while an explicit
transport label retains its codec. Complete and all-split streaming decoding
are checked together. BOM and transport precedence remain measured.

The owned iframe adapter recognizes the complete HTML signature table for
missing/unknown MIME types, ignores only the five standard whitespace bytes,
requires exact SPACE/> tag termination and limits inspection to the resource
header. Explicit MIME types and nosniff are preserved. This implements the
HTML rows of unknown-type sniffing, not the complete MIME subsystem.

Sources: https://encoding.spec.whatwg.org/#names-and-labels,
https://html.spec.whatwg.org/multipage/parsing.html#prescan-a-byte-stream-to-determine-its-encoding,
https://mimesniff.spec.whatwg.org/#rules-for-identifying-an-unknown-mime-type.

Reproduction (Boat native source/target, never local): release amazing build,
owned amazing/amazing-html/amazing-encoding nextest, workspace nextest, workspace
doctests, scripts/html5lib.py with the WPT root and --files listing all 62 .dat filenames,
and python3 crates/amazing-html/evidence/html46/encoding_sweep.py
"$HOME/corpora/wpt" "$CARGO_TARGET_DIR/release/amazing" "$OUT/html46-encoding.jsonl" "$JOBS".
The encoding sweep is an unchanged copy of the lane's existing HTML41 runner.

Final native Boat gate: **559 owned passed, 6 skipped; 3531 workspace passed,
17 skipped; four doctests passed**. Whole CLI corpus **1894/1894** (42 script
variants skipped), and expanded decoded probe **1930 passes/6 unsupported**.
Exact-base full encoding sweep: **11753 pass/869 fail ->11755 pass/867 fail**,
zero former subtest passes lost. Both runs select194 files, with
18 pass/26 fail/1 timeout/8 error/141 unsupported. The two gains are the
UTF-32-labeled UTF-16LE-BOM iframe cases' canonical document encoding names.
These are document metadata gains, not new codec decoding behavior. Full
results are compared by exact name-byte hash to preserve deliberately invalid
Unicode names and tolerate asynchronous subtest order. DOM's separate fixture
handler will measure the canonical-name/meta-label normalization cluster.
