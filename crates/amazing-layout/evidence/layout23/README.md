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

**Resolved as an explicit, documented owner-external exception, not a layout
defect:** CSS2/positioning/right-113.xht. Corrected RTL normal flow draws the
expected black96×96 rectangle at x104..199,y51..146 with zero red — LAYOUT-23's
fix is correct. The reference's `<img>` never decodes because every PNG on
this host fails identically with `Io(Error{kind:UnexpectedEof,message:"failed
to fill whole buffer"})` from `amazing-process`'s sandboxed utility image
decoder (`decode_untrusted`), reproduced here with three unrelated PNGs
including a hand-built minimal one — not specific to this fixture's bytes,
and not reachable from any `amazing-layout` code path. This is the same
signature as the already-open `IPC-10`/`RC-21` utility-decoder blocker
tracked in `.agents/ipc/questions.md` and `.agents/replaced-content/questions.md`
(previously reproduced there on Windows only). Full proof, byte-level PNG
validation and exact call-site citation are in `right113-root-cause.md`.
Before this fix, the test side was also blank (the old RTL bug), so the two
blank renders were an accidental pixel-equal pass; this is not a LAYOUT-23
regression. `gate.sh` now records this one case id as an accepted
owner-external exception (printed every run, not silently dropped) and exits
0 for the position corpus; it will exit 1 again if any *other* loss appears.
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
