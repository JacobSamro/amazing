CSS44 computed CSSOM ownership seam and native-document font context.
Fixed CSS43 56877eaf base; identical15-directory parsing runner.

ComputedStyle::cssom_declarations returns sorted supported longhands followed
by sorted valid custom names/tokens, preserving case/empty values and omitting
guaranteed-invalid/cyclic values and unknown literal passthrough. Alias summaries
are excluded, including text-decoration; dynamic logical border longhands are
included. cssom_property_value owns canonical independent quad/pair/side-border
and existing derived font/text/image summaries. Reset-only image axes prevent
an unrepresentable global border summary. Unimplemented/unrepresentable shorthands
return None, not a stale raw declaration. Combined background axes with independent
winners remain unsupported until complete layer/order reconstruction.
Valid empty custom declarations and supports conditions now survive parsing;
ordinary empty declarations still fail. Used geometry/color CSSOM serialization,
WebIDL identity/readonly behavior and actual driver installation remain driver-owned.

compute_document_with_font_metrics styles a native Document arena root without
cloning/remapping indices, using supplied active sheets/environment/font context.
Body link hints and generated counter state are scoped to that root. Invalid or
non-document roots return an empty StyledDom. Existing NodeIdx(0) entry points
keep their behavior; driver rejects inert documents independently.

CSS43 affine API direct px/%/zero gap fixed; grouped negative/overflow/context
regressions cover literal pairs. Public used_position_axis additionally evaluates
calc/min/max/clamp against actual (possibly negative) reference dimensions;
unresolved font/container units are rejected. CSS-valid reversed clamp bounds
use minimum precedence rather than f64::clamp panic. Nonlinear image presentation
is not added; layout owns origin hooks using the actual-basis evaluator.

Six grouped regressions and full workspace/docs/wholeimages+transforms gates;
exact counts and case changes in gate.json. IDL property-name enumeration is a
separate additive follow-up requested after this fixed gate began.
