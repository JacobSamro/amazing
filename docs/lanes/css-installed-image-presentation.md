# Installed image presentation successor

Recorded dev base: ac72a9687c8a1ff76c3d0eb47db98a321ad263b8.
Prerequisite producer: lane/css-image-fit-rendering-ready @ 27425957118a7341b3846c9c28d5074c8bd0fc42. Its CSS API is composed explicitly in this source tree; package that producer independently before a successor ready ticket. Preserved, unqualified prior consumer: e1773cda7544770b3d9c03e331d9c4765433e68e. Initial+clean tooling allowance for e177 is exhausted; this is changed source, not a third run of e177.

The whole installed pipeline now obtains the fitting policy through the CSS producer's typed ComputedStyle::object_fit API, retaining image positioning through the existing affine-position API. Replaced layout still resolves intrinsic ratio, content/padding/border boxes and CSS constraints; fitting applies to that resulting content box, with actual PagePaint and inline StyledImages forwarding computed presentation. Geometry does not change the replaced element's measured outer size.

Actual CLI integration controls parse page HTML and CSS, decode a relative resource, perform replaced layout and inspect decoded painted pixels. They cover all five singleton fits, both contain/cover scale-down orders, escaped/commented identifiers, natural objects smaller and larger than content, corners, edge offsets, percentages, affine calc, custom properties, invalid declarations, importance, invalid substitution, intrinsic sizing/aspect ratio and border-box constraints. Existing paint helper, ancestor clip, alpha, buffer and finite-value controls remain intact. No image sampling or rendering-policy adoption is claimed.

Qualification selection is unchanged: workspace nextest + doctests, all 17 parsing selectors and original 20 render cohorts, full 226 RC21 entries and the complete 491 resource-backed css-images reftest selection. Both source sides use fresh distinct targets with the separately qualified public debug-profile wrapper; all raw originals stay on the native build host and only compact summaries are returned. Source is not qualified yet; no test result is inferred from preparation or file presence.

Specs: https://drafts.csswg.org/css-images-4/#the-object-fit and https://drafts.csswg.org/css-images-3/#the-object-position.
