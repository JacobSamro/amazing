# Browser module driver adoption

Fixed implementation base: `680822e9`. DOM decoding/metadata dependency:
`4ce0126d`; source-context/import.meta URL API: `f86b814d`;
typed module error and canonical failure-cache API: `ae89a0af`.
The driver source is isolated from these three owner dependency commits.

The driver owns one `ModuleLinker` per realm. Browser canonical URLs identify
external records; inline modules have distinct node/realm record identifiers
and document URL metadata. Static and dynamic imports share fetch caches,
namespace singletons, and live bindings. No registry/state borrow survives
module execution or a microtask callback. Module responses are UTF-8 with a
stripped UTF-8 BOM, and HTTP module responses require a JavaScript MIME type.
Document base URLs use the existing DOM first-base-href algorithm, captured
for each script before execution. Document.currentScript uses a per-document execution stack: classic element wrappers while executing and null for modules/callbacks. Module evaluation failures report their original object through Window ErrorEvent and retain script load events; graph fetch failures fire script error events. About:srcdoc modules inherit their parent document base.

Classic scripts keep the shared global environment while carrying an immutable
source URL into delayed closures. Unmapped bare specifiers reject.

Installed paths: interleaved top-level/deferred scripts, iframe script dispatch,
connected dynamic scripts, CLI `run <filename>`, and persistent DevTools pages.
Classic `nomodule` fallbacks are skipped. The WPT loader verifies original
pinned source bytes and maps canonical fixture URLs to the read-only WPT tree.

The pinned area is all testharness HTML/XHTML pages in
`html/semantics/scripting-1/the-script-element/module` at WPT
`c2a2fce69e5c2509d9dc28440ec8b42532e5c293`: 111 files, timeout 5000ms,
8 workers. Before driver adoption: 0 passing files, 52 failing, 53 timing out,
5 error, 1 unsupported; 2 passing subtests. Adopting DOM source decoding alone
produced identical classifications and subtest counts. Evidence and the
reproducible corpus command live under `crates/amazing/evidence/module-driver`.

General async module graph coordination, import maps, worker module execution,
credential/CORS/referrer policy, and server-dependent WPT response synthesis
remain separate interfaces. Root DevTools source is instrumented; imported
dependencies do not yet produce the PageOwner's Network/Debugger events.

Validation results are recorded after the final Boat workspace nextest,
workspace doctests, and pinned corpus zero-loss gate.
