Remaining obsolete HTML reflected attributes

Implements 98 reflected attributes on 32 existing interfaces from HTML 16.3.3.
Shared member slices preserve the pre-existing interface API; receiver validation
uses the actual element interface before any user conversion. Includes aliases,
legacy null-to-empty strings, boolean presence, signed/unsigned numbers and URLs.
Reflected unsigned spacing uses the specified 2^31-1 maximum. URL reflection
normalizes surrogate input for parsing and falls back to attribute text on failure.
The driver implementation lives in the lane adapter, with one shared forwarding call.

Spec: https://html.spec.whatwg.org/multipage/obsolete.html#other-elements,-attributes-and-apis
and https://html.spec.whatwg.org/multipage/common-dom-interfaces.html#reflecting-content-attributes-in-idl-attributes

Range iframe fixtures remain deferred for HTML-25 part 2;
JS realms are merged. WebSocket belongs to net and is not queued here.

Completed remaining obsolete HTML reflection: `e78c8e4c`, fixed base `270e9735` (includes JSRT realms). Adds 98 attributes on 32 interfaces, aliases and correct receiver checks, legacy-null/boolean/numeric/URL conversions. Driver regression includes conversion boundaries, UTF16, aliasing and cloning.

Linux workspace **2874 passed /10 skipped**, doctests **4/4**. Whole333-source nodes/collections/obsolete API corpus: **8159→8165 pass**, **752→746 fail**, **six gains /zero lost passes**. Five script event/htmlFor assertions and one moveBefore name-map assertion improve. Files158→160 pass,103→102 fail,38→37 timeout,27 unsupported and7 harness errors unchanged;6 subtest errors unchanged. Node-compareDocumentPosition changed timeout→pass with the same1444 passing assertions on both runs; this is deadline variation, not additional conformance credit.

Evidence: crates/amazing-bindings/evidence/obsolete-reflection/. Next: ErrorEvent and PromiseRejectionEvent payloads. Iframe Range remains deferred for HTML-25 part2; JSRT-9..12 are merged. WebSocket belongs to net.
