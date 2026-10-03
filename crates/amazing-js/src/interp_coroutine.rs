// Async-generator Return/Await portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, return statement/yield_one)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Synchronous delegated-yield raw-result adapter portions ported from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, yield_delegate) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
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
// `Value`; the `Coroutine::{Thread,Vm}` split and the frame-ownership panic recovery tied to
// `Interp::cur_coro`/`fn_frames` are not reproduced here (this crate has no per-call frame stack
// to tag). JSVM-9 adds lumen's `Suspend::Await`-driven async driver (`coroutine_await` and
// `Interp::drive_async`, crates/lumen/src/interpreter.rs, same commit): a plain async function
// the VM does not compile runs here too, parking at each `await` and resumed from the awaited
// value's reaction (`start_thread_async_function`/`thread_async_function_step`). Async
// generator bodies retain the legacy `await_value` policy outside Return-specific Await.
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
    /// Sync delegated yield forwards the original inner result object.
    YieldIteratorResult(JsValue),
    /// The body ran to completion / `return v`.
    Done(JsValue),
    /// The body threw `e` and it escaped.
    Throw(JsValue),
    /// `await v` in a coroutine-backed async function body -- parked until `v` settles; the
    /// driver resumes with `Resume::Next(value)` or `Resume::Throw(reason)`.
    Await(JsValue),
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
    /// The running body's execution context (see `Segment`), while a coroutine body runs.
    static SEGMENT: RefCell<Option<Segment>> = const { RefCell::new(None) };
}

/// A coroutine body's own execution context. The body runs in *segments* -- from its start or a
/// resumption up to its next suspension or completion -- each on top of whatever call stack the
/// driver happens to have at that moment (the original call, a later `.next()`, a promise
/// reaction). Every segment therefore re-establishes the function's own call frame (`this`,
/// `new.target`, `super`, `[[HomeObject]]`) and strictness on entry and restores the driver's on
/// exit, the way the VM keeps them in its resumable frame.
struct Segment {
    context_id: u64,
    this: JsValue,
    home: Option<Obj>,
    super_constructor: Option<JsValue>,
    new_target: Option<JsValue>,
    own_function: Option<Obj>,
    strict: bool,
    /// The driver's strictness, restored when the current segment ends.
    saved_strict: bool,
    completion: Option<JsValue>,
    saved_completion: Option<JsValue>,
    /// `i.frames.len()` inside the current segment: an `await` evaluated at exactly this depth
    /// belongs to the body itself (anything deeper is a nested, non-coroutine call).
    base: usize,
    /// An async function body: its `await`s park the coroutine.
    awaits: bool,
}

impl Segment {
    fn capture(i: &Interpreter, strict: bool, awaits: bool) -> Segment {
        let frame = i.frames.last();
        Segment {
            context_id: frame.map(|frame| frame.context_id).unwrap_or_else(CallFrame::next_context_id),
            this: frame.map(|f| f.this.clone()).unwrap_or(JsValue::Undefined),
            home: frame.and_then(|f| f.home.clone()),
            super_constructor: frame.and_then(|f| f.super_constructor.clone()),
            new_target: frame.and_then(|f| f.new_target.clone()),
            own_function: frame.and_then(|f| f.own_function.clone()),
            strict,
            saved_strict: false,
            completion: None,
            saved_completion: None,
            base: 0,
            awaits,
        }
    }
}

/// Start a segment of the running body: push its frame, switch to its strictness.
fn enter_segment(i: &mut Interpreter) {
    SEGMENT.with(|segment| {
        if let Some(segment) = segment.borrow_mut().as_mut() {
            i.frames.push(CallFrame {
                context_id: segment.context_id,
                this: segment.this.clone(),
                home: segment.home.clone(),
                super_constructor: segment.super_constructor.clone(),
                new_target: segment.new_target.clone(),
                own_function: segment.own_function.clone(),
            });
            segment.saved_completion = std::mem::replace(&mut i.completion, segment.completion.clone());
            segment.saved_strict = i.strict_eval_context;
            i.strict_eval_context = segment.strict;
            segment.base = i.frames.len();
        }
    });
}

/// End the current segment: pop its frame, restore the driver's strictness.
fn leave_segment(i: &mut Interpreter) {
    SEGMENT.with(|segment| {
        if let Some(segment) = segment.borrow_mut().as_mut() {
            segment.completion = std::mem::replace(&mut i.completion, segment.saved_completion.clone());
            i.frames.pop();
            i.strict_eval_context = segment.saved_strict;
        }
    });
}

/// `park` from inside a running body, closing the current segment first and opening the next
/// one once resumed.
fn park_segment(i: &mut Interpreter, msg: Suspend) -> Resume {
    leave_segment(i);
    let resumed = park(msg);
    enter_segment(i);
    resumed
}

/// `await value` in a coroutine-backed async function body: park until the driver settles it.
/// `None` when the `await` does not belong to such a body (the caller then keeps the
/// pre-existing synchronous `await_value` behaviour).
pub(crate) fn coroutine_await(i: &mut Interpreter, value: JsValue) -> Option<Result<JsValue, Throw>> {
    coroutine_await_mode(i, value, false, false)
}

/// A synchronous iterator's value is first normalized by its Promise wrapper,
/// which rejects instead of propagating a constructor-get abrupt completion.
pub(crate) fn coroutine_await_sync(i: &mut Interpreter, value: JsValue) -> Option<Result<JsValue, Throw>> {
    coroutine_await_mode(i, value, true, false)
}

/// ReturnStatement and injected return own an Await in an async generator,
/// independently of the legacy policy for its other await/yield operations.
fn coroutine_return_value(i: &mut Interpreter, value: JsValue) -> Result<JsValue, Throw> {
    let is_async_generator = i.frames.last().and_then(|frame| frame.own_function.as_ref())
        .is_some_and(|function| function.borrow().func.as_ref()
            .is_some_and(|function| function.is_async && function.is_generator));
    if !is_async_generator { return Ok(value); }
    match coroutine_await_mode(i, value.clone(), false, true) {
        Some(result) => result,
        None => i.await_value(value),
    }
}

fn coroutine_await_mode(i: &mut Interpreter, value: JsValue, sync_wrapper: bool, return_owner: bool) -> Option<Result<JsValue, Throw>> {
    if !in_coroutine() {
        return None;
    }
    let owned = SEGMENT.with(|segment| {
        segment.borrow().as_ref().is_some_and(|segment| (segment.awaits || return_owner) && segment.base == i.frames.len())
    });
    if !owned {
        return None;
    }
    let promise = if sync_wrapper {
        vm::async_resolve_value(i, value)
    } else {
        match vm::async_resolve_value_checked(i, value) {
            Ok(promise) => promise,
            Err(error) => return Some(Err(error)),
        }
    };
    Some(match park_segment(i, Suspend::Await(promise)) {
        Resume::Next(value) => Ok(value),
        Resume::Throw(error) => Err(Throw(error)),
        Resume::Return(_) => Err(Throw::type_error("an async function cannot be returned from")),
    })
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
fn coroutine_yield_to_expr(i: &mut Interpreter, value: JsValue) -> Result<JsValue, Throw> {
    match park_segment(i, Suspend::Yield(value)) {
        Resume::Next(v) => Ok(v),
        Resume::Throw(e) => Err(Throw(e)),
        Resume::Return(v) => {
            let v = coroutine_return_value(i, v)?;
            set_pending_return(v);
            Err(Throw(JsValue::Undefined))
        }
    }
}

/// `yield* iterable` (spec 15.5.5, sync delegation). Forwards `next`/`throw`/`return` resumptions
/// to the inner iterator, yielding each of its non-done results in turn, and completes the outer
/// `yield*` expression with the inner iterator's own final `value` once it reports `done`.
/// Reuses the sync `@@iterator` protocol uniformly for both sync and async generator bodies (an
/// async generator delegating via its own `@@asyncIterator`, and truly awaiting each delegated
/// value, is out of scope here for the same reason plain `await` is -- see the module doc).
fn yield_delegate(i: &mut Interpreter, iterable: JsValue) -> Result<JsValue, Throw> {
    let state = i.start_sync_yield_delegate(&iterable)?;
    // The legacy async coroutine driver still wraps values synchronously. Preserve that policy;
    // genuine asynchronous delegation remains the VM async_delegate implementation.
    let legacy_async = SEGMENT.with(|segment| segment.borrow().as_ref()
        .and_then(|segment| segment.own_function.as_ref())
        .is_some_and(|function| function.borrow().func.as_ref().is_some_and(|function| function.is_async)));
    let mut resume = Resume::Next(JsValue::Undefined);
    loop {
        let (kind, input) = match resume {
            Resume::Next(value) => (0, value),
            Resume::Return(value) => (1, value),
            Resume::Throw(value) => (2, value),
        };
        match i.step_sync_yield_delegate(&state, kind, input)? {
            crate::interp::YieldDelegateStep::Complete(value) => return Ok(value),
            crate::interp::YieldDelegateStep::Return(value) => {
                set_pending_return(value);
                return Err(Throw(JsValue::Undefined));
            }
            crate::interp::YieldDelegateStep::YieldResult(result) => {
                let suspend = if legacy_async { Suspend::Yield(i.get_prop(&result, "value")?) }
                    else { Suspend::YieldIteratorResult(result) };
                resume = park_segment(i, suspend);
            }
        }
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
        coroutine_yield_to_expr(i, value)
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

/// Most idle workers kept parked for reuse. A burst of concurrently suspended coroutines needs
/// one OS thread each; once they finish, only this many stay parked and the rest exit, so a
/// burst's threads are not retained for the life of the process (JSVM-9 measured 5000 in-flight
/// async functions leaving 5000 idle workers behind before this cap).
const MAX_IDLE_WORKERS: usize = 16;

fn worker_loop(job_rx: Receiver<CoroutineJob>, self_tx: Sender<CoroutineJob>) {
    while let Ok(job) = job_rx.recv() {
        run_job(job);
        let mut idle = IDLE.lock().unwrap();
        if idle.len() >= MAX_IDLE_WORKERS {
            return;
        }
        idle.push(self_tx.clone());
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
    SEGMENT.with(|segment| *segment.borrow_mut() = None);
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
    /// A coroutine-backed plain async function (not a generator): its promise.
    async_target: Option<Obj>,
    // Async-generator requests retain their capabilities while a Return Await parks.
    async_queue: VecDeque<ThreadAsyncGeneratorRequest>,
    async_request: Option<Obj>,
    async_executing: bool,
    async_awaiting: bool,
    async_await_return: bool,
    body_realm: RealmHandle,
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

/// Shared `next`/`return`/`throw` driver for a synchronous coroutine-backed generator.
/// Async generator requests use the private queued continuation adapter instead.
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
        Suspend::YieldIteratorResult(result) => Ok(result),
        Suspend::Done(v) => Ok(create_iter_result(i, v, true)),
        Suspend::Throw(e) => Err(Throw(e)),
        // Generator bodies never park at an await (`Segment::awaits` is false for them).
        Suspend::Await(_) => Err(Throw::type_error("await in a generator coroutine")),
    }
}

/// The receiver's generator kind: `Some(is_async)` for a VM-backed or coroutine-backed
/// generator instance, `None` for anything else (GeneratorValidate's brand check).
fn generator_brand(this: &JsValue) -> Option<bool> {
    if let Some(is_async) = vm::vm_generator_kind(this) {
        return Some(is_async);
    }
    match this {
        JsValue::Object(o) => match &o.borrow().exotic {
            Exotic::ThreadGenerator(state) => Some(state.borrow().is_async),
            _ => None,
        },
        _ => None,
    }
}

/// %GeneratorPrototype%.next/return/throw (kind 0/1/2), shared by both generator backends: the
/// bytecode VM's frames (`vm::generator_*`) and this file's coroutine-backed bodies.
fn generator_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, kind: u8) -> Result<JsValue, Throw> {
    if generator_brand(&this) != Some(false) {
        return Err(Throw::type_error("method called on an incompatible receiver"));
    }
    if vm::vm_generator_kind(&this).is_some() {
        return match kind {
            0 => vm::generator_next(i, this, args),
            1 => vm::generator_return(i, this, args),
            _ => vm::generator_throw(i, this, args),
        };
    }
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    thread_generator_resume(i, &this, match kind { 0 => Resume::Next(v), 1 => Resume::Return(v), _ => Resume::Throw(v) })
}

/// %AsyncGeneratorPrototype%.next/return/throw: a receiver that is not an async generator
/// rejects the returned promise rather than throwing (AsyncGeneratorValidate under
/// IfAbruptRejectPromise).
fn async_generator_method(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, kind: u8) -> Result<JsValue, Throw> {
    if generator_brand(&this) != Some(true) {
        let error = Throw::type_error("method called on an incompatible receiver");
        let error = i.normalize_throw(error);
        return Ok(new_promise(i, PromiseFate::Rejected(error.0)));
    }
    if vm::vm_generator_kind(&this).is_some() {
        return match kind {
            0 => vm::async_generator_next(i, this, args),
            1 => vm::async_generator_return(i, this, args),
            _ => vm::async_generator_throw(i, this, args),
        };
    }
    let v = args.into_iter().next().unwrap_or(JsValue::Undefined);
    thread_async_generator_enqueue(i, thread_generator_state(&this)?, kind, v)
}

fn generator_prototype_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_method(i, this, args, 0)
}
fn generator_prototype_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_method(i, this, args, 1)
}
fn generator_prototype_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_method(i, this, args, 2)
}
fn async_generator_prototype_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_method(i, this, args, 0)
}
fn async_generator_prototype_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_method(i, this, args, 1)
}
fn async_generator_prototype_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_method(i, this, args, 2)
}
fn thread_generator_return_self(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
}
/// %AsyncIteratorPrototype%[@@asyncIterator] (27.1.3.1): returns `this`.
fn async_iterator_prototype_async_iterator(_i: &mut Interpreter, this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
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
            ("next", 1, generator_prototype_next as crate::value::NativeFn),
            ("return", 1, generator_prototype_return as crate::value::NativeFn),
            ("throw", 1, generator_prototype_throw as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.generator_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        // %GeneratorPrototype%'s [[Prototype]] is %IteratorPrototype% (whose own @@iterator
        // returns `this`), installed just before this by `install_std_iterator_helpers`.
        if let Some((iterator_proto, _, _)) = self.std_iterator_prototypes() {
            self.generator_proto.borrow_mut().proto = Some(iterator_proto);
        } else {
            let iter_method = symbol_method(self, "[Symbol.iterator]", thread_generator_return_self);
            self.generator_proto.borrow_mut()
                .define(WellKnownSymbol::Iterator.symbol(), symbol_data(iter_method, true, true)).unwrap();
        }
        self.generator_proto.borrow_mut()
            .define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str("Generator"), false, true)).unwrap();
        // 27.5.1.1/27.6.1.1: `constructor` links back to %GeneratorFunction.prototype% (resp. the
        // async one), {writable:false, enumerable:false, configurable:true}.
        let generator_function_proto = JsValue::Object(self.generator_function_proto.clone());
        self.generator_proto.borrow_mut()
            .define("constructor", symbol_data(generator_function_proto, false, true)).unwrap();
        let async_generator_function_proto = JsValue::Object(self.async_generator_function_proto.clone());
        self.async_generator_proto.borrow_mut()
            .define("constructor", symbol_data(async_generator_function_proto, false, true)).unwrap();

        for (name, length, native) in [
            ("next", 1, async_generator_prototype_next as crate::value::NativeFn),
            ("return", 1, async_generator_prototype_return as crate::value::NativeFn),
            ("throw", 1, async_generator_prototype_throw as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.async_generator_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        self.async_generator_proto.borrow_mut()
            .define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str("AsyncGenerator"), false, true)).unwrap();
        // %AsyncIteratorPrototype% (27.1.3): an ordinary object holding only @@asyncIterator,
        // the [[Prototype]] of %AsyncGeneratorPrototype%. Reachable as that object's prototype.
        let async_iterator_proto = Obj(Rc::new(RefCell::new({
            let mut o = JsObject::ordinary();
            o.proto = Some(self.object_proto.clone());
            o
        })));
        let async_iter_method = symbol_method(self, "[Symbol.asyncIterator]", async_iterator_prototype_async_iterator);
        async_iterator_proto.borrow_mut()
            .define(WellKnownSymbol::AsyncIterator.symbol(), symbol_data(async_iter_method, true, true)).unwrap();
        self.async_generator_proto.borrow_mut().proto = Some(async_iterator_proto);
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
        // Captured now, inside the generator function's own call, so every later segment (each
        // driven from an arbitrary `.next()` call site) runs with this function's frame and
        // strictness rather than the driver's (see `Segment`).
        let segment = Segment::capture(self, strict, false);
        let job = SendBody(Box::new(move |i: &mut Interpreter| -> Suspend {
            SEGMENT.with(|s| *s.borrow_mut() = Some(segment));
            enter_segment(i);
            let completion = if has_using {
                i.using_stack.push(Vec::new());
                let result = i.exec_stmts(&body.body, &env);
                let frame = i.using_stack.pop().unwrap_or_default();
                i.dispose_frame(frame, result)
            } else {
                i.exec_stmts(&body.body, &env)
            };
            leave_segment(i);
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
            async_target: None,
            async_queue: VecDeque::new(),
            async_request: None,
            async_executing: false,
            async_awaiting: false,
            async_await_return: false,
            body_realm: self.binding_current_realm(),
        })));
        JsValue::object(object)
    }

    /// Run a plain async function's body (one the bytecode VM did not compile) on a coroutine so
    /// its `await`s truly suspend until the awaited value settles, instead of the synchronous
    /// `await_value` degrade. Parameters are bound and the body hoisted by the caller, inside the
    /// function's own call; the body runs synchronously up to its first `await` right here, like
    /// AsyncFunctionStart. Returns the function's promise, or `None` when no coroutine can be
    /// spawned (no OS threads), in which case the caller keeps its synchronous path.
    pub(crate) fn start_thread_async_function(
        &mut self,
        body: impl Into<Rc<FunctionBody>>,
        env: EnvHandle,
        strict: bool,
    ) -> Option<JsValue> {
        let body = body.into();
        let segment = Segment::capture(self, strict, true);
        let job = SendBody(Box::new(move |i: &mut Interpreter| -> Suspend {
            SEGMENT.with(|s| *s.borrow_mut() = Some(segment));
            enter_segment(i);
            let completion = match body.as_ref() {
                FunctionBody::Expr(expression) => i.eval_expr(expression, &env).map(Flow::Return),
                FunctionBody::Block(block) => {
                    if block.body.iter().any(stmt_declares_resource) {
                        i.using_stack.push(Vec::new());
                        let result = i.exec_stmts(&block.body, &env);
                        let frame = i.using_stack.pop().unwrap_or_default();
                        i.dispose_frame(frame, result)
                    } else {
                        i.exec_stmts(&block.body, &env)
                    }
                }
                _ => Ok(Flow::Normal),
            };
            leave_segment(i);
            thread_result_to_suspend(completion)
        }));
        let coro = spawn_coroutine(self as *mut Interpreter, job).ok()?;
        let promise = new_promise(self, PromiseFate::Pending);
        let JsValue::Object(target) = &promise else { unreachable!() };
        let state = Rc::new(RefCell::new(ThreadGeneratorState {
            coro: Some(coro),
            is_async: true,
            started: true,
            async_target: Some(target.clone()),
            async_queue: VecDeque::new(),
            async_request: None,
            async_executing: false,
            async_awaiting: false,
            async_await_return: false,
            body_realm: self.binding_current_realm(),
        }));
        if let Err(Throw(error)) = thread_async_function_step(self, state, Resume::Next(JsValue::Undefined)) {
            let error = self.normalize_throw(Throw(error)).0;
            let _ = settle_promise(target, PromiseFate::Rejected(error), self);
        }
        Some(promise)
    }
}

/// Drive a coroutine-backed async function one segment: resume it with `resume`, then act on
/// how it parked -- chain the next resumption onto an awaited value (Await: PromiseResolve +
/// PerformPromiseThen), or settle the function's promise on completion.
fn thread_async_function_step(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    resume: Resume,
) -> Result<(), Throw> {
    // The coroutine is taken out for the resume so no `RefCell` borrow is held while the body
    // runs (it may re-enter the driver through nested calls).
    let (mut coro, target) = {
        let mut guard = state.borrow_mut();
        match (guard.coro.take(), guard.async_target.clone()) {
            (Some(coro), Some(target)) => (coro, target),
            _ => return Ok(()),
        }
    };
    let suspend = coro.resume(resume);
    let done = coro.done();
    if !done {
        state.borrow_mut().coro = Some(coro);
    }
    match suspend {
        Suspend::Await(value) => {
            let fulfilled = thread_async_callback(i, state.clone(), false);
            let rejected = thread_async_callback(i, state, true);
            let promise = promise_object(&value)?;
            perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None)?;
            Ok(())
        }
        Suspend::Done(value) => vm::async_complete(i, &target, value),
        Suspend::Throw(error) => {
            let error = i.normalize_throw(Throw(error)).0;
            settle_promise(&target, PromiseFate::Rejected(error), i)
        }
        Suspend::Yield(_) | Suspend::YieldIteratorResult(_) => {
            let error = i.normalize_throw(Throw::type_error("yield in an async function")).0;
            settle_promise(&target, PromiseFate::Rejected(error), i)
        }
    }
}

/// The await reaction for a coroutine-backed async function: a native closure carrying the
/// function's state in its own environment (like the VM's `async_handler`).
fn thread_async_callback(i: &Interpreter, state: Rc<RefCell<ThreadGeneratorState>>, reject: bool) -> JsValue {
    let env = EnvHandle::new(EnvKind::Function, Some(i.global_env.clone()));
    let mut holder = JsObject::ordinary();
    holder.exotic = Exotic::ThreadGenerator(state);
    let _ = env.declare_var("[[ThreadAsyncState]]", JsValue::object(holder));
    let mut function = JsObject::function(JsFunction {
        name: if reject { "awaitRejected" } else { "awaitFulfilled" }.into(),
        params: vec![],
        body: FunctionBody::Native(if reject { thread_async_rejected } else { thread_async_fulfilled }),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        env,
        home: None,
        proto_for_new: None,
    });
    function.proto = Some(i.function_proto.clone());
    JsValue::object(function)
}

fn thread_async_callback_state(i: &Interpreter) -> Result<Rc<RefCell<ThreadGeneratorState>>, Throw> {
    let native = i.native_stack.last().ok_or_else(|| Throw::type_error("missing async callback"))?;
    let env = native.borrow().func.as_ref().unwrap().env.clone();
    let Some(JsValue::Object(holder)) = env.resolve("[[ThreadAsyncState]]") else {
        return Err(Throw::type_error("missing async function state"));
    };
    let state = match &holder.borrow().exotic {
        Exotic::ThreadGenerator(state) => state.clone(),
        _ => return Err(Throw::type_error("invalid async function state")),
    };
    Ok(state)
}

fn thread_async_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = thread_async_callback_state(i)?;
    thread_async_function_step(i, state, Resume::Next(args.into_iter().next().unwrap_or(JsValue::Undefined)))?;
    Ok(JsValue::Undefined)
}

fn thread_async_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = thread_async_callback_state(i)?;
    thread_async_function_step(i, state, Resume::Throw(args.into_iter().next().unwrap_or(JsValue::Undefined)))?;
    Ok(JsValue::Undefined)
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
    fn async_parameter_abrupt_completions_reject_with_real_errors() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for function in [
                "async function f(a=a){body++;}",
                "async function f(a=b,b=0){body++;}",
                "async (a=a)=>{body++;}",
                "async (a=b,b=0)=>{body++;}",
                "({async m(a=a){body++;}}).m",
                "({async m(a=b,b=0){body++;}}).m",
                "(class {static async m(a=a){body++;}}).m",
                "(class {static async m(a=b,b=0){body++;}}).m",
                "(new (class {async m(a=a){body++;}})).m",
                "(new (class {async m(a=b,b=0){body++;}})).m",
                // Explicit test control keeps the coroutine body path distinct.
                "async function f(a=a){with({}){} body++;}",
                "async function f(a=b,b=0){with({}){} body++;}",
            ] {
                let mut i = Interpreter::new();
                i.force_ast_callables = function.contains("with");
                let source = format!(r#"
                    var body=0, seen=0, synchronous=false, trace=[];
                    var f=({function});
                    try {{
                        var p=f();
                        trace.push(p instanceof Promise);
                        p.then(()=>seen=-1,e=>{{seen=e instanceof ReferenceError ? 1 : -2;trace.push('reject');}});
                    }} catch(e) {{synchronous=true;}}
                    trace.push('returned');
                "#);
                i.run_script_with_backend(&source, backend).unwrap();
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("!synchronous && body===0 && seen===1 && trace.join(',')==='true,returned,reject'").unwrap(), JsValue::Bool(true), "{backend:?}: {function}");
            }
            for parameters in ["{x}=null", "[x]=null", "{[null.key]:x}={}"] {
                let mut i = Interpreter::new();
                let source = format!("var body=0,result=false;async function f({parameters}){{body++;}} f().then(()=>{{}},e=>result=e instanceof TypeError);");
                i.run_script_with_backend(&source, backend).unwrap();
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("result && body===0").unwrap(), JsValue::Bool(true), "{backend:?}: {parameters}");
            }
        }
    }

    #[test]
    fn async_parameter_rejection_preserves_identity_context_and_sync_call_boundaries() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend(r#"
                var sentinel=Object.create(null), reasons=[], trace=[], body=0;
                function fail(){trace.push('default');throw sentinel;}
                var iterable={[Symbol.iterator](){return {next(){throw sentinel;}}}};
                var object={get x(){throw sentinel;}};
                var functions=[
                    async function(a=fail()){body++;},
                    async function({x}=object){body++;},
                    async function([x]=iterable){body++;},
                    async function(a=fail()){with({}){} body++;}
                ];
                for(var f of functions){f().then(()=>reasons.push(false),e=>reasons.push(e===sentinel));}
                var receiver={tag:17,async method(a=this.tag,b=arguments[0]){return a===17&&b===undefined;}};
                receiver.method().then(v=>reasons.push(v));
                var synchronous=0;
                for(var g of [function(a=a){},function*(a=a){},async function*(a=a){}]){
                    try{g();}catch(e){if(e instanceof ReferenceError)synchronous++;}
                }
                trace.push('returned');
            "#, backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("body===0 && synchronous===3 && reasons.length===5 && reasons.every(Boolean) && trace.join(',')==='default,default,returned'").unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn promise_error_boundaries_normalize_internal_errors_and_preserve_user_values() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend(r#"
                var checks=[],sentinel=Object.create(null);
                for(var method of ['all','race','allSettled','any']){
                    class C extends Promise{}
                    C.resolve=1;
                    C[method]([]).catch(e=>checks.push(e instanceof TypeError));
                    Promise[method](null).catch(e=>checks.push(e instanceof TypeError));
                    Promise[method]({[Symbol.iterator](){return {next(){return null;}}}})
                        .catch(e=>checks.push(e instanceof TypeError));
                    Object.defineProperty(C,'resolve',{get(){throw sentinel;}});
                    C[method]([]).catch(e=>checks.push(e===sentinel));
                    Promise[method]({[Symbol.iterator](){throw sentinel;}})
                        .catch(e=>checks.push(e===sentinel));
                }
                var cycle=Promise.resolve().then(()=>cycle);
                cycle.catch(e=>checks.push(e instanceof TypeError));
                var userError=new TypeError('user');
                Object.setPrototypeOf(userError,null);
                Promise.reject(userError).catch(e=>checks.push(e===userError && Object.getPrototypeOf(e)===null));
            "#, backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("checks.length===22 && checks.every(Boolean)").unwrap(), JsValue::Bool(true), "{backend:?}");

            let legacy = new_promise(&i, PromiseFate::Pending);
            let JsValue::Object(target) = &legacy else { unreachable!(); };
            assert!(adopt_promise_result(target, &legacy, &mut i).unwrap());
            let error = match &target.borrow().exotic {
                Exotic::Promise(state) => match &state.state {
                    PromiseFate::Rejected(error) => error.clone(),
                    _ => panic!("legacy cycle did not reject"),
                },
                _ => unreachable!(),
            };
            let JsValue::Object(error) = error else { panic!("legacy cycle error was not an object"); };
            let prototype = i.run_script("TypeError.prototype").unwrap();
            assert_eq!(error.borrow().proto.clone().map(JsValue::Object), Some(prototype), "{backend:?}");
        }
    }

    #[test]
    fn instance_identity_uses_prototypes_instead_of_forgeable_names() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var valid=true;
                for(var C of [Error,EvalError,RangeError,ReferenceError,SyntaxError,TypeError,URIError,Array,Function]){
                    valid=valid && !({name:C.name} instanceof C);
                    var inherited=Object.create(C.prototype);
                    inherited.name='unrelated';
                    valid=valid && inherited instanceof C;
                }
                function Named(){}
                valid=valid && !({name:'Named'} instanceof Named) && new Named() instanceof Named;
                valid;
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn async_parameter_errors_keep_the_callee_realm_across_wrappers() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let parent = i.binding_current_realm();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            let function = i.binding_with_realm(child, |i| i.run_script_with_backend("(async function foreign(a=a){})", backend)).unwrap();
            let child_error = i.binding_eval_in_realm(child, "ReferenceError").unwrap();
            let child_promise = i.binding_eval_in_realm(child, "Promise").unwrap();
            i.global_env.declare_var("foreign", function).unwrap();
            i.global_env.declare_var("ChildReferenceError", child_error).unwrap();
            i.global_env.declare_var("ChildPromise", child_promise).unwrap();
            let function = i.global_env.resolve("foreign").unwrap();
            let promise = i.call_value(&function, JsValue::Undefined, vec![]).unwrap();
            let child_promise_proto = i.binding_eval_in_realm(child, "Promise.prototype").unwrap();
            let child_error_proto = i.binding_eval_in_realm(child, "ReferenceError.prototype").unwrap();
            let JsValue::Object(promise) = promise else { panic!("async call did not return an object"); };
            assert_eq!(promise.borrow().proto.clone().map(JsValue::Object), Some(child_promise_proto), "{backend:?}: promise realm");
            let error = match &promise.borrow().exotic {
                Exotic::Promise(state) => match &state.state {
                    PromiseFate::Rejected(error) => error.clone(),
                    state => panic!("expected parameter rejection, got {state:?}"),
                },
                _ => panic!("async call did not return a Promise"),
            };
            let JsValue::Object(error) = error else { panic!("TDZ did not reject with an object"); };
            assert_eq!(error.borrow().proto.clone().map(JsValue::Object), Some(child_error_proto), "{backend:?}: error realm");
            i.run_script_with_backend(r#"
                var checked=0, sameRealm=true;
                for(var f of [foreign,foreign.bind(null),new Proxy(foreign,{})]){
                    var p=f();
                    sameRealm=sameRealm && p instanceof ChildPromise && !(p instanceof Promise);
                    p.catch(e=>{if(e instanceof ChildReferenceError && !(e instanceof ReferenceError))checked++;});
                }
            "#, backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("sameRealm+':'+checked").unwrap(), JsValue::str("true:3"), "{backend:?}");
            assert_eq!(i.binding_current_realm(), parent);
        }
    }

    #[test]
    fn ast_await_uses_internal_promise_reactions() {
        for source in [
            "var result=0,reads=0;var p=Promise.resolve(7);Object.defineProperty(p,'then',{get(){reads++;throw 99}});async function f(){with({}){} return await p} f().then(v=>result=v);",
            "var result=0,reads=0,resolve;var p=new Promise(r=>resolve=r);Object.defineProperty(p,'then',{get(){reads++;throw 99}});async function f(){with({}){} return await p} f().then(v=>result=v);resolve(7);",
            "var result=0,reads=0;var p=Promise.reject(7);Object.defineProperty(p,'then',{get(){reads++;throw 99}});async function f(){with({}){} try{await p}catch(v){return v}} f().then(v=>result=v);",
            "var result=0,reads=0;var p=Promise.resolve(7);Object.defineProperty(Promise,Symbol.species,{get(){reads++;throw 99}});async function f(){with({}){} result=await p} f();",
        ] {
            let mut i = Interpreter::new();
            i.force_ast_callables = true;
            // Select the coroutine-owned consumer independently of syntax.
            i.run_script_with_backend(source, ExecutionBackend::TreeWalk).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("result===7 && reads===0").unwrap(), JsValue::Bool(true), "{source}");
        }
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
