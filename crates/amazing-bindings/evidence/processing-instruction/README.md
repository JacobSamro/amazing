# DOM-B ProcessingInstruction ready

Implementation `ab38fc57`, fixed base `ca5e01fe`. Added the constructor and
complete attribute API, with ordered UTF-16 values, XML pseudo-attribute parsing,
reference decoding, serialization and reparsing after character-data changes.
Attribute mutations use existing character-data observer/range machinery;
no-op toggles preserve data. Target validation now permits xml per current DOM.

Linux workspace **2763 passed /10 skipped**, doctests **4/4**. Complete
**314-source dom/nodes** corpus: **8077→8082 passing assertions**, **741→736
failing**, **five gains /zero lost passes**. Files unchanged:148 pass,95 fail,
37 timeouts,27 unsupported,7 harness errors;6 subtest errors unchanged.
Baseline is legacy-elements/after.json restricted to dom/nodes.

The attribute fixture's first five assertions now pass. Its pre-existing
`ReferenceError: source is not defined` still prevents later registrations;
that JS lexical-scope issue is routed to js-runtime. Native and installed-driver
regressions independently cover parsing, mutation records, ordering, UTF-16,
conversion order and branding. Continuing navigation event payload interfaces.

Specs: https://dom.spec.whatwg.org/#interface-processinginstruction and https://www.w3.org/TR/xml-stylesheet/#pseudo-attributes .
Reproduce with dom-b-audit/run-corpus.py and `--directory dom/nodes`.
