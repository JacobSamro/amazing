CSS45 additive CSSOM property-name registry.
Fixed CSS44 d7bd3002 base; identical 15-directory parsing runner and whole
images/transforms classifications. Driver requested lower-owned IDL accessors
without repeating shorthand grammar.

cssom_property_names returns sorted, deduplicated ordinary supported property
names, including shorthand and legacy aliases, excluding dynamic custom names.
cssom_shorthand_names exposes the recognized shorthand registry.
cssom_shorthand_longhands projects recognized reset axes using existing lower
expansion, including font/background/grid/flex/animation families. Enumeration
is independent of serialization: unsupported composite font/background getters
continue returning None instead of joining reset axes into invalid CSS.
An existing grouped regression covers names, ordering, projection and the
unsupported-composite boundary. Installed JS/CSSOM behavior remains driver-owned.
Exact workspace, parsing and render counts are recorded in gate.json.
