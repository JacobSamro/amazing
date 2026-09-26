ErrorEvent and PromiseRejectionEvent complete: `8c0efe84`, fixed base `131fe706`.

Native payloads preserve object identity and arbitrary error/reason values,
including undefined versus null. Dictionary conversion order, required object
validation, numeric wrapping, UTF16/USV strings and interface checks follow HTML.
Value roots are retained only after all conversions succeed. Constructor lengths
and native dispatch use the installed interfaces; three new regression tests pass.

Linux workspace **2877 passed /10 skipped**, doctests **4/4**. Whole228-source
dom/events, HTML scripting/events and unhandled-promise-rejections corpus:
**399→410 pass**, **345→334 fail**, **11 gains /zero lost passes**.
Files59→61 pass,56→54 fail,78 timeouts and35 harness errors unchanged.
Ten ErrorEvent payload/handler assertions and one PromiseRejectionEvent
constructor assertion improve. Runtime error and rejection scheduling remain
embedding/JS event-loop work; no automatic-reporting claim is made.

Spec: https://html.spec.whatwg.org/multipage/webappapis.html#the-errorevent-interface
Next: checkbox/radio activation. Iframe Range remains deferred for HTML-25 part2;
JSRT realms are merged. WebSocket belongs to net.
