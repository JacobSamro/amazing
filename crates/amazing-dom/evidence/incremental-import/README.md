# Incremental parser import topology

Fixed base 58a2342c adopts HTML51 and excludes its document.write gains from this
ticket. Candidate c17694ae preserves mapped node identity across parser moves,
mirrors external Documents and detached containers, exposes actual Document
children to parser insertion guards, retries validated moves, orders siblings
from the end, and discovers shadow roots added after their hosts were imported.
External document mirrors copy actual context metadata; this synchronization is
not Document.cloneNode and does not alter active/inert destination context.

Native regressions cover external-document adoption and return, document
replacement and nonempty validation, ancestor reversal and sibling ordering,
detached fragments, replacement-root allocation and late shadow roots.

Boat native gate: 3663 workspace passed / 17 skipped; four doctests passed.
Whole pinned 72 syntax pages: 2696/12 -> 2702/6 (+6 importer assertions), zero
lost passing multiplicity, including all ordinary/inner-loop/shadow/template
cycle fixtures. Remaining failures are four reopened doc.open/write assertions
(HTML adapter body-fragment fallback) and two event timing assertions.
Whole eight serialization files: 373/2 unchanged, zero loss.
Whole 314 DOM-node files: 12691/288/6 -> 12693/287/6, zero loss against this
fixed baseline and the earlier accepted 12692-pass report. One observed node
gain is the independently adopted Window global-this event-target helper
(5e9bf5b5); the other is known XHR timing recovery and is uncredited. The
importer measurement claims only the six syntax gains.

WPT pin c2a2fce69e5c2509d9dc28440ec8b42532e5c293 remains unmodified.
Existing corpus runners verify pinned inventory/source bytes; both runs used
their own frozen release binary. Compact node pass-name hashes use UTF-16LE
with surrogatepass and retain duplicate multiplicity. Comparisons were made
on raw (test path, assertion name) Counters, not hash sets or aggregate counts.
The mistaken --ref HEAD run resolved main source 9a6742f4 and is excluded from
all evidence; final source was explicitly c17694ae.

Algorithms use Amazing DOM validation; no engine code was ported. References:
[DOM pre-insert](https://dom.spec.whatwg.org/#concept-node-pre-insert),
[HTML adoption agency](https://html.spec.whatwg.org/multipage/parsing.html#adoption-agency-algorithm).
