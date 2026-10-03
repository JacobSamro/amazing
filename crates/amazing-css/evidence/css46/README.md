CSS46 font shorthand grammar and computed CSSOM serialization.
Fixed CSS45 d9dbf6b9 base; identical 15-directory parsing runner, plus full
supported static css-fonts render selection measured before/after.

A single lower parser validates and expands unordered optional style/variant/
weight/legacy width groups, adjacent oblique angles, decimal weights, complete
nested math size/line-height components and quoted/identifier font families.
Invalid declarations produce no expansion, preserving prior valid winners.
Invalid winning substitutions still default all shorthand reset axes.
Reset-only font properties are reset independently; synthesis and palette survive.
System keywords use the permitted UA default fallback, with no OS preference query.
CSSOM font serialization uses actual axes, requires representable reset-only state,
converts exact legacy width percentages to keywords, and refuses unknown widths
or unresolved relative weights. Bold is serialized as 700. General computed font
weight/oblique math normalization is a separate follow-up; no contextual metrics
or fixture dimensions are guessed. Public driver API stays unchanged.

Three grouped regressions cover grammar/cascade/reset/serialization boundaries;
the older raw-invalid compatibility expectations are updated to reject atomically.
Exact counts and case gains are recorded in gate.json.
