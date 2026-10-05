# Containment keyword grammar and cascade

Recorded origin/dev base: `a035b2b558065d72664ce7617da0a7261027bb0d`.
Qualified source: `6fe016be2aac56aef6a919cb4ac916481605bfcc`; native command owner session 58263 exited zero.

The exact recorded seventeen-selection parsing baseline accepted fourteen
invalid values from pinned `css/css-contain/parsing/contain-invalid.html`.
`contain` had no owned grammar or initial/inheritance metadata and therefore
fell through to an unparsed value. This ticket owns the whole property value
and cascade area, without importing Layout containment solvers or held CQ,
font, paint, viewport or DPR source.

The parser accepts the independent size-or-inline-size, layout, style and
paint categories, or one exclusive none/strict/content keyword. It validates
decoded identifier tokens, accepts case/escapes/comments, rejects duplicate
categories and mutually exclusive size axes, and canonicalizes specified
lists in grammar order. Computed/CSSOM serialization preserves the equivalent
strict/content compact keywords. Metadata supplies actual initial none and
noninheritance; CSS-wide keywords, deferred custom-property substitution,
invalid-at-computed-value handling, importance and all resets use existing
cascade paths.

Primary grammar: https://drafts.csswg.org/css-contain-2/#contain-property.
Pinned original WPT `c2a2fce69e5c2509d9dc28440ec8b42532e5c293` valid/computed
fixtures supply the unchanged canonical order and equivalent-keyword controls.
No upstream engine code was copied and no dependencies were introduced.

Five grouped controls cover every valid category combination and permutation,
the complete invalid keyword/token/duplicate matrix, escaped/case/comment
boundaries, actual parent/child custom-property and CSS-wide inheritance,
importance and invalid substitution, registry/CSSOM and all reset behavior.
The complete paired Linux gate qualified workspace nextest and doctests, all
seventeen original parsing selections and all twenty original render directories.
Baseline workspace: 4698 passed, zero failed; candidate:
4703 passed, zero failed. Four doctests passed on both sources.
Parsing: 4323 to 4339/5126, zero lost;
514 files and 169 nonliteral skips unchanged. Render: 1327 passing, 887 mismatches
on each source, zero lost passes across all twenty directories.

The initially prepared source failed the original registry-classification
control. The qualified source adds conservative Rebuild classification for
containment's counter scope, containing-block and stacking effects; all original
controls remain. Its first corrected-source tooling run exhausted /tmp. The one
clean rerun on bx_fqxx9522 completed; bx_yvdvxk7y and its raw output remain intact.
Raw phase logs and result images remain on Boat (some native render paths are
under /tmp). Compact proof records authentic terminal exits, endpoint identities,
source/corpus identities, raw log hashes and complete render classifications.
Evidence totals 1380327 bytes, within 512 KiB/file and 2 MiB/ticket.
The integrator's independent 16-shard Test262 gate remains required before dev.
This ticket qualifies property grammar and cascade, not Layout containment solvers.
