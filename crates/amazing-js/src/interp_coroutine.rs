// Generators and async generators for the *default* (tree-walking) execution path.
//
// The tree walker has no CPS/bytecode representation to suspend mid-body (that machinery lives
// only in `vm/`, gated behind whole-program eligibility). This file gives it one directly, by
// running a generator's body on its own pooled OS thread and handing control back and forth with
// a strict ping-pong channel handoff -- exactly one of {driver, generator thread} runs at any
// instant, so the shared `Interpreter` is never touched concurrently, which is what makes
// shuttling a `*mut Interpreter` across the thread boundary sound.
//
// Portions ported from lucid-softworks/lumen (crates/lumen/src/coroutine.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT: the pooled-worker design (`Resume`/`Suspend`,
// `ThreadCoro`, the idle pool, the panic/teardown handling in `resume`/`park`) is adapted here
// onto this crate's own `Interpreter`/`JsValue`/`Throw` types in place of lumen's `Interp`/
// `Value`; the `Coroutine::{Thread,Vm}` split, the frame-ownership panic recovery tied to
// `Interp::cur_coro`/`fn_frames`, and the `Suspend::Await`-driven cross-microtask async driver are
// not reproduced here (this crate has no per-call frame stack to tag, and `await` inside a
// coroutine body currently falls through to this crate's own pre-existing, separately tracked
// `await_value` fallback -- see its doc comment -- rather than truly parking; wiring `await`
// through a `coroutine_await` the way lumen's own async bodies use it is a follow-up, not
// attempted here).
//
// `yield`'s "inject a `.return(v)`/`.throw(e)` completion at the suspended point" is not
// representable in `eval_expr`'s `Result<JsValue, Throw>` return type (only `exec_stmt`'s
// `Result<Flow, Throw>` can carry a `Flow::Return`), so a `.return(v)` is smuggled through the
// existing `Err` channel via a thread-local marker (`PENDING_RETURN`) that `exec_try`'s catch
// dispatch consults to skip `catch` (a non-local return must not be catchable) while still
// running `finally` (already unconditional there); a `.throw(e)` needs no smuggling at all --
// `Err(Throw(e))` already models a real JS throw correctly, so it is caught by `catch` exactly
// like any other exception, which is the spec-correct behaviour.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

/// Driver -> generator: resume the body.
enum Resume {
    /// `next(v)` -- the `yield` expression evaluates to `v`.
    Next(JsValue),
    /// `return(v)` -- inject a return completion at the suspended `yield`.
    Return(JsValue),
    /// `throw(e)` -- inject a throw at the suspended `yield`.
    Throw(JsValue),
}

// `Resume`/`Suspend` carry `JsValue`s (non-`Send`, `Rc`-based). Transferring them across the
// channel is sound because of the strict ping-pong: a value is produced on one side only after
// the other side has parked, so it is never touched on two threads at once.
unsafe impl Send for Resume {}
unsafe impl Send for Suspend {}

/// Generator -> driver: the body parked or finished.
enum Suspend {
    /// `yield v` -- parked, produced `v`.
    Yield(JsValue),
    /// The body ran to completion / `return v`.
    Done(JsValue),
    /// The body threw `e` and it escaped.
    Throw(JsValue),
}

/// A `*mut Interpreter` carried to the generator thread. Sound only under the strict ping-pong
/// handoff: when the generator thread dereferences it the driver is parked (not touching the
/// interpreter), and vice versa, so the two `&mut` reborrows are never *used* concurrently.
struct InterpPtr(*mut Interpreter);
unsafe impl Send for InterpPtr {}

/// The generator body, boxed. It captures `Rc`s (the function's env, its AST body) so it is not
/// really `Send`; the strict handoff makes moving it to the worker thread sound.
struct SendBody(Box<dyn FnOnce(&mut Interpreter) -> Suspend>);
unsafe impl Send for SendBody {}

/// The generator-thread side of the channels, kept in the worker thread's TLS.
struct Yielder {
    suspend_tx: Sender<Suspend>,
    resume_rx: Receiver<Resume>,
}

thread_local! {
    static YIELDER: RefCell<Option<Yielder>> = const { RefCell::new(None) };
    /// Smuggles a `.return(v)` completion through the `Err(Throw(_))` channel (see the module
    /// doc). Set right before returning a placeholder `Err` at a `yield`/`yield*` point that
    /// received `Resume::Return`; consulted by `exec_try`'s catch dispatch (to skip `catch`) and
    /// finally taken (and cleared) once at the top of the body, in `thread_result_to_suspend`.
    static PENDING_RETURN: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

/// Whether the current thread is executing a coroutine-backed generator body (so `yield` is legal
/// here, and a pending-return marker in flight must skip `catch`).
fn in_coroutine() -> bool {
    YIELDER.with(|y| y.borrow().is_some())
}

pub(crate) fn has_pending_return() -> bool {
    PENDING_RETURN.with(|p| p.borrow().is_some())
}

fn set_pending_return(v: JsValue) {
    PENDING_RETURN.with(|p| *p.borrow_mut() = Some(v));
}

fn take_pending_return() -> Option<JsValue> {
    PENDING_RETURN.with(|p| p.borrow_mut().take())
}

/// An OS-thread-backed coroutine (a pooled worker runs the body; see `spawn_coroutine`).
struct ThreadCoro {
    resume_tx: Sender<Resume>,
    suspend_rx: Receiver<Suspend>,
    /// Set once the body has finished (Done/Throw); further resumes are no-ops.
    done: bool,
}

impl ThreadCoro {
    /// Hand control to the generator and block until it next parks or finishes. Only called when
    /// `!done` (callers short-circuit the completed case themselves, since a bare resume-on-done
    /// would have to fabricate a `Suspend` without knowing the caller's own `Resume` kind).
    fn resume(&mut self, signal: Resume) -> Suspend {
        let _ = self.resume_tx.send(signal);
        match self.suspend_rx.recv() {
            Ok(s) => {
                if matches!(s, Suspend::Done(_) | Suspend::Throw(_)) {
                    self.done = true;
                }
                s
            }
            // The worker died (panicked). Treat the generator as finished rather than wedging
            // the driver forever waiting on a channel nothing will ever send on again.
            Err(_) => {
                self.done = true;
                Suspend::Done(JsValue::Undefined)
            }
        }
    }
}

/// The driver side of one generator's coroutine. A single variant today (`Thread`); a `Vm`
/// variant compiling the body with the bytecode VM instead (JSVM-8's option (a), the VM fast
/// path) is a follow-up commit, once js-lang's merge onto `vm/` has landed -- see
/// `docs/lanes/js-vm-8-design.md`.
enum Coroutine {
    Thread(ThreadCoro),
}

impl Coroutine {
    fn resume(&mut self, signal: Resume) -> Suspend {
        match self {
            Coroutine::Thread(c) => c.resume(signal),
        }
    }
    fn done(&self) -> bool {
        match self {
            Coroutine::Thread(c) => c.done,
        }
    }
}

/// Park the running coroutine, hand `msg` to the driver, and block until resumed.
fn park(msg: Suspend) -> Resume {
    let resumed = YIELDER.with(|y| {
        let b = y.borrow();
        let yl = b.as_ref().expect("yield outside a coroutine body");
        let _ = yl.suspend_tx.send(msg);
        yl.resume_rx.recv()
    });
    match resumed {
        Ok(r) => r,
        // `recv` errors only when the driver's `resume_tx` was dropped -- the `Coroutine` (and
        // usually its owning `Interpreter`) is being torn down. Resuming the body here would run
        // JS (`finally` blocks) that dereferences the shared `*mut Interpreter` from this thread
        // while the main thread tears it down. Instead, never touch the interpreter again: park
        // forever, holding only the captured `Rc`s. The detached thread is reaped at process exit.
        Err(_) => loop {
            std::thread::park();
        },
    }
}

/// `yield value` (non-delegating) -- park producing a generator value, and convert the eventual
/// resumption to `eval_expr`'s return shape. Callers must have already checked `in_coroutine()`;
/// `park` panics with a clear message otherwise instead of silently misbehaving.
fn coroutine_yield_to_expr(value: JsValue) -> Result<JsValue, Throw> {
    match park(Suspend::Yield(value)) {
        Resume::Next(v) => Ok(v),
        Resume::Throw(e) => Err(Throw(e)),
        Resume::Return(v) => {
            set_pending_return(v);
            Err(Throw(JsValue::Undefined))
        }
    }
}

/// `done`/`value` via ordinary property gets, matching IteratorComplete/IteratorValue (7.4.1).
fn iterator_result_parts(i: &mut Interpreter, result: &JsValue) -> Result<(bool, JsValue), Throw> {
    if !matches!(result, JsValue::Object(_)) {
        return Err(Throw::type_error("iterator result is not an object"));
    }
    let done = i.get_prop(result, "done")?.to_boolean();
    let value = i.get_prop(result, "value")?;
    Ok((done, value))
}

/// `yield* iterable` (spec 15.5.5, sync delegation). Forwards `next`/`throw`/`return` resumptions
/// to the inner iterator, yielding each of its non-done results in turn, and completes the outer
/// `yield*` expression with the inner iterator's own final `value` once it reports `done`.
/// Reuses the sync `@@iterator` protocol uniformly for both sync and async generator bodies (an
/// async generator delegating via its own `@@asyncIterator`, and truly awaiting each delegated
/// value, is out of scope here for the same reason plain `await` is -- see the module doc).
fn yield_delegate(i: &mut Interpreter, iterable: JsValue) -> Result<JsValue, Throw> {
    let method = i.get_prop(&iterable, WellKnownSymbol::Iterator.symbol())?;
    let iterator = i.call_value(&method, iterable.clone(), Vec::new())?;
    let mut resume = Resume::Next(JsValue::Undefined);
    loop {
        resume = match resume {
            Resume::Next(sent) => {
                let next = i.get_prop(&iterator, "next")?;
                let result = i.call_value(&next, iterator.clone(), vec![sent])?;
                let (done, value) = iterator_result_parts(i, &result)?;
                if done {
                    return Ok(value);
                }
                park(Suspend::Yield(value))
            }
            Resume::Throw(e) => {
                let throw_method = i.get_prop(&iterator, "throw")?;
                if !i.binding_is_callable(&throw_method) {
                    return Err(Throw::type_error(
                        "delegated iterator does not have a throw method",
                    ));
                }
                let result = i.call_value(&throw_method, iterator.clone(), vec![e])?;
                let (done, value) = iterator_result_parts(i, &result)?;
                if done {
                    return Ok(value);
                }
                park(Suspend::Yield(value))
            }
            Resume::Return(v) => {
                let return_method = i.get_prop(&iterator, "return")?;
                if !i.binding_is_callable(&return_method) {
                    set_pending_return(v);
                    return Err(Throw(JsValue::Undefined));
                }
                let result = i.call_value(&return_method, iterator.clone(), vec![v])?;
                let (done, value) = iterator_result_parts(i, &result)?;
                if done {
                    set_pending_return(value);
                    return Err(Throw(JsValue::Undefined));
                }
                park(Suspend::Yield(value))
            }
        };
    }
}

/// Evaluate a `yield`/`yield*` expression. Only reachable while `in_coroutine()` -- the generator
/// call path (`start_thread_generator`) always runs a generator body on a coroutine, so ordinary
/// script/non-generator-function evaluation never reaches here; kept as an explicit, precise
/// error instead of a panic in case some future caller evaluates a `yield` outside of one.
pub(crate) fn eval_yield(
    i: &mut Interpreter,
    value: JsValue,
    delegate: bool,
) -> Result<JsValue, Throw> {
    if !in_coroutine() {
        return Err(Throw::type_error("generators are not implemented"));
    }
    if delegate {
        yield_delegate(i, value)
    } else {
        coroutine_yield_to_expr(value)
    }
}

/// Idle worker threads waiting for their next coroutine. Guarded by a plain `Mutex`: under the
/// strict ping-pong exactly one thread touches the interpreter at a time, so contention is
/// near-zero (a worker only pushes itself back *after* handing its final value to the driver).
static IDLE: Mutex<Vec<Sender<CoroutineJob>>> = Mutex::new(Vec::new());

/// One unit of work for a pooled worker.
struct CoroutineJob {
    ptr: InterpPtr,
    body: SendBody,
    resume_rx: Receiver<Resume>,
    suspend_tx: Sender<Suspend>,
}

/// Grab an idle worker, or start a new one. `Err` only if the platform cannot spawn threads.
fn get_worker() -> std::io::Result<Sender<CoroutineJob>> {
    if let Some(tx) = IDLE.lock().unwrap().pop() {
        return Ok(tx);
    }
    let (job_tx, job_rx) = channel::<CoroutineJob>();
    let self_tx = job_tx.clone();
    std::thread::Builder::new()
        // Generous stack: the tree-walker recurses deeply through nested expression/statement
        // evaluation, matching MAX_CALL_DEPTH's own generosity (512 call frames, each of which is
        // itself several Rust stack frames deep).
        .stack_size(64 * 1024 * 1024)
        .spawn(move || worker_loop(job_rx, self_tx))?;
    Ok(job_tx)
}

fn worker_loop(job_rx: Receiver<CoroutineJob>, self_tx: Sender<CoroutineJob>) {
    while let Ok(job) = job_rx.recv() {
        run_job(job);
        IDLE.lock().unwrap().push(self_tx.clone());
    }
}

/// Set up this thread's coroutine TLS, run the body from its first drive to completion, and hand
/// the outcome to the driver.
fn run_job(job: CoroutineJob) {
    let CoroutineJob { ptr, body, resume_rx, suspend_tx } = job;
    let SendBody(body) = body;
    YIELDER.with(|y| {
        *y.borrow_mut() = Some(Yielder { suspend_tx: suspend_tx.clone(), resume_rx })
    });
    // Park until the first next()/return()/throw(); the body doesn't run before then.
    let first = YIELDER.with(|y| y.borrow().as_ref().unwrap().resume_rx.recv());
    let outcome = match first {
        Err(_) => {
            YIELDER.with(|y| *y.borrow_mut() = None);
            return; // dropped before first drive
        }
        Ok(Resume::Next(_)) => {
            let interp = unsafe { &mut *ptr.0 };
            body(interp)
        }
        Ok(Resume::Return(v)) => Suspend::Done(v),
        Ok(Resume::Throw(e)) => Suspend::Throw(e),
    };
    let _ = suspend_tx.send(outcome);
    YIELDER.with(|y| *y.borrow_mut() = None);
}

/// Spawn a coroutine over `body` on a pooled worker, parked until its first resume.
fn spawn_coroutine(interp: *mut Interpreter, body: SendBody) -> std::io::Result<Coroutine> {
    let (resume_tx, resume_rx) = channel::<Resume>();
    let (suspend_tx, suspend_rx) = channel::<Suspend>();
    let worker = get_worker()?;
    let job = CoroutineJob { ptr: InterpPtr(interp), body, resume_rx, suspend_tx };
    worker
        .send(job)
        .map_err(|_| std::io::Error::other("coroutine worker unavailable"))?;
    Ok(Coroutine::Thread(ThreadCoro { resume_tx, suspend_rx, done: false }))
}

/// Thrown/rejected when a coroutine cannot be spawned (no OS threads on this platform).
const UNSUPPORTED_MSG: &str =
    "generators require OS threads, which this platform build does not have";

/// Per-generator-instance runtime state (`Exotic::ThreadGenerator`'s payload). `coro` is `None`
/// only when `spawn_coroutine` itself failed (no OS threads); every method then throws/rejects
/// with `UNSUPPORTED_MSG` instead of touching a coroutine that doesn't exist. `started` mirrors
/// GeneratorState's own field of the same name (suspendedStart vs. a real suspended yield), needed
/// because `.return(v)`/`.throw(e)` on an unstarted generator must not have run any body code, and
/// our coroutine only tells us "done", not "never started".
pub struct ThreadGeneratorState {
    coro: Option<Coroutine>,
    is_async: bool,
    started: bool,
}

impl std::fmt::Debug for ThreadGeneratorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadGeneratorState")
            .field("is_async", &self.is_async)
            .field("started", &self.started)
            .finish()
    }
}

/// Convert the tree walker's own statement-execution outcome for a just-run generator body into
/// the driver-facing `Suspend`, taking (and clearing) `PENDING_RETURN` if a `.return(v)` was
/// smuggled through as an `Err` along the way (see the module doc). Unconditional on every path
/// out, so a pooled worker never carries a stale marker into its next, unrelated job.
fn thread_result_to_suspend(result: Result<Flow, Throw>) -> Suspend {
    match result {
        Ok(Flow::Return(v)) => {
            take_pending_return();
            Suspend::Done(v)
        }
        Ok(_) => {
            take_pending_return();
            Suspend::Done(JsValue::Undefined)
        }
        Err(th) => match take_pending_return() {
            Some(v) => Suspend::Done(v),
            None => Suspend::Throw(th.0),
        },
    }
}

fn create_iter_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut object = JsObject::ordinary();
    object.proto = Some(i.object_proto.clone());
    object.set_own("value", value);
    object.set_own("done", JsValue::Bool(done));
    JsValue::object(object)
}

fn thread_generator_state(this: &JsValue) -> Result<Rc<RefCell<ThreadGeneratorState>>, Throw> {
    if let JsValue::Object(o) = this {
        if let Exotic::ThreadGenerator(state) = &o.borrow().exotic {
            return Ok(state.clone());
        }
    }
    Err(Throw::type_error("method called on an incompatible receiver"))
}

/// Shared `next`/`return`/`throw` driver for a coroutine-backed generator (sync or async --
/// callers wrap the `{value, done}` object in a settled Promise for the async prototype methods).
/// Handles the suspendedStart/completed short-circuits itself (matching GeneratorResume /
/// GeneratorResumeAbrupt, spec 27.5.3.3/27.5.3.4), so a `.return(v)`/`.throw(e)` before the body
/// ever ran, or after it already completed, never touches the coroutine and never loses `v`.
fn thread_generator_resume(
    i: &Interpreter,
    this: &JsValue,
    signal: Resume,
) -> Result<JsValue, Throw> {
    let state = thread_generator_state(this)?;
    let mut guard = state.borrow_mut();
    if guard.coro.is_none() {
        return match signal {
            Resume::Throw(e) => Err(Throw(e)),
            Resume::Return(_) | Resume::Next(_) => Err(Throw::type_error(UNSUPPORTED_MSG)),
        };
    }
    let already_started = guard.started;
    guard.started = true;
    if !already_started && matches!(signal, Resume::Return(_) | Resume::Throw(_)) {
        // suspendedStart: GeneratorResumeAbrupt completes the generator without running any body
        // code, unlike GeneratorResume's Next case (which the coroutine's own first-resume
        // handling in `run_job` starts the body from).
        let coro = guard.coro.as_mut().expect("checked above");
        return match signal {
            Resume::Return(v) => {
                coro.resume(Resume::Return(v.clone()));
                Ok(create_iter_result(i, v, true))
            }
            Resume::Throw(e) => {
                coro.resume(Resume::Throw(e.clone()));
                Err(Throw(e))
            }
            Resume::Next(_) => unreachable!(),
        };
    }
    let coro = guard.coro.as_mut().expect("checked above");
    if coro.done() {
        return match signal {
            Resume::Next(_) => Ok(create_iter_result(i, JsValue::Undefined, true)),
            Resume::Return(v) => Ok(create_iter_result(i, v, true)),
            Resume::Throw(e) => Err(Throw(e)),
        };
    }
    let suspend = coro.resume(signal);
    match suspend {
        Suspend::Yield(v) => Ok(create_iter_result(i, v, false)),
        Suspend::Done(v) => Ok(create_iter_result(i, v, true)),
        Suspend::Throw(e) => Err(Throw(e)),
    }
}

fn thread_async_settle(i: &Interpreter, result: Result<JsValue, Throw>) -> JsValue {
    match result {
        Ok(v) => new_promise(i, PromiseFate::Fulfilled(v)),
        Err(Throw(e)) => new_promise(i, PromiseFate::Rejected(e)),
    }
}

fn thread_generator_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    thread_generator_resume(i, &this, Resume::Next(v))
}
fn thread_generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    thread_generator_resume(i, &this, Resume::Return(v))
}
fn thread_generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let e = args.into_iter().next().unwrap_or(JsValue::Undefined);
    thread_generator_resume(i, &this, Resume::Throw(e))
}
fn thread_generator_return_self(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
}

fn thread_async_generator_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    Ok(thread_async_settle(i, thread_generator_resume(i, &this, Resume::Next(v))))
}
fn thread_async_generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    Ok(thread_async_settle(i, thread_generator_resume(i, &this, Resume::Return(v))))
}
fn thread_async_generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let e = args.into_iter().next().unwrap_or(JsValue::Undefined);
    Ok(thread_async_settle(i, thread_generator_resume(i, &this, Resume::Throw(e))))
}

impl Interpreter {
    /// Called once, during interpreter construction (`install_builtins`), to give the shared
    /// `%GeneratorPrototype%`/`%AsyncGeneratorPrototype%` singletons their `next`/`return`/
    /// `throw`/`@@(async)iterator` methods -- every coroutine-backed generator instance's own
    /// `.prototype` inherits from these (see `make_function`/`method_value`'s proto wiring),
    /// exactly like every other shared iterator prototype in this crate (`regexp_string_iterator_
    /// proto`, the std collection iterator protos, ...).
    pub(crate) fn install_thread_generator_methods(&mut self) {
        for (name, length, native) in [
            ("next", 1, thread_generator_next as crate::value::NativeFn),
            ("return", 1, thread_generator_return as crate::value::NativeFn),
            ("throw", 1, thread_generator_throw as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.generator_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        let iter_method = symbol_method(self, "[Symbol.iterator]", thread_generator_return_self);
        symbol_method_length(&iter_method, 0);
        self.generator_proto.borrow_mut()
            .define(WellKnownSymbol::Iterator.symbol(), symbol_data(iter_method, true, true)).unwrap();

        for (name, length, native) in [
            ("next", 1, thread_async_generator_next as crate::value::NativeFn),
            ("return", 1, thread_async_generator_return as crate::value::NativeFn),
            ("throw", 1, thread_async_generator_throw as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.async_generator_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        let async_iter_method = symbol_method(self, "[Symbol.asyncIterator]", thread_generator_return_self);
        symbol_method_length(&async_iter_method, 0);
        self.async_generator_proto.borrow_mut()
            .define(WellKnownSymbol::AsyncIterator.symbol(), symbol_data(async_iter_method, true, true)).unwrap();
    }

    /// Build and spawn a coroutine-backed generator object for a just-called generator function
    /// (`is_generator`; `is_async` also set for an async generator). `env` is the already-built
    /// call environment (params bound, `this` bound if applicable) and `body`'s statements have
    /// already been hoisted into it by the caller -- both must happen synchronously at call time,
    /// matching OrdinaryCallEvaluateBody's eager FunctionDeclarationInstantiation, before any
    /// generator-body statement runs (which happens lazily, on the first `.next()`).
    pub(crate) fn start_thread_generator(
        &mut self,
        o: &Obj,
        body: ast::Block,
        env: EnvHandle,
        is_async: bool,
        strict: bool,
    ) -> JsValue {
        let has_using = body.body.iter().any(stmt_declares_resource);
        let job = SendBody(Box::new(move |i: &mut Interpreter| -> Suspend {
            // `strict_eval_context` is a scalar on `Interpreter`, not part of the captured
            // `env`'s scope chain, and the driver already restored it to whatever it was before
            // *this* call by the time a later `.next()` actually drives the body (this call
            // itself returned immediately, without running any body statements). Save/restore it
            // around the body the same way the VM's own generator/async driver does for its
            // resumed frames (`resume_frame_with_strictness`), so a strict-mode generator body
            // stays strict on every resume regardless of the driver's own context at that moment.
            let previous_strict = i.strict_eval_context;
            i.strict_eval_context = strict;
            let completion = if has_using {
                i.using_stack.push(Vec::new());
                let result = i.exec_stmts(&body.body, &env);
                let frame = i.using_stack.pop().unwrap_or_default();
                i.dispose_frame(frame, result)
            } else {
                i.exec_stmts(&body.body, &env)
            };
            i.strict_eval_context = previous_strict;
            thread_result_to_suspend(completion)
        }));
        let ptr = self as *mut Interpreter;
        let coro = match spawn_coroutine(ptr, job) {
            Ok(c) => Some(c),
            Err(_) => None,
        };
        let mut object = JsObject::ordinary();
        object.proto = match o.borrow().get_own("prototype") {
            Some(JsValue::Object(p)) => Some(p),
            _ => Some(if is_async { self.async_generator_proto.clone() } else { self.generator_proto.clone() }),
        };
        object.exotic = Exotic::ThreadGenerator(Rc::new(RefCell::new(ThreadGeneratorState {
            coro,
            is_async,
            started: false,
        })));
        JsValue::object(object)
    }
}

#[cfg(test)]
mod thread_generator_tests {
    use super::*;

    fn run(source: &str) -> JsValue {
        Interpreter::new().run_script(source).unwrap()
    }

    #[test]
    fn calling_a_generator_function_returns_a_real_generator_object() {
        let v = run("function* g() {} typeof g();");
        assert_eq!(v, JsValue::str("object"));
    }

    #[test]
    fn basic_next_sequence_and_completion() {
        let v = run(
            "
            function* g() { yield 1; yield 2; return 3; }
            var it = g();
            var a = it.next(); var b = it.next(); var c = it.next(); var d = it.next();
            [a, b, c, d].map(r => r.value + ':' + r.done).join(',');
            ",
        );
        assert_eq!(v, JsValue::str("1:false,2:false,3:true,undefined:true"));
    }

    #[test]
    fn next_argument_becomes_the_yield_expressions_value() {
        let v = run(
            "
            function* g() { var x = yield 1; return x + 1; }
            var it = g();
            it.next();
            it.next(41).value;
            ",
        );
        assert_eq!(v, JsValue::num(42.0));
    }

    #[test]
    fn symbol_iterator_returns_self_and_for_of_drains_it() {
        let v = run(
            "
            function* g() { yield 'a'; yield 'b'; }
            var it = g();
            var same = it[Symbol.iterator]() === it;
            var out = [];
            for (var x of g()) out.push(x);
            same + ':' + out.join(',');
            ",
        );
        assert_eq!(v, JsValue::str("true:a,b"));
    }

    #[test]
    fn return_before_start_completes_without_running_the_body() {
        let v = run(
            "
            var ran = false;
            function* g() { ran = true; yield 1; }
            var it = g();
            var r = it.return(99);
            ran + ':' + r.value + ':' + r.done;
            ",
        );
        assert_eq!(v, JsValue::str("false:99:true"));
    }

    #[test]
    fn return_while_suspended_at_a_yield_completes_the_generator() {
        let v = run(
            "
            function* g() { yield 1; yield 2; }
            var it = g();
            it.next();
            var r = it.return(7);
            var after = it.next();
            r.value + ':' + r.done + ':' + after.value + ':' + after.done;
            ",
        );
        assert_eq!(v, JsValue::str("7:true:undefined:true"));
    }

    #[test]
    fn return_while_suspended_inside_try_finally_runs_finally_but_skips_catch() {
        let v = run(
            "
            var log = [];
            function* g() {
                try {
                    try {
                        yield 1;
                        log.push('unreachable');
                    } catch (e) {
                        log.push('caught:' + e);
                    } finally {
                        log.push('finally');
                    }
                } finally {
                    log.push('outer-finally');
                }
            }
            var it = g();
            it.next();
            var r = it.return(5);
            log.join(',') + '|' + r.value + ':' + r.done;
            ",
        );
        assert_eq!(v, JsValue::str("finally,outer-finally|5:true"));
    }

    #[test]
    fn throw_while_suspended_is_catchable_inside_the_body() {
        let v = run(
            "
            function* g() {
                try {
                    yield 1;
                    return 'unreachable';
                } catch (e) {
                    return 'caught:' + e;
                }
            }
            var it = g();
            it.next();
            it.throw('boom').value;
            ",
        );
        assert_eq!(v, JsValue::str("caught:boom"));
    }

    #[test]
    fn throw_before_start_throws_synchronously_without_running_the_body() {
        let mut i = Interpreter::new();
        let err = i
            .run_script(
                "
                var ran = false;
                function* g() { ran = true; yield 1; }
                var it = g();
                it.throw('nope');
                ",
            )
            .unwrap_err();
        assert_eq!(err.0, JsValue::str("nope"));
        assert_eq!(i.run_script("ran;").unwrap(), JsValue::Bool(false));
    }

    #[test]
    fn yield_star_delegates_next_to_the_inner_iterable_and_yields_its_values() {
        let v = run(
            "
            function* inner() { yield 'a'; yield 'b'; return 'inner-done'; }
            function* outer() { var r = yield* inner(); yield r; }
            var it = outer();
            var a = it.next().value;
            var b = it.next().value;
            var c = it.next().value;
            [a, b, c].join(',');
            ",
        );
        assert_eq!(v, JsValue::str("a,b,inner-done"));
    }

    #[test]
    fn generator_methods_and_object_literal_generator_methods_work() {
        let v = run(
            "
            class C { *gen() { yield this.tag; } }
            var obj = { *gen() { yield 'literal'; } };
            var c = new C(); c.tag = 'cls';
            [c.gen().next().value, obj.gen().next().value].join(',');
            ",
        );
        assert_eq!(v, JsValue::str("cls,literal"));
    }

    #[test]
    fn async_generator_next_returns_a_promise_resolving_to_an_iter_result() {
        let mut i = Interpreter::new();
        // Calling an async generator returns the generator instance itself, synchronously and
        // directly -- not a promise wrapping it, unlike a plain (non-generator) async function
        // call (spec 27.4.3.2 EvaluateAsyncGeneratorBody).
        let shape = i
            .run_script(
                "
                async function* g() { yield 1; }
                var it = g();
                var itProto = Object.getPrototypeOf(it);
                (itProto === g.prototype) + ':' + typeof itProto.next + ':' + typeof it.next;
                ",
            )
            .unwrap();
        assert_eq!(shape, JsValue::str("true:function:function"));
        i.run_script(
            "
            var results = [];
            async function* g2() { yield 1; yield 2; return 3; }
            var it2 = g2();
            it2.next().then(r => results.push(r.value + ':' + r.done));
            it2.next().then(r => results.push(r.value + ':' + r.done));
            it2.next().then(r => results.push(r.value + ':' + r.done));
            ",
        )
        .unwrap();
        let joined = i.run_script("results.join('|');").unwrap();
        assert_eq!(joined, JsValue::str("1:false|2:false|3:true"));
    }

    #[test]
    fn generator_function_cannot_be_constructed() {
        let mut i = Interpreter::new();
        let err = i.run_script("function* g() {} new g();").unwrap_err();
        assert!(matches!(err.0, JsValue::Object(_)));
    }

    #[test]
    fn generator_prototype_chain_reaches_the_shared_generator_prototype() {
        let v = run(
            "
            function* g() {}
            var it = g();
            var proto = Object.getPrototypeOf(it);
            (proto === g.prototype) + ':' + (typeof proto.next) + ':' + (typeof proto.throw);
            ",
        );
        assert_eq!(v, JsValue::str("true:function:function"));
    }
}
