# Template contents document context

Fixed base 6480e473; final code 7ade4da5 (f2053c41, 79a3d10a,
9a50e244, 7ade4da5). Template contents retain a separate inert Document with
fresh about:blank URL, UTF-8 encoding, no quirks and disabled scripting; only
the source HTML document type is inherited. Nested templates reuse that
Document. Generation-tagged associations prevent recycled slots from aliasing
old templates, contents or source Documents. A retained inert Document keeps
its intrinsic creation flag after its original source Document is collected.

Markup setters choose template contents before capturing document scripting
state, while retaining the template host's parser insertion context. Fragment
parent outerHTML/adjacent insertion use the standard body context. Native tests
also change the actual contents Document flag to prove parsing uses document
state. Pre-insert validation uses the existing host-including ancestor algorithm
and rejects cycles through template contents before any mutation.

Final Boat gate: 3,670 workspace tests passed / 17 skipped; four doctests passed.
Whole 67 template files: 608/557 -> 610/555 (+both hierarchy assertions).
Whole eight serializer files: 373/2 -> 374/1 (+template.innerHTML escaping).
Whole 314 node files: all 12,693 accepted passes retained, no gains or losses.
Whole 72 syntax pages: 2,702/6 unchanged; all previous cycle passes retained.
Every comparison preserves duplicate assertion-name multiplicity and passing
file classifications. Inventories and pinned WPT source remain unchanged.

The first final-source sweep lost one known XHR timing assertion. A complete
389-file serial recheck on the identical binary retained it. That XHR file then
timed out after the retained assertion and omitted its previous failing
assertion: raw node failures 287 -> 286 are not credited as an engine gain.
Provenance-after and gate.json describe that complete recheck; native and syntax
results come from the same exact final source and frozen binary. The fresh
syntax baseline uses the saved 6480 release binary (same binary as the original
389-file baseline). Compact node evidence uses UTF-16LE with surrogatepass name
hashes and multiplicities. Full reports and native logs remain in MAIN scratch.

Remaining template-for parsing/patching and htmlFor reflection are separate
areas. XHTML navigation/serialization and reopened parser sessions also remain
classified independently. The one remaining escaping.html failure is missing
Range.createContextualFragment, the next approved interface area.

Original Amazing code; no engine code was ported. References:
[appropriate template owner Document](https://html.spec.whatwg.org/multipage/scripting.html#appropriate-template-contents-owner-document),
[innerHTML](https://html.spec.whatwg.org/multipage/dynamic-markup-insertion.html#dom-element-innerhtml),
[pre-insert validation](https://dom.spec.whatwg.org/#concept-node-ensure-pre-insertion-validity).
Template parsing/serialization scripting interoperability is tracked in
[WHATWG issue 10879](https://github.com/whatwg/html/issues/10879); the measured
pinned expectation is satisfied using the actual contents Document state.
