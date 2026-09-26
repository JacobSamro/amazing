# LAYOUT-22 — vertical inline alignment and composition (held)

Fixed baseec384d80; baseline harness40c722aa. Pinned WPT
c2a2fce69e5c2509d9dc28440ec8b42532e5c293, Linux800×600, first-reference comparator.
The full static writing-mode directory has1112 cases;28 script-dependent cases
are explicitly excluded. `before-final/report.json` is the authoritative before;
`before/` is an earlier ruby checkpoint preserved only for diagnosis.

| Final Linux corpus | Before | After | Gains | Losses |
|---|---:|---:|---:|---:|
| Writing modes |230/1112|243/1112|13|0|
| Ruby |68/118|70/118|2|0|
| Tables |310/386|309/386|0|1|
| Retained union |299/449|303/449|4|0|

Workspace **2809 passed/10 skipped**,59.874s; **four doctests passed**.
Layout **356 passed/one skipped**,zero docs. All four final case/hash projections
reproduce candidate4 exactly. Every source hash in `final-after/provenance.txt`
was checked against the committed implementation. Writing modes retains133
unsupported XML/DTD cases; no new unsupported cases, errors, timeouts or crashes.
The other corpora have zero non-rendered cases. Windows is deferred.

**Gate held on css-tables/th-text-align.html.** The CSS UA rule unconditionally
centers th, while HTML's default requires inherited non-start alignment. This
was hidden while shaped text ignored alignment. The CSS request includes the
fixture/reference and expected default behavior; layout does not override
computed author alignment or hide the loss. `gate.sh` exits nonzero on losses.
Do not mark this ticket done until that dependency is resolved or explicitly
accepted by the integrator. All layout-owned intermediate losses are repaired.

Changes: line-relative alignment and interword justification; plaintext base
direction through bounded cached provider probes; anonymous formatting inheritance;
vertical atomic advances, column thickness and stable auto extents; orthogonal
available max-height separate from definite percentage bases; provider-based
shrink widths; and horizontal composition using complete font hwid/twid/qwid
variants before geometric scaling, with multi-cluster fullwidth reversal.
Eleven geometry tests cover alignment, atomics, struts, auto-size stability and
percentage-basis separation. Parser/shaper budgets remain bounded.

Both original LAYOUT-14 open cases now pass: direction-upright-001 and
text-combine-upright-compression-001. Full compression002/003/004/006/006a also
gain; the raw reports retain all remaining gaps. Text-transform full-width,
full inline edge fragmentation, orthogonal block flow and positioned constraints
remain follow-ups; this is not complete Writing Modes conformance.

Specifications: [Writing Modes4](https://www.w3.org/TR/css-writing-modes-4/)
sections2,4,5,9.1 and [CSS Text3](https://www.w3.org/TR/css-text-3/) section6.
Run only through `scripts/linux-run.sh layout -- 'bash crates/amazing-layout/evidence/layout22/gate.sh <unique-tag> <source>'`.
