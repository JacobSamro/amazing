# Amazing browser engine

Shared language for describing Amazing's compatibility evidence and progress.

## Language

**Render engine**:
The style, layout, and paint pipeline that turns a document into pixels.
_Avoid_: Renderer, when referring only to rasterization

**Shell chrome**:
The persistent browser controls surrounding document content: windows, tabs,
address and navigation controls, sidebars, menus, overlays, and status surfaces.
_Avoid_: Render engine, page chrome

**Browsing frame**:
The shell region that presents and interacts with one document, including its
page pixels, controls, selection, scrolling, and in-document media.
_Avoid_: WebView, viewport when referring to the whole interactive surface

**In-document media**:
Audio or video owned by an HTML media element and presented inside its browsing
frame with document lifecycle, layout, controls, timing, and events.
_Avoid_: Media tab, codec demo, playback probe

**Standard conformance**:
Behavior required by the applicable web specification and measured by a pinned,
unmodified upstream WPT case.
_Avoid_: Parity, compatibility, or standards-compliant without a measured scope

**Upstream WPT result**:
The recorded outcome of running an unmodified test from a named WPT revision.
_Avoid_: Calling a locally modeled or curated fixture a WPT result

**Curated conformance fixture**:
A local regression case inspired by a specification or WPT scenario but not
executed unchanged from upstream WPT.
_Avoid_: WPT test, WPT pass rate

**Differential oracle**:
Observed Blink or WebKit behavior used to investigate an ambiguous conformance
failure; the applicable specification remains authoritative.
_Avoid_: Treating one browser's output as the standard

**Convergence milestone**:
A named, revision-pinned WPT selection with measured pass, mismatch, timeout,
unsupported, and crash counts.
_Avoid_: WebKit parity, Blink parity
