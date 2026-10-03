# HTML57 root classic script gateway

Fixed READY56 source `3f6595f3` → `d11c2780`. Source series: `e79b98af` then
`d11c2780`; no facade or module policy source is included.

DriverScheduler::load_classic_page_script shares the actual owner Document,
charset/base/query encoding, final response URL, MIME/status/bytes and policy
loader already used by child/reopened classic scripts. Classic requests snapshot
actual nonce and integrity attributes before callbacks; Document requests carry
empty strings. The embedding owns canonical nonce/SRI enforcement. Inline
source preserves its existing source URL; module, stale identity and missing
owner/resource boundaries throw honestly.

PageScript stores its actual node generation at creation, including the shared
collector and synthetic records. The bridge rejects absent or unequal captured
generation before reading replacement-node attributes or calling transport.
Native coverage includes the same-index/different-generation boundary and
successful matching identity, without exposing private DOM slot retirement.

The intact adapter test module moved to runtime_host_html/tests.rs, preserving
private access and test names; runtime_host_html.rs is 2341 lines.

Boat bulk gate: 670 owned passed/6 skipped, 4107 workspace passed/17 skipped,
four doctests; all 1894 CLI html5lib trees and 1930 expanded passes/6 unsupported
retained. Whole13 WPT directories, all111 module pages, whole220 html/syntax and
192 CSSOM pages retain every passing name multiplicity and passing file: zero
losses and zero WPT gains. Pins match exactly. Comparator and parser unchanged.
The previously accepted application/xml XHR assertion remains passing.

`comparison.json` and compact records preserve unsupported/error/timeout
classifications. Raw logs stay in MAIN .agents/html/html57-generation-after;
hashes and fixed refs are recorded in gate.json. Original e79 proof also passed
669 owned/4106 workspace; its generation gap was corrected before readiness.
The invalid-ref launcher attempt exited101 and contributed no evidence.
