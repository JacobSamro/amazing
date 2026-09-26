# LAYOUT-21: ruby pairing and placement (active)

Fixed before source: `6ab9a612` (LAYOUT-20 final source, checkpoint name
415fa342-layout20-candidate4). The binary and source provenance are recorded
in ../layout20/final-after; ../layout20/final-after-2.log contains the before
ruby run too. Full static css-ruby: **33 pass /85 mismatch of118**, zero renderer
failures. Inventory excludes25 script-dependent reftests. Pinned WPT c2a2fce69,
800×600, first-reference comparator. No after result yet.

Largest filename cluster is ruby-overhang (35 failures); pairing/box generation,
annotation levels and nested ruby are missing in the current flattened model.
Current code collects every base into one list and every annotation into one
list, losing segment boundaries and annotation levels; nested annotation text
is concatenated into its outer base. Ruby-align and ruby-position are computed
by CSS but not carried to shaping. Intra-level whitespace is discarded wholesale.

Implementation scope: preserve segment and annotation-level pairing, independent
spanning ranges, nested ruby content, whitespace pairing and auto-hidden levels;
carry placement/alignment styles and perform contextual overhang in horizontal
and vertical flow. Preserve column/source/text budgets. Write the complete model
and shaper before compiling; one bulk build/crate/full-directory cycle.

Spec: https://www.w3.org/TR/css-ruby-1/ (sections2.2–2.5,3.1,4.1–4.3,5.1).
Fixtures downloaded for read-only analysis at /tmp/layout21-ruby-fixtures/css/css-ruby.
Previous retained449 and table386 corpora must have zero losses at the final gate.
