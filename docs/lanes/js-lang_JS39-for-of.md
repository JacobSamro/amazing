# JS39: synchronous for-of IteratorRecord

Fixed dependency: 35217ac3231228a0c95b8209fd959642dec30bff. Tests-only baseline: 59f78279eca89cd1485c1c3fbf2f1dcccfcf7bba.

The interpreter uses PatternIter to obtain the actual iterator and cache next once. Arrays and primitive strings use their observable iterator, preserving live length, prototype return hooks and overrides. No bare-next fallback or eager next callability check remains. Iterator next/result/done/value errors bypass closing; binding/body exits close, preserving original throws and replacing return/break with close errors.

The VM starts the same record in its existing iterator slot. IteratorStep marks exhausted before calling next or reading done/value, and clears it only after successful IteratorValue. The existing try/finally close handler therefore skips iterator-step abrupts and handles binding/body exits. ForAwait, jobs, frame layout, scope metadata and completion normalization are unchanged.

Protocol state and methods remain strongly retained by existing Rc-backed values in the interpreter stack or VM slot, including suspended generators. No collector tracing seam exists in this ownership model.

Algorithm port: lucid-softworks/lumen, crates/lumen/src/eval.rs, commit f6c9860d44fa52249cfd647122edea184d32c863, MIT, Copyright (c) 2026 Lucid Softworks limited. License verified in cached upstream source; mandatory source header and appended notice included.

Primary reference: [ECMA-262 2025 ForIn/OfBodyEvaluation](https://tc39.es/ecma262/2025/multipage/ecmascript-language-statements-and-declarations.html#sec-runtime-semantics-forin-div-ofbodyevaluation-lhs-stmt-iterator-lhskind-labelset). IteratorStep abrupts occur before the status/binding/body IteratorClose branches.

Validation pending: 15 grouped actual-native/forced-AST fixtures; workspace nextest and doctests; matched whole language, Array, Iterator, Function, generator and disposal corpora in default/VM modes, release profile and 60-second deadline. No readiness claim before paired zero-loss proof.
