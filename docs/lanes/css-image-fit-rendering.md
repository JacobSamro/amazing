# CSS image fitting and rendering property producers

Recorded origin/dev base a035b2b558065d72664ce7617da0a7261027bb0d.
Qualified native source `d1204e6a4fcc2a5309aa67c3d369f71e532e0b50`; actual owner54894 exited zero.

Object-fit accepts every singleton and the contain/cover plus scale-down flag
in either order, serializing the sizing keyword before its flag. Image-rendering
owns the full singleton grammar auto, smooth, high-quality, crisp-edges and
pixelated, initial auto, inherited values and PaintOnly invalidation. All
CSS token boundaries, duplicate/extra components, case/comments/escapes,
CSS-wide resets, variables, attribute fallbacks and importance use the actual
shared cascade. Original registry coverage remains, with its PaintOnly count
updated for the registered property. No dependencies, sources from held font,
math, viewport, containment or paint tickets, or corpus changes are imported.

Four grouped controls cover every fit/flag order, every rendering keyword and
actual inherited/noninherited split, malformed atomic declarations preserving
their preceding winner, and actual variable/attribute/reset/importance and supports-condition flows.
Pinned parsing currently has five object-fit and two image-rendering failures.
The complete workspace/docs and seventeen original parsing selections plus
twenty original render directories will qualify this independently before/after.
The source owns CSS property values, not new sampling kernels, natural image
size selection or object-fit paint geometry; those remain consumer-owned.

Image-orientation's seven pinned invalid-angle/flip cases are deliberately not
changed here: CSS Images 3 still permits those deprecated optional alternatives,
and the current engine accepts them. Preserve them until a separate compatibility
choice resolves the difference between optional support and that WPT oracle.
This is a source-scope decision, not an added merge gate.

Primary specifications:
https://drafts.csswg.org/css-images-4/#the-object-fit
https://drafts.csswg.org/css-images-3/#the-image-rendering
https://drafts.csswg.org/css-images-3/#the-image-orientation

Native qualification: workspace 4698→4702 passed,
zero failed; doctests 4 and 4 passed.
All seventeen parsing selections 4323→4330/5126,
zero lost, unchanged files/nonliteral skips. Original twenty render cohorts:
{'before': {'pass': 1327, 'mismatch': 887, 'error': 0, 'timeout': 0, 'unsupported': 0, 'crash': 0}, 'after': {'pass': 1327, 'mismatch': 887, 'error': 0, 'timeout': 0, 'unsupported': 0, 'crash': 0}}; zero lost paired and previously recorded a035 passes.
Native raw output remains on Boat under recorded paths. Compact proof retains
all original test classifications and loss/gain lists plus native hashes while
removing duplicated diagnostic and unchanged-id lists. Evidence 317823 bytes.
The generic static render selection excludes image/url resources; this ticket
claims grammar/cascade qualification, not installed image geometry or sampling.
A concrete ImagePresentation consumer patch and resource-backed image proposal
are prepared separately; preserve the frozen RC21 226 entries and full image
resource cohort when qualifying that consumer. Integrator sixteen-shard Test262
is still required before dev. No native tooling rerun was needed for this ticket.
