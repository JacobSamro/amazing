# UI event dictionary conversion

Implementation `01e52255`, fixed base `05f71c05` (IPC DOM-side contract).
UIEvent and its derived constructors validate nullable Window views, validate
related EventTargets, read supported dictionary members in inherited/lexicographic
order, and convert UIEvent.detail to a signed WebIDL long. Payload rooting occurs
after fallible conversion, so an invalid later field cannot leak earlier roots.
Sources: [UIEventInit](https://w3c.github.io/uievents/#idl-uieventinit),
[FocusEventInit](https://w3c.github.io/uievents/#idl-focuseventinit),
[WebIDL dictionaries](https://webidl.spec.whatwg.org/#es-dictionary).

Linux Boat exact-source gate: workspace 2534 passed, 10 skipped, docs 2/2.
Owned/driver iteration: 517 passed, 4 skipped; owned docs 0.

The same seven-source pinned corpus as `../messageevent/after.json` gained one
passing file and assertion: dom/events/Event-subclasses-constructors.html
48/49 → 49/49. Remaining selections are unchanged; zero lost passes. Candidate
results include executable hash and source. Totals: 3 pass / 2 fail /
2 unsupported files; 235 passing / 205 failing / 6 error subtests.
