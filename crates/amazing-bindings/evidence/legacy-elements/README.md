# DOM-B legacy element reflection ready

Implementation `1cb4dbcf`, fixed base `e5f1a8c5`. Directory, font, frame and
frameset elements now use their specified interfaces and fourteen reflected
attributes, including boolean removal, USVString URL conversion and legacy
null-to-empty-string setters. Installed-driver coverage checks cloning, live
attributes, foreign-namespace/receiver rejection and UTF-16 conversion.

Linux workspace **2759 passed /10 skipped**, doctests **4/4**. Complete
node/collection/obsolete API directories (**333 sources**): **8144→8148 passes**,
**767→763 failures**, **four gains /zero lost passes**. Files158→159 pass,
104→103 fail;37 timeouts,27 unsupported,7 harness errors and6 subtest errors
unchanged. Node-cloneNode.html now **135/135**.

Initial gate exposed a test setup omission (no document URL for URL resolution);
corrected fixture and full rerun passed. Frame child navigables and frameset
Window handler forwarding remain deferred with their existing dependencies.
Continuing ProcessingInstruction constructor and attribute APIs.

Spec: https://html.spec.whatwg.org/multipage/obsolete.html#frames and #other-elements,-attributes-and-apis .
Reproduce with dom-b-audit/run-corpus.py and the three directories in before.json.
