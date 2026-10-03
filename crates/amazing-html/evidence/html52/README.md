# HTML52 actual Window topology and descriptors

Fixed source93981cfba6e11e34325fca253e68d1602d25f3f5, baseline0743cd06
(HTML51 plus exact DOM BASE dependency), pinned WPTc2a2fce69e5c2509d9dc28440ec8b42532e5c293.
Root and child Windows use actual same-agent stable WindowProxy targets.
frames returns that proxy; length and readonly numeric child properties follow
document-tree order. parent/top use real context relationships. Navigation
retargets stable proxies; removed/retired descendants cannot reappear from a
stale candidate sweep. Inert documents acquire no browsing context.

frames/length/parent retain Replaceable descriptors; top stays unforgeable.
Replaceable setters execute the receiver's actual DefineOwnProperty, including
ordinary Proxy traps and WindowProxy policy. Cross-origin readable topology
uses cached anonymous getter-only descriptors created in the caller realm,
bound to the actual target context, enumerablefalse/configurabletrue. Indexed
child descriptors are evaluated afresh. Target navigation revalidates policy
and descriptor synthesis; original globals are never temporarily modified.

Shared grants are only two HtmlWindow NativeEntry variants/two dispatcher arms,
the storage event actual-global-this line and runner self initialization line.
DOM dependencies5e9bf5b5/a1c12e1e preserve defaultView/event/noop identity.
Standalone JS semantic dependencies495201fc,246aa305,a157bdb4 supply initial
exotic prototype lookup/current-target Location, receiver DefineOwnProperty,
and caller-realm WindowProxy property synthesis. Their independently gated
core behavior remains separate from these installed-driver measurements.
No new engine dependency or upstream source port is introduced by this ticket.

Final native Boat:595 owned passed/6 skipped,3580 workspace passed/17 skipped,
four doctests. All HTML/XML/encoding/driver tests run together. Grouped actual
driver regressions cover nested context identity/order, descriptors/brands,
replacement on real/ordinary/Proxy/frozen receivers, sandbox cross-origin
getters/caller realm/cache/borrowed calls/navigation, inert and retired frames.
Both JS core descriptor groups exercise TreeWalk and Bytecode.

Whole Window90:233/329→246/318 (+13); WindowProxy8:6/12→13/5 (+7);
iframe150:12/57→14/52 (+2). Whole windows53 remains2/37, BASE11 remains13/3,
baseURL11 remains1/1; unsupported counts23/5/10/53/0/10 respectively.
Encoding194:12268/354→12277/357 (+9 existing frame BOM checks), DOM314:
12369/275→12370/273 (+1 existing iframe isConnected). Syntax72 remains2696/12,
serialization8 remains373/2. Zero lost passing assertion multiplicities.
The correction recheck also retains every pass gained on intermediate429.

The full gate includes all194 encoding pages,72 syntax pages,314 DOM nodes,
8 serialization pages, all1894 CLI html5lib cases and expanded1930 passing
cases/6 unsupported, plus six complete Window/frame/BASE selections. Every
unsupported/error/timeout record remains in the evidence. The Window collector
also records SHA256 of every raw pinned fixture, with query metadata removed
only for locating its physical source file. Comparison keys are exact test ID,
SHA256 of UTF8-surrogatepass subtest name bytes, status and multiplicity;
passing duplicates are never collapsed. Baseline/current fixture SHA must match.
The first939 eight-worker whole DOM sweep timed out one previously passing
XHR contentType assertion. Its complete classifications/loss comparison are
retained as dom-initial files. Whole314 plus serialization8 were rechecked at
the same25s deadline with four workers, build+all595 owned tests together;
the accepted XHR pass returned, no others were lost. Final and recheck binary
hashes are identical. No single-test retry, omitted assertion or timeout waiver.

The one-line runner self correction preserves actual root WindowProxy identity
after lower installation. It restored one prior passing assertion during an
intermediate gate; no reporting gain is credited. Frame topology separately
unblocks encoding/remove-only-one-bom.html and iframe isConnected assertions;
these are existing codec/DOM algorithms, rather than new parser/codec work.

Current normative references consulted2026-10-03:
https://html.spec.whatwg.org/multipage/nav-history-apis.html#the-window-object
https://html.spec.whatwg.org/multipage/nav-history-apis.html#crossorigingetownpropertyhelper-(-o,-p-)
https://html.spec.whatwg.org/multipage/document-sequences.html#document-tree-child-navigables

Next53 captures immutable creator/container about-base URLs for blank/srcdoc
documents using actual context state. Next54 is the actual HTML/XML fragment
context API for whole Range, ahead of reopened-document sessions55.
