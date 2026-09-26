Lossless UTF-16 event types complete: `268c5a4e`, fixed base `ac89b733`.

Constructors, legacy initializers and listener registration/removal preserve lone
surrogates without conflating them with replacement characters. Normal Unicode
names retain the existing native String representation. Abort/once listener
identity uses the same exact keys. Four new native/binding/driver regressions pass.

Linux workspace **2771 passed /10 skipped**, doctests **4/4**. Complete
482-source dom/nodes + dom/events corpus remains **8448 pass /1013 fail**,
**zero gained or lost passes**. File classifications unchanged (199 pass,131 fail,
99 timeout,27 unsupported,26 error); six subtest errors unchanged.
Baseline reuses the immediately preceding navigation-events after corpus on the
exact base. Frozen candidate binary and source hashes recorded in gate.json.

Next: merge local dev for JS realms, then remaining legacy reflected attributes.
Iframe-dependent Range remains deferred pending HTML-25 part 2. WebSocket is
owned by net and removed from this lane's queue.
