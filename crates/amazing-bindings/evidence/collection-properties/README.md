# DOM-B collection own-property surfaces ready

Implementation `12737f79`, fixed base `7b6678a2`. Collection length/value accessors
remain inherited across HTMLCollection, form/options/radio collections, live/static
NodeLists, NamedNodeMap, DOMTokenList and DOMStringList. An installed regression
covers live key order, named visibility and mutation updates.

Linux isolated-target workspace **2734 passed / 10 skipped**, docs **4/4**.
Full55-source collection/global-attribute corpus: **168→175 passing subtests**,
**107→100 failing**, **7 gains / zero lost passes**. Files **22→24 pass**,
**32→30 fail**, one harness error unchanged. Baseline is dataset/after.json;
evidence is `crates/amazing-bindings/evidence/collection-properties/`.

Generic intrinsic array iteration and strict legacy assignment/deletion remain
routed to JS. Continuing legacy document collections (embeds/plugins, anchors,
applets). Windows and iframe-dependent Range remain deferred.

Spec: https://webidl.spec.whatwg.org/#es-attributes .
The final gate ran on the exact source committed above. Reproduce the55-source
selection with ../dataset/run-corpus.py collections-after <source>.
