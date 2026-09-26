# DOM-B shadow event retargeting ready

Implementation `a4f60416`, fixed base `9e68980b`. FocusEvent/MouseEvent
relatedTarget now uses native dispatch state; retargeting works against non-node
targets, boundary suppression truncates the path, and cleanup uses shadow
membership captured before callbacks. Adjusted related targets survive ordinary
dispatch and redispatch instead of reverting to the original private node.

Linux workspace **2753 passed /10 skipped**, doctests **4/4**. Fresh complete
**168-source dom/events** corpus: **359→360 passing assertions**, **241→240
failing**, **one gain /zero lost passes**. Files unchanged:51 pass,35 fail,
20 harness errors and62 timeouts. Four added native/installed-driver regressions
cover external targets, callback reparenting, redispatch, and identical original
target/relatedTarget handling.

Remaining relatedTarget fixture cases include net's XHR facade missing native
EventTarget adoption, activation behavior, and a pinned-WPT/current-DOM early
return discrepancy recorded in questions.md. Child realms/Range remain deferred.
Continuing existing UI event modifier and numeric dictionary semantics.

Spec: https://dom.spec.whatwg.org/#concept-event-dispatch and #retarget .
Reproduce with dom-b-audit/run-corpus.py label `shadow-after`, frozen binary
in `$OUT/dom-b-audit-shadow-after/amazing`, and `--directory dom/events`.
