# js-vm lane decision map

## JSVM-1: atomic register VM fallback

Final-gate base: `cb3064b` (JSRT-5). The earlier `d2a610e` baseline and first
VM run are retained as pre-rebase evidence. The full pinned Test262 comparison
on this base found zero lost passing files or variants and six gains. Two VM
runs on identical inputs produced the same classification hash
`e446690f5f0ad03523583d1bae8338f9c9c703ff08312c43fd0d0856b545299d`.
JSRT-6 may require one more rebase.

- [x] Revive the JSRT-1 register compiler and executor from `8a04aed`.
- [x] Compile a complete script, including supported nested functions, before running any bytecode.
- [x] Route bytecode functions through a shared `FunctionBody::Bytecode` value and preserve closure environments.
- [x] Add explicit VM execution for differential tests and opt-in atomic fallback for the Test262 host.
- [x] Measure per-file VM compilation with the pinned Test262 runner and retain pass/fail counts for VM and fallback files.
- [x] Rebase after JSRT-5, then record full Test262 baseline and two repeat VM gates with zero lost passes.
- [x] Run crate nextest and doctests, workspace nextest and doctests on win-001, then report and stop.

Current compiled subset: primitive expressions, numeric and logical operators,
conditional expressions, identifier assignment and update, var/let/const,
blocks, if/while/do/var-for, ordinary functions and arrows, closure capture,
return and direct bytecode-function calls. Unsupported syntax aborts compilation
before execution. Calls to pre-existing AST functions are excluded from the
compiled subset; a bytecode call checks its target at runtime as a guard.

Pinned Test262 compiled 157 of 53,872 discovered files (0.2914%), or 157 of
34,488 eligible runtime files (0.4552%): 145 compiled passes and 12 compiled
failures. Fallback files had 7,136 passes, 27,005 failures, and 190 unsupported.
On win-001, amazing-js passed 209/209 nextest tests and 1/1 doctest; the
workspace passed 1,715 nextest tests (3 skipped) and 2/2 doctests.

## Following tickets

- JSVM-2: heap-allocated resumable frames, generators and async functions.
- Expand member references, object/array literals, complex bindings and
  control-flow edges with differential and Test262 evidence.
- Add tracing GC roots after VM frames become resumable (ADR 0003).
