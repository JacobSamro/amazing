CSS42 border-image family, fixed CSS41 b90e2147 base.

CSS Backgrounds3 §5 source/slice/width/outset/repeat grammar and whole shorthand
expand into five independent noninherited cascade axes. Unordered groups remain
atomic; quad compression, fill placement, slash omission, dimensional types and
negative literal rejection are enforced. Global border resets all image axes;
side borders preserve them. Deferred variables, invalid-at-computed defaults,
explicit inheritance and URL stylesheet provenance follow winning declarations.
Computed arithmetic resolves numbers, percentages and actual em/rem/absolute
lengths; font/container/viewport-dependent math stays symbolic without context.
Three grouped regressions cover grammar, resets/math and cross-sheet inheritance.

The existing static parsing runner does not recreate WPT fixture font-size or
containing-block geometry. Context-dependent expected values remain failures;
no guessed fixture metrics were introduced. Painting border images needs an
owning painter consumer; this ticket proves CSS behavior only.

Final gate numbers and renderer hash are recorded in gate.json. The identical
15-directory runner is measured before and after, keyed by input/classification,
with diagnostic value changes excluded from gained/lost identities. Whole pinned
css-backgrounds rendering is gated against the preserved baseline compact map.
