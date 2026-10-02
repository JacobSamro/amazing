# Installed first-font metric context

Fixed source is f120de60 plus ready LAYOUT42 (85cf006c, cherry-picked
5d77bc8c) and its additive policy delegate (d65d636f, cherry-picked
20f99e79). The delegate has owner-proven byte-identical renderer output;
our before renderer is exact 5d77bc8c. No moving main dependencies are used.

The driver owns FontMetrics, bridging CSS's metric enum to the actual
TextProvider font_size_adjust_ratio. Selection uses Layout's exact
font_metric_text_style policy, the originating node for pseudos, and the
unadjusted computed CSS font size. CSS resolves from-font before inheritance;
a descendant selecting another font retains its originating numeric aspect.
Missing actual fonts resolve to none rather than a fabricated metric.

PageText loads before styles in initial headless rendering, GUI load/repaint,
animation sampling, incremental RenderCoordinator candidates, the full oracle,
DevTools computed styles/hit testing/box models/accessibility, and the axtree
command. Font sources retain their stylesheet URLs; CSS retains parsed sheet
scope roots. GUI adopts the main scoped collector. Recursive import deduplication
no longer discards distinct inline sources that share a document URL.

Registered-font grouped tests exercise all five physical axes in horizontal
and vertical writing, principal/pseudo inheritance, missing provider behavior,
animated selection, incremental/full parity, and actual page-relative font
loading from multiple scoped sheets. Geometry-dependent contextual math remains
unresolved when geometry is unavailable. Horizontal ex/cap/ch/ic expressions
use actual provider ratios; vertical contextual ch/ic remains deferred.

Limits: font loading remains synchronous and providers are reconstructed at
existing driver boundaries; persistent font-set invalidation and asynchronous
FontFaceSet are separate work. Windows GUI code is source-reviewed but not
compiled or run under the current Linux-only policy. Window getComputedStyle
is absent from the existing engine; CDP's computed-style consumer is installed
here. Window CSSOM adoption requires a separate DOM-owner interface.

Corpus evidence covers every eligible self-contained static reftest in pinned
WPT css/css-fonts, not all upstream dynamic/font API cases. Before: 24 passes,
8 mismatches, 32 files, no error/timeout/crash. After: 24 passes, 8 mismatches, zero changes and zero lost passes.
Final Linux workspace: 3690 passed / 17 skipped (100.217s), four doctests
passed; all three grouped registered-font tests pass. Renderer SHA256
61138797cca4eacb34b282258d49ddbb05bd7ee1115482444854cbfd0213d45c.
