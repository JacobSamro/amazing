# DOM-B DOMException conformance ready

Implementation `82bb3868`, fixed base `129cde71`. Explicit undefined uses the
constructor defaults; native name/message/code accessors remain inherited.
The existing constructor, 25 legacy constants and native thrown exceptions
are covered by the full pinned WebIDL exception directory.

Linux Boat gate: **2709 passed / 10 skipped**, workspace doctests **4/4**.
Baseline owned/driver tests: **533 passed / 4 skipped**. Whole-directory WPT
subtests **112→123 passed, 19→8 failed**, **11 gains / zero lost passes**.
Files: **2→4 pass**, **4→2 fail**, **1 timeout**. The remaining eight failures
need JS Error.isError/stack support; the iframe portion of exceptions.html is
deferred for JSRT-9 → HTML-25 part 2. Its six local assertions now pass.

Evidence: `crates/amazing-bindings/evidence/dom-exception/`.
Windows deferred. Next: DOMStringMap, with numeric named-property hooks routed
to JS runtime; handler scope/compiler facade also routed there.

Reproduce on Linux with `run-corpus.py before <base>` and
`run-corpus.py after <source>` following build/tests. The script snapshots the
executable to avoid shared-target replacement and fetches a private sparse
WPT checkout at c2a2fce69e5c2509d9dc28440ec8b42532e5c293. A failed partial-clone
setup was repaired and the frozen baseline reused; no transport retry result
is counted as test evidence. The final gate ran on the uncommitted source
subsequently committed as the implementation SHA above.

Specs: https://webidl.spec.whatwg.org/#idl-DOMException and
https://webidl.spec.whatwg.org/#es-DOMException-specialness .
