# Aspect-ratio value and cascade producer

Recorded origin/dev base: a035b2b558065d72664ce7617da0a7261027bb0d.
Qualified source: `7ab1c8a65a282d19f2a1c281b7233e1fe3fc57c5`; actual native owner77324 exited zero.

The pinned seventeen-selection parsing run accepts eleven invalid aspect-ratio
forms and misses five canonical computed serializations. This ticket supplies
one complete auto/ratio component grammar rather than a collection of ad hoc
string substitutions. It rejects duplicate auto, extra components, invalid
solidus placement, dimensions, percentages and negative literal numbers.
Either auto order becomes the canonical prefix, omitted denominators become
one, and zero/zero-denominator pairs remain valid degenerate ratios. Numbers
are not divided, reduced or replaced with an invented preferred ratio.

The property uses its existing initial auto, noninheritance and Layout change
classification. CSS-wide inheritance, invalid-at-computed-value substitution,
custom properties, typed element attributes, importance, all resets and HTML width/height hints go
through the existing actual cascade. Shared finite number math is typed and
computed independently for each component; unsupported shared numeric
computation remains visibly unresolved. No held math, containment, font or
paint history is imported, and no new Cargo dependencies are introduced.

Four grouped controls cover the full literal/auto order matrix, malformed
atomic declarations retaining their prior winner, CSS token boundaries and
current shared typed finite math, and actual parent/child cascade, reset,
importance, classification and native HTML presentational hints.

Primary sources: CSS Sizing 4 section 4.1 and CSS Values 4 section 5.7:
https://drafts.csswg.org/css-sizing-4/#aspect-ratio
https://drafts.csswg.org/css-values-4/#ratios
Original pinned WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293 fixtures remain
unchanged. Qualification will cover the workspace, doctests, all seventeen
parsing selections and all original twenty render directories before/after.
Layout owns used sizing, replaced natural-ratio choice and animation; these
are not claimed by this CSS producer ticket. Shared numeric math extensions
remain their existing independent area, not a new merge check for this ticket.

Native paired workspace: baseline4698
passed, candidate4702 passed, zero failed;
Doctests4 and4 passed.
Parsing4323→4339/5126,
zero lost; same514 files,169 nonliteral skips. All17 selections retained.
All20 original render directories retained. Actual paired render counts:
{'before': {'pass': 1326, 'mismatch': 887, 'error': 0, 'timeout': 0, 'unsupported': 0, 'crash': 1}, 'after': {'pass': 1327, 'mismatch': 887, 'error': 0, 'timeout': 0, 'unsupported': 0, 'crash': 0}}. The paired baseline captured one caption crash that
had passed in the previous preserved a035 capture; original recorded passing
controls are also retained in the compact comparison. No extra capture gate was
introduced and no causal claim is made for any observed crash recovery.
Raw files remain on Boat under recorded native/HOME paths. Ready evidence is
compact only; the integrator16-shard Test262 remains required before dev.
