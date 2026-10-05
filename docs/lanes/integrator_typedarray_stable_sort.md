# Whole TypedArray stable sorting

Base ac72a9687c8a1ff76c3d0eb47db98a321ad263b8. Source-only, unqualified.

Merged viewport061 and independent EUC-JP e39 full16 runs both retained the original pinned 60-second deadline and lost staging/sm/TypedArray/sort_modifications.js non-strict to timeout. Every native shard completed; neither candidate is published. Original TypedArray sorting used quadratic insertion sort.

Port the whole fallible stable merge/comparison algorithm from lucid-softworks/lumen crates/lumen/src/builtins/typedarray.rs at f6c9860d44fa52249cfd647122edea184d32c863; MIT verified at that commit, Copyright (c) 2026 Lucid Softworks limited. Adapt native values/calls, keep comparator validation, captured element list, success-only buffer writes, signed-zero/NaN numeric order and BigInt comparison. No dependencies or deadline/oracle edits. Inconsistent comparators terminate without requiring Rust sort's total-order contract. Stop all further user comparisons on the first abrupt completion.

Three authored groups cover stable ordering/large-input comparison bound, original exception/inconsistent-order termination, and both execution backends across all twelve constructors, sort/toSorted, comparator result and mutation snapshots. Qualification must run workspace/doctests and whole TypedArray plus staging/sm/TypedArray corpora both modes in bulk; authoritative full16 follows on the exact integrated source. Original fixtures and deadlines remain unchanged.
