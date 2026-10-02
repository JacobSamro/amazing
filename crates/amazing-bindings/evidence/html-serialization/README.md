# HTML serialization and document scripting context

Fixed combined basebc9e8388 (includes HTML45-48 and DOM table API). Pinned
serialization directory8files:352pass23fail; full314nodes12686pass293fail6errors.
Earlier5c baseline retained in orchestration scratch; improvements from adopting
new HTML parsing APIs are not credited to this ticket.

Audit implements current HTML fragment serialization attribute escaping,
normalized namespace attribute names and HTML/SVG/MathML local tag names,
UTF16 attribute/comment/PI preservation, Text-like CDATA serialization and PI
closing ?>. Noscript literal/escaped text uses actual owner document scripting
state. New generation-tagged DOM API defaults disabled for no browsing context;
active root explicitly enabled; Document clone does not inherit active context.
HTML50 owner initialization of actual parser/iframe contexts is a separate coordinated follow-up. FragmentContext captures
the owner document's bool; normal/shadow fragment parsing uses explicit HTML45
mode and inert HTML DOMParser parsing chooses Disabled before tokenization.

Template contents currently use the active owner's document rather than the
separate inert template document; this separate gap is recorded, not replaced
by a template-name heuristic. Existing XML well-formedness requirements remain;
No XML fixture is rewritten and invalid XML is not accepted to fabricate PI nodes.
Full XML serialization is a separate missing algorithm; these HTML serialization
entry points remain explicitly HTML algorithms.

Native regressions cover namespace normalization, non-scalar UTF16 code units,
CDATA/Text, active/inert/cloned context, nullable/raw mode changes and generation
reuse. No upstream engine code copied. Algorithm references:
[HTML serialization](https://html.spec.whatwg.org/multipage/parsing.html#serialising-html-fragments),
[escaping](https://html.spec.whatwg.org/multipage/parsing.html#escapingString),
[fragment parsing](https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html#fragment-parsing-algorithm).

Final whole selection: serialization352/23→372/3 (+20), nodes12686/293→12692/288 (+6), zero lost passing assertions. Native Boat workspace3641pass17skip and4doctests. Initial full corpus hit the existing XHR deadline; complete unchanged-source322-file recheck restored it. Both reports are retained in lane scratch; accepted gate has zero lost passing multiplicity. Remaining serialization failures are document.write context, template contents owner-document separation, and missing Range.createContextualFragment; raw errors retained in reports.
