# Actual HTML and XML fragment context

Fixed source `575ee623567b82f0f157272214524c18680cbe70`, matched dependency-only
before `2c9b6b1b3287ee2b188107914a394db38da6be44`, on ready HTML53. Exact owned
series, lower context API dependencies and standalone policy helper are in
`source.json`. DOM Range activation is a separate owner ticket and is not
included here.

`HtmlFragmentContext` carries actual namespace URI, case-sensitive local name,
expanded attributes with exact UTF16 units, form ancestry, quirks and scripting
state. `parse_fragment_with_context` uses those snapshots for tokenizer state,
insertion mode, foreign integration points and the form pointer. Arbitrary/null
namespaces are preserved, interned in the parser arena and reinterned in the
returned arena. Element/attribute prefixes, template/shadow topology and exact
attribute code units survive export. Legacy string entry points retain their
normalization/default scripting behavior. The embedding owns the distinct
script marking rules for inert markup setters and HTML Range fragments.

`XmlFragmentContext`/`parse_xml_fragment` parse direct fragment grammar with the
actual default/prefixed namespace environment. Empty input, multiple roots,
text, CDATA, comments and PIs are retained; prefix shadowing/default undeclaration
and namespace restoration work without fabricated wrapper source. Malformed
names, references, bindings and expanded duplicate attributes, declarations,
doctype and unbalanced markup fail atomically. Existing limits/document grammar
are preserved. Nested XML HTML-template children use their inert contents owner.
The driver callback converts invalid source UTF16 and fatal XML parse errors to
SyntaxError. The DOM owner will add actual Range context selection and XML
imported-script already-started marking in its separate activation integration.

Standalone `FrameRuntime::document_context(&self, &Dom, document)` returns actual
committed URL, captured fallback base and typed document/top/ancestor origins.
It checks live realm/frame association and actual document generations at every
ancestor, preserves opaque identity and calls no JS/realm callback. Root/child
creation and committed root navigation capture actual generation. The host
wrapper reads only immutable state/arena borrows. PSL/site/ClientContext remain
owned by the network/storage profile adapter.

Boat: 605 owned passed/6 skipped, 3593 workspace passed/17 skipped, four docs.
Five grouped parser tests cover actual context/case/namespaces/UTF16 attributes,
form/quirks/scripting, XML scope/grammar/fatal errors and template ownership; one
policy group covers genuine root/blank/sandbox ancestry, opaque identity,
pending versus committed URLs and stale document/ancestor/retired associations.
All 1894 CLI html5lib and expanded 1930 passes/6 unsupported are retained.

Whole13 pinned selections retain every exact passing assertion name hash and
multiplicity, with zero gained and zero lost. Range66 stays40025/196,
domparsing68 223/1284, template67 608/557, DOM31412379/265,
encoding19412277/357, syntax722696/12, serialization8374/1; all six
Window/frame/BASE selections also stay unchanged. These APIs are construction
capabilities proven by native groups, not claimed WPT parser gains or comparator
corrections. Lower snapshot dependency gains precede the paired baseline.

The unchanged baseline XHR deadline was rechecked only in whole314+serialization
runs with the same binary/workers/deadline and restored before comparison.
Original and second deadline classifications remain in compact records; the
saved baseline binary checksum matches the initial release binary. First
candidate's 30 export-root panics shared Document.ownerDocument-null handling;
575ee623 fixed that cluster, followed by the full corrected gate.

Standards: [HTML fragments](https://html.spec.whatwg.org/multipage/parsing.html#parsing-html-fragments),
[XML fragments](https://html.spec.whatwg.org/multipage/xhtml.html#xml-fragment-parsing-algorithm).
