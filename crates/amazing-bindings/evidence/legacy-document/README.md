# DOM-B legacy document/body reflection ready

Implementation `b73d7bcd`, fixed base `1122ea30`. The five obsolete document and
body color IDL attributes reflect the same live body content attributes, preserve
UTF-16, distinguish null from undefined, and ignore absent bodies/framesets after
argument conversion. Document clear/captureEvents/releaseEvents and Window
captureEvents/releaseEvents implement the specified no-op behavior.

Linux isolated-target workspace **2739 passed / 10 skipped**, docs **4/4**.
The complete 71-source document-accessor/obsolete/collection selection improves
**172→197 passing subtests**, **142→117 failing**, **25 gains / zero lost passes**.
Files **29→33 pass**, **30→26 fail**, nine unsupported and three harness errors
unchanged. Three color cases still need iframe documents; native coverage proves
the corresponding missing-body/frameset behavior without iframe dependencies.
Evidence: `crates/amazing-bindings/evidence/legacy-document/`.

Continuing a fresh bulk nodes/traversal/events/collections baseline. Iframe Range,
JS-dependent named-property dispatch and generic array iteration remain deferred.

Spec: https://html.spec.whatwg.org/multipage/obsolete.html#dom-document-fgcolor .
Reproduce with ../document-collections/run-corpus.py legacy-after <source>.
The gate ran on the exact source subsequently committed above.
