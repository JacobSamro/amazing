# ADR 0003 — JavaScript moves from the tree-walking interpreter to a bytecode VM

Status: accepted (2026-09-23)

## Context

The tree-walking interpreter is one of the locked decisions that AGENTS.md
says must be revisited as a project-level event before Stage 2. It has hit a
hard semantic limit before then. `await` on a pending promise returns
`undefined` immediately, because a recursive AST evaluation cannot suspend a
function frame and resume it later. Generators have the same problem. Measured
cost: after JS-7 removed the parse barriers, 46 WPT `dom/` sources that load
`testdriver.js` time out instead of failing to parse, and large parts of
Test262 (async functions, generators, async iteration) cannot pass. Stage 1
already names "JS engine v2 (bytecode + GC + regex + modules)" as its goal.

Options considered:
1. **A register bytecode VM with heap-allocated, resumable frames.** Chosen.
2. CPS or state-machine rewriting of async and generator functions inside
   the tree-walker. Less upheaval now, but slower, complex, and rewritten again
   when a VM arrives.
3. Defer, and accept the timeouts.

## Decision

Build a register bytecode VM in `amazing-js/src/vm/`, starting from the
experimental JSRT-1 register VM draft. Frames live on the heap and can be
suspended and resumed, which is what generators, async functions, async
generators and top-level await need.

- **Single-threaded, cooperative.** Locked decision 1 stands. No threads.
- **Parity-gated migration.** The VM compiles and runs a growing subset. For
  syntax it does not yet support it rejects compilation atomically, and the
  tree-walker runs the script (the existing "never fall back mid-execution"
  rule of the draft). The fallback is removed only once full Test262 on the VM
  is at least the tree-walker's count with zero lost passes.
- **Value model shared.** `JsValue`, `JsString` (JSRT-5), Symbols (JSRT-6),
  objects and the BindingHost facade are shared, so DOM bindings do not change.
- **GC** stays Rc-based for now. A tracing collector (`gc/`) is a later
  ticket in the same lane; the driver's root registry is designed to become
  its root set.
- **Ownership:** a new `js-vm` lane owns `vm/` and `gc/`. js-runtime keeps the
  interpreter, built-ins and value model; js-lang keeps the front end and
  `ast.rs`.

## Consequences

- The tree-walking lock is replaced; ARCHITECTURE.md's locked list is updated
  to point here.
- Every VM ticket gates on full Test262 with zero lost passes and reports the
  share of scripts running on the VM against the fallback.
- async/await becomes correct only when the VM runs those functions. Until
  then the tree-walker keeps its current behaviour, recorded as a known gap.
