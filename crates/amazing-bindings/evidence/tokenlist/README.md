# DOM-B DOMTokenList semantics ready

Implementation `d6613150`, fixed base `54e681e4`. Token-list arguments convert
before validation and live reads; contains/supports use their own validation
rules. Empty add/remove calls normalize existing attributes, absent empty
attributes stay absent, and replacement preserves ordered-set position and
mutation/no-op semantics.

Linux workspace **2747 passed /10 skipped**, docs **4/4**. Complete dom/nodes
and dom/collections directories (**324 sources**): **7864→8004 passing**,
**1007→867 failing assertions**, **140 gains /zero lost passes**. classList is
**1420/1420**, previously1280/1420. Files151→152 pass,102→101 fail;
37 timeouts,27 unsupported,7 harness errors and6 subtest errors unchanged.
Baseline is dom-b-audit/after.json restricted to those two directories.

Continuing document/Window association reflection: missing defaultView is the
next non-iframe prerequisite in createElementNS's exception checks. Child realms
and iframe-dependent Range remain deferred.

Specs: https://dom.spec.whatwg.org/#interface-domtokenlist and https://infra.spec.whatwg.org/#set-replace .
Reproduce with the dom-b-audit runner using `--directory dom/nodes --directory dom/collections`; label `tokenlist-after`, frozen executable in `$OUT/dom-b-audit-tokenlist-after/amazing`.
