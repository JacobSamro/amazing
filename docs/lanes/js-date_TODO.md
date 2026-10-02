# js-date lane

## JSD-6: RegExp Test262 timeout-cluster profiling (not landed)

Fixed base: `e11f3cc9` (JSD-5 push-gate fixes, on top of `1efacdd0`).

The coordinator's standing instruction after JSD-5: the ~430 files that time
out under the full `built-ins/RegExp` Test262 run (mostly
`property-escapes/generated/*.js`) are the single biggest cost in every
fleet Test262 run, so profile one and fix the hot path (class-set
construction or the matcher).

No fix was proven with measured numbers this round. Two leads were pursued
and one ruled out; both are recorded here plus in `log.md` so the next pass
does not re-derive them. The tree is left clean -- no uncommitted
experiments (verified via `git status --short`).

**Ruled out: `Rc<Vec<Option<usize>>>` copy-on-write for `State.captures`/
`marks` (`crates/amazing-regex/src/vm.rs`).** Hypothesis: since
`Node::Repeat`'s unbounded-greedy-quantifier loop pushes one full `State`
clone per backtrack frame (`Op::Split`/`Mark`/body/`Progress`/`Jump`,
vm.rs ~700-715), and `State` clones `captures: Vec<Option<usize>>` and
`marks: Vec<Option<usize>>` on every push, making those `Rc`-backed with
`Rc::make_mut()` at the `Save`/`Clear`/`Mark` write sites should cut the
per-frame clone cost for large-property-escape patterns (which have few or
no actual capture groups, so most frames could share the same `Rc`
allocation). Implemented, compiled clean, all crate tests passed -- but
**measured zero wall-clock improvement** on the real target files. Fully
reverted (`git checkout -- crates/amazing-regex/src/vm.rs`); nothing to pick
up here unless a different data structure is tried (e.g. a single flat
`Vec` shared across frames with an undo log, rather than per-frame
snapshots at all -- a bigger VM change, not attempted).

**Open lead 1: `unicode_props::lookup()` is never cached.** Every
`SetExpr::Property`/`Member::Property` match calls
`crate::unicode_props::contains(name, x)`
(`crates/amazing-regex/src/unicode_props.rs`), which calls `lookup(expr)`
fresh -- `split_once('=')` plus several `BTreeMap` alias lookups
(`name_alias()`, `gc_value_alias()`, `sc_value_alias()`, `binary()`,
`general_category()`, `scripts()`) -- **on every single character checked**,
not once per compiled `Op::Set`/`Op::Class`. For a property-escape pattern
matched against a huge generated string (one code point per member of a
large category, often 100k+), this re-resolves the same property name
100k+ times. Not yet measured in isolation -- the isolated regex-only
benchmark (`profile_scratch_large_property_escape_timing`, added and
removed this round, see below) hit an unrelated resource-limit wall before
this cost could be isolated. Next step: cache the resolved `Lookup` once at
compile time on the `SetExpr::Property`/`Member::Property` node itself (or
in a small side table keyed by the compiled `Op` index), so the per-
character cost is a lookup table scan (already `&'static [(u32,u32)]`, a
binary-searchable sorted range list) with no string-parsing/`BTreeMap`
overhead repeated per character.

**Open lead 2 (a real bug, not yet root-caused): an unexplained early
`Error::Limit("backtracking storage")`.** An isolated benchmark matching
`"a".repeat(len)` against `/^\p{Alphabetic}+$/u` hit the backtracking-
storage `RangeError` already at `len` in the 50,000-300,000 range -- far
sooner than the `captures.len() + marks.len() + 3` slots-per-frame formula
predicts for a pattern with zero capture groups (`budget.slots`'s cap is
1,048,576; a 3-per-frame minimum weight alone should allow ~349k frames,
and this pattern has no backtracking ambiguity at all -- a linear scan
should never approach that). Two live hypotheses, neither checked yet:
(a) `FoldedClass`/`Member::Property` compilation for `Alphabetic`
specifically has a per-frame cost weight higher than the naive
`c.members.len() + prepared.folded.len()` charge assumes; (b) the actual
compiled shape for `\p{Alphabetic}+` under the `/u`+`/v`-adjacent
`Op::Set` path pushes more than one frame per matched character for some
reason not yet inspected in the compiled program. Next step: dump the
compiled `Program`/`Op` sequence for this exact pattern and step through
`budget.slots`/`budget.saved` growth per character with `eprintln`
instrumentation, before pursuing either hypothesis further -- this needs
doing BEFORE lead 1's caching fix, since if this is a real logic bug (not
just a cost-model mismatch) it would invalidate benchmark numbers taken
against the current budget accounting.

Both leads need a working isolated benchmark that doesn't itself trip a
resource limit before reaching a measurable steady state -- the harness
used this round (`#[test] fn
profile_scratch_large_property_escape_timing()` in
`crates/amazing-regex/src/tests.rs`, first built the real ~1.8M-unit
`nonMatchSymbols` string from the actual test file and hit `MAX_INPUT`,
then switched to `"a".repeat(len)` for `len` in `[50_000, 150_000,
300_000]` and hit lead 2's limit instead) was removed before commit rather
than landed half-working. Whoever picks this up should fix lead 2 first (or
at least confirm it's a cost-model quirk rather than a bug) so a clean
timing benchmark is possible, then measure lead 1's caching fix against
real wall-clock time on the actual `property-escapes/generated/*.js` files
(via `scripts/linux-run.sh js-date -- '<test262 run>'`), not just the
isolated microbenchmark, before claiming a fix.

## JSD-6: deterministic repetitions and cached properties (2026-10-03)

Fixed base `2eb1229a`. The early storage wall was the independent
`MAX_STACK = 8192` bound, not erroneous slot accounting. Added a dedicated
instruction for greedy repetitions of deterministic, capture-free atoms:
characters, dot, classes and character sets without strings. It scans each
code point once and saves a single retry frame whose endpoint walks backward
on suffix failure. This preserves continuation-before-earlier-repetition
ordering from [ECMA-262 RepeatMatcher](https://tc39.es/ecma262/multipage/text-processing.html#sec-repeatmatcher).
Captures and marks share identical snapshots at every endpoint because the
optimized atom cannot write them. Lazy repeats, groups, assertions and string
sets retain generic execution.

Unicode properties resolve once during compilation to static range slices;
Script_Extensions also resolves its explicit, overridden and fallback ranges.
The input bound and JS adapter now admit 4,194,304 UTF-16 units, and the work
budget is 8,388,608, enough for a linear scan over the full Unicode repertoire
(2,162,688 units). Ambiguous matching still has the existing frame/slot bounds.
No code was ported from an upstream engine.

Measured on Boat `pool-js-date`, large (8 vCPU/16 GB), baremetal:

- Complete RegExp selection including Annex B at 5 seconds: before 1501
  pass / 21 fail / 440 timeout; after 1502 / 13 / 447. Before used 4 jobs;
  after used 8, so elapsed times (1113.3 / 568.356 seconds) are not an
  engine speed comparison. Seven storage failures became harness deadlines.
- One gained passing file: `S15.10.2_A1_T1.js` (XML shallow parsing).
  Zero lost passing paths against the latest full-push baseline (1501
  pass / 34 fail / 427 timeout at 60 seconds).
- Final workspace: 3517 nextest passed, 17 skipped, 73.928 seconds; four
  doctests passed. All 1502 baseline/new passing RegExp files also passed
  a 60-second recheck (5.187 seconds total) using the unchanged runner on
  copies of the original pinned files and all harness helpers.
- The long-repeat regression, covering a 2,162,688-unit property match,
  suffix retries, captures and astral endpoints, passed in 0.624 seconds.

Durable classifications and hashes: `js-date_jsd6_evidence.json`. Recheck
recipe: `bash docs/lanes/js-date_gate.sh` after the full before/after run.

The remaining generated-property deadlines are still open. `buildString`
constructs roughly 1.1 million code points through ordinary JS loops, which
run interpreted by default. js-vm confirmed ordinary VM compilation needs
`AMAZING_JS_VM=1`, and its current 10-million-instruction fuel can itself
expire on this loop. Bulk timing/default-VM feasibility is routed to js-vm.
Do not count opt-in VM measurements as default-engine gains.
