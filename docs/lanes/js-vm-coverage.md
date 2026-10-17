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
