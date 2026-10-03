# Generic Array iterator integration

The live CSSOM adapter exposed ArrayIterator.next reading only own numeric length. Port the complete ordinary next length/index/exhaustion algorithm from boa-dev/boa core/engine/src/builtins/array/array_iterator.rs at 39cd11214ecb366200157dab96b3919e61e9a391 (MIT; LICENSE-MIT verified at that commit). Adapt it to Amazing values and existing property/ToLength/binary-view operations. Preserve the separate TypedArray iterator implementation and dispatch its shared prototype next through the existing validated path.

Ordinary array-like length is obtained on every next call through Get/ToLength, before advancing the cursor. Exhaustion clears the iterated target permanently. Getter/conversion throws preserve the index; element access throws happen after advancing it. Array iterator factories apply ToObject. No new dependency or host-specific special case.

Five grouped tests exercise both execution backends: inherited/live length and primitive boxing, permanent exhaustion and uncapped lengths, abrupt getter/conversion identity and cursor ordering, element/keys/entries ordering, and TypedArray bounds/shared next. The pinned complete Array/TypedArray/Iterator directories run before and after; workspace nextest and doctests gate the candidate. Counts pending; no ready claim.
