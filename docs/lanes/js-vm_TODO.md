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

## JSVM-3-TRY: exception handlers on resumable frames

The VM now compiles `try`/`catch`/`finally` as control flow in the existing
heap frame. Throwing operations, explicit throws, and returns unwind lexical
scopes through the handler stack. A normal `finally` preserves the pending
completion; an abrupt `finally` replaces it. Catch bindings have their own
lexical environment. The handler stack survives instruction-boundary
suspension and is tested with nested handlers, optional catch binding,
runtime exceptions, return overrides, and loop re-entry. The compiler also
checks unresolved compound-assignment bindings before evaluating the right
side; full Test262 found the missing check when `try` became compilable.

On the pinned Linux checkout, same-host tree-walker versus VM comparison found
zero lost passing files or variants, 23 gained files and 28 gained variants.
Compiled coverage rose from 18,501/34,493 eligible files (53.6370%) on
JSVM-2 to 19,494/34,493 (56.5158%). The pinned Linux corpus input SHA-256
was `91109b82`; the baseline and VM classification hashes were `b9d73ca4`
and `cc08903c`. Final exact-commit Windows gates on win-002 passed: amazing-js
nextest 216 tests (one ignored) and its doctest 1/1; workspace nextest 1,770
tests (five ignored) and workspace doctests 2/2.

## Following tickets

## JSVM-3-GEN: generator frames and delegation

Generator functions now create branded objects with suspended VM frames.
`next`, `return`, and `throw` resume at yields or route abrupt completions
through catch/finally handlers. `yield*` delegates to arrays or iterator
methods and forwards normal, return, and throw completions. Generator calls
are non-constructible; the frame is initialized on first `next`. The
shared interpreter still owns values, calls, properties, and iterator methods.

Pinned Linux Test262 (`3655e746`, input SHA-256 `91109b82`) compared the
same-host tree walker with VM plus fallback: 10,967 versus 11,087 passing
files, and 21,118 versus 21,334 passing variants. There were zero lost
passes, 120 gained files, and 216 gained variants. The VM compiled
19,685/34,493 eligible files (57.0696%), up from 19,494/34,493
(56.5158%) at the try-handler gate. Classification hashes were `b9d73ca4`
and `b1b5d146`. Linux and exact-commit win-002 amazing-js nextest passed
218 tests (one skipped). Windows amazing-js doctest passed 1/1;
workspace nextest passed 1,772 tests (five skipped), and workspace
doctests passed 2/2.

The current generator prototype is per function and has own method properties;
shared `%GeneratorPrototype%` identity and full descriptor attributes remain
for a later conformance pass. Arrays and user iterators are delegated, while
other built-in iterable kinds still depend on the shared iterator model.

## Following tickets

## JSVM-4-ASYNC: await frames and Test262 async protocol

Ordinary async declarations and arrows now compile. VM `await` suspends a
boxed frame, registers fulfillment/rejection reactions, resumes through the
interpreter's promise job queue, and routes rejection through catch/finally.
Returned promises are adopted and thenables are resolved through the shared
Promise callbacks. The Test262 host recognizes `doneprintHandle.js` markers,
and the runner accepts an async-capable classic-script host while requiring
the print capability. The primary granted the runner edit.

An old/new runner replay on the same classic-only host and pinned input
produced the identical classification SHA-256 `b9d73ca4` across 53,872 files
(input SHA-256 `91109b82`). With async enabled, 5,624 previously excluded
async-flagged files entered the corpus. On source commit `603f3e7`, the
same-host tree walker passed 11,192 files / 21,526 variants; VM plus fallback
passed 11,316 / 21,750 on exact commit `f173f23`. There were zero lost passes,
124 gained files, and 224 gained variants. The VM compiled 20,331/37,818
eligible files (53.7601%). Classification hashes were `f90dd209` and
`007e7f27`; input SHA-256 was `91109b82`. These exact counts
include the primary-requested shared operator routing: VM unary and update
instructions call interpreter helpers, while binary/comparison already call
the shared binary operator. JSB-2 BigInt helper integration and the Windows
gate remain before this ticket is complete.

On Linux, exact commit `603f3e7` passed 220 amazing-js nextest tests (one
skipped), 1/1 amazing-js doctest, 1,771 workspace nextest tests (three
skipped), and 2/2 workspace doctests. The scheduling fix at `f173f23` passed
221 amazing-js tests (one skipped); a focused await replay gained four files
with zero losses. Its exact-commit Linux workspace gate passed 1,772 nextest
tests (three skipped) and 2/2 doctests. The full replay counts above include
that fix.

Async iteration syntax (`for await`) is not yet represented in the parser AST;
that requires a js-lang ticket. Many async Test262 failures also depend on
Promise/Array.fromAsync built-ins outside the VM lane.

## Following tickets

- JSVM-4 async iteration after parser support, then classes and complex bindings.
- Add tracing GC roots after VM frames become resumable (ADR 0003).

## JSVM-5 coverage, series 1

The VM now compiles BigInt and regular-expression literals, object methods and
accessors, object/array/call spread, untagged templates, and base classes.
Spread iteration and numeric/RegExp creation use the interpreter's shared
helpers. Derived classes remain an atomic fallback until `super` construction
is implemented.

On pinned Test262 `3655e746`, the complete venus-001 baseline and VM runs used
the same input SHA-256 `91109b82` and host SHA-256 `3befa104`. The tree walker
passed 21,246 files / 41,434 variants; VM plus fallback passed 21,420 files /
41,749 variants. **Zero lost passes**, 174 gained files and 315 gained
variants. The VM compiled 26,778/40,167 eligible files (66.6667%), up from
21,743/40,167 (54.1315%) at JSVM-4. Classification hashes were `9c54ada3`
and `d0741cd0`. Linux amazing-js nextest passed 242 tests (one skipped).

### Orphaned Test262 host investigation

An interrupted venus-003 coverage runner left six host processes alive for
over two hours. Their stdin was gone after termination, so the exact PID-to-
test mapping cannot be recovered. The six earliest timeout paths in the
complete pinned report were the likely sources:

- `built-ins/Array/prototype/indexOf/15.4.4.14-5-12.js`
- `built-ins/Array/prototype/indexOf/15.4.4.14-5-16.js`
- `built-ins/Array/prototype/indexOf/15.4.4.14-9-9.js`
- `built-ins/Array/prototype/map/15.4.4.19-3-28.js`
- `built-ins/Array/prototype/map/15.4.4.19-3-29.js`
- `built-ins/Array/prototype/map/15.4.4.19-3-8.js`

Both the tree walker and VM used the same Array builtins and timed out on these
tests. The `indexOf` builtin eagerly collected an array with length near 2^32,
even when `fromIndex` was Infinity or near the end; `map` walked objects whose
length exceeded the maximum Array length. The shared builtins now terminate
these cases. Focused replay at 10 seconds: five of these six paths pass in
both modes; the sparse-array `9-9` path terminates but exposes a separate
Array length error (it remains a failure).

The VM coverage runner now uses an independent `timeout 120` host watchdog on
POSIX and kills its process group at the 60-second per-test deadline. A full
53,872-file venus-001 baseline/VM replay on the repaired binary had zero lost
files, 171 gained files, one lost variant, and 312 gained variants. The sole
lost variant was `staging/sm/TypedArray/sort_sorted.js` strict mode; its two
VM variants also timed out against the previous VM report. This unrelated
typed-array stress test has passed and timed out across complete runs. A
focused replay at 120 seconds passed both variants. The v3 paired host SHA-256
was `31388346`, corpus input SHA-256 `91109b82`; VM compiled
26,784/40,173 eligible files (66.6716%). Linux amazing-js nextest passed 242
tests (one skipped). No orphaned `js-vm` host remained after the full run.

A complete release-host replay of the same 53,872-file corpus at commit
`acbe795b` resolved the timing-sensitive loss. The baseline passed 21,256 files
and 41,451 variants; VM plus fallback passed 21,428 files and 41,765 variants.
The paired comparison lost zero files and variants, gaining 172 files and 314
variants. Compared with the earlier series-1 VM report, the new VM gained eight
files and 16 variants, with zero losses. Both release modes used corpus input
SHA-256 `91109b82` and host SHA-256 `fdadb9e6`. The VM compiled
26,790/40,182 eligible files (66.6716%). Reports are on venus-001 under
`~/amazing/out/jsvm5-orphan-v4-release-{base,vm,compare,prior-vm-compare}`.
The release run left no orphaned js-vm host processes.

## JSVM-6 derived classes

The VM now compiles derived classes end to end: `extends`, `super()` (implicit
and explicit constructors), `super.prop`/`super.method()`, and lexical
`super`/`new.target` inheritance through arrow functions created inside a
constructor or method (matching the tree walker's `EnvKind::Arrow` and
`lexical_super_constructor`/`lexical_new_target`).

Reaching a genuine zero-loss gate required fixing six VM-only correctness
bugs that Test262 could only reach once class-with-`extends` programs stopped
forcing an eligibility fallback -- none of them are semantically about
derived classes, but AGENTS.md's zero-lost-passes gate requires fixing (or
explaining) everything a ticket's own change newly exposes:

- **Constructor backlink.** `FunctionCode::instantiate` never linked an
  ordinary (non-class) VM function's `.prototype.constructor` back to the
  function, so every `new SomeFunction()` -- including Test262's own
  `Test262Error` -- resolved `.constructor` through `Object.prototype`. This
  alone caused the bulk of an initial 1,141-file regression.
- **`class_constructor` flag.** `MakeClass` never set `class_constructor` on
  the produced callable, so `class C {}` called without `new` never threw.
- **Parameter binding vs. `var`/function-declaration hoisting.** The VM bound
  parameters with `declare_lexical`, which stays permanently lexical; a body
  `var x` or `function x(){}` sharing a parameter name then hit
  `hoist_var`'s/`declare_raw`'s "already declared" `SyntaxError` instead of
  the spec no-op. Fixed by mirroring the tree walker's `bind_params` exactly:
  `declare_uninitialized` once per name, `initialize_parameter` per bound
  value (including sloppy duplicates), `finish_parameter_initialization`
  only after every parameter is bound (S10.2.1_A4/A5 Test262 cluster).
- **Member-access leniency asymmetry.** `member_key`'s three call sites
  (plain read, call-target resolution, assignment reference) had all been
  given the same nullish-base strictness; only the assignment/reference path
  is spec-strict (`resolve_reference`), while plain reads and call targets
  are deliberately lenient (`get_prop_receiver`'s `Null|Undefined =>
  Undefined`). Fixed with a `require_coercible: bool` parameter, `false` for
  the first two call sites, `true` only for the reference path.
- **Arrow `this`/`super`/`new.target` inheritance.** `FunctionCode::call`
  used `EnvKind::Function` unconditionally and always called `bind_this`,
  even for arrows -- breaking `(() => super(x))()` inside a derived
  constructor with a spurious "super() has already initialized this". Fixed
  to use `EnvKind::Arrow` (transparent to `initialize_this`/
  `resolve_this_checked`) and skip `bind_this` for arrows entirely, matching
  `interp.rs`'s `FunctionBody::Block`/`Expr` handling exactly.
- **Global identifier assignment strictness.** `ResolveName` resolved `NaN`/
  `undefined`/any non-lexical global binding straight from the env cell
  (`Reference::Binding`), so `NaN = 12` under strict mode silently no-op'd
  instead of throwing. Fixed to check `env.global_binding_object(name)`
  first, matching the tree walker's `resolve_reference`, which routes such
  identifiers through the global *object*'s property (and its strict-mode
  `[[Set]]` failure path) instead.
- **Class-body/module strictness at compile time.** A class's heritage
  expression, computed member keys, and every method/constructor body must
  be strict regardless of the enclosing (possibly sloppy) code; module
  top-level code is always strict too. The VM compiler had no equivalent of
  the tree walker's `eval_class`/`method_value`/`run_program`'s
  `EnvKind::Module` check, so a strict-mode assignment failure inside any of
  these positions silently succeeded instead of throwing. Fixed with new
  `EnterStrict`/`ExitStrict` instructions (bracketing heritage expressions
  and computed keys) plus a save/restore of the compiler's own `strict` flag
  around constructor/class-method compilation, and a new `is_module: bool`
  parameter on `vm::compile` (module code forces `strict = true` at compile
  time, not just at the already-correct runtime `strict_eval_context`).
  `compile_method` is shared with plain object-literal methods, which are
  *not* inherently strict, so it takes an explicit `class_method: bool`
  rather than being unconditionally strict.
- Also fixed in interp.rs (shared, both engines): `make_arrow` hardcoded
  `function_proto` for every arrow regardless of `is_async`, unlike
  `make_function`'s existing is_async/is_generator branching. An async arrow
  now correctly gets `async_function_proto`. This is a real, long-standing,
  engine-wide bug, not new in this ticket; fixing it now (both engines
  identically) turns what had been a VM-only regression into a shared,
  pre-existing, and separately-tracked gap (see `questions.md`: the
  `%AsyncFunction%`/`%GeneratorFunction%`/`%AsyncGeneratorFunction%`
  constructor backlink is still missing).

On pinned Test262 `3655e746` (input SHA-256 `91109b82`, host SHA-256
`8be6f77a`), venus-001 release-host baseline (tree walker) vs. VM+fallback:

| | files pass | files fail | variants pass | variants fail |
|---|---|---|---|---|
| baseline | 33,375 | 19,737 | 64,598 | 37,653 |
| VM+fallback | 33,590 | 19,525 | 65,003 | 37,253 |

**Zero lost files/variants, +215 gained files.** A repeat run in the same
session flagged one *timeout* (not a correctness loss) on
`staging/sm/TypedArray/element-setting-converts-using-ToNumber.js` that did
not reproduce on the immediately following run; treated as host-load flake,
not a regression (both runs otherwise agree on every classification).
Workspace `cargo nextest run --workspace --locked --no-fail-fast`: 3,024
tests run, 3,024 passed, 12 skipped, 0 failed. `cargo test --workspace
--locked --doc`: all green (amazing-js/amazing-html carry the workspace's only
real doctests; every other crate reports 0 doctests, as expected).

## JSVM-7 generators and async generators in the VM

Removed `compile_function`'s outright rejection of `is_async && is_generator`
and built full async-generator support alongside the sync generators that
already existed from JSVM-5:

- `GeneratorState` gained a `queue: VecDeque<AsyncGeneratorRequest>` (each
  request its own promise -- an async generator's consumer can call
  `.next()`/`.return()`/`.throw()` again before a prior call's promise has
  settled, unlike a plain async function's single backing promise or a plain
  generator's synchronous one-call-in-flight-by-construction) and an
  `awaiting: bool` (true only while the one underlying VM frame is suspended
  at `await`, as opposed to idle at `yield`; `executing` alone cannot tell
  the two apart, since it is false in both cases between microtasks --
  without this a newly queued request could be fed into a frame that was
  actually still waiting on its own await's promise reaction).
- New driver: `async_generator_drain` (pops queued requests, short-circuits
  the same two frame-less cases `generator_resume` does -- a `.return`/
  `.throw()` before the body ever ran, and any call once the body already
  completed) and `async_generator_run` (resumes the frame once, mirroring
  `generator_resume`'s `yield_dst`/delegate handling and `async_continue`'s
  `await_dst` handling in the same function, since a body can now suspend at
  either in any order; settles the current request's promise and continues
  draining, or chains an `await` continuation via
  `async_generator_step_handler` -- like `async_handler` but also carrying
  the specific request's own promise alongside the shared state, since
  `async_handler`'s single-slot design assumes one promise per call).
- New native methods `async_generator_next/return/throw` (each enqueues and
  drains) and `async_generator_async_iterator` (returns `this`, mirroring
  `generator_iterator`).
- `FunctionCode::instantiate`'s per-function `.prototype` construction was
  gated `!is_arrow && !is_async && !is_method`, unconditionally excluding
  *every* async function including async generators; narrowed to
  `!(is_async && !is_generator)` so a generator (sync or async) still gets
  its own `.prototype` with the right method set (async ones get Promise-
  returning `next`/`return`/`throw` and `@@asyncIterator` instead of
  `@@iterator`), while a plain async function/method still correctly gets
  none.
- New bridge API for js-runtime (mirroring the existing async-function
  bridge, `compile_async_body`/`call_async_with_env`): `vm::compile_generator_body`/
  `vm::compile_async_generator_body(body: &ast::Block, strict: bool) ->
  Result<FunctionCode, CompileError>` and
  `FunctionCode::start_generator_with_env(&self, i: &Interpreter, env:
  EnvHandle, generator_proto: Option<Obj>) -> JsValue`. Posted to
  `js-runtime`'s `questions.md`.
- Fixed in the same series, found while isolating the generator/async-generator
  Test262 delta from unrelated noise (both real, pre-existing, both
  reproduced unchanged on base `b640521e` before any JSVM-7 code):
  - The sync generator's `[Symbol.iterator]` was defined under the literal
    string key `"@@iterator"` instead of the real well-known symbol (`methods
    .set_own("@@iterator", ...)`), so `it[Symbol.iterator]` could never find
    it -- fixed to `WellKnownSymbol::Iterator.symbol()` (and
    `AsyncIterator.symbol()` for the new async case).
  - `compile_class` never read `ClassMethod::is_static`, so every `static`
    method/accessor was compiled and installed exactly like an instance one
    -- onto `prototype`, with `prototype` as its `[[HomeObject]]` -- leaving
    the constructor's own static members permanently `undefined`. A class
    with only static methods (no fields/static blocks, both of which already
    force fallback) was fully VM-eligible, so this was live, not masked.
    Added `ClassMethodCode::is_static`; `MakeClass` now installs onto (and
    sets `[[HomeObject]]` to) the constructor for a static member, the
    prototype otherwise.
- Known limitation, left out of scope: `YieldDelegate`'s handler calls the
  delegate iterator's `next`/`throw`/`return` synchronously -- correct for a
  sync iterable delegated to from `yield*` (even inside an async generator),
  wrong for a genuinely async iterable (whose methods return Promises). The
  full async-iterator `yield*` protocol was not implemented.

**The 63-file generator/async-iteration false-pass set (JSRT-14's evidence,
`docs/lanes/js-runtime_class_private_elements_evidence.json`): 0 of the 63
now pass for real.** Verified directly, not estimated -- every one of the 63
still fails identically on both the tree walker and the VM. Root-caused into
two clusters, both pre-existing VM *compiler* gaps, unrelated to generator
suspension logic, that force whole-program fallback to the tree walker
before a VM-compiled generator/async-generator body (this ticket's new code)
ever runs:
- ~36 files (`*/class/dstr/async-gen-meth-static-*.js`): a `static async *`
  method with a destructured/defaulted parameter. `compile_function`/
  `compile_method` accept only `Binding::Ident` params; any pattern or
  default throws `CompileError("parameter pattern")` at VM compile time,
  confirmed directly (`static async *m({a,b} = {}) {...}` throws exactly
  that). This is a parameter-destructuring compiler gap, not specific to
  generators.
- ~27 files (`*-ptrn-rest-ary-empty.js`, spread across `let`/`const`/`var`/
  `for`/`for-of` bindings and destructured function/method/arrow
  parameters): an array pattern whose
  sole element is a rest element (`[...x]`, `[...[]]`) throws
  `CompileError("bytecode unsupported: binding pattern")`, confirmed
  directly against the exact repro in
  `language/statements/let/dstr/ary-ptrn-rest-ary-empty.js`
  (`let [...[]] = someGenerator();`). `eligibility.rs` does not flag this
  construct (its own recursive check bottoms out on an empty nested pattern
  and reports eligible), but the compiler's own binding-pattern lowering
  does not implement it, so the whole statement falls back atomically.
  Unrelated to generator suspension; the generator here is only used as an
  arbitrary iterable to exercise the rest-binding path.

Closing either gap (parameter destructuring; array/object rest-binding
patterns) is a separate, sizable VM-compiler feature, not a generator fix,
and was left out of this ticket's scope rather than attempted as a drive-by.

## JSVM-8: generators and async generators in the *default* execution path

Ticket: make `function*`/`async function*` work when called under the tree
walker (the default backend), not just when the VM compiles them (JSVM-7).
Per JSB-11's finding, calling a generator function under the tree walker
returned `undefined` and `yield` threw `"generators are not implemented"` --
`call_value_with_new_target_inner`'s `FunctionBody::Block` arm never
consulted `fun.is_generator`.

Investigation phase (design note, `docs/lanes/js-vm-8-design.md`) compared
(a) a per-function VM-compile fast path (mirroring `async_body_code`) against
(b) porting lumen's pooled-OS-thread stackful coroutine
(`lucid-softworks/lumen`, `crates/lumen/src/coroutine.rs`, MIT, `f6c9860d`,
per ADR 0013) and recommended the hybrid lumen itself ships (both, not
either/or). The coordinator approved the hybrid but scoped this ticket to
**(b) only**: the VM fast path is a separate follow-up.

### What landed

New `crates/amazing-js/src/interp_coroutine.rs` (attributed per ADR 0013,
row appended to `THIRD_PARTY_NOTICES.md`): a pooled-worker/`mpsc`
ping-pong-handoff coroutine, adapted onto this crate's own
`Interpreter`/`JsValue`/`Throw` types. `unsafe impl Send` for the
ping-pong payloads and the raw `*mut Interpreter` shuttled across the
worker-thread boundary is sound for the same reason as lumen's own: the
ping-pong protocol guarantees exactly one side runs at any instant, never
concurrently. Workers are pooled (`IDLE: Mutex<Vec<Sender<CoroutineJob>>>`)
to amortize spawn cost. Not reproduced: lumen's `Vm` coroutine variant
(that's the deferred VM-fast-path follow-up), its frame-ownership panic
recovery (no per-call frame stack to tag here), and `Suspend::Await` (an
`await` inside a coroutine body still uses the pre-existing synchronous-
degrade fallback, unchanged, out of this ticket's scope).

Hooked into `interp_stmt.rs` in five small places: `FunctionBody::Block`'s
call path defers a generator/async-generator body to a coroutine instead of
running it eagerly (right after `bind_params`+`hoist`, matching where
`async_body_code`'s bridge sits for plain async functions); `ExprKind::Yield`
now drives the coroutine (`eval_yield`) instead of throwing; `exec_try`'s
catch dispatch gained one arm so a smuggled `.return(v)` completion at a
suspended `yield` bypasses `catch` but still runs `finally` (real JS
`return` semantics, distinguished from a real throw via a `PENDING_RETURN`
thread-local); `method_value` gained the same per-method `.prototype`
construction generator methods need that `make_function` already had; and
the post-match async-wrapping code's condition narrowed from `fun.is_async`
to `fun.is_async && !fun.is_generator` (a genuine pre-existing bug found
along the way: it unconditionally wrapped every async function's return
value in a Promise, double-wrapping an async generator's own instance --
spec 27.4.3.2 says that call returns the generator object itself,
synchronously, not a promise).

### A regression found and fixed before the final gate

The crate's own differential test
`compiled_method_and_generator_boundaries_preserve_eval_strictness_and_names`
(`vm/tests.rs`) broke: `var o={*m(p=eval('var arguments')){let arguments;}};
try { o.m(); false; } catch(e) { e instanceof SyntaxError; }` stopped
throwing. Root cause: this tree walker fuses top-level lexical-name
declaration with statement *execution* (`exec_stmts` declares a `let`/
`const`/class binding only when it reaches that statement -- there is no
separate FunctionDeclarationInstantiation pre-pass, even for ordinary
functions; it "works" for them only because their body runs synchronously
immediately anyway). Deferring a generator body's execution to a coroutine
silently deferred this redeclaration conflict check too, past the call and
into the first `.next()` -- but spec (10.2.11/27.5.3) has FDI itself run
synchronously at call time even for a generator; only the body's own
statements suspend. Fixed with a new read-only, no-side-effect check,
`Env::would_conflict_as_lexical` (`env.rs`), run over the body's top-level
lexical names (`block_lexical_names`, already existed for Annex B.3.3) right
after `hoist()`, before spawning the coroutine -- if any name would collide,
throw immediately; otherwise do nothing and let the real declaration happen
later exactly as before.

### Measured numbers

Rebased onto `dev`'s `079f2030` (js-lang's JS-11..17 merge) per the
coordinator's mid-ticket instruction, so the baseline already includes that
merge's passes. Directory-scoped Test262 gate, `--timeout 10`, before
(`079f2030`) vs. after (this ticket's head), on `built-ins/Iterator`,
`built-ins/GeneratorPrototype`, `built-ins/AsyncGeneratorPrototype`,
`language/statements/generators`, `language/expressions/generators`,
`language/expressions/yield` (pinned Test262 `3655e746`, venus-001):

| | files pass | files fail |
|---|---|---|
| before (`079f2030`) | 510 | 144 |
| after (this ticket) | 518 | 136 |

**0 lost / 8 gained files.** (An earlier in-progress measurement against a
now-stale, pre-rebase base had shown 3 apparently-lost files --
`language/expressions/generators/named-strict-error-reassign-fn-name-in-body*`
-- traced to a *different*, pre-existing, generator-unrelated bug: this
interpreter's named function/generator expressions never bind their own
name as an immutable self-reference (`make_function` has no
`NewDeclarativeEnvironment`+`CreateImmutableBinding` step per spec 15.2.5),
confirmed via a direct Rust-level probe reproducing the identical no-throw
behavior for a *plain*, non-generator, synchronously-executed named function
expression. Against the correct `079f2030` baseline the loss does not
appear at all -- js-lang's concurrent JS-11..17 merge already closed it
independently.)

Crate: `cargo nextest run -p amazing-js --locked`: **441 tests run, 441
passed, 1 skipped, 0 failed** (includes 14 new
`thread_generator_tests`). `cargo test -p amazing-js --locked --doc`: 1
passed.

Workspace: `cargo nextest run --workspace --locked`: **3,443 tests run,
3,443 passed (4 slow), 17 skipped, 0 failed.** `cargo test --workspace
--locked --doc`: all green (4 crates with real doctests, each 1 passed).

No leaked-thread/hang issue observed: the after-run Test262 host completed
in 15.7s (before-run: 14.1s), no timeouts beyond the expected `--timeout 10`
per-test budget, no kill required -- consistent with the pooled coroutine
workers' park-forever-on-teardown design not blocking a per-file host
process's own exit.

### Commit

Landed as `9922d5f0` on `lane/js-vm-coverage`, base `079f2030`.
`.agents/js-vm/status.json` records `state: done`.

### Cross-lane notes (also in `questions.md`)

- The VM fast path (option a from the design note) is an explicit follow-up,
  not done here -- only the pooled-thread fallback shipped.
- `await` inside a coroutine-backed generator body still degrades to the
  pre-existing synchronous fallback rather than truly suspending across a
  microtask; flagged as a known, documented limitation, not attempted here.
- Named function/generator expressions' missing immutable self-binding
  (`make_function`, `interp_stmt.rs`) is flagged for whichever lane owns
  general function-expression semantics next -- confirmed pre-existing and
  generator-unrelated, not fixed here (out of this ticket's narrow scope),
  and no longer visible as a loss against the current baseline regardless.

On pinned Test262 `3655e746`, venus-001 release-host, this lane's own tree
vs. VM+fallback pair on **the same commit** (the rigorous comparison; the
fleet's `.agents/test262/baseline.json` is a separate, older snapshot and
disagrees with a fresh tree-walker run on ~34,000 unrelated files that
predate this ticket -- comparing against it is noise, not signal, for
isolating this ticket's effect):

| | files pass | files fail | variants pass | variants fail |
|---|---|---|---|---|
| tree walker (this session) | 37,218 | 15,896 | 72,223 | 30,031 |
| VM+fallback (this session) | 37,506 | 15,609 | 72,763 | 29,491 |

**2 lost / 290 gained files**, both losses diagnosed as not caused by this
ticket: `built-ins/Function/prototype/toString/built-in-function-object.js`
(a `timeout`, not a correctness mismatch -- host-load flake under a shared
build slot, the same pattern JSVM-6 reported for a different file) and
`staging/sm/Proxy/global-receiver.js` (a genuine pre-existing VM bug,
reproduced identically before and after every JSVM-7 change, just previously
buried in a much larger loss list before the static-method fix landed --
unrelated to generators, flagging in `questions.md` for whichever lane picks
up Proxy/global-receiver semantics next). Cross-checked against
`baseline.json`: 92 files show as "lost" there, but 90 of them already fail
identically on this session's own fresh tree-walker run (baseline staleness,
not a VM regression), and the remaining 2 are the exact same two files
above. Repeated the tree/VM pair before and after the static-method fix
(242 lost -> 2 lost) to confirm the fix, not flakiness, closed that cluster.

Workspace `cargo nextest run --workspace --locked --no-fail-fast`: **3,288
tests run, 3,288 passed (2 slow), 15 skipped, 0 failed.** `cargo test
--workspace --locked --doc`: all green (amazing_js 1 passed, amazing_html 1
passed, every other crate 0 as expected).

## JSVM-9: measuring and closing the generator/async gap at full scale

Base: dev `196439b5` (JS-18 cluster 1). The lane branch was rebased onto it mid-ticket; the
series is `765fc876..d9a7b16c`.

### Why JSVM-8 only gained 8 files

- **The timeout was not the cause.** Nine generator-heavy directories at `--timeout 60`
  (`statements|expressions/generators`, `statements|expressions/async-generator`,
  `statements/class/elements`, `GeneratorPrototype`, `AsyncGeneratorPrototype`, `Iterator`,
  `for-await-of`) went from 4200 passing files at `079f2030` to 4278 at `d4c0da6e` (+78, 0 lost).
  Neither run had a single timeout.
- **Most generators never reach the coroutine.** `make_function` and `method_value` compile every
  generator function or method body they can to per-function bytecode (`vm::compile_function_item`
  and `compile_class_method`), whether or not `AMAZING_JS_VM` is set. Only bodies the compiler
  rejects fall back to `FunctionBody::Block` and the JSVM-8 coroutine; before JSVM-8 there were 56
  of those in the nine directories ("generators are not implemented"). The fallback itself works:
  a `CompileError` leaves the function an ordinary `Block` body, which the coroutine hook catches.
- **The failures triage counted as "generator failures" were bugs in the VM's own async generator
  and in shared prototypes.** None of them was a missing coroutine:
  - `yield*` in an async generator used the sync iterator protocol and never awaited.
  - `yield` and `return` did not await their operand.
  - `return()` resumptions did not await.
  - Each VM generator got a private stand-in for `%GeneratorPrototype%` and
    `%AsyncGeneratorPrototype%`.
  - There was no `%AsyncIteratorPrototype%`.
  - Rejections carried un-normalized engine errors.
  - Async functions the VM does not compile ran synchronously, with `await` of a pending promise
    evaluating to `undefined`.

  In the whole `language/` directory, `class` statements and expressions (739 failures at base)
  and async-generator (191 failures) were mostly these clusters.

### What landed

| Commit | Change | Cluster closed |
|---|---|---|
| `765fc876` | VM async generators: `yield v`/`return v` await (compiler emits `Await`); a `return(v)` resumption at a yield awaits `v` first (AsyncGeneratorUnwrapYieldResumption, `VmFrame::await_return`); new `YieldDelegateAsync` instruction backed by `vm/async_delegate.rs` (GetIterator async with CreateAsyncFromSyncIterator fallback, every inner result awaited, AsyncIteratorClose when `throw` is missing); `return()` on a not-started or completed generator awaits (AsyncGeneratorAwaitReturn); Await's PromiseResolve reads `constructor`. VM generators inherit the realm's shared `%GeneratorPrototype%`/`%AsyncGeneratorPrototype%`, whose `next`/`return`/`throw` dispatch to either backend with brand checks (async ones reject instead of throwing); `@@toStringTag`, `constructor`, `%GeneratorPrototype%` → `%IteratorPrototype%`, and a real `%AsyncIteratorPrototype%` | yield* protocol (~450 files across the async-generator, class async-gen-method and private-method dirs), Iterator helpers on generators (~140), GeneratorFunction and AsyncGeneratorFunction `instance-prototype`/`prototype/prototype`/`proto-from-ctor-realm-prototype` |
| `6b61110c` | Normalize engine-internal errors before rejecting async function and generator promises | `reason.constructor === TypeError` checks (~200) |
| `624b8c78` | Async functions the VM does not compile run on the coroutine: they execute synchronously until their first `await`, park with `Suspend::Await`, and resume from the awaited value's reaction; completion settles through `vm::async_complete`, so thenables are adopted. `for-await-of` in such a body parks the same way. Every coroutine body runs in segments that re-establish the function's own frame (`this`/`new.target`/`super`/home) and strictness on each resumption. Port of lumen `drive_async`/`coroutine_await` @ `f6c9860d`, MIT (THIRD_PARTY_NOTICES row appended) | Private and static async methods returning or awaiting promises (class/elements), and async functions with pending awaits |
| `d2c5830d` | `for-await-of` awaits the value only for a sync iterator | Tick order (`await/for-await-of-interleaved.js`) |
| `d9a7b16c` | Idle coroutine workers are capped at 16 | Thread retention after bursts (below) |

### Measured numbers (venus-001, `--timeout 60`)

- **Nine directories, `079f2030` → `d9a7b16c`:** 4200 → 4849 files passing, **0 lost, +649**.
  Failures went from 811 to 162.
- **`language/` plus the generator, async-function, Promise and Iterator built-ins directories,
  `196439b5` → `d9a7b16c`:** 22363 → 23391 passing, **+1030, 2 lost**. `language/` alone went
  from 20835 to 21695 (failures 2629 → 1769).
  - Both losses were false passes that are now exposed:
    `statements/await-using/syntax/await-using-invalid-assignment-next-expression-for.js` and
    `statements/for-of/head-await-using-bound-names-fordecl-tdz.js`.
  - The inner `for (await using …)` never throws, at base or head; it is a pre-existing tree-walker
    bug.
  - At base, the outer `asyncTest` body could not be compiled either. It ran synchronously, and its
    `await assert.throwsAsync(...)` evaluated a pending promise as `undefined`, so `$DONE` fired
    before the assertion.
- **Tests:** crate nextest 445/445 (1 skipped), crate doctest 1/1. Workspace nextest 3465/3465
  (17 skipped); workspace doctests 4 passed, 0 failed.

### Thread and memory cost of the pooled-OS-thread coroutine

- **Full `language/` run.** Every Test262 file is its own host process. Sampled at 1 s over 292
  samples, no host process had more than 7 threads, and the peak RSS of a single process was
  59 MB (60408 kB). The corpus does not accumulate threads.
- **In-process probes** (`host_final`; the bodies use a private name so they stay off the VM):

  | Probe | Peak threads | Threads at end | Peak RSS |
  |---|---|---|---|
  | 20 000 generators run to completion one after another | 4 | 4 | 14 MB |
  | 5 000 async functions in flight together, each awaiting twice | 5001 | 17 | 193 MB |
  | 2 000 concurrently suspended generators, then drained | 1578 | 17 | 66 MB |
  | 2 000 generators suspended and abandoned | 2001 | — | 70 MB |

  - The sequential case shows the pool reuses workers.
  - In the two concurrent cases, each suspended coroutine held its own OS thread. Before the idle
    cap, all 5000 workers stayed parked after the burst; now 16 do.
  - Stacks reserve 64 MB of address space each but commit only about 35 kB.
- **Known limitation: threads grow with concurrency and leak on abandonment.**
  - The thread count grows with the number of concurrently suspended coroutine bodies, with no
    upper bound.
  - A coroutine that is suspended and never resumed or completed keeps its thread parked for the
    life of the process. When the driver is dropped, `park` refuses to unwind, because unwinding
    would run `finally`/`return()` JS against an interpreter that may be tearing down.
  - This applies only to bodies the bytecode VM cannot compile; VM generators and async functions
    use heap frames and no threads.
  - Real fixes, not done here, would be moving more bodies onto the VM, or unwinding abandoned
    coroutines with JS disabled.
  - If a platform cannot spawn another thread, generators throw `UNSUPPORTED_MSG` and async
    functions fall back to the old synchronous path.

### Remaining clusters (not generator machinery; flagged in `questions.md`)

In the nine directories:

- 46 files: `$DONE called more than once` in `for-await-of` dstr.
- 42 files in the wide run: `invalid assignment target` for an object-rest assignment pattern in
  `for (… of …)` heads, and no IteratorClose for assignment array patterns. Identical in plain
  `for-of`.
- 80 files: `ary-ptrn-elem-id-iter-val-array-prototype`. Destructuring ignores an overridden
  `Array.prototype[Symbol.iterator]`, for generator and ordinary functions alike.
- Parameter defaults referencing `arguments` throw a ReferenceError in ordinary functions too.
- Static class elements named `prototype` (methods, getters and setters as well as generators) do
  not throw.
- `GeneratorFunction/instance-restricted-properties.js`: `caller`/`arguments` are not restricted.
- Direct-eval private-name, `super` and `new.target` scoping in class elements.
- Async generator bodies on the coroutine path still use the synchronous `await_value`. They are
  rare now that the VM handles async generators correctly; porting lumen's `drive_async_gen`
  queue would close it.

## JSVM-10: shrinking the coroutine fallback

Goal: generator and async bodies that the VM compiler declined fell back to the pooled-OS-thread
coroutine (JSVM-8/9), one parked thread per concurrently suspended body. JSVM-10 measured why bodies
were declined and compiled the largest clusters instead. All code is original to this crate; nothing
was ported (no upstream construct mapped cleanly onto this register VM's `Instruction` set).

### How it was measured

A temporary, env-gated hook (`JSVM10_FALLBACK_LOG`, removed in the final commit) logged one line
per generator/async body compile attempt (`make_function`, `method_value`, `async_body_code`) with
`ok` or the `CompileError` message. Each construct group was measured on `language/` + `built-ins/`
of the pinned Test262, before and after, together with a zero-lost pass comparison.

### What landed (per commit series)

| Commit | Group | Fallbacks | Test262 |
|---|---|---|---|
| `71473758` (1) | break/continue (`ExitTo` + `Abrupt::Goto` through try/finally, `TryHandler::start`), switch, for-in (`ForInStart/Next` over the tree walker's `ForInState`), for-of (`IteratorStart`), labels | not measured on its own; the 2729 baseline below is taken after (0) and (1) | 0 lost |
| `59b19e9d` (0) | `Array.fromAsync` / `disposeAsync` rejections go through `normalize_throw` (4 coordinator-flagged false passes turned real) | — | fromAsync dir 0 lost |
| `05357c69` (2) | private members: `GetPrivateField`, `ResolvePrivate`, write-only `ResolvePrivateWrite` (setter-only accessors), `HasPrivateField` (`#x in o`) | 2729 → 995 ("private member" 1728 → 0) | language/ 0 lost |
| `5f6924ea` (5) | regression fix: for-heads whose pattern may `yield`/`await`, and every `using`/`await using`, stay on the coroutine (`BindFor` binds through the tree walker, which cannot suspend; the VM has no disposal) | — | for-of +15, for-await-of +12, await-using +31, staging/explicit-resource-management +17, 0 lost (the 19 files dev 61f76417 lost) |
| `ea94e5b7` (3) | `TreeEval`: an expression the compiler cannot lower and that contains no `yield`/`await` is evaluated by the tree walker's `eval_expr` against the frame's env; optional chains from their outermost link; spread/`super`/`#x` never alone | 1267 → 636 | 0 lost |
| `b5addca4` (4) | `for (let/const;;)`: head scope + `CopyScope` per-iteration `let` copies (CreatePerIterationEnvironment); `const` not copied | 636 → 538 | +1, 0 lost |
| `bb8c1a7d` (6) | body-level direct eval through `TreeEval` (parameter-default eval stays excluded) | 538 → 525 | 0 lost |

(The 1267 baseline for (3) is after (5), which deliberately moved some bodies back to the coroutine.)

Overall, from the first comparable measurement (`59b19e9d`): **2729 → 525** fallbacks of ~24.2k generator/async body compile attempts (11.3% → 2.2%).

### Thread cost after JSVM-10 (in-process probe, venus-003)

| Probe (5000 async bodies in flight, each awaiting twice) | Threads in flight | Threads after | Note |
|---|---|---|---|
| bodies using a private name (the JSVM-9 probe shape) | 2 | 2 | JSVM-9: 5001 threads, 193 MB; now VM heap frames |
| bodies with a `using` declaration (still declined) | 5002 | 18 | unchanged mechanism: one thread per suspended fallback body |

### Remaining fallbacks (525)

- `using declaration` 164 and `suspending for binding` 74: deliberate (disposal / tree-walker binding
  cannot suspend). A VM-native destructuring binder and disposal would remove both.
- `direct eval` 128: eval inside parameter initializers (parameters run in the tree walker).
- `spread` 56 and `assignment target` 48: destructuring *assignment* whose pattern itself contains
  `yield`/`await` (`[...x[yield]] = …`). Needs VM-native destructuring assignment.
- Tail: `expression kind` 14, `yield` 10, `statement kind` 10, `lexical for binding` 10 (`using`
  heads), `class field or static block` 8, `member access` 2, `binding pattern` 1.

### Found along the way

- Tree walker: `switch` ran case bodies in the caller's scope (a `let` in a case clashed with one
  after the switch) and swallowed labeled `break`s; both fixed in (1).
- Tree walker: `loop_for_inner` copies `const` for-heads as mutable, so `for (const c = 1;; c++)`
  loops forever (js-lang fixed it in `a4e206fe`; flagged in `questions.md`).

### Final gate (venus-003, head `135efe46` on dev `2635118e`)

- Full pinned Test262 (all 53578 files, `--timeout 20`): dev `2635118e` 48927 → head 48929 pass,
  **0 lost**, 2 gained.
- Workspace nextest 3493/3493 (17 skipped); workspace doctests 4 passed, 0 failed (31 crates).
- The measurement hook is removed (`135efe46`); `interp_async.rs` and `interp_coroutine.rs` are
  byte-identical to their JSVM-9 versions.

## JSVM-11: resumable destructuring and explicit resource management

Fixed base: `2eb1229a`. No shared interpreter files or dependency changes; the
implementation adapts the VM's own instructions and reuses its existing
interpreter value/property/iterator helpers. No upstream code was copied.

- Declaration and assignment patterns lower recursively into instructions.
  Pattern iterators live in the heap frame; default expressions, computed keys
  and assignment targets can suspend without an OS-thread coroutine. Protected
  finally regions close partially consumed iterators on return/throw and preserve
  an original throw over an IteratorClose error. Rest consumes remaining values;
  object rest records converted excluded keys, including symbols.
  Assignment references resolve before the iterator/property read; ordinary
  assignment's legacy function-name inference is excluded from pattern stores.
  Elisions step without observing the result's value getter.
- Suspending loop heads bind in fresh lexical scopes. Lexical right-hand sides
  use temporary TDZ environments, and native patterns reserve all their bound
  names before initialization. Catch patterns use the same native binder.
- Using declarations capture the method at declaration time into frame-owned
  resource slots. Scope exit, break, continue, return and throw route through the
  existing VM finally machinery and dispose in reverse declaration order. Loop
  resource heads dispose per iteration; C-style for-head resources dispose when
  the loop exits; switch cases share a resource scope.
- Async disposal awaits on the VM frame and records rejection before proceeding
  to older resources. Multiple errors form SuppressedError chains in disposal
  order. The async hint is kept independently of the resolved method: synchronous
  fallbacks ignore their result, nullish async resources coalesce the required
  await, and noncallable disposal methods reject at capture time. A synchronous
  fallback that throws produces an awaited rejection before older resources run.
- `AMAZING_JS_VM_COMPILE_LOG=<path>` optionally records actual runtime async and
  generator compilation attempts, one append-only line (`ok` or rejection
  reason) per attempt. It is disabled by default; Test262 subprocesses can share
  one log to rank the remaining coroutine fallback clusters.

### Measured gate (Boat, fixed base `2eb1229a`)

- Workspace nextest **3518/3518 passed**, 17 skipped; workspace doctests **4 passed**.
- Full pinned Test262, 53872 files, at 10 seconds: **49701 passed** versus base
  49679, 28 gained and six deadline-only losses. Rechecking the whole URI and
  DST clusters at 60 seconds recovered all six and four additional deadline
  passes. Combined classifications: **49711 passed, 32 gained, zero lost**;
  28 gains are native semantics and four are deadline recovery.
- The last correction translates a throwing synchronous disposal fallback into
  an awaited rejection. After it, workspace/doctests and the whole `language/`
  selection (24572 nonfixture files, 23304 passed) plus explicit-resource-management
  staging (53 files, 51 passed) passed their zero-loss comparisons. Every affected
  classification matched the full run taken before that correction.
- Runtime generator/async compile attempts: **27838 compiled / 28067 attempts**;
  **229 fallbacks** (historical JSVM-10: 525). Remaining reasons: parameter direct
  eval 128, spread 56, expression kind 14, yield 11, statement kind 10, class
  field/static block 8, member access 2. Using, suspending loop binding,
  assignment-target and binding-pattern rejection categories are gone.
- 5000 suspended async bodies with `using` completed in **0.101 seconds**, peak
  **one thread**, 73592 KiB RSS. JSVM-10's same using-body shape retained 5002
  threads. This probe uses the ordinary runtime backend, with no forced VM flag.
- Full Unicode `regExpUtils.js` buildString: tree walker completes in **88.538s**;
  forced VM compilation reaches its instruction limit in **30.886s**. Ordinary
  VM adoption needs a fuel/performance follow-up before claiming Unicode gains.

Compact durable evidence: [js-vm11_evidence.json](js-vm11_evidence.json). Parameter
initialization remains on the tree walker; direct eval inside parameter
initializers remains a deliberate compiler fallback. Plain identifier loop heads
retain the existing single-step binder; all declaration and assignment patterns
use the native lowering.

Next parameter-eval follow-up: preserve call-time generator
FunctionDeclarationInstantiation before removing the parameter probe's direct
eval guard. The tree-walker path eagerly hoists and checks body lexical names
against bindings introduced by parameter eval. The VM currently prepares its
frame at the first resume, so merely accepting those defaults would postpone a
required redeclaration SyntaxError until `.next()`. Carry the lexical-name check
and eager environment preparation in compiled-function metadata first.

Specification references: [destructuring assignment](https://tc39.es/ecma262/multipage/ecmascript-language-expressions.html#sec-destructuring-assignment)
and [DisposeResources](https://tc39.es/ecma262/multipage/abstract-operations.html#sec-disposeresources).

## JSVM-12: eager generator initialization and parameter eval

Fixed base: JSVM-11 `4bc63427`. Compiled functions retain their own body lexical
names. A generator call binds parameters, eagerly creates its frame and hoists
its declarations, then validates lexical conflicts before returning its iterator.
Only body instructions remain deferred. First resume reuses that frame; return
or throw before first resume discards it without executing the body.

The parameter compiler probe now accepts direct eval through the existing
evaluator path. Parameter binding itself still uses the same interpreter helper.
No shared interpreter files changed and no upstream code was copied. The
existing `block_lexical_names` helper supplies metadata without duplicating its
ported implementation.

Regressions cover synchronous parameter-eval conflicts
for sync/async generators; inspection of eagerly hoisted var/function bindings
before first resume; suppression of unnecessary arguments objects; and pre-start
return/throw never executing the eagerly prepared body.

The existing shared evaluator conflates parameter and body variable environments;
`scope-paramsbody-var-open.js` already fails. This series preserves that boundary
and its existing conflict behavior. Separating those environments requires a
runtime follow-up coordinated with the shared regression expectations.

### Measured gate (Boat, fixed base `4bc63427`)

- Final workspace nextest **3519/3519 passed**, 17 skipped; workspace doctests
  **4 passed**. All source and target directories used native `/tmp` storage.
- Full pinned Test262 at 10 seconds: **49700 passed**. Eleven deadline losses
  against JSVM-11's combined base were rechecked in whole URI, DST and staging
  TypedArray clusters at 60 seconds; all eleven recovered. Combined result:
  **49711 passed, zero gained, zero lost** against JSVM-11.
- After the last arguments-object metadata correction, the whole language
  selection (23304/24572), Generator (84/84), AsyncGenerator (71/71), Function
  (501/515) and eval (8/10) selections had classifications identical to both
  the full run and the prior ticket base. This ticket improves VM coverage
  without claiming new semantic passes.
- Runtime compile attempts: **27953 compiled / 28054 attempts**, **101 fallbacks**.
  Parameter direct eval fell from **128 to zero**. Other reasons remain spread
  56, expression kind 14, yield 11, statement kind 10, class field/static block
  8, member access 2. Counts come from the broad run before the final metadata
  correction, which does not change compiler acceptance.

Compact durable evidence: [js-vm12_evidence.json](js-vm12_evidence.json).

## JSVM-13: shared resolving pairs for VM async completion

Fixed base `7e3c07ba`. VM async completion and await wrapping now invoke the
runtime's JSRT-28 resolving pair. A returned native Promise still has its `then`
property read; thenables resolving to a pending Promise keep their first
resolution when they subsequently reject or throw. Duplicate VM thenable jobs
and target-only callback gates are removed. Await's native Promise reuse checks
the exact intrinsic constructor, rather than a different constructor's prototype.

The shared helper is called directly; no upstream code is copied and no shared
interpreter files are edited. Regressions exercise async return/await and async
generator yield/return, throwing then accessors, and alternate constructors with
the intrinsic Promise prototype.

### Measured gate (Boat, fixed base `7e3c07ba`)

- Final workspace nextest **3590/3590 passed**, 17 skipped; workspace doctests
  **4 passed**. All source/target directories used native `/tmp` storage.
- Before/after whole affected Test262 selections, 20s and eight jobs: language
  **23346 to 23348 / 24572**, Promise **520/732** unchanged, Generator **84/84**
  unchanged, AsyncGenerator **71/71** unchanged. **Two gained, zero lost** files:
  async-generator `yield-thenable-create-resolving-functions-reject.js` and
  `yield-thenable-create-resolving-functions-resolve.js`.
- No full Unicode corpus rerun was needed for this shared resolver consumer;
  the full affected language selection includes async functions, generators,
  await, modules and disposal boundaries. Baseline runs used exact `--ref`.

Compact durable evidence: [js-vm13_evidence.json](js-vm13_evidence.json).
The next runtime-coordinated follow-up replaces observable native Promise `then`
access in await drivers with internal PerformPromiseThen reaction attachment.
This series preserves that existing driver boundary.

Specification references: [CreateResolvingFunctions](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-createresolvingfunctions),
[NewPromiseResolveThenableJob](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-newpromiseresolvethenablejob)
and [PromiseResolve](https://tc39.es/ecma262/multipage/control-abstraction-objects.html#sec-promise-resolve).

## JSVM-14: atomic class declaration lowering and rejection context

Fixed base JSVM-13 `9785e236`. A class declaration now lowers its value through
the same expression path as a named class expression before initializing its
outer lexical binding. Fields, static blocks and private elements can delegate
atomically when heritage/computed keys do not suspend; suspended class evaluation
continues to fall back as a whole coroutine. No new interpreter implementation
or copied upstream code is introduced.

`AMAZING_JS_VM_REJECT_CONTEXT=<path>` optionally appends rejected parameter/body
AST context independently of the existing compact compile-acceptance log.
Diagnostics are disabled by default.

### Measured gate (Boat, fixed base `9785e236`)

- Workspace nextest **3591/3591 passed**, 17 skipped; workspace doctests **4 passed**.
- Before/after 25151 whole affected Test262 files, 20s/eight jobs: language
  **23348/24572**, Function **501/515**, AsyncFunction **18/18**, GeneratorFunction
  **23/23**, AsyncGeneratorFunction **23/23**; **zero gained, zero lost**, every
  classification unchanged. New explicit-bytecode regressions verify declaration
  coverage independently of existing coroutine fallback passes.
- Compile attempts **22382/22482 accepted**, **100 fallbacks**, unchanged. The
  eight class-field/static-block rejections all include suspending computed keys;
  those deliberately retain whole-coroutine fallback. This series does not claim
  a measured rejection reduction.
- The 100 rejection contexts identify the largest remaining shape: object spread
  carries `ObjElem::Spread(ExprKind::Spread(inner))`; native object-spread lowering
  mistakenly passes the wrapper to `expression()`. Suspending inner expressions
  then prevent whole-object delegation, accounting for 56 spread rejections.
  Normalize the wrapper before lowering in the next owned compiler series.

Compact durable evidence: [js-vm14_evidence.json](js-vm14_evidence.json).
