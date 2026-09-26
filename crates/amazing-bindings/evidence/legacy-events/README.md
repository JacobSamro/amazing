# Legacy event factory

Legacy event factory complete: implementation `03856580`, fixed base `f14340c8`.
Document.createEvent selects the matching UI event class, preserves ASCII-case
aliases, and throws DOMException NotSupportedError for unsupported names.
Linux final gate: **2536 passed / 10 skipped**, doctests **2/2**; owned/driver
iteration **519 passed / 4 skipped**, owned doctests **0**.
Pinned Document-createEvent improves **75→249 passing subtests**, with **zero
lost passes** across the seven-source selection (409 pass / 31 fail / 6 errors).
Evidence: `crates/amazing-bindings/evidence/legacy-events/`.

Spec: https://dom.spec.whatwg.org/#dom-document-createevent

The final gate ran on exact implementation source before the pooled-sandbox
rule arrived; results are retained rather than repeating a completed gate.
