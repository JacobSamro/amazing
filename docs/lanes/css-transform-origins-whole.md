# Whole used transform/perspective origins through installed CSSOM

Recorded origin/dev base `d265908ea158e34d82084d0c94e8b0f429751cd1`. This independent source composes the existing position grammar/used axis API with actual selected reference geometry. Original CSS computed percentages and assertions remain unchanged; the public parser corpus still honestly reports symbolic computed values rather than pretending it executes WPT fixture layout.

CSS-owned `used_origin` accepts computed style plus explicit local reference-box offset/dimensions, resolves X/Y/Z and piecewise math, defaults absent Z to0, and rejects nonfinite/negative geometry or unresolved context units. No guessed font/viewport/SVG box. The caller supplies selected geometry; no new parser or external algorithm was copied.

Installed `runtime_host_cssom.rs` now resolves both origin properties using the actual root-document LayoutBox from the existing snapshot. For associated CSS boxes, content/fill map to content geometry and border/stroke/view map to border geometry, per [Transforms1 section5](https://drafts.csswg.org/css-transforms-1/#transform-box). CSSOM reports offsets from the selected box top-left; the separate matrix handoff can supply that box's local offset. Effective zoom converts actual layout dimensions back to CSS pixels; computed font math comes from actual document styles/font loading. Nonzero transform Z is serialized; zero Z remains omitted. Detached, display-contents, generated/pseudo/other-document or absent layout uses the existing fallback. Unknown unit geometry declines; no fake pixel values.

Five CSS bulk control groups cover whole keyword/percentage axes, symbolic computed values, changed actual dimensions, 3D/math/font values, actual offsets, piecewise math and finite/context failure. Five installed runtime groups cover all five box keywords, border/content sizing, actual inherited document font and loaded font face, computed font/math/Z, nested effective zoom, live geometry/style changes and unavailable box/detached fallback. Existing controls and their assertions are unchanged.

Normative origins are resolved-value special cases in [Transforms1 section4](https://drafts.csswg.org/css-transforms-1/#transform-origin-property) and [Transforms2 section9](https://drafts.csswg.org/css-transforms-2/#perspective-origin-property), read2026-10-06. No complete3D paint/context/hit-test claim; retained property-tree author-origin shortcut remains unchanged.

UNBUILT/UNQUALIFIED source only. Whole final plan retains workspace nextest+doctests and all original17 parsing selections/20 static directories/491 resource cases/all226 RC21, actual800x600/20s/jobs4, truthful authenticated1af AFTER/native0 baseline. Source/commands/public helpers/actual-device capacity freeze and root review precede any allocation. Running4bd transform source, originals and raw boxes are preserved.


The authentic a1fc clean gate retained4978 workspace passes but failed the
original contextual-unit fallback assertion. Literal1cqw was discarded by
position validation, silently exposing the initial50%50% value instead of an
unresolved computed contextual length. The successor delegates literal/math
position validation to the existing dimensional parser for all container and
logical viewport unit families; symbolic values remain unresolved and the
original assertion stays unchanged. Three new whole groups cover allfour
position properties, all14 units, math/end edges and invalid dimension/arity.
The original lost font-size-zero reference crash remains separately recorded
and is not waived or reclassified. Successor is UNBUILT.


The11d native gate passed alloriginal10 groups and all25 non-workspace phases;
workspace4981/1/docs31/0. The sole failure was an added control incorrectly
expecting four-token edge offsets to be valid transform-origin. Transforms1
section4 uses XY plus optional Z; Transforms2 perspective-origin uses generic
position grammar. The corrected group keeps the four-token input as an explicit
negative transform-origin check and positive perspective-origin check, and adds
the equivalent canonical calc axes as positive transform-origin controls across
all14 units. Original10 assertions are untouched; production code equals11d.
The font-size-zero reference passes in this complete run; originala1fc crash is
retained and not retrospectively reclassified. Corrected controls UNBUILT.
Spec: https://drafts.csswg.org/css-transforms-1/#transform-origin-property
Spec: https://drafts.csswg.org/css-transforms-2/#perspective-origin-property


QUALIFIED exact731e7aa0 on fixed c220 source base: authenticated controller/native0; workspace4982/0/17skip, docs31/0, all26 phases0. Original17 parsing4350/5126,20 renders1328pass/886mismatch,491 resources(all226RC21)318pass/112mismatch/23unsupported/38crash; zero lost throughout. Ready packaged on recorded a5a56a90 base; CSS/installedCSSOM paths have no drift from testedbase. All13 groups, including original10, pass. Previous a1fc/11d actual1 receipts remain independent; no retroactive waiver. Raw/source/1000 capturedELFs retained on bx_s6449xhp.
