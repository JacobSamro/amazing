# JSVM-8 design note: generators/async-generators in the default execution path

Status: **investigation complete, implementation blocked on the js-lang merge landing in
`dev`** (per coordinator instruction -- no VM code or rebase yet). This note picks the
approach and records the evidence behind it, for the agent (me, next session, or a
handoff) that implements it once unblocked.

## Problem, precisely

`crates/amazing-js/src/interp.rs`, `call_value_with_new_target_inner`'s
`FunctionBody::Block(b)` arm (~line 2723) runs `self.exec_stmts(&b.body, &e)` directly for
every non-async block function, generator or not -- `fun.is_generator` is never consulted
there. So calling a generator function under the tree walker just executes the body
start-to-finish and returns whatever `Flow::Return` produced (`Undefined` if none), instead
of constructing a Generator object. Any `yield` reached along the way hits
`ExprKind::Yield => Err(Throw::type_error("generators are not implemented"))` at
`interp.rs:1934`. This is the *only* thing wrong: `make_function`/`method_value` already
set `is_generator` and the right `%GeneratorFunction.prototype%` /
`%AsyncGeneratorFunction.prototype%` correctly (interp.rs ~2916-2937, ~2987-2996); the VM's
`instantiate()` (JSVM-7) already builds a spec-correct per-function `.prototype` with
`next`/`return`/`throw`/`Symbol.(async)iterator` when a function *is* compiled by the VM.
The gap is purely "the tree walker's call path for `FunctionBody::Block` never branches on
`is_generator`," confirming js-binary's JSB-11 finding exactly (note: the throw site is
`interp.rs:1934`, not `interp_stmt.rs:863` as JSB-11 approximated -- there is no
`interp_stmt.rs` file in this tree; `interp.rs` is the single file that owns both the call
dispatch and the `Yield` expression handling).

Plain async functions (non-generator) already avoid an equivalent gap today, via
`interp_async.rs`'s `async_body_code`: on every call it tries
`vm::compile_async_body(&body, strict)`, caches the `Result` on the function object
(`object.borrow_mut().async_body_code`), and on success routes the call through
`code.call_async_with_env(self, e)` instead of the tree walker's own statement executor.
Generators and async generators are explicitly excluded from that bridge today
(`if !function.is_async || function.is_generator { return None; }`) -- they have no
bridge at all, sync or async.

## Options compared

**(a) Per-function VM hybrid.** Add a `generator_body_code`-style cache mirroring
`async_body_code` exactly: on a generator call, try `vm::compile_generator_body`/
`compile_async_generator_body` (both already exist, built in JSVM-7) on just that
function's own AST body, cache the `Result` on the function object, and on success call
`FunctionCode::start_generator_with_env` (also already built in JSVM-7) to construct the
Generator object bound to the already-built call environment `e`. This needs no whole-program
eligibility check (`vm_eligibility::eligible`) at all -- it compiles one function body in
isolation, exactly like the async bridge already does, so it is unaffected by what
surrounds the generator in the rest of the (possibly VM-ineligible) program. Cost: only
covers generator bodies the VM compiler can already compile. Confirmed VM-compiler gaps
that would still fail here (found during JSVM-7, direct `CompileError` repros): destructured/
defaulted parameters (`CompileError("bytecode unsupported: parameter pattern")`), array/object
rest-binding patterns anywhere in the body (`"bytecode unsupported: binding pattern"`), and
`for...of` statements (`"bytecode unsupported: for-of"`). Per JSVM-7's own 63-file
investigation, these two gaps alone account for 100% of that set's remaining failures, so
they are common in real Test262 generator bodies, not edge cases.

**(b) Port lumen's stackful-coroutine mechanism** (`~/scratch/lumen/crates/lumen/src/
coroutine.rs`, MIT, pinned `f6c9860d`, read in full this session -- 341 lines). Mechanism:
a pooled OS-thread worker runs the *tree-walking* body; `yield`/`await` inside it calls
`park()`, which sends a `Suspend::{Yield,Await,Done,Throw}` over an `mpsc` channel to the
driver and blocks in `recv()` on a second channel for the next `Resume::{Next,Return,Throw}`.
The handoff is strictly ping-pong (exactly one side runs at a time), which is what makes it
sound to (a) send non-`Send` `Value`/`Resume`/`Suspend` payloads across the channel via
`unsafe impl Send`, and (b) shuttle a raw `*mut Interp` into the worker thread
(`InterpPtr`, `unsafe impl Send`) and dereference it there -- the driver provably isn't
touching the interpreter while the worker has the pointer, and vice versa. Workers are
pooled (spawn cost ~30us dominated async/generator overhead; pooling drops the recurring
cost to ~6us/suspend) and carry a `cur_coro` frame-ownership tag so a panicking worker's
stack frames (which interleave with the driver's own frames across suspensions) can be
evicted by id rather than by a watermark truncate. A thread-local `YIELDER` lets a `yield`
buried arbitrarily deep in eval find the right channel without threading it through every
call. Amazing's `Interpreter` (`interp.rs:223`) is architecturally the same shape lumen's
`Interp` is for this purpose: built entirely from `Obj` (`Rc<RefCell<JsObject>>`), `Weak`,
and other `Rc`-based non-`Send`/non-`Sync` fields -- so the same "strict ping-pong makes a
raw-pointer handoff sound" argument transfers directly; this is not a mechanism that only
works for lumen's specific types.

**The two options are not actually exclusive -- lumen itself runs both.** Its own
`Coroutine` enum is `{ Thread(ThreadCoro), Vm(VmCoro) }`, and its own doc comment says
plainly: "Either an OS-thread-backed coroutine (generators, and async bodies the bytecode
compiler declined) or a bytecode `VmCoro` (async bodies that compile) -- both drive the
same way." That is, lumen's production answer to "which of (a) or (b)" is: prefer a
bytecode-compiled body when the compiler accepts it, and fall back to a pooled OS thread
running the interpreter body when it doesn't. That fallback is exactly what makes lumen's
generator support total (not gated on compiler feature coverage) while still getting the
VM's speed for the common case.

## Recommendation

Adopt the same hybrid lumen already runs, not one option chosen over the other:

1. **Primary path -- (a).** Add `generator_body_code`/`async_generator_body_code` caches on
   the function object mirroring `async_body_code`'s exact shape (compile once, cache the
   `Result`, keyed off `fun.is_generator`/`fun.is_async`). On success, route the
   `FunctionBody::Block` generator call through `FunctionCode::start_generator_with_env`
   instead of `exec_stmts`. This is almost entirely additive to JSVM-7's own code (the
   compiler entry points and the generator-object/driver machinery already exist and are
   already tested) and costs one small, isolated branch in `interp.rs`'s existing
   `FunctionBody::Block` arm, right next to the pre-existing `if let Some(code) =
   &async_body { return code.call_async_with_env(self, e); }` line it mirrors --the same
   file the `async_body` bridge already lives in, not a new touch point, and not
   `interp_stmt.rs` at all (there is no such file), so it stays clear of JSRT-19's editing
   in that area.
2. **Fallback path -- port (b).** For a generator body the VM compiler declines (destructured
   params, rest patterns, `for...of`, or any other gap), port `coroutine.rs`'s pooled
   ping-pong OS-thread mechanism, attributed per ADR 0013 (header comment citing
   `lucid-softworks/lumen`, `crates/lumen/src/coroutine.rs`, `f6c9860d`, MIT; a
   `THIRD_PARTY_NOTICES.md` row), adapted onto `Interpreter`/`JsValue`/`Throw` in place of
   `Interp`/`Value`. Concretely: a `Coroutine::Thread` variant alongside a
   `Coroutine::Vm` variant (the latter wrapping JSVM-7's existing `GeneratorState`/
   `FunctionCode` driver) behind one small enum, so the call site does not care which backend
   produced the generator object. The `ExprKind::Yield` arm (interp.rs:1934) changes from
   an unconditional throw to: if running inside a coroutine body (thread-local check,
   mirroring lumen's `in_coroutine()`), call `coroutine_yield`/`coroutine_await` and resume
   with whatever `Resume` comes back; otherwise (not inside a coroutine -- ordinary script
   top level, or a VM-driven frame that never reaches this arm) keep the current throw. That
   is a single, self-contained, low-line-count change to the one `ExprKind::Yield` match arm,
   which is the only interpreter-side hook this option needs beyond the call-site branch --
   satisfying "keep your interpreter-side hooks minimal" without leaving any generator body
   unsupported.

   This fallback is what makes the goal ("every generator and async-generator function
   works by default, whatever backend runs the surrounding program") actually true on day
   one, rather than true only for the subset of bodies the VM compiler happens to already
   handle. Given the VM compiler's current gaps (destructured/defaulted params, rest
   patterns, `for...of`) are common, not rare, in real generator bodies -- confirmed by
   JSVM-7's own 63-file set, where those two gaps were the entire remaining blocker --
   shipping (a) alone would leave a large, uncertain fraction of `built-ins/Iterator` and
   `language/statements|expressions/generators` still failing, with no fallback to close the
   gap until every VM-compiler gap is separately fixed by whichever lane owns that surface.
   Shipping (a)+(b) together closes the whole surface immediately: the fast path for
   everything the compiler already handles, and a slower-but-always-correct path for
   everything else, with no dependency on unrelated compiler work landing first.

3. **Do not gate on VM eligibility.** Neither path touches `vm_eligibility::eligible` or the
   `AMAZING_JS_VM` env var -- both operate per generator-function-call, independent of
   whatever backend (tree walker or whole-program VM) is running the calling code. That
   satisfies "whatever backend runs the surrounding program" directly: a generator called
   from a VM-compiled caller, or from tree-walked code, or from top-level script, all reach
   the same call-site branch in `call_value_with_new_target_inner`.

## Risks / open questions for the implementing session

- **Thread-pool lifetime vs. `Interpreter` lifetime.** Lumen's soundness argument for
  `InterpPtr` rests on "a coroutine never outlives the `Engine` that owns the interpreter,
  and that `Engine` is not moved between the `eval` calls that create and drive it." Need to
  confirm Amazing's own `Interpreter` ownership/move discipline (host embedding, realms,
  `agent_id`/`realms: HashMap<u64, RealmState>`) gives the same guarantee before reusing the
  raw-pointer trick verbatim; if Amazing ever moves or relocates an `Interpreter` while a
  coroutine is parked, the port needs an extra invariant check or a different pointer
  strategy (e.g. re-deriving the pointer on each resume rather than capturing it once).
- **Stack size.** Lumen sizes worker stacks at 64 MiB against its own `MAX_EVAL_DEPTH`
  (1500). Amazing's own `MAX_CALL_DEPTH` (interp.rs, used at the depth guard just above the
  `FunctionBody::Block` arm) should be checked and the ported worker stack sized to match,
  not copied blindly.
- **`async_generator_body_code`/`generator_body_code` cache correctness with the fallback
  enum.** The cache should store "compiled" vs. "known to need the thread fallback"
  (not re-attempt VM compilation on every call to a body that will never compile), mirroring
  `async_body_code`'s `Option<Result<FunctionCode, CompileError>>` cached-`Result` shape.
- **JSRT-19 overlap.** Confirmed by direct grep this session: `interp_stmt.rs` does not
  exist in this tree, and the `ExprKind::Yield` arm plus the `FunctionBody::Block` call
  dispatch are both in `interp.rs`, not wherever JSRT-19's NamedEvaluation work lives.
  Re-check this at implementation time (js-runtime may have since restructured), but as of
  this investigation there is no file-level collision, only the general
  "same file at the same time" caution `interp.rs` always carries as a shared file.
- **Async-generator `await` under the thread fallback.** `coroutine_await` on the ported
  path parks the worker thread waiting for a promise reaction exactly like `coroutine_yield`
  parks it waiting for the next driver resume -- both go through the same `park()` and
  thread-local `YIELDER`, so no separate mechanism is needed; the driver side re-enters via
  the promise's `.then()` reaction the same way JSVM-7's own `async_generator_step_handler`
  already does for the VM path. `ASYNC_GEN`'s thread-local flag (set via
  `set_async_gen(true)` before running an async-generator body) is what tells a plain
  `yield` inside that body to await its operand first (AsyncGeneratorYield) -- port this
  flag as-is; it is orthogonal to the `Coroutine::Thread`/`Coroutine::Vm` choice.

## Gate (once unblocked and implemented)

Per brief: one `linux-run.sh` with `--timeout 10`, directory-scoped Test262 before/after on
`built-ins/Iterator`, `built-ins/GeneratorPrototype`, `built-ins/AsyncGeneratorPrototype`,
`language/statements/generators`, `language/expressions/generators`,
`language/expressions/yield`; workspace nextest + doctests; no full Boat sweep. Commit, set
`done` with SHA and numbers, then stop (this ticket's brief does not ask for a next item
after).

## Blocking condition

Per coordinator instruction, no rebase and no VM code until js-lang's merge onto
`crates/amazing-js/src/vm` lands in `dev` and the coordinator signals it. This note is the
full output of the investigation phase; the next session should rebase onto the
post-merge `dev` first, re-confirm the `interp.rs` line numbers and `async_body_code`
shape have not shifted under the merge, then implement per the plan above.
