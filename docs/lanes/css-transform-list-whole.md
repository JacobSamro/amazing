# Whole CSS transform-list producer and matrix handoff

Current recorded origin/dev base `d265908ea158e34d82084d0c94e8b0f429751cd1`; original source `e5025b67a11e0c23ca2d01de9a9746a2ae1fd892` on `2ac44102cf5a8a909658e0ffc75a94f6eade31aa` is preserved. Own CSS source only. Authentic current `1af30cd39b94f4c57d26a8d95ed140da9087f9b7` AFTER baseline passed all26 whole phases and original491 resource/226 RC21 controls; original408 failure cluster inventory:20 valid transform grammar failures and207 transform render mismatches. Counts identify a real producer gap; they do not imply all207 render failures are repaired. The color70 parsing cluster already has a separate precise-color API, so collector gaps are not disguised as this source ticket.

Whole function list includes all2D and3D translate/scale/rotate/skew/matrix/perspective derivatives, nested math arguments, percentage scale, zero-angle handling, dimension/range/separator/arity negatives, EOF recovery and adjacent functions. Existing2D typed variants, f32 matrix shortcut and original assertions remain. Font lengths compute in CSS; reference-box percentages and unresolved relative bases remain explicit. Perspective `none` retains a nonempty identity function, negative literal depths are invalid, calculated depths are range-clamped and used depths clamp to1px.

Typed4×4 ABI uses row-major arrays on column vectors, CSS column-major matrix3d argument order, post-multiplied function order, explicit actual reference box and3D origin, guarded axis normalization, and finite output. Axis-angle code is ported in whole from permissive [glam-rs e7e58845](https://github.com/bitshifter/glam-rs/blob/e7e588454babd944376baabaae539407992a7408/src/f64/dmat4.rs); MIT license verified at that commit. No crate/Cargo dependency. Normative references: [Transforms1](https://drafts.csswg.org/css-transforms-1/#transform-functions), [Transforms2](https://drafts.csswg.org/css-transforms-2/#three-d-transform-functions).

Consumer boundary: the existing CSS-owned4×4 property-tree invalidation matrix consumes all typed functions with actual cached box sizes. Existing animation shortcut conservatively declines new3D/math families for main-thread handling. Scene flattening,3D rendering contexts/backface visibility, author origin adoption outside the established default-origin shortcut, and paint/hit-test consumption remain questions for compositor/driver owners; no complete3D paint claim.

Source is UNBUILT/UNQUALIFIED. No allocation yet. Final whole plan must include workspace nextest plus doctests, whole17 originalCSS selections,20 render dirs and491 resource/all226 RC21 controls against the authentic successful1af AFTER baseline on exact fixed source/profile, original2D controls and new10 bulk test groups; source/commands/capacity freeze precedes any native allocation. Compact evidence only; root16 merge gate remains independent. Other prepared contain/aspect/image producers and failedpipeline receipts are preserved.

Questions for the consumer owners: adopt `amazing_css::transform_matrix` with the actual transform-box and three-dimensional transform-origin in the retained compositor/driver; preserve existing context flattening and backface rules. The present producer ticket does not change paint or hit testing. These follow-up consumers are not additional merge proof requirements for this CSS source area.


## Genuine angle serialization successor

The authentic 4bd gate failed the unchanged SVG presentation assertion:
`rotate(30deg)` became `rotate(29.999999999999996deg)`. A public hash-bound
native diagnostic retained the original nextest exit100 and outer exit1.
The successor evaluates computed angle text directly in degrees using the
existing dimension grammar and number evaluator, across all transform rotation
and skew functions and individual rotation. Typed matrix angles still use
radians. Three additional whole groups cover every function, all four units,
mixed-unit math, invalid dimensions/ranges and typed-matrix agreement.
Original assertions and all10 original transform groups are unchanged.
The genuine successor1c682 native gate passed: workspace4946/0, docs30/0,
all26 phases0; parsing4350/5126 (+20), render1328/2214 (+1), resources491
with all226RC21 retained and zero lost passes throughout. Raw/source/999ELFs
remain on the native host; compact evidence binds exact sources and hashes.
The ready commit records the current published base separately from the actual
qualified source and authentic1af AFTER baseline. Full3Dpaint remains outside
this producer ticket.
