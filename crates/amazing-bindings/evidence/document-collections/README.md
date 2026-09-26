# DOM-B document legacy collections ready

Implementation `d8f5ac84`, fixed base `1f219daa`. Native document.embeds/plugins
share the same live collection; anchors matches HTML a elements with a name
attribute (including empty); applets remains an empty collection. Tests cover
namespace exclusion, attribute mutation, canonical identity and document roots.

Linux isolated-target workspace **2736 passed / 10 skipped**, docs **4/4**.
The complete71-source document-accessor, obsolete-API and collection corpus
improves **166→172 passing subtests**, **148→142 failing**, **6 gains / zero
lost passes**. Files **27→29 pass**, **32→30 fail**, nine unsupported and three
harness errors unchanged. Baseline owned/driver **543 passed / 4 skipped**.
Evidence: `crates/amazing-bindings/evidence/document-collections/`.

Continuing obsolete document/body color reflection and legacy no-op methods.
Iframe Range and JS-dependent collection dispatch/iteration remain deferred.

Specs: https://html.spec.whatwg.org/multipage/dom.html#dom-document-embeds
and https://html.spec.whatwg.org/multipage/obsolete.html#dom-document-anchors .
Reproduce with run-corpus.py before <base> and after <source>. Four independent
workers run the same pinned selection before/after, with fresh per-file storage.
The final gate ran on the exact source subsequently committed above.
