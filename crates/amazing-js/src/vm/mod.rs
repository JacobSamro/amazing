// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// declare_block_lexicals/annexb_fn_sync_eval) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//! Experimental register VM. Compilation rejects unsupported syntax atomically;
//! execution never falls back to AST statement/expression evaluation.
#![forbid(unsafe_code)]

mod async_delegate;
mod async_from_sync;
mod compiler;
mod fast_path;
mod resources;
mod patterns;
#[cfg(test)]
mod tests;

use super::{callable_function, for_iteration_env, new_promise, perform_promise_then, promise_resolving_functions, reference_name, settle_promise, Interpreter, Reference};
use crate::ast::{BinOp, ForLeft, UnOp, VarKind};
use crate::env::{EnvHandle, EnvKind};
use crate::tokens::{Numeric, Span};
use crate::value::{DerivedConstructor, Exotic, FunctionBody, JsFunction, JsObject, JsString, JsValue, Obj, PromiseFate, Property, PropertyKey, Throw, WellKnownSymbol};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

thread_local! {
    /// Reuse frame register storage across calls; values are cleared at return.
    static REGISTER_POOL: RefCell<Vec<Vec<JsValue>>> = const { RefCell::new(Vec::new()) };
}

fn take_registers(count: usize) -> Vec<JsValue> {
    let mut registers = REGISTER_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        pool.iter().position(|registers| registers.capacity() >= count)
            .map(|index| pool.swap_remove(index))
            .unwrap_or_default()
    });
    registers.resize(count, JsValue::Undefined);
    registers
}

pub use compiler::compile;
pub(crate) use compiler::compile_class_method;
pub(crate) use compiler::{compile_arrow_item, compile_function_item};
#[allow(unused_imports)] // Re-exported for the runtime async bridge in interp.rs.
pub(crate) use compiler::{compile_async_body, compile_async_generator_body, compile_generator_body};

pub const DEFAULT_INSTRUCTION_LIMIT: usize = 10_000_000;
// Ordinary callables previously used the unmetered AST path. A larger internal
// per-call budget admits substantial finite loops; explicit Program.run fuel
// remains caller-controlled, and coroutine budgets are unchanged.
const ORDINARY_FUNCTION_INSTRUCTION_LIMIT: usize = 100_000_000;

pub(crate) fn ordinary_functions_enabled() -> bool {
    // Keep an independent AST-factory baseline for rollout gates/profiling.
    std::env::var_os("AMAZING_JS_AST_FUNCTIONS").is_none()
}
type Register = usize;

/// This is a capability error, not a JavaScript syntax error. No code has run.
#[derive(Debug, Clone)]
pub struct CompileError {
    pub span: Span,
    pub message: &'static str,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bytecode unsupported: {} at {:?}",
            self.message, self.span
        )
    }
}
impl std::error::Error for CompileError {}

#[derive(Debug, Clone, Copy)]
enum Condition {
    Truthy,
    Falsy,
    Nullish,
    Undefined,
}

#[derive(Debug, Clone)]
enum KeyOperand {
    Static(JsString),
    Dynamic(Register),
}

#[derive(Debug)]
enum CallArgument {
    Value(Register),
    Spread(usize),
}

#[derive(Debug)]
struct ClassMethodCode {
    key: KeyOperand,
    kind: crate::ast::MethodKind,
    code: FunctionCode,
    // A `static` method/accessor installs on the constructor itself (with
    // the constructor as its [[HomeObject]], for `super` inside it), not on
    // `prototype` -- this was never read before JSVM-7 found it (every
    // method compiled as if it were an instance member; `class C { static
    // m(){} }`'s `C.m` was `undefined` under the VM). See `MakeClass`.
    is_static: bool,
}

/// A compiled class's super-class expression, plus whether the constructor
/// was synthesized (no explicit `constructor` member): an implicit derived
/// constructor forwards its own arguments to the super call.
#[derive(Debug)]
struct ClassSuper {
    register: Register,
    implicit: bool,
}

#[derive(Debug)]
struct ClassCode {
    constructor: FunctionCode,
    inferred_name: Option<KeyOperand>,
    methods: Vec<ClassMethodCode>,
    super_class: Option<ClassSuper>,
}

fn expand_arguments(
    registers: &[JsValue],
    spreads: &[Option<Vec<JsValue>>],
    arguments: &[CallArgument],
) -> Result<Vec<JsValue>, Throw> {
    let mut values = Vec::new();
    for argument in arguments {
        match argument {
            CallArgument::Value(register) => values.push(registers[*register].clone()),
            CallArgument::Spread(slot) => values.extend(spreads[*slot].as_ref()
                .ok_or_else(|| Throw::type_error("missing VM spread capture"))?.iter().cloned()),
        }
    }
    Ok(values)
}

impl KeyOperand {
    fn resolve(&self, registers: &[JsValue]) -> Result<PropertyKey, Throw> {
        match self {
            Self::Static(key) => Ok(PropertyKey::String(key.clone())),
            Self::Dynamic(register) => match &registers[*register] {
                JsValue::Str(key) => Ok(PropertyKey::String(key.clone())),
                JsValue::Symbol(key) => Ok(PropertyKey::Symbol(key.clone())),
                _ => Err(Throw::type_error("invalid VM property key")),
            },
        }
    }
}

#[derive(Debug)]
enum Instruction {
    InitBlockFunctions(Vec<Rc<crate::ast::FunctionItem>>),
    CopyBlockFunction(String),
    NoOp,
    Constant(Register, JsValue),
    Numeric(Register, Numeric),
    RegExp(Register, Rc<amazing_regex::Regex>),
    ToKey(Register),
    LoadName(Register, String, bool),
    LoadCallName(Register, Register, String),
    LoadThis(Register),
    ResolveName(usize, Register, String),
    ResolveWriteName(usize, String),
    RequireResolved(usize),
    GetMember(Register, Register, KeyOperand),
    ResolveMember(usize, Register, Register, KeyOperand),
    StoreReference(usize, Register),
    StorePatternReference(usize, Register),
    ResolveMemberWrite(usize, Register, KeyOperand),
    ResolveRawMemberWrite(usize, Register, Register),
    NewObject(Register),
    SetObjectProperty(Register, KeyOperand, Register),
    /// A non-computed literal `__proto__: value` property definition sets
    /// [[Prototype]] instead of creating an own property (`B.3.1`).
    SetPrototype(Register, Register),
    /// Throws TypeError if the register holds null/undefined. Emitted before
    /// a member access's key conversion so a nullish base is rejected before
    /// `ToPropertyKey` gets a chance to run user code, matching the tree
    /// walker's `resolve_reference` ordering.
    RequireObjectCoercible(Register),
    /// Brackets a class's heritage expression and each computed member key:
    /// both are always evaluated in strict mode, per spec, even when the
    /// surrounding code is sloppy (matching the tree walker's `eval_class`,
    /// which saves/restores `strict_eval_context` around the whole class).
    EnterStrict,
    ExitStrict,
    SetObjectMethod(Register, KeyOperand, FunctionCode, crate::ast::MethodKind),
    SpreadObject(Register, Register),
    NewArray(Register),
    SetArrayElement(Register, usize, Register),
    SetArrayLength(Register, usize),
    AppendArrayElement(Register, Register, Option<Register>),
    AppendArraySpread(Register, Register, Register),
    SetArrayLengthFrom(Register, Register),
    CaptureSpread(usize, Register),
    TemplateAppend(Register, Vec<u16>, Option<Register>),
    Declare(String, Register, VarKind),
    ReserveBinding(String, bool),
    NamedEvaluation(Register, Rc<crate::ast::Expr>, String),
    NamedPropertyEvaluation(Register, Rc<crate::ast::Expr>, KeyOperand),
    Unary(Register, UnOp, Register),
    Binary(Register, BinOp, Register, Register),
    Move(Register, Register),
    Jump(usize),
    Branch(Register, Condition, usize),
    /// `break`/`continue` (JSVM-10): unlike a plain `Jump`, goes through
    /// `route_abrupt` so a jump that actually leaves an active try/finally
    /// (including a for-of/for-await-of's `IteratorClose` finally) runs it
    /// first, per spec. `(target, scope_depth)`: the compile-time-resolved
    /// target and the frame's `scopes` depth there, so every block/iteration
    /// scope the jump leaves is popped (a `break` out of `{ let x; ... }`
    /// never runs that block's `LeaveScope`).
    ExitTo(usize, usize),
    EnterScope,
    EnterObjectScope(Register),
    LeaveScope,
    PushTry {
        catch: Option<usize>,
        finally: Option<usize>,
        end: usize,
    },
    EnterCatch(Option<String>),
    TryNormal,
    EndFinally,
    Complete(Register),
    Return(Register),
    MakeFunction(Register, FunctionCode),
    MakeNamedFunction(Register, FunctionCode, String),
    MakeClass(Register, ClassCode),
    Call(Register, Register, Register, Vec<CallArgument>),
    Construct(Register, Register, Vec<CallArgument>),
    Throw(Register),
    RejectCallTarget,
    Yield(Register, Register),
    YieldDelegate(Register, Register),
    /// `yield*` in an async generator: awaits each inner result (see `async_delegate`).
    YieldDelegateAsync(Register, Register),
    Await(Register, Register),
    AsyncIteratorStart(usize, Register),
    AsyncIteratorNext(Register, usize),
    AsyncIteratorNextAwait(Register, usize),
    AsyncIteratorBeginBody(usize),
    AsyncIteratorNeedsClose(Register, usize),
    AsyncIteratorDone(Register, usize, Register),
    AsyncIteratorValue(Register, Register),
    AsyncIteratorClose(Register, usize),
    AsyncIteratorCloseAwait(Register, usize),
    AsyncIteratorCloseCheck(Register),
    /// Plain (sync-only) `for-of`'s iterator start (JSVM-10): unlike
    /// `AsyncIteratorStart`, never tries `@@asyncIterator` -- a sync `for-of`
    /// must use `@@iterator` even when both are present. Reuses
    /// `AsyncIteratorState`'s `Iterator`/`Values` shape (and the existing
    /// `AsyncIteratorNext`/`Done`/`Value`/`Close`/`CloseCheck` instructions
    /// unchanged) since the sync half of that state machine is exactly a
    /// plain iterator's.
    IteratorStart(usize, Register),
    /// `for-in`'s enumeration start (JSVM-10): builds a `ForInState` from the
    /// tree walker's own `array_receiver`/prototype-walk rules, `None` for a
    /// nullish right-hand side (the loop body then never runs).
    ForInStart(usize, Register),
    /// `for-in`'s per-iteration step: `next_for_in_key` into `value` (a
    /// string), `done` set once the key chain is exhausted.
    ForInNext(Register, Register, usize),
    BindFor(ForLeft, Register),
    PatternStart(usize, Register),
    PatternStep(Register, usize),
    PatternSkip(usize),
    PatternRest(Register, usize),
    PatternClose(usize),
    ObjectRest(Register, Register, Vec<KeyOperand>),
    ResourceStart(usize),
    ResourceCapture(usize, Register, bool),
    ResourceBeginDispose(usize),
    ResourceNext(Register, Register, Register, usize),
    ResourceError(usize, Register),
    ResourceFinish(usize),
    EnterCatchValue(Register),
    /// Private field read (JSVM-10): `#x` resolves lexically through the
    /// active PrivateEnvironment chain at *runtime* (`env.resolve_private`),
    /// not at compile time -- a private name closes over whichever class
    /// body lexically encloses this code, which the compiler (unlike the
    /// tree walker's env chain) has no static view of once it's compiling a
    /// nested function/closure body in isolation. `(dst, base, name)`.
    GetPrivateField(Register, Register, String),
    /// Private field reference, for plain/compound assignment and
    /// update expressions (`obj.#x = v`, `+=`, `++`) -- mirrors
    /// `ResolveMember`: reads the current value into `dst` and records a
    /// `Reference::Private` in `slot` for `StoreReference` to write back
    /// through (already generic over every `Reference` variant).
    /// `(slot, dst, base, name)`.
    ResolvePrivate(usize, Register, Register, String),
    /// `#x in obj` (`dst, base, name`); the general `Binary(_, BinOp::In, ..)`
    /// path never applies here since a bare `#x` isn't itself a compilable
    /// expression outside this one position (checked directly on the AST at
    /// compile time, not through a resolved register).
    HasPrivateField(Register, Register, String),
    /// Evaluate an expression the compiler cannot lower with the tree
    /// walker's `eval_expr`, against the frame's current environment. Only
    /// emitted for expressions with no `yield`/`await`, so the tree walker
    /// never needs to suspend; the enclosing body stays on the VM instead of
    /// falling back to a coroutine thread.
    TreeEval(Register, Rc<crate::ast::Expr>),
    TreeEvalNamedClass(Register, Rc<crate::ast::Expr>, KeyOperand),
    /// Replace the innermost block scope with a fresh one holding copies of
    /// the named `let` bindings (a `for (let ...;;)` iteration boundary).
    CopyScope(Vec<String>),
    /// `ResolvePrivate` without the read, for a plain `=` target.
    ResolvePrivateWrite(usize, Register, String),
    Update(Register, usize, Register, crate::ast::UpdateOp, bool),
    /// `new.target`: the active call's constructor target, or `undefined`
    /// for an ordinary call.
    LoadNewTarget(Register),
    /// `super` as the object of a member expression: the active method's
    /// `[[HomeObject]].[[GetPrototypeOf]]()`.
    LoadSuperHome(Register),
    /// `super(...)` in a derived constructor.
    SuperCall(Register, Vec<CallArgument>),
}

#[derive(Debug, Clone)]
pub struct FunctionCode(Rc<CompiledFunction>);

#[derive(Debug)]
struct CompiledFunction {
    name: String,
    params: Vec<crate::ast::Param>,
    body_lexical_names: Vec<String>,
    skip_implicit_arguments: bool,
    program: Program,
    is_arrow: bool,
    is_generator: bool,
    is_async: bool,
    strict: bool,
    is_method: bool,
}

#[derive(Debug)]
struct HoistedFunction {
    name: String,
    code: FunctionCode,
}

/// Immutable, compiler-produced code. No external bytecode loader is exposed.
#[derive(Debug)]
pub struct Program {
    code: Vec<Instruction>,
    statement_spans: Vec<(usize, crate::tokens::Span)>,
    registers: usize,
    references: usize,
    spread_slots: usize,
    hoisted: Vec<String>,
    functions: Vec<HoistedFunction>,
    is_function: bool,
    async_iterator_slots: usize,
    resource_slots: usize,
    for_in_slots: usize,
}

/// Mutable execution state owned by a heap allocation. The try stack is empty
/// until exception handlers are compiled, but belongs to the frame so it can
/// survive an instruction-boundary suspension with the other VM state.
struct VmFrame {
    // A resume can arrive through an unrelated next()/promise callback. Keep
    // the original method/arrow call context, not the resumer's context.
    call_context: Option<super::CallFrame>,
    strict_context: bool,
    registers: Vec<JsValue>,
    caches: Vec<Option<fast_path::InlineCache>>,
    references: Vec<Option<Reference>>,
    spreads: Vec<Option<Vec<JsValue>>>,
    scopes: Vec<EnvHandle>,
    env: EnvHandle,
    pc: usize,
    fuel: usize,
    try_stack: Vec<TryHandler>,
    completed: Option<JsValue>,
    yield_dst: Option<Register>,
    pending_resume: Option<Abrupt>,
    delegate: Option<DelegateState>,
    delegate_input: Option<(u8, JsValue)>,
    async_delegate: Option<async_delegate::AsyncDelegate>,
    await_dst: Option<Register>,
    /// Async generators: a `return(v)` resumption at a yield is awaiting `v`
    /// (AsyncGeneratorUnwrapYieldResumption) before it is delivered.
    await_return: bool,
    async_iterators: Vec<Option<AsyncIteratorState>>,
    pattern_iterators: Vec<Option<super::PatternIter>>,
    resources: Vec<Vec<resources::Resource>>,
    resource_errors: Vec<Option<Throw>>,
    resource_needs_await: Vec<bool>,
    resource_has_awaited: Vec<bool>,
    /// `for-in`'s enumeration state (JSVM-10): one slot per syntactic `for-in`
    /// in the compiled body, mirroring `async_iterators`'s per-slot shape.
    /// Reuses the tree walker's own `ForInState`/`next_for_in_key` (private
    /// items in `interp.rs`/`interp_property_access.rs`, visible here because
    /// `vm` is declared as a child module of `interp`) rather than
    /// re-deriving for-in's live-mutation-safe enumeration order.
    for_in_states: Vec<Option<super::ForInState>>,
}

enum AsyncIteratorState {
    Values { values: Vec<JsValue>, index: usize, exhausted: bool },
    Iterator { value: JsValue, next: JsValue, exhausted: bool, sync: bool },
}

impl Drop for VmFrame {
    fn drop(&mut self) {
        if self.registers.capacity() > 4096 { return; }
        let mut registers = std::mem::take(&mut self.registers);
        registers.clear();
        let _ = REGISTER_POOL.try_with(|pool| {
            let mut pool = pool.borrow_mut();
            if pool.len() < 16 { pool.push(registers); }
        });
    }
}

struct DelegateState {
    iterator: JsValue,
    array_index: Option<usize>,
}

enum FrameOutcome {
    Suspended,
    Yielded(JsValue),
    Awaited(JsValue),
    Completed(JsValue),
}

pub struct GeneratorState {
    code: FunctionCode,
    env: EnvHandle,
    frame: Option<Box<VmFrame>>,
    started: bool,
    executing: bool,
    completion: JsValue,
    async_promise: Option<Obj>,
    // Async generators only: a FIFO of outstanding `.next()`/`.return()`/
    // `.throw()` calls. Unlike a plain async function (one call, one backing
    // promise) or a plain generator (synchronous, one call in flight by
    // construction), a consumer may call an async generator's methods again
    // before a prior call's promise has settled -- e.g. while the body is
    // mid-`await`. Each request carries its own promise (`target`); requests
    // are drained one at a time as the previous one's step completes.
    queue: VecDeque<AsyncGeneratorRequest>,
    // Async generators only: true while `frame` is suspended at an `await`
    // (as opposed to idle, suspended at a `yield` and ready for the next
    // queued request). `executing` alone cannot tell the two apart: it is
    // false in both cases between microtasks. Without this, `drain` would
    // wrongly feed a newly queued request's resume value into a frame that
    // is actually still waiting on the awaited promise's own reaction --
    // that reaction (`async_generator_step_fulfilled`/`_rejected`) is the
    // only thing allowed to resume the frame while this is true, and it
    // clears it before doing so.
    awaiting: bool,
}

/// One queued `.next(value)`/`.return(value)`/`.throw(value)` call on an async
/// generator. `kind` matches `generator_resume`'s convention: 0=next,
/// 1=return, 2=throw. `target` is that call's own promise.
struct AsyncGeneratorRequest {
    kind: u8,
    value: JsValue,
    target: Obj,
}

impl std::fmt::Debug for GeneratorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratorState")
            .field("started", &self.started)
            .field("executing", &self.executing)
            .field("queued", &self.queue.len())
            .finish()
    }
}

enum Abrupt {
    Return(JsValue),
    Throw(Throw),
    /// `break`/`continue` (JSVM-10): jump to a compile-time-resolved `pc`,
    /// running any try/finally (including a for-of/for-await-of's
    /// `IteratorClose` finally) whose protected region the jump actually
    /// leaves, exactly like `Return`/`Throw` already do -- but, unlike
    /// those, a `Goto` may resolve *within* an active handler's own region
    /// (a `continue` back to a loop nested inside a `try`, or a `break` to
    /// just past that same loop but still inside the `try`), in which case
    /// no finally runs at all. See `route_abrupt`'s early-exit check.
    Goto(usize, usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TryPhase {
    Body,
    Catch,
    Finally,
}

struct TryHandler {
    catch: Option<usize>,
    finally: Option<usize>,
    /// The pc a `break`/`continue` target must be `>=` to still be within
    /// this handler's own protected region (JSVM-10) -- the pc right after
    /// `PushTry` itself, i.e. where its own body starts.
    start: usize,
    end: usize,
    scope_depth: usize,
    env: EnvHandle,
    phase: TryPhase,
    caught: Option<JsValue>,
    pending: Option<Abrupt>,
    saved_completion: Option<JsValue>,
    iterator_close: bool,
}

impl Program {
    pub fn instruction_count(&self) -> usize {
        self.code.len()
    }

    /// Executes with a per-invocation fuel budget. Existing bindings persist;
    /// temporary registers/scopes/references drop on success or error.
    /// Legacy user callbacks reached through coercion are not fuel-metered yet.
    pub fn run(&self, interpreter: &mut Interpreter, limit: usize) -> Result<JsValue, Throw> {
        let previous = interpreter.completion.clone();
        interpreter.completion = JsValue::Undefined;
        let env = interpreter.global_env.clone();
        let result = self.execute(interpreter, env, limit);
        interpreter.completion = previous;
        result
    }

    fn execute(&self, i: &mut Interpreter, env: EnvHandle, fuel: usize) -> Result<JsValue, Throw> {
        let mut frame = self.start_frame(i, env, fuel)?;
        match self.resume_frame(i, &mut frame, usize::MAX)? {
            FrameOutcome::Completed(value) => Ok(value),
            FrameOutcome::Suspended => unreachable!("unbounded VM slice suspended"),
            FrameOutcome::Yielded(_) => unreachable!("non-generator program yielded"),
            FrameOutcome::Awaited(_) => unreachable!("non-async program awaited"),
        }
    }

    fn start_frame(
        &self,
        i: &mut Interpreter,
        env: EnvHandle,
        fuel: usize,
    ) -> Result<Box<VmFrame>, Throw> {
        let mut frame = Box::new(VmFrame {
            call_context: i.frames.last().cloned(),
            strict_context: i.strict_eval_context,
            registers: take_registers(self.registers),
            caches: (0..self.code.len()).map(|_| None).collect(),
            references: (0..self.references).map(|_| None).collect(),
            spreads: (0..self.spread_slots).map(|_| None).collect(),
            scopes: Vec::new(),
            env,
            pc: 0,
            fuel,
            try_stack: Vec::new(),
            completed: None,
            yield_dst: None,
            pending_resume: None,
            delegate: None,
            delegate_input: None,
            async_delegate: None,
            await_dst: None,
            await_return: false,
            async_iterators: (0..self.async_iterator_slots).map(|_| None).collect(),
            pattern_iterators: (0..self.async_iterator_slots).map(|_| None).collect(),
            resources: (0..self.resource_slots).map(|_| Vec::new()).collect(),
            resource_errors: (0..self.resource_slots).map(|_| None).collect(),
            resource_needs_await: vec![false; self.resource_slots],
            resource_has_awaited: vec![false; self.resource_slots],
            for_in_states: (0..self.for_in_slots).map(|_| None).collect(),
        });
        for name in &self.hoisted {
            consume(&mut frame.fuel)?;
            frame.env.hoist_var(name)?;
        }
        for function in &self.functions {
            consume(&mut frame.fuel)?;
            let value = function.code.instantiate(i, &frame.env);
            frame.env.declare_var(&function.name, value)?;
        }
        Ok(frame)
    }

    fn resume_frame(
        &self,
        i: &mut Interpreter,
        frame: &mut Box<VmFrame>,
        quantum: usize,
    ) -> Result<FrameOutcome, Throw> {
        let frame_depth = i.frames.len();
        let previous_strict = i.strict_eval_context;
        if let Some(context) = &frame.call_context {
            // The initial synchronous resume still has its invocation on
            // the stack. Install a saved context only after it has left;
            // recursive invocations of the same function are distinct.
            if !i.frames.last().is_some_and(|active| active.context_id == context.context_id) {
                i.frames.push(context.clone());
            }
        }
        i.strict_eval_context = frame.strict_context;
        let result = self.resume_frame_inner(i, frame, quantum);
        i.frames.truncate(frame_depth);
        i.strict_eval_context = previous_strict;
        result
    }

    fn resume_frame_inner(
        &self,
        i: &mut Interpreter,
        frame: &mut Box<VmFrame>,
        quantum: usize,
    ) -> Result<FrameOutcome, Throw> {
        if let Some(value) = &frame.completed {
            return Ok(FrameOutcome::Completed(value.clone()));
        }
        if let Some(abrupt) = frame.pending_resume.take() {
            if let Some(value) = route_abrupt(
                &mut frame.try_stack, &mut frame.scopes, &mut frame.env,
                &mut frame.pc, i, abrupt,
            )? {
                frame.completed = Some(value.clone());
                return Ok(FrameOutcome::Completed(value));
            }
        }
        let VmFrame {
            call_context: _,
            strict_context: _,
            registers,
            caches,
            references,
            spreads,
            scopes,
            env,
            pc,
            fuel,
            try_stack,
            completed,
            yield_dst,
            pending_resume: _,
            delegate,
            delegate_input,
            async_delegate,
            await_dst,
            await_return: _,
            async_iterators,
            pattern_iterators,
            resources,
            resource_errors,
            resource_needs_await,
            resource_has_awaited,
            for_in_states,
        } = frame.as_mut();
        let mut steps = 0;
        while let Some(op) = self.code.get(*pc) {
            if steps == quantum {
                return Ok(FrameOutcome::Suspended);
            }
            consume(fuel)?;
            steps += 1;
            // Preserve statement breakpoints and depth stepping even when
            // a debugger attaches after this function was compiled. The
            // side table adds no opcodes or fuel cost to ordinary execution.
            if let Some(hook) = i.debug_hook.clone() {
                let start = self.statement_spans.partition_point(|(index, _)| *index < *pc);
                for (_, span) in self.statement_spans[start..].iter().take_while(|(index, _)| *index == *pc) {
                    hook.borrow_mut().on_statement(&i.current_script_id, &i.current_script_url, span.start.line, i.depth);
                }
            }
            *pc += 1;
            // PromiseResolve runs before leaving the suspended function's
            // context. The outcome carries that prepared Promise, not the
            // operand, so drivers never repeat its observable constructor read.
            let mut awaited_promise = None;
            let step = (|| -> Result<Option<Abrupt>, Throw> {
                match op {
                    Instruction::NoOp => {}
                    Instruction::Constant(dst, value) => registers[*dst] = value.clone(),
                    Instruction::Numeric(dst, number) => registers[*dst] = i.numeric(number)?,
                    Instruction::RegExp(dst, regex) => registers[*dst] = i.make_regexp(regex.clone())?,
                    Instruction::RequireObjectCoercible(register) => {
                        if registers[*register].is_null() || registers[*register].is_undefined() {
                            return Err(Throw::type_error("cannot access a property on nullish value"));
                        }
                    }
                    Instruction::EnterStrict => {
                        i.vm_strict_stack.push(i.strict_eval_context);
                        i.strict_eval_context = true;
                    }
                    Instruction::ExitStrict => {
                        i.strict_eval_context = i.vm_strict_stack.pop()
                            .unwrap_or(i.strict_eval_context);
                    }
                    Instruction::ToKey(register) => {
                        registers[*register] = match i.to_property_key(&registers[*register])? {
                            PropertyKey::String(key) => JsValue::Str(key),
                            PropertyKey::Symbol(key) => JsValue::Symbol(key),
                        };
                    }
                    Instruction::LoadName(dst, name, optional) => {
                        registers[*dst] = match i.identifier_value(&env, name)? {
                            Some(value) => value,
                            None if *optional => JsValue::Undefined,
                            None => {
                                return Err(Throw::reference_error(format!(
                                    "{name} is not defined"
                                )));
                            }
                        };
                    }
                    Instruction::LoadCallName(dst, receiver, name) => {
                        let (value, this) = i.scoped_call(&env, name)?;
                        registers[*dst] = value;
                        registers[*receiver] = this;
                    }
                    // Checked, matching the tree walker's `ExprKind::This`:
                    // `this` is a ReferenceError before a derived
                    // constructor's `super()` call has run.
                    Instruction::LoadThis(dst) => registers[*dst] = env.resolve_this_checked()?,
                    Instruction::LoadNewTarget(dst) => {
                        registers[*dst] = i.current_new_target().unwrap_or(JsValue::Undefined);
                    }
                    Instruction::LoadSuperHome(dst) => {
                        // `[[HomeObject]]` of the active frame; not exposed as
                        // a shared accessor (only `current_new_target` is),
                        // so vm/mod.rs -- a child module of interp.rs -- reads
                        // the private `frames` field directly.
                        let home = i.frames.last().and_then(|frame| frame.home.clone())
                            .ok_or_else(|| Throw::syntax_error("super used outside a method"))?;
                        let prototype = home.borrow().proto.clone()
                            .ok_or_else(|| Throw::type_error("super prototype is null"))?;
                        registers[*dst] = JsValue::Object(prototype);
                    }
                    Instruction::SuperCall(dst, arguments) => {
                        let super_constructor = i.frames.last().and_then(|frame| frame.super_constructor.clone())
                            .ok_or_else(|| Throw::syntax_error("super() used outside a derived constructor"))?;
                        let new_target = i.current_new_target()
                            .ok_or_else(|| Throw::type_error("derived constructor requires new"))?;
                        let args = expand_arguments(&registers, spreads, arguments)?;
                        let result = i.construct_with_new_target(&super_constructor, args, new_target)?;
                        env.initialize_this(result.clone())?;
                        if let Some(frame) = i.frames.last_mut() {
                            frame.this = result.clone();
                        }
                        let elements = i.frames.last().and_then(|frame| frame.own_function.clone())
                            .and_then(|function| function.borrow().class_elements.clone());
                        if let Some(elements) = elements {
                            super::initialize_instance_elements(i, &result, &elements)?;
                        }
                        registers[*dst] = result;
                    }
                    Instruction::ResolveName(slot, dst, name) => {
                        let (reference, value) = i.name_reference_value(&env, name)?;
                        references[*slot] = Some(reference);
                        registers[*dst] = value;
                    }
                    Instruction::ResolveWriteName(slot, name) => {
                        references[*slot] = Some(i.name_reference(&env, name)?);
                    }
                    Instruction::RequireResolved(slot) => match &references[*slot] {
                        Some(Reference::Unresolvable(_, _)) => {
                            return Err(Throw::reference_error(
                                "assignment to an unresolved binding",
                            ));
                        }
                        Some(_) => {}
                        None => return Err(Throw::type_error("invalid VM reference lifetime")),
                    },
                    Instruction::GetMember(dst, object, key) => {
                        let key = key.resolve(&registers)?;
                        registers[*dst] = fast_path::get(i, &registers[*object], key, &mut caches[*pc - 1])?;
                    }
                    Instruction::ResolveMember(slot, dst, object, key) => {
                        let key = key.resolve(&registers)?;
                        let receiver = registers[*object].clone();
                        registers[*dst] = fast_path::get(i, &receiver, key.clone(), &mut caches[*pc - 1])?;
                        references[*slot] = Some(Reference::Property(receiver, key));
                    }
                    Instruction::GetPrivateField(dst, base, name) => {
                        let private = env.resolve_private(name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{name} is not defined"))
                        })?;
                        registers[*dst] = super::private_field_get(i, &registers[*base], &private)?;
                    }
                    Instruction::ResolvePrivate(slot, dst, base, name) => {
                        let private = env.resolve_private(name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{name} is not defined"))
                        })?;
                        let receiver = registers[*base].clone();
                        registers[*dst] = super::private_field_get(i, &receiver, &private)?;
                        references[*slot] = Some(Reference::Private(receiver, private));
                    }
                    Instruction::ResolvePrivateWrite(slot, base, name) => {
                        let private = env.resolve_private(name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{name} is not defined"))
                        })?;
                        references[*slot] = Some(Reference::Private(registers[*base].clone(), private));
                    }
                    Instruction::TreeEval(dst, expression) => {
                        registers[*dst] = i.eval_expr(expression, env)?;
                    }
                    Instruction::TreeEvalNamedClass(dst, expression, key) => {
                        let name = Interpreter::property_function_name(&key.resolve(&registers)?);
                        registers[*dst] = i.eval_named_class_expr(expression, env, &name)?;
                    }
                    Instruction::HasPrivateField(dst, base, name) => {
                        let private = env.resolve_private(name).ok_or_else(|| {
                            Throw::syntax_error(format!("private name #{name} is not defined"))
                        })?;
                        registers[*dst] = JsValue::Bool(super::private_field_has(&registers[*base], &private)?);
                    }
                    Instruction::StoreReference(slot, src) | Instruction::StorePatternReference(slot, src) => {
                        let mut reference = references[*slot]
                            .take()
                            .ok_or_else(|| Throw::type_error("invalid VM reference lifetime"))?;
                        let value = registers[*src].clone();
                        if matches!(op, Instruction::StoreReference(..)) {
                            i.coerce_reference_key(&mut reference)?;
                        }
                        match reference {
                            Reference::Property(receiver, key) => {
                                fast_path::set(i, &receiver, key, value, &env, &mut caches[*pc - 1])?;
                            }
                            other => i.put_reference(other, value, &env)?,
                        }
                    }
                    Instruction::ResolveMemberWrite(slot, base, key) => {
                        references[*slot] = Some(Reference::Property(registers[*base].clone(), key.resolve(registers)?));
                    }
                    Instruction::ResolveRawMemberWrite(slot, base, key) => {
                        references[*slot] = Some(Reference::RawProperty(registers[*base].clone(), registers[*key].clone()));
                    }
                    Instruction::Update(dst, slot, old, op, prefix) => {
                        let reference = references[*slot]
                            .take()
                            .ok_or_else(|| Throw::type_error("invalid VM update reference"))?;
                        let previous = i.to_numeric_value(registers[*old].clone())?;
                        let next = i.update_value(*op, previous.clone())?;
                        i.put_reference(reference, next.clone(), &env)?;
                        registers[*dst] = if *prefix { next } else { previous };
                    }
                    Instruction::NewObject(dst) => {
                        registers[*dst] = i.object_value();
                        if let JsValue::Object(object) = &registers[*dst] {
                            object.borrow_mut().vm_cacheable = true;
                        }
                    }
                    Instruction::SetObjectProperty(object, key, src) => {
                        let key = key.resolve(&registers)?;
                        let JsValue::Object(target) = &registers[*object] else { unreachable!() };
                        target.borrow_mut().define(&key, Property::data(registers[*src].clone()))
                            .map_err(|_| Throw::type_error("could not define object property"))?;
                    }
                    Instruction::SetPrototype(object, src) => {
                        let JsValue::Object(target) = &registers[*object] else { unreachable!() };
                        match &registers[*src] {
                            JsValue::Object(prototype) => target.borrow_mut().proto = Some(prototype.clone()),
                            JsValue::Null => target.borrow_mut().proto = None,
                            _ => {}
                        }
                    }
                    Instruction::SetObjectMethod(object, key, code, kind) => {
                        let key = key.resolve(&registers)?;
                        let JsValue::Object(target) = &registers[*object] else { unreachable!() };
                        let method = code.instantiate(i, env);
                        i.configure_object_method(&method, &registers[*object], &key, *kind, &code.0.params);
                        if matches!(kind, crate::ast::MethodKind::Get | crate::ast::MethodKind::Set) {
                            let mut property = target.borrow().own_prop(&key).cloned()
                                .filter(|property| property.getter.is_some() || property.setter.is_some())
                                .unwrap_or(Property {
                                    value: JsValue::Undefined, writable: false,
                                    enumerable: true, configurable: true,
                                    getter: None, setter: None,
                                });
                            if *kind == crate::ast::MethodKind::Get {
                                property.getter = Some(method.clone());
                            } else {
                                property.setter = Some(method.clone());
                            }
                            target.borrow_mut().define(&key, property)
                                .map_err(|_| Throw::type_error("could not define object accessor"))?;
                        } else {
                            target.borrow_mut().define(&key, Property::data(method))
                                .map_err(|_| Throw::type_error("could not define object property"))?;
                        }
                    }
                    Instruction::SpreadObject(target, source) => {
                        let source = registers[*source].clone();
                        let JsValue::Object(target) = &registers[*target] else { unreachable!() };
                        for key in i.enumerable_property_keys(&source)? {
                            let value = i.get_prop(&source, &key)?;
                            target.borrow_mut().define(&key, Property::data(value))
                                .map_err(|_| Throw::type_error("could not define object property"))?;
                        }
                    }
                    Instruction::NewArray(dst) => {
                        let mut array = JsObject::array();
                        array.proto = Some(i.array_proto.clone());
                        registers[*dst] = JsValue::object(array);
                    }
                    Instruction::SetArrayElement(array, index, src) => {
                        i.set_prop(
                            &registers[*array],
                            index.to_string(),
                            registers[*src].clone(),
                        )?;
                    }
                    Instruction::SetArrayLength(array, len) => {
                        i.set_prop(&registers[*array], "length", JsValue::num(*len as f64))?;
                    }
                    Instruction::AppendArrayElement(array, index, source) => {
                        let JsValue::Num(next) = &registers[*index] else { unreachable!() };
                        let next = *next as usize;
                        if let Some(source) = source {
                            i.set_prop(&registers[*array], next.to_string(), registers[*source].clone())?;
                        }
                        registers[*index] = JsValue::num((next + 1) as f64);
                    }
                    Instruction::AppendArraySpread(array, index, source) => {
                        let JsValue::Num(next) = &registers[*index] else { unreachable!() };
                        let mut next = *next as usize;
                        for value in i.iterate(&registers[*source])? {
                            i.set_prop(&registers[*array], next.to_string(), value)?;
                            next += 1;
                        }
                        registers[*index] = JsValue::num(next as f64);
                    }
                    Instruction::SetArrayLengthFrom(array, index) => {
                        i.set_prop(&registers[*array], "length", registers[*index].clone())?;
                    }
                    Instruction::CaptureSpread(slot, source) => {
                        spreads[*slot] = Some(i.iterate(&registers[*source])?);
                    }
                    Instruction::TemplateAppend(dst, quasi, expression) => {
                        let mut units = registers[*dst].as_str().expect("template accumulator").units().to_vec();
                        units.extend_from_slice(quasi);
                        if let Some(expression) = expression {
                            units.extend_from_slice(i.to_js_string(&registers[*expression])?.units());
                        }
                        registers[*dst] = JsValue::str_units(units);
                    }
                    Instruction::Declare(name, src, kind) => match kind {
                        VarKind::Var => {
                            if !env.assign(name, registers[*src].clone())? {
                                i.global_env.declare_var(name, registers[*src].clone())?;
                            }
                        }
                        VarKind::Let | VarKind::Const => {
                            env.declare_lexical(
                                name,
                                registers[*src].clone(),
                                *kind == VarKind::Let,
                            )?;
                        }
                    },
                    Instruction::ReserveBinding(name, mutable) => { env.declare_uninitialized(name, *mutable)?; }
                    Instruction::InitBlockFunctions(functions) => {
                        for function in functions {
                            if let Some(name) = &function.name {
                                let value = i.make_function(function, env);
                                if env.local_cell(&name.name).is_some() { env.assign(&name.name, value)?; }
                                else { env.declare_lexical(&name.name, value, true)?; }
                            }
                        }
                    }
                    Instruction::CopyBlockFunction(name) => {
                        let value = env.local_cell(name).ok_or_else(|| Throw::reference_error("missing block function"))?.read()?;
                        let scope = env.var_scope();
                        if !scope.assign(name, value.clone())? { scope.declare_var(name, value)?; }
                    }
                    Instruction::NamedEvaluation(dst, expression, name) => { i.named_evaluation(expression, &mut registers[*dst], name); }
                    Instruction::NamedPropertyEvaluation(dst, expression, key) => {
                        let name = Interpreter::property_function_name(&key.resolve(&registers)?);
                        i.named_evaluation_utf16(expression, &mut registers[*dst], &name);
                    }
                    Instruction::Unary(dst, op, src) => {
                        registers[*dst] = i.unary_value(*op, registers[*src].clone())?;
                    }
                    Instruction::Binary(dst, op, left, right) => {
                        registers[*dst] = match fast_path::binary(*op, &registers[*left], &registers[*right]) {
                            Some(value) => value,
                            None => i.binary(*op, registers[*left].clone(), registers[*right].clone())?,
                        };
                    }
                    Instruction::Move(dst, src) => registers[*dst] = registers[*src].clone(),
                    Instruction::Jump(target) => *pc = *target,
                    Instruction::Branch(src, condition, target) => {
                        let value = &registers[*src];
                        if match condition {
                            Condition::Truthy => value.to_boolean(),
                            Condition::Falsy => !value.to_boolean(),
                            Condition::Nullish => value.is_null() || value.is_undefined(),
                            Condition::Undefined => value.is_undefined(),
                        } {
                            *pc = *target;
                        }
                    }
                    Instruction::EnterScope => {
                        scopes.push(env.clone());
                        *env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    }
                    Instruction::EnterObjectScope(source) => {
                        let bound = super::native_object(i, JsValue::Undefined, vec![registers[*source].clone()])?;
                        let JsValue::Object(bound) = bound else {
                            return Err(Throw::type_error("with statement requires an object"));
                        };
                        scopes.push(env.clone());
                        *env = EnvHandle::new_object(bound, env.clone());
                    }
                    Instruction::CopyScope(names) => {
                        let outer = scopes.last().cloned()
                            .ok_or_else(|| Throw::type_error("invalid VM scope"))?;
                        let next = EnvHandle::new(EnvKind::Block, Some(outer));
                        for name in names {
                            next.declare_lexical(name, env.resolve(name).unwrap_or(JsValue::Undefined), true)?;
                        }
                        *env = next;
                    }
                    Instruction::LeaveScope => {
                        *env = scopes
                            .pop()
                            .ok_or_else(|| Throw::type_error("invalid VM scope"))?;
                    }
                    Instruction::PushTry {
                        catch,
                        finally,
                        end,
                    } => try_stack.push(TryHandler {
                        catch: *catch,
                        finally: *finally,
                        start: *pc,
                        end: *end,
                        scope_depth: scopes.len(),
                        env: env.clone(),
                        phase: TryPhase::Body,
                        caught: None,
                        pending: None,
                        saved_completion: None,
                        iterator_close: false,
                    }),
                    Instruction::EnterCatch(name) => {
                        let handler = try_stack
                            .last_mut()
                            .ok_or_else(|| Throw::type_error("invalid VM catch"))?;
                        let thrown = handler
                            .caught
                            .take()
                            .ok_or_else(|| Throw::type_error("missing VM catch value"))?;
                        scopes.push(env.clone());
                        *env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                        if let Some(name) = name {
                            env.declare_lexical(name, thrown, true)?;
                        }
                    }
                    Instruction::TryNormal => {
                        let handler = try_stack
                            .last_mut()
                            .ok_or_else(|| Throw::type_error("invalid VM try exit"))?;
                        if let Some(finally) = handler.finally {
                            handler.phase = TryPhase::Finally;
                            handler.saved_completion = Some(i.completion.clone());
                            *pc = finally;
                        } else {
                            let end = handler.end;
                            try_stack.pop();
                            *pc = end;
                        }
                    }
                    Instruction::EndFinally => {
                        let handler = try_stack
                            .pop()
                            .ok_or_else(|| Throw::type_error("invalid VM finally exit"))?;
                        if let Some(completion) = handler.saved_completion {
                            i.completion = completion;
                        }
                        if let Some(pending) = handler.pending {
                            return Ok(Some(pending));
                        }
                        *pc = handler.end;
                    }
                    Instruction::Complete(src) => i.completion = registers[*src].clone(),
                    Instruction::Return(src) => {
                        return Ok(Some(Abrupt::Return(registers[*src].clone())));
                    }
                    Instruction::ExitTo(target, depth) => {
                        return Ok(Some(Abrupt::Goto(*target, *depth)));
                    }
                    Instruction::MakeFunction(dst, code) => {
                        registers[*dst] = code.instantiate(i, &env);
                    }
                    Instruction::MakeNamedFunction(dst, code, name) => {
                        let named_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                        let value = code.instantiate(i, &named_env);
                        named_env.declare_function_name(name, value.clone())?;
                        registers[*dst] = value;
                    }
                    Instruction::MakeClass(dst, class) => {
                        let constructor = class.constructor.instantiate(i, &env);
                        let JsValue::Object(callable) = &constructor else { unreachable!() };
                        if let Some(key) = &class.inferred_name {
                            let name = Interpreter::property_function_name(&key.resolve(&registers)?);
                            let mut object = callable.borrow_mut();
                            object.func.as_mut().unwrap().name = name.to_utf8_lossy();
                            object.define("name", Property {
                                value: JsValue::str(name), writable: false, enumerable: false,
                                configurable: true, getter: None, setter: None,
                            }).map_err(|_| Throw::type_error("could not define class name"))?;
                        }
                        callable.borrow_mut().class_constructor = true;
                        let Some(JsValue::Object(prototype)) = callable.borrow().get_own("prototype") else {
                            return Err(Throw::type_error("class constructor has no prototype"));
                        };
                        // A class constructor throws TypeError when called
                        // without `new` (matching the tree walker, which sets
                        // this in its own class evaluation); this had never
                        // been set for a VM-compiled class of any kind.
                        callable.borrow_mut().class_constructor = true;
                        if let Some(super_class) = &class.super_class {
                            let super_value = registers[super_class.register].clone();
                            // Matches the tree walker's ClassDefinitionEvaluation: a
                            // non-object super class (including `extends null`)
                            // is rejected rather than given full null-prototype
                            // semantics, which the runtime does not model yet.
                            let super_prototype = match &super_value {
                                JsValue::Object(super_constructor) => match super_constructor.borrow().get_own("prototype") {
                                    Some(JsValue::Object(prototype)) => prototype,
                                    _ => return Err(Throw::type_error("super constructor has no prototype")),
                                },
                                _ => return Err(Throw::type_error("super class is not a constructor")),
                            };
                            // ClassDefinitionEvaluation: the constructor's own
                            // [[Prototype]] is the super constructor (static
                            // inheritance); the class prototype's is the super
                            // constructor's `prototype` (instance inheritance).
                            prototype.borrow_mut().proto = Some(super_prototype);
                            callable.borrow_mut().proto = match &super_value {
                                JsValue::Object(parent) => Some(parent.clone()),
                                _ => unreachable!(),
                            };
                            callable.borrow_mut().derived_constructor = Some(DerivedConstructor {
                                parent: super_value,
                                implicit: super_class.implicit,
                            });
                        }
                        // MakeConstructor(F, false, proto): a class constructor's
                        // `prototype` is non-writable/non-enumerable/non-configurable.
                        callable.borrow_mut().define("prototype", Property {
                            value: JsValue::Object(prototype.clone()),
                            writable: false,
                            enumerable: false,
                            configurable: false,
                            getter: None,
                            setter: None,
                        }).ok();
                        prototype.borrow_mut().define("constructor", Property {
                            value: constructor.clone(),
                            writable: true,
                            enumerable: false,
                            configurable: true,
                            getter: None,
                            setter: None,
                        }).map_err(|_| Throw::type_error("could not define class constructor"))?;
                        callable.borrow_mut().func.as_mut().unwrap().home = Some(prototype.clone());
                        for method in &class.methods {
                            let key = method.key.resolve(&registers)?;
                            let mut value = method.code.instantiate(i, &env);
                            let name = Interpreter::property_function_name(&key);
                            Interpreter::name_class_method(&mut value, &name, method.kind, &method.code.0.params);
                            let JsValue::Object(function) = &value else { unreachable!() };
                            // A `static` method/accessor is a property of the
                            // constructor itself, with the constructor (not
                            // the prototype) as its [[HomeObject]] -- matching
                            // the tree walker's `eval_class_inner`. Getting
                            // this wrong left every VM-compiled static method
                            // undefined on the constructor (JSVM-7 found it
                            // while isolating the generator/async-generator
                            // Test262 delta: this pre-existing gap was masked
                            // whenever a static method's own class also hit
                            // some other, unrelated fallback trigger, but a
                            // class with only a static method compiled clean
                            // and silently dropped it). `define_class_method`
                            // already takes an arbitrary target, so this reuses
                            // it instead of duplicating its accessor-merge logic.
                            let target = if method.is_static { &callable } else { &prototype };
                            function.borrow_mut().func.as_mut().unwrap().home = Some(target.clone());
                            i.define_class_method(&JsValue::Object(target.clone()), &key,
                                method.kind, value)?;
                        }
                        registers[*dst] = constructor;
                    }
                    Instruction::Call(dst, callee, this, arguments) => {
                        let args = expand_arguments(&registers, spreads, arguments)?;
                        registers[*dst] =
                            i.call_value(&registers[*callee], registers[*this].clone(), args)?;
                    }
                    Instruction::Construct(dst, callee, arguments) => {
                        let args = expand_arguments(&registers, spreads, arguments)?;
                        registers[*dst] = i.construct(&registers[*callee], args)?;
                    }
                    Instruction::RejectCallTarget => {
                        return Err(Throw::reference_error("function call is not an assignment reference"));
                    }
                    Instruction::Throw(src) => {
                        return Ok(Some(Abrupt::Throw(Throw(registers[*src].clone()))))
                    }
                    Instruction::Yield(dst, _) => {
                        *yield_dst = Some(*dst);
                        return Ok(None);
                    }
                    Instruction::YieldDelegate(dst, src) => {
                        if delegate.is_none() {
                            let iterable = registers[*src].clone();
                            let array_index = match &iterable {
                                JsValue::Object(object) if matches!(object.borrow().exotic, crate::value::Exotic::Array) => Some(0),
                                _ => None,
                            };
                            let iterator = if array_index.is_some() {
                                iterable
                            } else {
                                i.custom_iterator(&iterable)?.ok_or_else(|| Throw::type_error("yield* value is not iterable"))?
                            };
                            *delegate = Some(DelegateState { iterator, array_index });
                        }
                        let state = delegate.as_mut().unwrap();
                        let (kind, input) = delegate_input.take().unwrap_or((0, JsValue::Undefined));
                        if state.array_index.is_some() && kind != 0 {
                            *delegate = None;
                            return Ok(Some(if kind == 1 {
                                Abrupt::Return(input)
                            } else {
                                Abrupt::Throw(Throw::type_error("iterator has no throw method"))
                            }));
                        }
                        let result = if let Some(index) = state.array_index.as_mut() {
                            let length_value = i.get_prop(&state.iterator, "length")?;
                            let length = i.to_number(&length_value)? as usize;
                            if *index >= length {
                                generator_result(i, JsValue::Undefined, true)
                            } else {
                                let value = i.get_prop(&state.iterator, index.to_string())?;
                                *index += 1;
                                generator_result(i, value, false)
                            }
                        } else {
                            let method = i.get_prop(&state.iterator, match kind { 1 => "return", 2 => "throw", _ => "next" })?;
                            if method.is_undefined() {
                                *delegate = None;
                                return Ok(Some(if kind == 1 {
                                    Abrupt::Return(input)
                                } else {
                                    Abrupt::Throw(Throw::type_error(if kind == 2 { "iterator has no throw method" } else { "iterator has no next method" }))
                                }));
                            }
                            i.call_value(&method, state.iterator.clone(), vec![input])?
                        };
                        if !result.is_object() {
                            return Err(Throw::type_error("iterator result is not an object"));
                        }
                        let done = i.get_prop(&result, "done")?.to_boolean();
                        let value = i.get_prop(&result, "value")?;
                        if done {
                            *delegate = None;
                            if kind == 1 {
                                return Ok(Some(Abrupt::Return(value)));
                            }
                            registers[*dst] = value;
                        } else {
                            registers[*dst] = value;
                            *yield_dst = Some(*dst);
                            *pc -= 1;
                        }
                    }
                    Instruction::YieldDelegateAsync(dst, src) => {
                        let (kind, input) = delegate_input.take().unwrap_or((0, JsValue::Undefined));
                        if async_delegate.is_none() {
                            *async_delegate = Some(async_delegate::start(i, &registers[*src])?);
                        }
                        let step = async_delegate::step(i, async_delegate.as_mut().unwrap(), kind, input);
                        match step {
                            Ok(async_delegate::AsyncDelegateStep::Yield(value)) => {
                                registers[*dst] = value;
                                *yield_dst = Some(*dst);
                                *pc -= 1;
                            }
                            Ok(async_delegate::AsyncDelegateStep::Await(value)) => {
                                let promise = match async_resolve_value_checked(i, value) {
                                    Ok(promise) => promise,
                                    Err(error) => { *async_delegate = None; return Err(error); }
                                };
                                awaited_promise = Some(promise);
                                *await_dst = Some(*dst);
                                *pc -= 1;
                            }
                            Ok(async_delegate::AsyncDelegateStep::Complete(value)) => {
                                *async_delegate = None;
                                registers[*dst] = value;
                            }
                            Ok(async_delegate::AsyncDelegateStep::Return(value)) => {
                                *async_delegate = None;
                                return Ok(Some(Abrupt::Return(value)));
                            }
                            Err(error) => {
                                *async_delegate = None;
                                return Err(error);
                            }
                        }
                    }
                    Instruction::Await(dst, src) => {
                        awaited_promise = Some(async_resolve_value_checked(i, registers[*src].clone())?);
                        *await_dst = Some(*dst);
                        return Ok(None);
                    }
                    Instruction::AsyncIteratorStart(slot, source) => {
                        let iterable = registers[*source].clone();
                        let method = i.get_prop(&iterable, WellKnownSymbol::AsyncIterator.symbol())?;
                        let state = if !method.is_undefined() && !method.is_null() {
                            if callable_function(&method).is_none() {
                                return Err(Throw::type_error("@@asyncIterator is not callable"));
                            }
                            let value = i.call_value(&method, iterable, Vec::new())?;
                            if !value.is_object() {
                                return Err(Throw::type_error("async iterator is not an object"));
                            }
                            let next = i.get_prop(&value, "next")?;
                            if callable_function(&next).is_none() {
                                return Err(Throw::type_error("async iterator next is not callable"));
                            }
                            AsyncIteratorState::Iterator { value, next, exhausted: false, sync: false }
                        } else {
                            let method = i.get_prop(&iterable, WellKnownSymbol::Iterator.symbol())?;
                            if callable_function(&method).is_none() {
                                return Err(Throw::type_error("value is not async iterable"));
                            }
                            let value = i.call_value(&method, iterable, Vec::new())?;
                            if !value.is_object() {
                                return Err(Throw::type_error("sync iterator is not an object"));
                            }
                            let next = i.get_prop(&value, "next")?;
                            AsyncIteratorState::Iterator { value, next, exhausted: false, sync: true }
                        };
                        async_iterators[*slot] = Some(state);
                    }
                    Instruction::AsyncIteratorNext(dst, slot) => {
                        let state = async_iterators[*slot].as_mut()
                            .ok_or_else(|| Throw::type_error("missing async iterator frame"))?;
                        registers[*dst] = match state {
                            AsyncIteratorState::Values { values, index, .. } => {
                                let value = values.get(*index).cloned().unwrap_or(JsValue::Undefined);
                                let done = *index >= values.len();
                                *index += 1;
                                generator_result(i, value, done)
                            }
                            AsyncIteratorState::Iterator { value, next, .. } =>
                                i.call_value(next, value.clone(), Vec::new())?,
                        };
                    }
                    Instruction::AsyncIteratorNextAwait(dst, slot) => {
                        let state = async_iterators[*slot].as_mut()
                            .ok_or_else(|| Throw::type_error("missing async iterator frame"))?;
                        // An abrupt step does not run the loop's later close.
                        // The sync wrapper performs its own close-on-rejection.
                        let (iterator, next, sync) = match state {
                            AsyncIteratorState::Iterator { value, next, exhausted, sync } => {
                                *exhausted = true;
                                (value.clone(), next.clone(), *sync)
                            }
                            _ => unreachable!("ForAwait uses a real iterator record"),
                        };
                        let promise = if sync {
                            async_from_sync_request(i, iterator, next, 0, None)
                        } else { i.call_value(&next, iterator, Vec::new())? };
                        awaited_promise = Some(async_resolve_value_checked(i, promise)?);
                        *await_dst = Some(*dst);
                    }
                    Instruction::AsyncIteratorBeginBody(slot) => {
                        match async_iterators[*slot].as_mut().expect("iterator started") {
                            AsyncIteratorState::Iterator { exhausted, .. }
                            | AsyncIteratorState::Values { exhausted, .. } => *exhausted = false,
                        }
                    }
                    Instruction::AsyncIteratorNeedsClose(dst, slot) => {
                        registers[*dst] = JsValue::Bool(match async_iterators[*slot].as_ref().expect("iterator started") {
                            AsyncIteratorState::Iterator { exhausted, .. }
                            | AsyncIteratorState::Values { exhausted, .. } => !exhausted,
                        });
                    }
                    Instruction::AsyncIteratorDone(dst, slot, source) => {
                        let result = registers[*source].clone();
                        if !result.is_object() {
                            return Err(Throw::type_error("async iterator result is not an object"));
                        }
                        let done = i.get_prop(&result, "done")?.to_boolean();
                        if done {
                            match async_iterators[*slot].as_mut().expect("iterator started") {
                                AsyncIteratorState::Values { exhausted, .. }
                                | AsyncIteratorState::Iterator { exhausted, .. } => *exhausted = true,
                            }
                        }
                        registers[*dst] = JsValue::Bool(done);
                    }
                    Instruction::AsyncIteratorValue(dst, source) => {
                        registers[*dst] = i.get_prop(&registers[*source], "value")?;
                    }
                    Instruction::AsyncIteratorClose(dst, slot) | Instruction::AsyncIteratorCloseAwait(dst, slot) => {
                        // AsyncIteratorClose preserves a pending throw over any
                        // return-method failure, including a later await rejection.
                        if let Some(handler) = try_stack.last_mut() {
                            handler.iterator_close = true;
                        }
                        registers[*dst] = match async_iterators[*slot].as_ref()
                            .ok_or_else(|| Throw::type_error("missing async iterator frame"))? {
                            AsyncIteratorState::Values { .. }
                            | AsyncIteratorState::Iterator { exhausted: true, .. } =>
                                generator_result(i, JsValue::Undefined, true),
                            AsyncIteratorState::Iterator { value, sync: true, .. }
                                if matches!(op, Instruction::AsyncIteratorCloseAwait(_, _)) => {
                                async_from_sync_request(i, value.clone(), JsValue::Undefined, 1, None)
                            }
                            AsyncIteratorState::Iterator { value, .. } => {
                                let method = i.get_prop(value, "return")?;
                                if method.is_undefined() || method.is_null() {
                                    generator_result(i, JsValue::Undefined, true)
                                } else {
                                    if callable_function(&method).is_none() {
                                        return Err(Throw::type_error("async iterator return is not callable"));
                                    }
                                    i.call_value(&method, value.clone(), Vec::new())?
                                }
                            }
                        };
                    }
                    Instruction::AsyncIteratorCloseCheck(source) => {
                        if !registers[*source].is_object() {
                            return Err(Throw::type_error("async iterator return result is not an object"));
                        }
                    }
                    Instruction::IteratorStart(slot, source) => {
                        let iterable = registers[*source].clone();
                        let state = if let Some(value) = i.custom_iterator(&iterable)? {
                            let next = i.get_prop(&value, "next")?;
                            if callable_function(&next).is_none() {
                                return Err(Throw::type_error("iterator next is not callable"));
                            }
                            AsyncIteratorState::Iterator { value, next, exhausted: false, sync: true }
                        } else {
                            AsyncIteratorState::Values { values: i.iterate(&iterable)?, index: 0, exhausted: false }
                        };
                        async_iterators[*slot] = Some(state);
                    }
                    Instruction::ForInStart(slot, source) => {
                        let v = registers[*source].clone();
                        let object = match v {
                            JsValue::Null | JsValue::Undefined => None,
                            other => match super::array_receiver(i, other)? {
                                JsValue::Object(object) => Some(object),
                                _ => None,
                            },
                        };
                        for_in_states[*slot] = Some(super::ForInState {
                            object,
                            object_was_visited: false,
                            visited_keys: std::collections::HashSet::new(),
                            remaining_keys: std::collections::VecDeque::new(),
                        });
                    }
                    Instruction::ForInNext(value, done, slot) => {
                        let state = for_in_states[*slot].as_mut()
                            .ok_or_else(|| Throw::type_error("missing for-in frame"))?;
                        match i.next_for_in_key(state)? {
                            Some(key) => {
                                registers[*value] = JsValue::str(key);
                                registers[*done] = JsValue::Bool(false);
                            }
                            None => {
                                registers[*value] = JsValue::Undefined;
                                registers[*done] = JsValue::Bool(true);
                            }
                        }
                    }
                    Instruction::BindFor(left, source) => {
                        let previous = env.clone();
                        let iteration = for_iteration_env(left, env);
                        i.bind_for(left, registers[*source].clone(), &iteration)?;
                        scopes.push(previous);
                        *env = iteration;
                    }
                    Instruction::PatternStart(slot, source) => {
                        pattern_iterators[*slot] = Some(super::PatternIter::new(i, &registers[*source])?);
                    }
                    Instruction::PatternStep(dst, slot) => {
                        registers[*dst] = pattern_iterators[*slot].as_mut().expect("pattern started")
                            .step(i)?.unwrap_or(JsValue::Undefined);
                    }
                    Instruction::PatternSkip(slot) => {
                        patterns::skip(i, pattern_iterators[*slot].as_mut().expect("pattern started"))?;
                    }
                    Instruction::PatternRest(dst, slot) => {
                        let values = pattern_iterators[*slot].as_mut().expect("pattern started").collect_rest(i)?;
                        registers[*dst] = super::array_values(i, values)?;
                    }
                    Instruction::PatternClose(slot) => {
                        let closed = pattern_iterators[*slot].take().expect("pattern started").close(i);
                        if !try_stack.last().is_some_and(|h| matches!(h.pending, Some(Abrupt::Throw(_)))) {
                            closed?;
                        }
                    }
                    Instruction::ObjectRest(dst, source, keys) => {
                        let keys = keys.iter().map(|key| key.resolve(registers)).collect::<Result<Vec<_>, _>>()?;
                        registers[*dst] = i.object_rest(&registers[*source], &keys)?;
                    }
                    Instruction::ResourceStart(slot) => {
                        resources[*slot].clear();
                        resource_errors[*slot] = None;
                        resource_needs_await[*slot] = false;
                        resource_has_awaited[*slot] = false;
                    }
                    Instruction::ResourceCapture(slot, source, is_async) => {
                        let value = &registers[*source];
                        if let Some(resource) = resources::capture(i, value.clone(), *is_async)? { resources[*slot].push(resource); }
                    }
                    Instruction::ResourceBeginDispose(slot) => {
                        resource_errors[*slot] = try_stack.last().and_then(|handler| match &handler.pending {
                            Some(Abrupt::Throw(error)) => Some(i.normalize_throw(error.clone())),
                            _ => None,
                        });
                    }
                    Instruction::ResourceNext(dst, awaited, done, slot) => {
                        registers[*done] = JsValue::Bool(resources[*slot].is_empty());
                        registers[*awaited] = JsValue::Bool(false);
                        registers[*dst] = JsValue::Undefined;
                        if let Some(resource) = resources[*slot].pop() {
                            if resource.method.is_undefined() {
                                resource_needs_await[*slot] = true;
                            } else if !resource.await_result && resource_needs_await[*slot] && !resource_has_awaited[*slot] {
                                // Null async resources coalesce into one await before
                                // the next synchronous disposer, or at scope end.
                                resources[*slot].push(resource);
                                resource_needs_await[*slot] = false;
                                registers[*awaited] = JsValue::Bool(true);
                            } else {
                                match i.call_value(&resource.method, resource.value, Vec::new()) {
                                    Ok(value) => {
                                        registers[*dst] = if resource.use_result { value } else { JsValue::Undefined };
                                        registers[*awaited] = JsValue::Bool(resource.await_result);
                                        resource_has_awaited[*slot] |= resource.await_result;
                                    }
                                    Err(error) if resource.await_result && !resource.use_result => {
                                        // GetDisposeMethod's async wrapper translates a
                                        // synchronous disposer throw into a rejected
                                        // promise, so older resources wait for it too.
                                        let fate = rejection(i, error.0);
                                        registers[*dst] = new_promise(i, fate);
                                        registers[*awaited] = JsValue::Bool(true);
                                        resource_has_awaited[*slot] = true;
                                    }
                                    Err(error) => resource_error(i, &mut resource_errors[*slot], error),
                                }
                            }
                        } else if resource_needs_await[*slot] && !resource_has_awaited[*slot] {
                            resource_has_awaited[*slot] = true;
                            registers[*done] = JsValue::Bool(false);
                            registers[*awaited] = JsValue::Bool(true);
                        }
                    }
                    Instruction::ResourceError(slot, source) => {
                        resource_error(i, &mut resource_errors[*slot], Throw(registers[*source].clone()));
                    }
                    Instruction::ResourceFinish(slot) => {
                        if let Some(error) = resource_errors[*slot].take() { return Err(error); }
                    }
                    Instruction::EnterCatchValue(dst) => {
                        let handler = try_stack.last_mut().ok_or_else(|| Throw::type_error("invalid resource catch"))?;
                        registers[*dst] = handler.caught.take().ok_or_else(|| Throw::type_error("missing disposal error"))?;
                        scopes.push(env.clone());
                        *env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
                    }
                }
                Ok(None)
            })();
            let abrupt = match step {
                Ok(abrupt) => abrupt,
                Err(error) => Some(Abrupt::Throw(error)),
            };
            if let Some(abrupt) = abrupt {
                if let Some(value) = route_abrupt(try_stack, scopes, env, pc, i, abrupt)? {
                    *completed = Some(value.clone());
                    return Ok(FrameOutcome::Completed(value));
                }
            }
            if yield_dst.is_some() {
                let value = match op {
                    Instruction::Yield(_, src) => registers[*src].clone(),
                    Instruction::YieldDelegate(dst, _) | Instruction::YieldDelegateAsync(dst, _) => registers[*dst].clone(),
                    _ => unreachable!(),
                };
                return Ok(FrameOutcome::Yielded(value));
            }
            if await_dst.is_some() {
                return Ok(FrameOutcome::Awaited(awaited_promise.expect("await prepared its Promise")));
            }
        }
        let value = if self.is_function {
            JsValue::Undefined
        } else {
            if i.external_scheduling.is_none() {
                i.drain_jobs()?;
            }
            i.completion.clone()
        };
        *completed = Some(value.clone());
        Ok(FrameOutcome::Completed(value))
    }
}

/// Pop `scopes` back to `depth`, restoring the environment that was current
/// there -- `LeaveScope` repeated for every scope a `break`/`continue` leaves.
fn leave_scopes_to(scopes: &mut Vec<EnvHandle>, env: &mut EnvHandle, depth: usize) {
    if scopes.len() > depth {
        *env = scopes[depth].clone();
        scopes.truncate(depth);
    }
}

fn resource_error(i: &mut Interpreter, pending: &mut Option<Throw>, error: Throw) {
    let error = i.normalize_throw(error);
    *pending = Some(match pending.take() {
        Some(previous) => Throw(i.make_suppressed_error(error.0, previous.0)),
        None => error,
    });
}

/// Continue a return or throw through active handlers. An instruction fuel
/// failure is handled outside this path so a catch cannot reset the VM budget.
fn route_abrupt(
    try_stack: &mut Vec<TryHandler>,
    scopes: &mut Vec<EnvHandle>,
    env: &mut EnvHandle,
    pc: &mut usize,
    i: &Interpreter,
    mut abrupt: Abrupt,
) -> Result<Option<JsValue>, Throw> {
    loop {
        let Some(handler) = try_stack.last_mut() else {
            return match abrupt {
                Abrupt::Return(value) => Ok(Some(value)),
                Abrupt::Throw(error) => Err(error),
                Abrupt::Goto(target, depth) => {
                    leave_scopes_to(scopes, env, depth);
                    *pc = target;
                    Ok(None)
                }
            };
        };
        if let Abrupt::Goto(target, depth) = &abrupt {
            // The jump's target is still inside this handler's own protected
            // region [start, end) -- e.g. `continue` back to a for-of's own
            // loop top, which is inside ITS iterator-close try but before
            // any try nested in the body, or `break` to a point still
            // inside an enclosing try -- resolve directly, no finally, and
            // leave the handler active on `try_stack`. A plain `< end` check
            // (this ticket's first attempt) wrongly matched a *backward*
            // jump to a target lexically BEFORE this handler even began
            // (continue jumping past a `try` nested in the loop body, back
            // to the for-of's own loop top) -- `>= start` excludes that.
            if *target >= handler.start && *target < handler.end {
                leave_scopes_to(scopes, env, *depth);
                *pc = *target;
                return Ok(None);
            }
        }
        scopes.truncate(handler.scope_depth);
        *env = handler.env.clone();
        if handler.phase == TryPhase::Body {
            if let (Abrupt::Throw(error), Some(catch)) = (&abrupt, handler.catch) {
                handler.phase = TryPhase::Catch;
                handler.caught = Some(i.normalize_throw(error.clone()).0);
                *pc = catch;
                return Ok(None);
            }
        }
        if handler.phase != TryPhase::Finally {
            if let Some(finally) = handler.finally {
                handler.phase = TryPhase::Finally;
                handler.pending = Some(abrupt);
                handler.saved_completion = Some(i.completion.clone());
                *pc = finally;
                return Ok(None);
            }
        }
        if handler.iterator_close && matches!(handler.pending, Some(Abrupt::Throw(_))) {
            abrupt = handler.pending.take().unwrap();
        }
        try_stack.pop();
    }
}

impl FunctionCode {
    pub(crate) fn instantiate(&self, i: &Interpreter, env: &EnvHandle) -> JsValue {
        let name = self.0.name.clone();
        let mut object = JsObject::function(JsFunction {
            name: name.clone(),
            params: self.0.params.clone(),
            body: FunctionBody::Bytecode(self.clone()),
            is_arrow: self.0.is_arrow,
            is_generator: self.0.is_generator,
            is_async: self.0.is_async,
            env: env.clone(),
            home: None,
            proto_for_new: None,
        });
        object.proto = Some(if self.0.is_async && self.0.is_generator {
            i.async_generator_function_proto.clone()
        } else if self.0.is_async {
            i.async_function_proto.clone()
        } else if self.0.is_generator {
            i.generator_function_proto.clone()
        } else {
            i.function_proto.clone()
        });
        object.strict_function = self.0.strict;
        let handle = Obj(Rc::new(RefCell::new(object)));
        if self.0.is_generator || self.0.is_async || self.0.is_method {
            handle.borrow_mut().constructible = false;
        }
        if self.0.is_arrow {
            let grammar = i.lexical_eval_grammar();
            {
                let mut arrow = handle.borrow_mut();
                arrow.lexical_new_target_allowed = grammar.allow_new_target;
                arrow.lexical_class_initializer = grammar.class_initializer;
            }
            // An arrow inherits `new.target` and the super constructor
            // lexically from the frame that creates it, matching the tree
            // walker's `make_arrow` (interp.rs). Without this, a VM-compiled
            // arrow's own `new.target`/`super()` resolve to nothing instead
            // of the enclosing (derived) constructor's.
            if let Some(frame) = i.frames.last() {
                let mut arrow = handle.borrow_mut();
                arrow.func.as_mut().unwrap().home = grammar.allow_super_property.then(||frame.home.clone()).flatten();
                arrow.lexical_super_constructor = grammar.allow_super_call.then(||frame.super_constructor.clone()).flatten();
                arrow.lexical_new_target = grammar.allow_new_target.then(||frame.new_target.clone()).flatten();
            }
        }
        // `JsObject::function` already defines `name` and `length` with the
        // spec descriptors; do not overwrite them here.
        // A plain (non-generator) async function/method has no `.prototype`
        // at all, matching the tree walker -- but a generator does, sync or
        // async, since `new.target`-less `.next()`/`.return()`/`.throw()`
        // live there, even when it is also a method: unlike an ordinary
        // (non-generator) method, a generator method still gets its own
        // `prototype` per GeneratorMethod's PropertyDefinitionEvaluation.
        // `!self.0.is_method` alone (dropping the `is_generator` escape)
        // would silently take that away from every generator method.
        if !self.0.is_arrow && (!self.0.is_async || self.0.is_generator)
            && (!self.0.is_method || self.0.is_generator) {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(i.object_proto.clone());
            if self.0.is_generator {
                // OrdinaryObjectCreate(%GeneratorFunction.prototype.prototype%)
                // (or the async equivalent): the realm's shared generator
                // prototype, whose `next`/`return`/`throw` dispatch to this VM's
                // drivers (interp_coroutine.rs `install_thread_generator_methods`).
                prototype.proto = Some(if self.0.is_async {
                    i.async_generator_proto.clone()
                } else {
                    i.generator_proto.clone()
                });
            }
            let prototype = Obj(Rc::new(RefCell::new(prototype)));
            if !self.0.is_generator {
                // MakeConstructor(F, false, prototype): an ordinary function's
                // own `prototype.constructor` links back to F, matching the
                // tree walker's `make_function`. Without this, `new F()`
                // instances resolve `.constructor` through Object.prototype
                // instead of F, breaking every consumer that identifies a
                // thrown/constructed value by its constructor (Test262Error
                // included) -- this had been silently missing for every
                // VM-compiled ordinary function, not just classes.
                prototype.borrow_mut().define("constructor", Property {
                    value: JsValue::Object(handle.clone()),
                    writable: true,
                    enumerable: false,
                    configurable: true,
                    getter: None,
                    setter: None,
                }).ok();
            }
            // Function `prototype` is a non-enumerable data property
            // (`{writable:true, enumerable:false, configurable:false}`).
            handle
                .borrow_mut()
                .define("prototype", Property {
                    value: JsValue::Object(prototype.clone()),
                    writable: true,
                    enumerable: false,
                    configurable: false,
                    getter: None,
                    setter: None,
                })
                .ok();
            handle.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(prototype);
        }
        if !self.0.is_arrow && !self.0.is_async && !self.0.is_generator
            && !self.0.is_method && !self.0.strict {
            let getter = super::symbol_method(i, "", super::native_legacy_function_caller_getter);
            let setter = super::symbol_method(i, "", super::native_legacy_function_caller_setter);
            handle.borrow_mut().define("caller", Property {
                value: JsValue::Undefined, writable: false, enumerable: false,
                configurable: true, getter: Some(getter), setter: Some(setter),
            }).unwrap();
        }
        JsValue::Object(handle)
    }

    pub(crate) fn call(
        &self,
        i: &mut Interpreter,
        parent: EnvHandle,
        is_arrow: bool,
        this: JsValue,
        args: Vec<JsValue>,
        generator_function: Option<Obj>,
        derived: Option<DerivedConstructor>,
        new_target: Option<JsValue>,
    ) -> Result<JsValue, Throw> {
        // An arrow's own environment owns no `this`/`super`/TDZ state of its
        // own: it is `EnvKind::Arrow`, transparent to `resolve_this_checked`,
        // `initialize_this` and `mark_this_uninitialized`, which all walk
        // through it to the enclosing (possibly still-uninitialized, in a
        // derived constructor) frame -- matching the tree walker's
        // `FunctionBody::Block`/`Expr`. `bind_this` must never run for an
        // arrow: on a Function-kind env it would wrongly look "already
        // initialized" to `initialize_this`, and even harmlessly it would
        // freeze a `this` read before the enclosing `super()` runs.
        let env = EnvHandle::new(if is_arrow { EnvKind::Arrow } else { EnvKind::Function }, Some(parent));
        // A derived constructor's `this` starts uninitialized (TDZ until
        // `super()` runs); an implicit one (no explicit `constructor` member)
        // forwards its own arguments to the super call itself, matching the
        // tree walker's `FunctionBody::Block` derived-constructor handling.
        if !is_arrow && derived.is_some() {
            if new_target.is_none() {
                return Err(Throw::type_error("derived constructor requires new"));
            }
            env.mark_this_uninitialized();
            if let Some(meta) = derived.as_ref().filter(|meta| meta.implicit) {
                let bound = i.construct_with_new_target(
                    &meta.parent, args.clone(), new_target.clone().expect("derived constructor new_target"),
                )?;
                env.initialize_this(bound.clone())?;
                if let Some(frame) = i.frames.last_mut() {
                    frame.this = bound.clone();
                }
                let elements = i.frames.last().and_then(|frame| frame.own_function.clone())
                    .and_then(|function| function.borrow().class_elements.clone());
                if let Some(elements) = elements {
                    super::initialize_instance_elements(i, &bound, &elements)?;
                }
            }
        } else if !is_arrow {
            env.bind_this(this.clone());
            if new_target.is_some() {
                let elements = i.frames.last().and_then(|frame| frame.own_function.clone())
                    .and_then(|function| function.borrow().class_elements.clone());
                if let Some(elements) = elements {
                    super::initialize_instance_elements(i, &this, &elements)?;
                }
            }
        }
        // Parameter binding (including destructuring/defaults/rest) and the
        // `arguments` object are handled by the tree walker's `bind_params`,
        // shared with this VM path so both backends bind identically; it
        // already skips the `arguments` object for arrows and for a
        // parameter literally named `arguments`.
        // The helper's boolean suppresses the implicit arguments object. FDI
        // also suppresses it for a callable with no parameter expressions
        // whose body already owns an arguments declaration.
        let env = i.prepare_function_parameter_environment(&env, &self.0.params, self.0.strict);
        i.bind_params(&env, &self.0.params, args, is_arrow || self.0.skip_implicit_arguments, self.0.strict)?;
        let env = i.prepare_function_body_environment(&env, &self.0.params, &self.0.program.hoisted)?;
        if self.0.is_generator {
            // FDI runs during the call, while only body execution is deferred.
            // Parameter eval may have created a var binding that conflicts
            // with a body lexical declaration; report that before returning
            // the generator, even if it is never resumed.
            let frame = self.0.program.start_frame(i, env.clone(), DEFAULT_INSTRUCTION_LIMIT)?;
            for name in &self.0.body_lexical_names {
                if env.would_conflict_as_lexical(name) {
                    return Err(Throw::syntax_error(format!("identifier '{name}' has already been declared")));
                }
            }
            // Read `functionObject.prototype` now, not before parameters were
            // bound: a parameter default can replace it during
            // FunctionDeclarationInstantiation (just above), and per spec
            // (OrdinaryCreateFromConstructor) the generator's `[[Prototype]]`
            // must observe that later value, matching the tree walker.
            let generator_proto = generator_function.and_then(|function|
                match function.borrow().get_own("prototype") {
                    Some(JsValue::Object(proto)) => Some(proto),
                    _ => None,
                });
            return Ok(self.generator_object(i, env, generator_proto, Some(frame)));
        }
        if self.0.is_async {
            return self.call_async(i, env);
        }
        if derived.is_some() {
            // A derived constructor's completion, per spec: an object return
            // value wins outright, a non-undefined non-object return value is
            // a TypeError, and otherwise the (by now super()-initialized)
            // `this` is the result -- a ReferenceError if `super()` never ran.
            return match self.0.program.execute(i, env.clone(), ORDINARY_FUNCTION_INSTRUCTION_LIMIT) {
                Ok(value) if value.is_object() => Ok(value),
                Ok(value) if !value.is_undefined() =>
                    Err(Throw::type_error("derived constructor returned a non-object")),
                Ok(_) => env.resolve_this_checked(),
                Err(error) => Err(error),
            };
        }
        self.0.program.execute(i, env, ORDINARY_FUNCTION_INSTRUCTION_LIMIT)
    }

    /// Start the resumable body in the environment prepared by AST parameter binding.
    #[allow(dead_code)] // Consumed by the runtime async bridge in interp.rs.
    pub(crate) fn call_async_with_env(&self, i: &mut Interpreter, env: EnvHandle) -> Result<JsValue, Throw> {
        self.call_async(i, env)
    }

    /// Start a resumable generator/async-generator body in the environment
    /// prepared by AST parameter binding -- the generator/async-generator
    /// half of the runtime bridge (mirrors `call_async_with_env`). `self`
    /// must be code compiled with `is_generator: true` (via
    /// `compile_generator_body`/`compile_async_generator_body`); whether the
    /// returned object drives synchronously (`.next()` returning a result
    /// directly) or asynchronously (`.next()` returning a Promise) follows
    /// `self`'s own `is_async` flag, exactly as it does for a VM-instantiated
    /// generator function. `generator_proto` is typically the enclosing
    /// function object's own `.prototype` (as built by `instantiate`, whose
    /// `methods` sub-object supplies `next`/`return`/`throw`/the iterator
    /// symbol); `None` falls back to `Object.prototype`, matching `call()`'s
    /// own default when no such prototype exists.
    #[allow(dead_code)] // Consumed by the runtime generator bridge in interp.rs.
    pub(crate) fn start_generator_with_env(
        &self,
        i: &Interpreter,
        env: EnvHandle,
        generator_proto: Option<Obj>,
    ) -> JsValue {
        self.generator_object(i, env, generator_proto, None)
    }

    fn generator_object(&self, i: &Interpreter, env: EnvHandle,
        generator_proto: Option<Obj>, frame: Option<Box<VmFrame>>) -> JsValue
    {
        let mut object = JsObject::ordinary();
        object.proto = generator_proto.or_else(|| Some(if self.0.is_async {
            i.async_generator_proto.clone()
        } else {
            i.generator_proto.clone()
        }));
        object.exotic = Exotic::Generator(Rc::new(RefCell::new(GeneratorState {
            code: self.clone(),
            env,
            frame,
            started: false,
            executing: false,
            completion: JsValue::Undefined,
            async_promise: None,
            queue: VecDeque::new(),
            awaiting: false,
        })));
        JsValue::object(object)
    }

    fn call_async(&self, i: &mut Interpreter, env: EnvHandle) -> Result<JsValue, Throw> {
        let promise = new_promise(i, PromiseFate::Pending);
        let JsValue::Object(target) = &promise else { unreachable!() };
        let frame = match self.0.program.start_frame(i, env.clone(), DEFAULT_INSTRUCTION_LIMIT) {
            Ok(frame) => Some(frame),
            Err(Throw(error)) => {
                settle_promise(target, rejection(i, error), i)?;
                return Ok(promise);
            }
        };
        let state = Rc::new(RefCell::new(GeneratorState {
            code: self.clone(),
            env,
            frame,
            started: true,
            executing: false,
            completion: JsValue::Undefined,
            async_promise: Some(target.clone()),
            queue: VecDeque::new(),
            awaiting: false,
        }));
        async_continue(i, state, 0, JsValue::Undefined)?;
        Ok(promise)
    }
}

fn async_handler(i: &Interpreter, state: Rc<RefCell<GeneratorState>>, reject: bool) -> JsValue {
    let env = EnvHandle::new(EnvKind::Function, Some(i.global_env.clone()));
    let mut holder = JsObject::ordinary();
    holder.exotic = Exotic::Generator(state);
    let _ = env.declare_var("[[VmAsyncState]]", JsValue::object(holder));
    let mut function = JsObject::function(JsFunction {
        name: if reject { "awaitRejected" } else { "awaitFulfilled" }.into(),
        params: vec![],
        body: FunctionBody::Native(if reject { async_rejected } else { async_fulfilled }),
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

fn async_state(i: &Interpreter) -> Result<Rc<RefCell<GeneratorState>>, Throw> {
    let native = i.native_stack.last().ok_or_else(|| Throw::type_error("missing async callback"))?;
    let env = native.borrow().func.as_ref().unwrap().env.clone();
    let JsValue::Object(holder) = env.resolve("[[VmAsyncState]]")
        .ok_or_else(|| Throw::type_error("missing async frame"))? else {
        return Err(Throw::type_error("invalid async frame"));
    };
    let state = match &holder.borrow().exotic {
        Exotic::Generator(state) => state.clone(),
        _ => return Err(Throw::type_error("invalid async frame")),
    };
    Ok(state)
}

fn async_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = async_state(i)?;
    async_continue(i, state, 0, args.into_iter().next().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

fn async_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = async_state(i)?;
    async_continue(i, state, 2, args.into_iter().next().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

/// Resume a suspended frame with the compiled function's own strictness.
///
/// Suspending at `await`/`yield` leaves the interpreter's strict-eval context,
/// so a continuation that starts from a promise reaction or a native `next`
/// call would otherwise run the post-suspension body in sloppy mode. The
/// branch's strictness is fixed at compile time (`CompiledFunction::strict`).
fn resume_frame_with_strictness(
    i: &mut Interpreter,
    code: &FunctionCode,
    frame: &mut Box<VmFrame>,
) -> Result<FrameOutcome, Throw> {
    let previous = i.strict_eval_context;
    i.strict_eval_context = code.0.strict;
    let outcome = code.0.program.resume_frame(i, frame, usize::MAX);
    i.strict_eval_context = previous;
    outcome
}

fn async_continue(i: &mut Interpreter, state: Rc<RefCell<GeneratorState>>, kind: u8, value: JsValue) -> Result<(), Throw> {
    let (code, mut frame, target, completion) = {
        let mut state = state.borrow_mut();
        if state.executing {
            return Err(Throw::type_error("async frame is already executing"));
        }
        let Some(frame) = state.frame.take() else { return Ok(()) };
        state.executing = true;
        (state.code.clone(), frame, state.async_promise.as_ref().unwrap().clone(), state.completion.clone())
    };
    if let Some(dst) = frame.await_dst.take() {
        if kind == 0 {
            frame.registers[dst] = value;
        } else {
            frame.pending_resume = Some(Abrupt::Throw(Throw(value)));
        }
    }
    let previous_completion = std::mem::replace(&mut i.completion, completion);
    let outcome = resume_frame_with_strictness(i, &code, &mut frame);
    let completion = std::mem::replace(&mut i.completion, previous_completion);
    {
        let mut state = state.borrow_mut();
        state.executing = false;
        state.completion = completion;
        if matches!(&outcome, Ok(FrameOutcome::Awaited(_))) {
            state.frame = Some(frame);
        }
    }
    match outcome {
        Ok(FrameOutcome::Awaited(value)) => {
            let fulfilled = async_handler(i, state.clone(), false);
            let rejected = async_handler(i, state, true);
            async_attach_reactions(i, value, fulfilled, rejected)?;
        }
        Ok(FrameOutcome::Completed(value)) => {
            async_complete(i, &target, value)?;
        }
        Ok(FrameOutcome::Yielded(_)) => unreachable!("async function yielded"),
        Ok(FrameOutcome::Suspended) => unreachable!("unbounded async slice suspended"),
        Err(Throw(error)) => settle_promise(&target, rejection(i, error), i)?,
    }
    Ok(())
}

/// A rejection with an engine-internal error normalized to its JS error object (a bare
/// `Throw::type_error` is not yet a `TypeError` instance until `normalize_throw`).
fn rejection(i: &Interpreter, error: JsValue) -> PromiseFate {
    PromiseFate::Rejected(i.normalize_throw(Throw(error)).0)
}

pub(crate) fn async_resolve_value(i: &mut Interpreter, value: JsValue) -> JsValue {
    match async_resolve_value_checked(i, value) {
        Ok(promise) => promise,
        Err(Throw(error)) => new_promise(i, rejection(i, error)),
    }
}

pub(crate) fn async_from_sync_request(
    i: &mut Interpreter, iterator: JsValue, next: JsValue, kind: u8, input: Option<JsValue>,
) -> JsValue {
    async_from_sync::request(i, iterator, next, kind, input)
}

/// PromiseResolve(%Promise%, value), with abrupt constructor reads preserved.
/// Direct Await propagates that completion before suspension; protocol wrappers
/// and async-generator Return convert it into their own rejected Promise.
pub(crate) fn async_resolve_value_checked(i: &mut Interpreter, value: JsValue) -> Result<JsValue, Throw> {
    if matches!(&value, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Promise(_))) {
        // PromiseResolve(%Promise%, value) reuses only this realm's intrinsic
        // constructor, regardless of a different constructor's prototype.
        // The observable constructor read can itself reject the await.
        if i.get_prop(&value, "constructor")? == i.intrinsic_promise_constructor {
            return Ok(value);
        }
    }
    let promise = new_promise(i, PromiseFate::Pending);
    let JsValue::Object(target) = &promise else { unreachable!() };
    if let Err(Throw(error)) = async_complete(i, target, value) {
        let _ = settle_promise(target, rejection(i, error), i);
    }
    Ok(promise)
}

/// Await attaches reactions internally to its intrinsic Promise. It creates
/// no public `then` result, reads no observable `then` or species property,
/// and does not resolve a discarded capability with the handler's return.
fn async_attach_reactions(
    i: &mut Interpreter, promise: JsValue, fulfilled: JsValue, rejected: JsValue,
) -> Result<(), Throw> {
    let JsValue::Object(promise) = promise else {
        unreachable!("async_resolve_value always returns an intrinsic Promise")
    };
    perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None)
}

pub(crate) fn async_complete(i: &mut Interpreter, target: &Obj, value: JsValue) -> Result<(), Throw> {
    // Returning a Promise still performs Get(value, "then"). The shared
    // resolving pair also keeps a pending adoption from being overwritten
    // when a thenable resolves and subsequently rejects or throws.
    let (resolve, _) = promise_resolving_functions(i, target);
    i.call_value(&resolve, JsValue::Undefined, vec![value]).map(|_| ())
}

fn generator_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut result = JsObject::ordinary();
    result.proto = Some(i.object_proto.clone());
    result.set_own("value", value);
    result.set_own("done", JsValue::Bool(done));
    JsValue::object(result)
}

/// `Some(is_async)` when `value` is a VM-backed generator instance.
pub(crate) fn vm_generator_kind(value: &JsValue) -> Option<bool> {
    let JsValue::Object(object) = value else { return None };
    match &object.borrow().exotic {
        Exotic::Generator(state) => Some(state.borrow().code.0.is_async),
        _ => None,
    }
}

fn generator_state(this: JsValue) -> Result<Rc<RefCell<GeneratorState>>, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error("generator method receiver is not an object"));
    };
    let state = match &object.borrow().exotic {
        crate::value::Exotic::Generator(state) => Some(state.clone()),
        _ => None,
    };
    state.ok_or_else(|| Throw::type_error("generator method receiver is not a generator"))
}

pub(crate) fn generator_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_call(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 0)
}

pub(crate) fn generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_call(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 1)
}

pub(crate) fn generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_call(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 2)
}

fn generator_call(i: &mut Interpreter, this: JsValue, value: JsValue, kind: u8) -> Result<JsValue, Throw> {
    let is_async = generator_state(this.clone())?.borrow().code.0.is_async;
    let result = generator_resume(i, this, value, kind);
    if !is_async { return result; }
    let promise = new_promise(i, PromiseFate::Pending);
    let JsValue::Object(target) = &promise else { unreachable!() };
    match result {
        Ok(value) => settle_promise(target, PromiseFate::Fulfilled(value), i)?,
        Err(Throw(error)) => settle_promise(target, rejection(i, error), i)?,
    }
    Ok(promise)
}

fn generator_resume(i: &mut Interpreter, this: JsValue, value: JsValue, kind: u8) -> Result<JsValue, Throw> {
    let state = generator_state(this)?;
    let (code, mut frame, completion) = {
        let mut state = state.borrow_mut();
        if state.executing {
            return Err(Throw::type_error("generator is already executing"));
        }
        if !state.started && kind != 0 {
            state.started = true;
            state.frame = None;
            return if kind == 1 { Ok(generator_result(i, value, true)) } else { Err(Throw(value)) };
        }
        if state.started && state.frame.is_none() {
            return if kind == 2 { Err(Throw(value)) } else {
                Ok(generator_result(i, if kind == 1 { value } else { JsValue::Undefined }, true))
            };
        }
        let frame = if state.started {
            state.frame.take().unwrap()
        } else {
            state.started = true;
            match state.frame.take() {
                Some(frame) => frame,
                None => state.code.0.program.start_frame(i, state.env.clone(), DEFAULT_INSTRUCTION_LIMIT)?,
            }
        };
        state.executing = true;
        (state.code.clone(), frame, state.completion.clone())
    };
    if let Some(dst) = frame.yield_dst.take() {
        if frame.delegate.is_some() || frame.async_delegate.is_some() {
            frame.delegate_input = Some((kind, value));
        } else {
            match kind {
                0 => frame.registers[dst] = value,
                1 => frame.pending_resume = Some(Abrupt::Return(value)),
                _ => frame.pending_resume = Some(Abrupt::Throw(Throw(value))),
            }
        }
    }
    let previous_completion = std::mem::replace(&mut i.completion, completion);
    let mut outcome = resume_frame_with_strictness(i, &code, &mut frame);
    if code.0.is_async {
        for _ in 0..1_000 {
            let value = match &outcome {
                Ok(FrameOutcome::Awaited(value)) => value.clone(),
                _ => break,
            };
            let destination = frame.await_dst.take().expect("await instruction destination");
            let settled = i.await_iteration_value(value);
            if frame.async_delegate.is_some() {
                frame.delegate_input = Some(match settled {
                    Ok(value) => (0, value),
                    Err(error) => (2, i.normalize_throw(error).0),
                });
            } else {
                match settled {
                    Ok(value) => frame.registers[destination] = value,
                    Err(error) => frame.pending_resume = Some(Abrupt::Throw(error)),
                }
            }
            outcome = resume_frame_with_strictness(i, &code, &mut frame);
        }
    }
    let completion = std::mem::replace(&mut i.completion, previous_completion);
    {
        let mut state = state.borrow_mut();
        state.executing = false;
        state.completion = completion;
    }
    match outcome {
        Ok(FrameOutcome::Yielded(value)) => {
            state.borrow_mut().frame = Some(frame);
            let value = if code.0.is_async { i.await_iteration_value(value)? } else { value };
            Ok(generator_result(i, value, false))
        }
        Ok(FrameOutcome::Completed(value)) => {
            let value = if code.0.is_async { i.await_iteration_value(value)? } else { value };
            Ok(generator_result(i, value, true))
        }
        Ok(FrameOutcome::Suspended) => unreachable!("unbounded generator slice suspended"),
        Ok(FrameOutcome::Awaited(_)) => Err(Throw::range_error("async generator await limit exceeded")),
        Err(error) => Err(error),
    }
}

// --- Async generators ---------------------------------------------------
//
// An async generator combines a generator's suspend points (`yield`/`yield*`)
// with an async function's (`await`), and unlike either alone, its methods
// must never run its body synchronously to completion nor reject the caller
// for calling `.next()` again before a previous call's promise settled --
// each call gets its own promise, queued and drained in order. The driver
// below is `generator_resume` (synchronous yield-only) and `async_continue`
// (promise-settling await-only) fused: `async_generator_run` resumes the one
// underlying VM frame exactly like each of those does, then, on the frame's
// outcome, either settles the current request's promise and drains the next
// queued one (`Yielded`/`Completed`/`Err`) or chains an await continuation
// that re-enters this same function when the awaited value settles
// (`Awaited`) -- so a body can suspend at `await` and `yield` in any order.

fn async_generator_enqueue(i: &mut Interpreter, this: JsValue, value: JsValue, kind: u8) -> Result<JsValue, Throw> {
    let state = generator_state(this)?;
    let promise = new_promise(i, PromiseFate::Pending);
    let JsValue::Object(target) = &promise else { unreachable!() };
    state.borrow_mut().queue.push_back(AsyncGeneratorRequest { kind, value, target: target.clone() });
    async_generator_drain(i, state)?;
    Ok(promise)
}

pub(crate) fn async_generator_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_enqueue(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 0)
}

pub(crate) fn async_generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_enqueue(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 1)
}

pub(crate) fn async_generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    async_generator_enqueue(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 2)
}

/// Settle a queued request's promise without ever touching a VM frame --
/// mirrors `generator_resume`'s two frame-less early exits: a `.return()`/
/// `.throw()` before the body has ever run, and any call once the body has
/// already run to completion.
fn async_generator_settle_without_frame(
    i: &mut Interpreter,
    request: AsyncGeneratorRequest,
) -> Result<(), Throw> {
    match request.kind {
        2 => settle_promise(&request.target, PromiseFate::Rejected(request.value), i),
        _ => {
            let done = request.kind == 1;
            let value = if done { request.value } else { JsValue::Undefined };
            settle_promise(&request.target, PromiseFate::Fulfilled(generator_result(i, value, true)), i)
        }
    }
}

/// Pop and process queued requests one at a time until the queue empties or
/// the generator is already mid-step (a later call to this same function,
/// from that step's own continuation, will resume draining).
fn async_generator_drain(i: &mut Interpreter, state: Rc<RefCell<GeneratorState>>) -> Result<(), Throw> {
    loop {
        // Busy mid-step (actively running bytecode) or mid-`await` (frame
        // suspended, waiting on a promise reaction to resume it): either
        // way, nothing queued can run yet. The reaction/step that clears
        // this drains again itself once it settles.
        if state.borrow().executing || state.borrow().awaiting {
            return Ok(());
        }
        let Some(request) = ({ state.borrow_mut().queue.pop_front() }) else {
            return Ok(());
        };
        // A return/throw before the body has ever run, per spec, completes
        // the generator directly without running any of its code.
        if !state.borrow().started && request.kind != 0 {
            state.borrow_mut().started = true;
            state.borrow_mut().frame = None;
            if request.kind == 1 {
                return async_generator_await_return(i, state, request);
            }
            async_generator_settle_without_frame(i, request)?;
            continue;
        }
        // The body already ran to completion on an earlier request.
        if state.borrow().started && state.borrow().frame.is_none() {
            if request.kind == 1 {
                return async_generator_await_return(i, state, request);
            }
            async_generator_settle_without_frame(i, request)?;
            continue;
        }
        if !state.borrow().started {
            state.borrow_mut().started = true;
            if state.borrow().frame.is_some() {
                let target = request.target.clone();
                return async_generator_run(i, state, target, request.kind, request.value);
            }
            let (code, env) = {
                let s = state.borrow();
                (s.code.clone(), s.env.clone())
            };
            match code.0.program.start_frame(i, env, DEFAULT_INSTRUCTION_LIMIT) {
                Ok(frame) => state.borrow_mut().frame = Some(frame),
                Err(Throw(error)) => {
                    settle_promise(&request.target, rejection(i, error), i)?;
                    continue;
                }
            }
        }
        let target = request.target.clone();
        let kind = request.kind;
        let value = request.value;
        return async_generator_run(i, state, target, kind, value);
    }
}

/// Feed one resume value into the generator's single suspended VM frame and
/// run it until its next suspension or completion, settling `target`
/// (the current request's promise) and continuing to drain the queue.
fn async_generator_run(
    i: &mut Interpreter,
    state: Rc<RefCell<GeneratorState>>,
    target: Obj,
    mut kind: u8,
    value: JsValue,
) -> Result<(), Throw> {
    let (code, mut frame, completion) = {
        let mut s = state.borrow_mut();
        let frame = s.frame.take().expect("async generator frame missing at resume");
        s.executing = true;
        s.awaiting = false;
        (s.code.clone(), frame, s.completion.clone())
    };
    if frame.await_return {
        // The awaited `return(v)` resumption settled: deliver it as a return
        // of the awaited value, or as a throw of the rejection reason.
        frame.await_return = false;
        kind = if kind == 0 { 1 } else { 2 };
    } else if kind == 1 && frame.yield_dst.is_some() {
        // AsyncGeneratorUnwrapYieldResumption: a `return(v)` delivered at a
        // yield first awaits `v`; the frame stays suspended at its yield and
        // is resumed from that await's reaction instead.
        frame.await_return = true;
        {
            let mut s = state.borrow_mut();
            s.executing = false;
            s.awaiting = true;
            s.frame = Some(frame);
        }
        let awaited = async_resolve_value(i, value);
        let fulfilled = async_generator_step_handler(i, state.clone(), target.clone(), false);
        let rejected = async_generator_step_handler(i, state, target, true);
        async_attach_reactions(i, awaited, fulfilled, rejected)?;
        return Ok(());
    }
    if let Some(dst) = frame.yield_dst.take() {
        if frame.delegate.is_some() || frame.async_delegate.is_some() {
            frame.delegate_input = Some((kind, value));
        } else {
            match kind {
                0 => frame.registers[dst] = value,
                1 => frame.pending_resume = Some(Abrupt::Return(value)),
                _ => frame.pending_resume = Some(Abrupt::Throw(Throw(value))),
            }
        }
    } else if let Some(dst) = frame.await_dst.take() {
        if frame.async_delegate.is_some() {
            // An await inside `yield*`: the delegation state machine consumes
            // the settlement itself (see `async_delegate::step`).
            frame.delegate_input = Some((kind, value));
        } else {
            match kind {
                0 => frame.registers[dst] = value,
                _ => frame.pending_resume = Some(Abrupt::Throw(Throw(value))),
            }
        }
    }
    let previous_completion = std::mem::replace(&mut i.completion, completion);
    let outcome = resume_frame_with_strictness(i, &code, &mut frame);
    let completion = std::mem::replace(&mut i.completion, previous_completion);
    {
        let mut s = state.borrow_mut();
        s.executing = false;
        s.completion = completion;
        s.awaiting = matches!(&outcome, Ok(FrameOutcome::Awaited(_)));
        if matches!(&outcome, Ok(FrameOutcome::Awaited(_)) | Ok(FrameOutcome::Yielded(_))) {
            s.frame = Some(frame);
        }
    }
    match outcome {
        Ok(FrameOutcome::Yielded(value)) => {
            settle_promise(&target, PromiseFate::Fulfilled(generator_result(i, value, false)), i)?;
            async_generator_drain(i, state)
        }
        Ok(FrameOutcome::Completed(value)) => {
            settle_promise(&target, PromiseFate::Fulfilled(generator_result(i, value, true)), i)?;
            async_generator_drain(i, state)
        }
        Ok(FrameOutcome::Awaited(value)) => {
            let fulfilled = async_generator_step_handler(i, state.clone(), target.clone(), false);
            let rejected = async_generator_step_handler(i, state, target, true);
            async_attach_reactions(i, value, fulfilled, rejected)?;
            Ok(())
        }
        Ok(FrameOutcome::Suspended) => unreachable!("unbounded async generator slice suspended"),
        Err(Throw(error)) => {
            settle_promise(&target, rejection(i, error), i)?;
            async_generator_drain(i, state)
        }
    }
}

/// Build the fulfilled/rejected reaction callback for an async generator's
/// in-flight `await`, mirroring `async_handler` -- but an async generator's
/// continuation must resume with both the shared `state` (as `async_handler`
/// carries) and the *specific request's* promise (`target`), since each
/// queued call has its own, unlike a plain async function's single backing
/// promise. Both are stashed as env-declared internal slots on the callback's
/// own closure, read back by `async_generator_step_state`.
fn async_generator_step_handler(i: &Interpreter, state: Rc<RefCell<GeneratorState>>, target: Obj, reject: bool) -> JsValue {
    if reject {
        async_generator_callback(i, state, target, "asyncGeneratorStepRejected", async_generator_step_rejected)
    } else {
        async_generator_callback(i, state, target, "asyncGeneratorStepFulfilled", async_generator_step_fulfilled)
    }
}

/// AsyncGeneratorAwaitReturn (27.6.3.9): a `return(v)` on a generator that
/// never started or already completed awaits `v` before settling. The queue
/// stays blocked (`awaiting`) until then, like the spec's awaiting-return state.
fn async_generator_await_return(
    i: &mut Interpreter,
    state: Rc<RefCell<GeneratorState>>,
    request: AsyncGeneratorRequest,
) -> Result<(), Throw> {
    state.borrow_mut().awaiting = true;
    let awaited = async_resolve_value(i, request.value);
    let fulfilled = async_generator_callback(i, state.clone(), request.target.clone(),
        "asyncGeneratorAwaitReturnFulfilled", async_generator_return_fulfilled);
    let rejected = async_generator_callback(i, state.clone(), request.target.clone(),
        "asyncGeneratorAwaitReturnRejected", async_generator_return_rejected);
    let chained = async_attach_reactions(i, awaited, fulfilled, rejected);
    if let Err(Throw(error)) = chained {
        state.borrow_mut().awaiting = false;
        settle_promise(&request.target, rejection(i, error), i)?;
        return async_generator_drain(i, state);
    }
    Ok(())
}

fn async_generator_return_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (state, target) = async_generator_step_state(i)?;
    state.borrow_mut().awaiting = false;
    let value = args.into_iter().next().unwrap_or(JsValue::Undefined);
    settle_promise(&target, PromiseFate::Fulfilled(generator_result(i, value, true)), i)?;
    async_generator_drain(i, state)?;
    Ok(JsValue::Undefined)
}

fn async_generator_return_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (state, target) = async_generator_step_state(i)?;
    state.borrow_mut().awaiting = false;
    settle_promise(&target, PromiseFate::Rejected(args.into_iter().next().unwrap_or(JsValue::Undefined)), i)?;
    async_generator_drain(i, state)?;
    Ok(JsValue::Undefined)
}

fn async_generator_callback(
    i: &Interpreter,
    state: Rc<RefCell<GeneratorState>>,
    target: Obj,
    name: &str,
    native: crate::value::NativeFn,
) -> JsValue {
    let env = EnvHandle::new(EnvKind::Function, Some(i.global_env.clone()));
    let mut holder = JsObject::ordinary();
    holder.exotic = Exotic::Generator(state);
    let _ = env.declare_var("[[VmAsyncState]]", JsValue::object(holder));
    let _ = env.declare_var("[[VmAsyncGeneratorTarget]]", JsValue::Object(target));
    let mut function = JsObject::function(JsFunction {
        name: name.into(),
        params: vec![],
        body: FunctionBody::Native(native),
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

fn async_generator_step_state(i: &Interpreter) -> Result<(Rc<RefCell<GeneratorState>>, Obj), Throw> {
    let native = i.native_stack.last().ok_or_else(|| Throw::type_error("missing async generator callback"))?;
    let env = native.borrow().func.as_ref().unwrap().env.clone();
    let JsValue::Object(holder) = env.resolve("[[VmAsyncState]]")
        .ok_or_else(|| Throw::type_error("missing async generator frame"))? else {
        return Err(Throw::type_error("invalid async generator frame"));
    };
    let state = match &holder.borrow().exotic {
        Exotic::Generator(state) => state.clone(),
        _ => return Err(Throw::type_error("invalid async generator frame")),
    };
    let JsValue::Object(target) = env.resolve("[[VmAsyncGeneratorTarget]]")
        .ok_or_else(|| Throw::type_error("missing async generator target"))? else {
        return Err(Throw::type_error("invalid async generator target"));
    };
    Ok((state, target))
}

fn async_generator_step_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (state, target) = async_generator_step_state(i)?;
    async_generator_run(i, state, target, 0, args.into_iter().next().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

fn async_generator_step_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (state, target) = async_generator_step_state(i)?;
    async_generator_run(i, state, target, 2, args.into_iter().next().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

fn consume(fuel: &mut usize) -> Result<(), Throw> {
    if *fuel == 0 {
        return Err(Throw::range_error("bytecode instruction limit exceeded"));
    }
    *fuel -= 1;
    Ok(())
}
