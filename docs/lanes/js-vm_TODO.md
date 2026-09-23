# js-vm lane decision map

## JSVM-1: atomic register VM fallback

Final-gate base: `cb3064b` (JSRT-5). The earlier `d2a610e` baseline and first
VM run are retained as pre-rebase evidence. JSRT-6 may require one more rebase.

- [x] Revive the JSRT-1 register compiler and executor from `8a04aed`.
- [x] Compile a complete script, including supported nested functions, before running any bytecode.
- [x] Route bytecode functions through a shared `FunctionBody::Bytecode` value and preserve closure environments.
- [x] Add explicit VM execution for differential tests and opt-in atomic fallback for the Test262 host.
- [ ] Measure per-file VM compilation with the pinned Test262 runner and retain pass/fail counts for VM and fallback files.
- [ ] Rebase after JSRT-5, then record full Test262 baseline and two repeat VM gates with zero lost passes.
- [ ] Run crate nextest and doctests, workspace nextest and doctests on win-001, then report and stop.

Current compiled subset: primitive expressions, numeric and logical operators,
conditional expressions, identifier assignment and update, var/let/const,
blocks, if/while/do/var-for, ordinary functions and arrows, closure capture,
return and direct bytecode-function calls. Unsupported syntax aborts compilation
before execution. Calls to pre-existing AST functions are excluded from the
compiled subset; a bytecode call checks its target at runtime as a guard.

## Following tickets

- JSVM-2: heap-allocated resumable frames, generators and async functions.
- Expand member references, object/array literals, complex bindings and
  control-flow edges with differential and Test262 evidence.
- Add tracing GC roots after VM frames become resumable (ADR 0003).
