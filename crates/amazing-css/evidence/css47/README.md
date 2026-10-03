CSS47 basic font selection grammars and computed normalization.
Fixed CSS46 8c45bd1d; identical 15-directory parsing runner and whole supported
static css-fonts render classifications measured before/after.

Weight/style/stretch/family now validate complete values atomically. Actual
parent weights determine bolder/lighter; decimals survive, number math clamps
at computed time to 1..1000. Pure oblique angles normalize to degrees with
computed math clamping to -90..90. Legacy width keywords normalize to percentages;
font-width aliases the same stretch cascade/animation axis and CSSOM value.
Family serialization preserves quoted generics/reserved names while unquoting
safe identifier names. Existing Values5 random-item syntax remains Unparsed;
this ticket does not invent a random selection. Typed sign/container expressions
remain accepted but unresolved when actual dimensions are unavailable.
The existing length evaluator is reused for pure number/angle/percentage math
without fake percentage geometry. Existing literal affine position behavior and
CSS-valid reversed clamp bounds are covered by grouped regressions.

Three grouped regressions cover grammars, actual parent/inheritance, width alias
and animation, serialization, unknown context and independent dimension types.
Exact counts/case gains are recorded in gate.json; driver adoption is separate.
