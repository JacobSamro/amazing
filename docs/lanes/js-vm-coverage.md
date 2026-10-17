# JS VM program coverage (r4-vmclass)

Base `d813a2177`. Source change: three gates removed, one limit replaced. No new opcodes were needed
and nothing was ported from boa or lumen: class fields, accessors, private names and static blocks
already run through the shared `ClassEvaluation` machinery that the VM drives
(`begin_class_evaluation` .. `finish_class_evaluation`); `catch` patterns already lower through
`binding_pattern`. Only the eligibility gate in `vm/eligibility.rs` refused them.

## Changes
- `vm/eligibility.rs`: class fields, static blocks and catch binding patterns no longer force the
  tree walker. Direct `eval` inside field initialisers, keys, decorators and static blocks still does.
- Fuel: `UNMETERED_INSTRUCTION_LIMIT` (usize::MAX) for top-level programs run through the interpreter,
  ordinary callables and generator/async frames. The tree walker is unmetered, so the VM no longer turns
  a long loop into `RangeError: bytecode instruction limit exceeded`. Explicit `Program::run(limit)`
  callers keep their own limit.
- `scripts/test262-boat.sh`: optional `T262_EXTRA_ENV` (for example `AMAZING_JS_VM=1`) exported in each shard.

## Test262 (pinned, whole suite, runner `scripts/test262.py`, file-by-file vs `d813a2177`)
| Mode | Pass files base -> new | Pass variants base -> new | Lost | Gained |
|---|---|---|---|---|
| default | 53,266 -> 53,266 | 102,307 -> 102,307 | 0 | 0 |
| `AMAZING_JS_VM=1` | 53,260 -> 53,265 | 102,295 -> 102,305 | 0 | 15 |

These ran as unsharded single-box runs: the 16-shard `test262-boat.sh` failed four times and again on
a clean rerun with source-upload timeouts (`native-source-upload exited 124`), see questions.md.

## VM coverage (`AMAZING_JS_VM=1`, 94,258 processes)
| | base | new |
|---|---|---|
| Test bodies on the VM | 88,103 (93.47%) | 91,621 (97.20%) |
| Remaining fallback reasons (bodies) | class field 3,348, catch pattern 196, static block 76, ... | direct-eval-context 2,049 (deliberate), direct eval call 358, top-level `with` ("statement kind") 222, resource limit 6, register limit 2 |

Top-level `with` stays a documented atomic rejection (`unsupported_program_is_rejected_before_hoisting_or_side_effects`).

## Benchmarks (`scripts/js-bench.py --runs 7`, median ms, default / vm / wrapped; base -> new)
No program slower in any mode (box noise favoured new; all 36 pairs equal or faster). Raw: evidence JSON.

## VM as the default (ticket vm-default, stacked on the coverage commit)
Eligible top-level programs now run as bytecode by default; `AMAZING_JS_VM=0` selects the tree walker. Ineligible or
uncompilable programs, direct-eval code and the test-only `force_ast_callables` mode still use the tree walker.

- Bug found by Test262 and fixed: a `for (let {a} = ...; ...)` head with destructured bindings lost them at the first
  per-iteration copy (`CopyScope` listed only plain identifier bindings), `staging/sm/lexical-environment/bug-1216623.js`.
  Regression test `for_let_head_pattern_bindings_survive_per_iteration_copies`.
- Test262 default mode, file+variant diff, origin/dev `fbb640a41` -> this commit: files 53,415 -> 53,416, variants 102,605 -> 102,607,
  0 lost, 3 gained (the bug-1216623 entries). Raw sha256 base `ae9d5ef6...608`, new `1010d6b3...31f` (box ~/amazing/out/r4-vmclass/).
- WPT dom/nodes, traversal, events, collections (510 files, `crates/amazing-bindings/evidence/dom-b-audit/run-corpus.py`):
  14,558 passing subtests before and after, 0 lost, 0 gained. DOM bindings work from VM frames.
- Benchmarks (median of 7, ms, default mode, origin/dev -> this commit): top-level VM is not faster than the tree walker;
  the earlier finding stands (global variable access dominates, identical on both backends), so the default flip is
  speed-neutral (all programs within +/-6%, run-to-run noise; method_calls 276 -> 285, scheduler 197 -> 208,
  array_loop 151 -> 148, class_inherit 163 -> 151, string_ops 159 -> 139). The top-level speedup needs slot-resolved
  globals (r3-jsperf territory). Wrapped-in-function code stays 1.3x to 2.3x faster than top-level on both.
