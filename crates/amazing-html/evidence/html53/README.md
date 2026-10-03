# HTML53 captured child navigation context

Fixed source `6fa78cb4d8a05c9077ec5b7a06094c48de6a99dd`, matched dependency-aware
before `3865e458b974277778021af41919009c0e007ca5`. `source.json` records every
exact code SHA after canonical Window52, including DOM effective-base, explicit
DocumentHost URL capability and committed-document URL helper dependencies.
Both sides preserve all Window52 accepted assertions; the DOM helper contributes
one separate baseURL assertion before this ticket, never credited to HTML53.

Child navigation captures actual incumbent source Document identity/generation,
effective base, committed URL and origin before queueing. Blank URLs including
query/fragment load a real empty HTML document locally, without a doctype and
with quirks mode. Blank and srcdoc inherit immutable creation/navigation base;
retired Documents retain their own snapshot, while inert/cloned/recycled nodes
cannot inherit a browsing context snapshot. Normal URL resolution uses the actual
owner base; request referrer remains the actual source URL. Embedding checks use
real typed parent/top/ancestor origins, including inherited and sandbox origins.

Each child realm installs its genuine bound Location with Window/Document
PutForwards. Borrowed functions follow their actual Location receiver, with
origin policy checked in the source context and exceptions built in that realm.
Child href/assign/replace resolution uses the incumbent effective base. Child
pending navigation drains only its real realm; stable proxies retarget to new
Documents. Reload retains captured creator state and the srcdoc resource. Active
Document.location returns its actual Location; inert/retired documents return null.
Committed root URL/origin stays separate from pending Location and temporary
child script scope. Pure `FrameRuntime::document_url(document)` and host helper
support CSSOM without entering JS or consulting mutable script scope.

Root scheduler replacement snapshot transfer, full session history/fragment
navigation, lazy loading policy and complete Location encoding remain explicit
follow-ups. Existing root pending href/navigation/reload behavior is preserved.
No broader GUI/scheduler change or new engine dependency/source port was made.

Boat gate: 599 owned passed/6 skipped, 3584 workspace passed/17 skipped, four
doctests. Three grouped native regressions cover blank/srcdoc immutable bases,
full blank URLs and quirks, clone/retired identity, repeated sibling source
navigation/reload, pending root versus actual URL, borrowed receiver/cross-origin
policy and genuine source identity/generation. All 1894 CLI html5lib cases and
expanded 1930 passes/6 unsupported retained, zero failures.

Whole ten pinned WPT selections retain every exact passing name multiplicity:
encoding194 12277/357 unchanged; syntax72 2696/12 unchanged; DOM314
12370/273 to 12379/265 (+9 frame removeChild assertions); serialization8 373/2
unchanged; Window90 246/318 and WindowProxy8 13/5 unchanged; windows53 2/37 to
4/35 (+2 browsing context assertions); iframe150 14/52 to 15/51 (+1 base URL
assertion); BASE11 13/3 to 15/1 (+2 blank/srcdoc fallback assertions); baseURL11
2 passes unchanged. Total +14, zero lost. These are adapter/context gains, not
new parser tree passes or comparator corrections. No accepted assertion timed out.
All unsupported/error/timeout classifications remain in compact records.

`comparison.json` records raw before/after hashes, selections, every gained/lost
exact name-byte hash and multiplicity; compact records retain all classifications.
The six Window/frame/BASE selections additionally retain every raw fixture SHA256.
Evidence uses the existing collectors and exact same deadlines/four workers.
Standards: [document creation](https://html.spec.whatwg.org/multipage/document-sequences.html),
[about base URLs](https://html.spec.whatwg.org/multipage/urls-and-fetching.html#document-base-url),
[Location operations](https://html.spec.whatwg.org/multipage/nav-history-apis.html#the-location-interface).
