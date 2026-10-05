# Image fitting consumer

Recorded origin/dev base ac72a9687c8a1ff76c3d0eb47db98a321ad263b8.

ImagePresentation consumes complete contain/cover scale-down combinations,
using naturally sized image geometry capped against the corresponding fit.
The existing replaced-block, inline/generated-image and PagePaint adapters
already call this presentation. No driver or sampler is substituted.
Grouped controls cover every spelling, small/wide/tall natural images,
position offsets, real painted pixels, content and ancestor clips.
Original image geometry, malformed-buffer and alpha controls remain intact.

CSS declaration production depends on the independent ready ticket
27425957118a7341b3846c9c28d5074c8bd0fc42. This paint source does not import
that lane history. Direct computed-style controls exercise the real consumer;
end-to-end new declarations require publication of that CSS producer.

Qualification selection: workspace nextest and doctests, original seventeen
CSS parsing selections and twenty static render cohorts, all226 frozen RC21
cases and the entire pinned css/css-images resource reftest family. Every RC21
entry and match/mismatch reference is retained, including unsupported cases.
No sampling adoption or gate result is claimed before native qualification.
