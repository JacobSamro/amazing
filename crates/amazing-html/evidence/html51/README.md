# HTML51 synchronous active document writes and parser insertion drops

Fixed source base HTML50 f69bf0aa, pinned WPT c2a2fce. No new upstream port
or engine dependency. The existing streaming tokenizer now bounds written
input without EOF, retaining incomplete tokens and a logical insertion point
that may follow its current cursor. Nested writes suspend and restore each
input tail. Original buffered page input remains unavailable until the
writing script returns.

The owned adapter synchronously publishes parsed nodes before returning from
document.write, swapping the real top-level arena or using the child import
in both directions. Nested parser-blocking scripts run synchronously against
the actual document/realm. Deferred events remain queued for the outer driver.
The original write API remains available for existing queue-only consumers.

Fresh HTML/foreign element insertion shares the current standard parent,
host-inclusive cycle, full-document, and reference-child guards. Text into
a Document is dropped. Initial doctype append requires no preexisting doctype
or element child; discarded tokens do not overwrite existing doctype data.
Native tests cover empty/doctype/element document targets, complete and split
tags/entities, nested scripts, main and iframe visibility, and original-tail
isolation.

Native Boat gate: 591 owned passed/6 skipped, 3,572 workspace passed/17 skipped,
four doctests. Whole syntax72 2694/14→2696/12: two actual document.write
assertions in ambiguous-ampersand.html. Serialization8 372/3→373/2: one actual
main-document write assertion in escaping.html. Every prior passing assertion
is retained across syntax, serialization, DOM314, encoding194, and trees.
Encoding12268/354 classifications are identical. DOM retains12369 passes;
one previously timed-out zero-pass file now reports its existing failure,
so fail count274→275 and timeout files16→15; zero lost or gained passes.
TreeCLI1894/1894 and expanded1930pass6unsupported retain all records.

The first test compile found four references to one missing NodeKind import;
fixed together, then the complete gate passed. There were no native runtime
failures. Compact full name-byte hashes/statuses and comparisons are durable;
raw reports remain orchestration scratch. binaries.sha256 records both gated
release hosts. The complete existing cross-document/cycle syntax selection
was preserved. Ten importer-related reparenting/nonempty-document assertions
remain coordinated with the DOM owner, whose previously reverted move pass
had lost a cycle fixture. No DOM implementation is changed here.

Current standard consulted2026-10-03:
https://html.spec.whatwg.org/multipage/parsing.html#appropriate-place-for-inserting-a-node
https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html#document.write()

Next HTML52: whole Window.frames/length/indexed-child/parent/top using existing
real browsing contexts, before captured creator/container fallback integration.
