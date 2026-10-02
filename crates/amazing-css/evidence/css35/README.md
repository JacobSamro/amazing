CSS-35 completes CSS font-synthesis grammar/expansion/serialization for all four
axes, including oblique-only, variables, inheritance and independent font reset.
Pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293, identical corrected CSS-33
runner on base 0e59c664cdbbd9cdccf064ba66c557a23c79cace and candidate.

Parsing3565→3599/4805 (+34 exact,lost0),498 files,169 nonliteral skips equally.
Workspace3527 passes/17 skips+4 doctests; crate368 passes. Whole static fonts
rendering24/32 on both sides; all32 classifications unchanged, zero losses.
Boat Linux, native /tmp source+target; 800×600 viewport,4 jobs,20s render timeout.
`gate.sh` batches workspace,doctests,corpus,explicit driver and rendering gate.

Layout's existing synthesis consumer must distinguish oblique-only from auto
when font-style requests italic. This follow-up is recorded in lane questions.
