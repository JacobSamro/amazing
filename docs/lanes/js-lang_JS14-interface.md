# Event-handler compiler facade (JS-14)

```rust,ignore
let callback = interpreter.compile_event_handler(
    "onclick",
    &["event"],
    source,
    &[document, form, element],
)?;
```

Use the handler's realm interpreter. The scope slice is outermost first; omit
`form` for elements without a form owner. Window handlers pass `&[]`. Window
`onerror` passes `&["event", "source", "lineno", "colno", "error"]` as parameters.
The result is a `JsValue` callable; compilation executes no handler statements
and registers no listener. A failed parse returns a `Throw` for the caller's
error-reporting path.

The parser consumes the body to EOF in an ordinary FunctionBody context with
initial strictness false. Its own `use strict` directive still applies. Name
and parameters are separate inputs, so source cannot terminate an interpolated
wrapper. `parser::parse_event_handler` also exposes the AST-only operation.

Scope objects remain live. Lookup checks function/block bindings first, then
objects in element/form/document order, then the global realm. It consults
prototype properties, DOM legacy named-property hooks and `Symbol.unscopables`.
Plain identifier writes invoke the setter without reading the getter.
Reads/writes use ordinary accessor operations; identifier method calls receive
the binding object. Nested closures retain the scope chain. Calls supply their
own `this`, including ordinary sloppy receiver conversion.

DOM owns lazy source/compiled state, mutation invalidation, body/window target
forwarding, shared EventTarget registration, and return-value/default-action
handling. This API replaces none of those mechanisms.

Specification: [HTML, getting the current value of the event handler](https://html.spec.whatwg.org/multipage/webappapis.html#getting-the-current-value-of-the-event-handler).
The compiler captures no active module environment. The runtime currently has
no per-function ScriptOrModule metadata and dynamic import execution remains
unsupported; relative-import provenance cannot yet be exercised through this
facade. This limitation must be preserved when dynamic import is implemented:
HTML handler functions have null ScriptOrModule.
