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

## JSVM-2: coverage before resumable frames

First-blocker ranking across 34,331 eligible fallback files on pinned Test262
(`3655e746`): call target 12,283; `new` 5,389; object literal 4,691; class
3,522; array literal 2,555; async/generator function 1,610; member access
1,315; assignment target 686; `try` 543; parameter pattern 516; for-of 290;
RegExp literal 253. The diagnostic reports the first compiler rejection per
file; it does not estimate independent support for later constructs.

The compiler now accepts calls, construction, simple object and array literals,
member reads and writes, computed keys, and receiver-preserving member calls.
All call, construction, and property operations use the existing interpreter
value model and built-ins. On the fixed `d8a228d` Windows base, the initial
expanded run compiled 18,498/34,486 eligible files (53.6392%) but lost four
passing files. Raising fuel for one finite loop and imposing an atomic
compiler-depth fallback for three deep expressions resolved those losses.
The full corrected run compiled 18,497/34,488 eligible files (53.6331%),
retained every tree-walker pass, and gained 18 file and variant passes.

The execution state now lives in a boxed VM frame: registers, references,
scope chain, environment, program counter, instruction fuel, and future try
handler stack. A unit test suspends at instruction boundaries, resumes the same
allocation through a loop and nested scope, and verifies completion without
repeating side effects. Generator objects and async functions remain future
tickets.

Final Windows Test262 on fixed `origin/dev` base `b42383a` used the rebased
JSVM-2 commit `8db71e8`, pinned revision `3655e746`, 53,872 files, 102,918
variants, and input SHA-256 `0aa8de46`. The tree walker passed 10,965 files
and 21,114 variants. VM plus fallback passed 10,983 files and 21,132 variants:
zero lost file or variant passes, 18 gains of each. The VM compiled
18,497/34,488 eligible files (53.6331%), up from JSVM-1's 157/34,488
(0.4552%). The final baseline classification hash is `bfdcb4a0`; the VM hash
is `6aad91c7`. Windows amazing-js nextest passed 214 tests (one ignored) and
its doctest passed 1/1. Under a Windows build slot, workspace nextest passed
1,768 tests (five ignored) and the workspace doctests passed 2/2.

## Following tickets

- JSVM-3: generator objects and async functions built on resumable frames.
- Expand classes, complex bindings, exception handlers, and other remaining
  compiler fallbacks with differential and Test262 evidence.
- Add tracing GC roots after VM frames become resumable (ADR 0003).
