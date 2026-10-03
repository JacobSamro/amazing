CSS43 generic positions, transform/perspective origins and reference boxes.
Fixed CSS42 6cd6ffe5 base, identical 15-directory static parsing runner.

Generic one/two/four-value positions reject background-only three-value syntax;
only all-keyword pairs may reorder by axis. Transform origin accepts one/two
axes plus an optional third length, forbids Z percentages and edge-offset pairs.
Reference-box keywords have independent noninherited metadata and cascade rules.
Pure computed px/em/rem arithmetic resolves against actual computed font-size;
percentages remain geometry dependent, unavailable font/container units symbolic.
Zero Z is omitted; origin defaults50%50%, reference boxview-box.

Parent precisely granted image.rs::position consumer and grouped tests. A lower
CSS PositionAxis affine query rejects unsupported units/nonlinear expressions;
image presentation retains fractions plus offsets until actual content/object
size is known, including negative leftover space under cover. Existing legacy
edge strings remain usable. This consumer prevents canonical calc complements
from silently reverting to centered positioning. Other paint effect hooks stay
layout L44-owned. No installed perspective projection or reference-box effect
claim is made by this ticket.

Final workspace/docs and whole css-images/css-transforms rendering compare
against preserved baseline maps; full per-case gains/counts in gate.json.
Geometry-specific CSSOM resolved values remain static-runner context limits.
