# DOM-B listener registration and AbortSignal lifetime ready

Implementation `f9fde9d7`, fixed base `93156830`. Listener callbacks and the
complete options dictionary convert before null/duplicate/aborted early returns.
Abort removal shares the signal's ordered algorithm list, preserves a duplicate
registration's original signal, and releases callback roots only when removed.
The recursive abort fixture no longer crashes.

Linux isolated-target workspace **2744 passed /10 skipped**, docs **4/4**.
All **510** testharness sources in dom/nodes, dom/traversal, dom/events and
 dom/collections: **9812→9823 passing assertions**, **1263→1255 failing**,
**11 gains /zero lost passes**. Files **214→217 pass**, **141→139 fail**,
**28→27 errors**;100 timeouts and27 unsupported unchanged. Subtest errors6
unchanged. Signal11/11, capture4/4 and passive5/5 now pass.

Continuing DOMTokenList's140-failure classList cluster. Traversal's1600 passing
assertions and7 cross-realm failures confirm that remaining traversal work and
iframe Range remain deferred to JSRT-9/HTML-25 part2.

Specs: https://dom.spec.whatwg.org/#concept-event-listener-add and
https://dom.spec.whatwg.org/#run-the-abort-steps .

Reproduction: build `amazing`, freeze it in `$OUT/dom-b-audit-after/amazing`,
run workspace nextest and doctests, then `python3 crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py after <source> --reuse-binary`.
`after.json` labels the uncommitted tested candidate; gate.json maps its verified
source hashes to the implementation commit.
