# LAYOUT-23 — positioned axis constraints (held on image reference)

Fixed engine base749555ca; baseline/harnessae28b840. Pinned WPT
c2a2fce69e5c2509d9dc28440ec8b42532e5c293;Linux800×600,first-reference comparator.
Complete static css-position + CSS2/positioning directories:645 cases,
126 script-dependent exclusions. Baseline binary label40c722aa-layout22-candidate4
is byte-identical to749555ca (audit in layout22/final-after/provenance.txt).
The frozen manifest retains its mistaken LAYOUT-22 label to preserve its SHA.

| Final Linux corpus | Before | After | Gains | Losses |
|---|---:|---:|---:|---:|
| Positioning |226/645|255/645|30|1|
| Writing modes |243/1112|245/1112|2|0|
| Ruby |70/118|70/118|0|0|
| Tables |309/386|310/386|1|0|
| Retained union |303/449|304/449|1|0|

Workspace **2823 passed/10 skipped**,60.334s;**four doctests passed**.
Layout **370 passed/one skipped**,zero docs,including14 new geometry tests.
All five final case/hash projections exactly reproduce candidate2. Final source
hashes and Cargo.lock verified against the committed tree. Positioning retains9
unsupported cases,writing133;no new unsupported cases,errors,timeouts or crashes.
Windows deferred. Full logs split into files of at most2000 lines.

**Held:** CSS2/positioning/right-113.xht. Corrected RTL normal flow draws the
expected black96×96 rectangle at x104..199,y51..146 with zero red. Reference
right-103-ref.xht has no black below the paragraph because its PNG is absent.
The original actual was also blank,an accidental pass. Exact before/after color
counts are in right113-diagnosis.json;raw reports keep the loss. Owner request
is in questions.md. gate.sh exits1. Do not mark zero-loss or done until this
image dependency is fixed or explicitly accepted by the integrator.
The older LAYOUT-22 CSS th default loss remains separately held,not erased by
this baseline. Two other missing-image examples have color-count evidence;
214 remaining mismatches have image references,a triage hint rather than an exclusion.

The axis solver covers opposing insets,auto margins,negative-space overflow,
provider shrink widths including atomic tables,content heights,min/max limits,
border-box sizes,zoom and natural replaced ratios. Containing-block direction
is separate from child direction,including root direction for the initial CB.
Relative percentages use the normal content box;static slots precede siblings'
relative offsets. Normal RTL blocks use the same direction rule. Sticky creates
an absolute containing block. Temporary used sizes preserve computed style.

Candidate1 had13 table and two writing losses. Repairs size atomic placeholders
before intrinsic queries,distinguish anonymous source ranges and align normal
RTL boxes consistently. Candidate2 and final have no remaining layout-owned losses.

Remaining scope:inline fragment containing blocks and exact inline static slots;
full vertical block-axis mapping;transformed fixed containing blocks;modern
self-alignment,sticky scroll offsets and fragmentation. Vertical static origins
mirror the existing normal block implementation. This is not complete Position3.

Rules:[CSS2.2 used dimensions](https://www.w3.org/TR/CSS22/visudet.html)
sections10.3.7–8 and10.6.4–5,and
[relative positioning](https://www.w3.org/TR/CSS22/visuren.html#relative-positioning).
Run through `scripts/linux-run.sh layout -- 'bash crates/amazing-layout/evidence/layout23/gate.sh <unique-tag> <source>'`.
