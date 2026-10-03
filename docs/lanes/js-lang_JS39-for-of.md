# JS39: synchronous for-of IteratorRecord

Fixed dependency: 35217ac3231228a0c95b8209fd959642dec30bff. Corrected tests-only baseline: 68350e3720779ddc5d2f8f007e211e0301edf0a2.

The interpreter uses PatternIter to obtain the actual iterator and cache next once. Arrays and primitive strings use their observable iterator, preserving live length, prototype return hooks and overrides. No bare-next fallback or eager next callability check remains. Iterator next/result/done/value errors bypass closing; binding/body exits close, preserving original throws and replacing return/break with close errors.

The VM starts the same record in its existing iterator slot. IteratorStep marks exhausted before calling next or reading done/value, and clears it only after successful IteratorValue. The existing try/finally close handler therefore skips iterator-step abrupts and handles binding/body exits. ForAwait, jobs, frame layout, scope metadata and completion normalization are unchanged.

Protocol state and methods remain strongly retained by existing Rc-backed values in the interpreter stack or VM slot, including suspended generators. No collector tracing seam exists in this ownership model.

Algorithm port: lucid-softworks/lumen, crates/lumen/src/eval.rs, commit f6c9860d44fa52249cfd647122edea184d32c863, MIT, Copyright (c) 2026 Lucid Softworks limited. License verified in cached upstream source; mandatory source header and appended notice included.

Primary reference: [ECMA-262 2025 ForIn/OfBodyEvaluation](https://tc39.es/ecma262/2025/multipage/ecmascript-language-statements-and-declarations.html#sec-runtime-semantics-forin-div-ofbodyevaluation-lhs-stmt-iterator-lhskind-labelset). IteratorStep abrupts occur before the status/binding/body IteratorClose branches.

Validation: 15 grouped actual-native/forced-AST fixtures. Corrected baseline 809 owned passes, seven new failing groups (68 backend-case failures), one skip. Final source 0f0ef6ad3f02fa2ef405d0ee630592450c31a277 passed 4,223 workspace tests, 17 skipped, and four doctests. All 20 matched whole reports retain every passing file/variant. Language increases 24,470/45,263 to 24,475/45,273 in each mode (+5 files/+10 variants); nine builtin selections are unchanged. Remaining nonpassing reason/detail classifications are unchanged.

Actual corpus sources are 59f78279 before and 380910ce after. Subsequent changes only correct test fixtures and documentation; the final release host is byte-identical to the measured candidate host. Malformed setup failures are retained as raw diagnostics and excluded from the engine improvement claim. Legacy positive tests now expose Symbol.iterator, and new negative coverage proves bare-next objects reject. [Complete paired evidence](js-lang_JS39-evidence.json).

Two TypedArray resizable-buffer ForOf cases remain failing on both sides; they belong to the underlying TypedArray iterator validation, not a lost classification or a conformance claim for that separate area.
