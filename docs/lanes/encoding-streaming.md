# Encoding streaming boundaries

Base: `ccbe84327b1e177cdf05bc4d9b6ce27b433e4b5f`.

The ISO-2022-JP decoder's incomplete escape EOF path must clear its output
flag before reporting an error. Previously a decoder which selected a mode,
buffered ESC, and flushed it retained the consecutive-escape flag. Resuming
that decoder with a valid escape produced another error despite the earlier
EOF error having reset the escape state. Both replacement and fatal handling
share the corrected EOF transition. Character mode remains unchanged.

Reference: [Encoding Standard, ISO-2022-JP decoder](https://encoding.spec.whatwg.org/#iso-2022-jp-decoder),
escape start step 3 and escape step 9. This is an original change; no algorithm
code or data was ported. Existing pinned label/index bytes remain unchanged.

The regression exercises four output modes, three incomplete escape prefixes,
both error settings, every chunk split, restored bytes, repeated flushes, and
subsequent valid escape decoding. Existing UTF-8/UTF-16 BOM, fatal and flush
coverage and all codec/index vectors remain in the gate.

Gate preparation: exact base decoder/tests are restored and SHA checked for
BEFORE, then candidate files are restored for AFTER. Each side runs Linux
workspace nextest, workspace doctests, release driver build, and the complete
pinned `encoding/` testharness source/query-variant inventory. Fixtures and
source bytes are unchanged; generated window wrappers are classified by the
installed runner rather than old inventory exclusions. Raw results stay on
the host. Compact evidence compares passing file IDs and passing subtest
names with multiplicity; every lost pass blocks readiness.

Historical host clusters are separate prerequisites: SharedArrayBuffer
constructor availability (16 streaming and 55 encodeInto failures) and XHR
transport (40 single-byte decoder failures). Existing ArrayBuffer streaming
and TextDecoder single-byte label cases pass. These are historical triage
counts, not a gate for this source. Current qualification: Boat alias `encoding-streaming-20261005`, box
`bx_h6pjer3z`, tested source `2dae426276df259243db23c503e934c23b7b5e88`.
Both paired gate and compact-native-summary job exited 0. Workspace nextest
passed **4,651 before / 4,652 after**, with 17 skipped on each side; workspace
doctests passed **4 before / 4 after**. Release driver builds passed.

The complete testharness source/META-variant selection has **1,262 identities**:
32 passing files, 19 failing files, 1,210 errors and 1 timeout on both sides;
1,473 passing and 513 failing subtests on both sides. BEFORE and AFTER raw
JSONL are byte-identical (`8ebaef3f02a962ab6f4799b51607e0949f8e34037756f6875c14676be65b9656`).
Zero lost files and zero lost passing subtests, retaining multiplicities.
The native addition covers 136 boundary combinations. This ticket claims
a native correction, with no WPT gain or change in existing unsupported/failing
identity classifications. Query-variant coverage is broader than the old
source-only historical triage; its counts are not directly comparable.

Compact classifications, counts, gained/lost lists and raw hashes are in
`encoding-streaming-evidence/`; raw logs/results remain under
`/home/user/amazing/out/encoding-streaming-20261005/` on the captured box.
No local builds/tests, fixture edits, Cargo dependencies or ported code.
Integrator still runs the required merged-candidate 16-shard Test262 gate.
