# EUC-JP EOF selector state

Recorded published base: `a035b2b558065d72664ce7617da0a7261027bb0d`.

EUC-JP keeps its JIS0212 selector separately from the leading byte. The
Encoding Standard's EOF error clears only the leading byte. The native
implementation previously represented the selector only through buffered
`8F` prefixes, so EOF discarded it as well. A resumed native decoder then
used JIS0208 for a pair which the handler state selects from JIS0212.

The decoder now stores the selector explicitly. A valid `8F` second byte
sets it; EOF, ASCII, valid halfwidth katakana and invalid initial bytes retain
it. Generic pair paths reset it on both successful and failed mapping.
Existing successful streams produce the same scalars and error consumption.
This correction concerns the native resumable-EOF contract; TextDecoder's
non-streaming calls replace their codec instance, so no WPT gain is predicted.

Reference: [Encoding Standard EUC-JP decoder](https://encoding.spec.whatwg.org/#euc-jp-decoder),
steps 1, 3, 4 and 5.5. No source algorithm was ported, no data changed and no
Cargo dependency was added. Index108 is U+02D8 in the existing pinned JIS0212
index and null in JIS0208; regression expectations use that observable
charset difference rather than inspecting private state.

Five native groups cover 786 contexts: all 94 JIS0212 lead bytes with EOF,
both error modes and every prefix split; all 63 halfwidth bytes; all 32
invalid initial bytes; six selector-reset paths; and unchanged mixed JIS0208,
JIS0212, halfwidth and ASCII streams over every split. Repeated EOF calls,
error offsets and restored ASCII are covered. Separate test files and EOF
controls make the source independent of the fatal input-queue and ISO EOF
flag packets; original UTF-16/fatal controls remain unchanged.

Native Boat qualification passed on the recorded base: workspace 4,698 →
4,703 passing, 17 skipped and zero failures; four doctests before and after.
All 1,262 pinned encoding source/META identities remained unchanged: 32 passing,
19 failing, 1,210 harness errors and one timeout; 1,473 passing assertions and
513 failing assertions. Zero lost passing files or assertions. These are measured
native results, not complete Encoding API coverage.

The initial after-nextest run ended with output error 110; one clean paired
rerun completed with actual exit zero. Compact classifications, exact raw
hashes/paths, binary/corpus hashes and captured job identity are under
`crates/amazing-encoding/evidence/euc-jp-eof-state/`. Raw JSONL, logs and binaries
stay on the Boat host. The integrator's required 16-shard Test262 merge gate
remains outstanding.
