# DOM-B document defaultView ready

Implementation `cc808abd`, fixed base `56035411`. Document.defaultView uses
an embedding hook to return its associated WindowProxy, or null for inert
DOMImplementation/DOMParser/cloned documents. The current driver associates its
single window with the active document; child-realm embedders can supply their
own association through the default-Unsupported hook.

Linux workspace **2749 passed /10 skipped**, doctests **4/4**. Complete
node/collection corpus (**324 sources**): **8004→8114 passing assertions**,
**867→757 failing**, **110 gains /zero lost passes**. File classifications
unchanged:152 pass,101 fail,37 timeouts,27 unsupported,7 harness errors;
6 subtest errors unchanged. All110 gains are createElementNS exception checks
that access document.defaultView.DOMException. Remaining390 assertions in that
file are XML/XHTML iframe cases and remain deferred.

Continuing shadow event dispatch retargeting/cleanup. Inline handler compilation,
iframe Range, and CSS selector escape/scoping dependencies remain deferred.

Spec: https://html.spec.whatwg.org/multipage/nav-history-apis.html#dom-document-defaultview .
Reproduce with dom-b-audit/run-corpus.py label `defaultview-after`, frozen binary
in `$OUT/dom-b-audit-defaultview-after/amazing`, and `--directory dom/nodes --directory dom/collections`.
