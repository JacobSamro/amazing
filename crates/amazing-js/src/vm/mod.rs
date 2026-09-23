//! Experimental register VM. Compilation rejects unsupported syntax atomically;
//! execution never falls back to AST statement/expression evaluation.
#![forbid(unsafe_code)]

mod compiler;
#[cfg(test)]
mod tests;

use super::{reference_name, Interpreter, Reference};
use crate::ast::{BinOp, UnOp, VarKind};
use crate::env::{EnvHandle, EnvKind};
use crate::tokens::{Numeric, Span};
use crate::value::{FunctionBody, JsFunction, JsObject, JsString, JsValue, Obj, Throw};
use std::cell::RefCell;
use std::rc::Rc;

pub use compiler::compile;

pub const DEFAULT_INSTRUCTION_LIMIT: usize = 10_000_000;
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
}

#[derive(Debug)]
enum KeyOperand {
    Static(JsString),
    Dynamic(Register),
}

impl KeyOperand {
    fn resolve(&self, i: &mut Interpreter, registers: &[JsValue]) -> Result<JsString, Throw> {
        match self {
            Self::Static(key) => Ok(key.clone()),
            Self::Dynamic(register) => i.to_js_string(&registers[*register]),
        }
    }
}

#[derive(Debug)]
enum Instruction {
    Constant(Register, JsValue),
    Numeric(Register, Numeric),
    ToKey(Register),
    LoadName(Register, String, bool),
    LoadThis(Register),
    ResolveName(usize, Register, String),
    RequireResolved(usize),
    GetMember(Register, Register, KeyOperand),
    ResolveMember(usize, Register, Register, KeyOperand),
    StoreReference(usize, Register),
    NewObject(Register),
    SetObjectProperty(Register, KeyOperand, Register),
    NewArray(Register),
    SetArrayElement(Register, usize, Register),
    SetArrayLength(Register, usize),
    Declare(String, Register, VarKind),
    Unary(Register, UnOp, Register),
    Binary(Register, BinOp, Register, Register),
    Move(Register, Register),
    Jump(usize),
    Branch(Register, Condition, usize),
    EnterScope,
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
    Call(Register, Register, Register, Vec<Register>),
    Construct(Register, Register, Vec<Register>),
    Throw(Register),
    Yield(Register, Register),
    YieldDelegate(Register, Register),
}

#[derive(Debug, Clone)]
pub struct FunctionCode(Rc<CompiledFunction>);

#[derive(Debug)]
struct CompiledFunction {
    name: String,
    params: Vec<String>,
    program: Program,
    is_arrow: bool,
    is_generator: bool,
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
    registers: usize,
    references: usize,
    hoisted: Vec<String>,
    functions: Vec<HoistedFunction>,
    is_function: bool,
}

/// Mutable execution state owned by a heap allocation. The try stack is empty
/// until exception handlers are compiled, but belongs to the frame so it can
/// survive an instruction-boundary suspension with the other VM state.
struct VmFrame {
    registers: Vec<JsValue>,
    references: Vec<Option<Reference>>,
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
}

struct DelegateState {
    iterator: JsValue,
    array_index: Option<usize>,
}

enum FrameOutcome {
    Suspended,
    Yielded(JsValue),
    Completed(JsValue),
}

pub struct GeneratorState {
    code: FunctionCode,
    env: EnvHandle,
    frame: Option<Box<VmFrame>>,
    started: bool,
    executing: bool,
    completion: JsValue,
}

impl std::fmt::Debug for GeneratorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratorState")
            .field("started", &self.started)
            .field("executing", &self.executing)
            .finish()
    }
}

enum Abrupt {
    Return(JsValue),
    Throw(Throw),
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
    end: usize,
    scope_depth: usize,
    env: EnvHandle,
    phase: TryPhase,
    caught: Option<JsValue>,
    pending: Option<Abrupt>,
    saved_completion: Option<JsValue>,
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
        }
    }

    fn start_frame(
        &self,
        i: &mut Interpreter,
        env: EnvHandle,
        fuel: usize,
    ) -> Result<Box<VmFrame>, Throw> {
        let mut frame = Box::new(VmFrame {
            registers: vec![JsValue::Undefined; self.registers],
            references: (0..self.references).map(|_| None).collect(),
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
        });
        for name in &self.hoisted {
            consume(&mut frame.fuel)?;
            frame.env.declare_var(name, JsValue::Undefined)?;
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
            registers,
            references,
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
        } = frame.as_mut();
        let mut steps = 0;
        while let Some(op) = self.code.get(*pc) {
            if steps == quantum {
                return Ok(FrameOutcome::Suspended);
            }
            consume(fuel)?;
            steps += 1;
            *pc += 1;
            let step = (|| -> Result<Option<Abrupt>, Throw> {
                match op {
                    Instruction::Constant(dst, value) => registers[*dst] = value.clone(),
                    Instruction::Numeric(dst, number) => registers[*dst] = i.numeric(number)?,
                    Instruction::ToKey(register) => {
                        registers[*register] = JsValue::Str(i.to_js_string(&registers[*register])?);
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
                    Instruction::LoadThis(dst) => registers[*dst] = env.resolve_this(),
                    Instruction::ResolveName(slot, dst, name) => {
                        let (reference, value) = if let Some(value) = env.resolve(name) {
                            (Reference::Binding(name.clone()), value)
                        } else if i.global_obj.borrow().has_property(name) {
                            let global = JsValue::Object(i.global_obj.clone());
                            let value = i.get_prop(&global, name)?;
                            (Reference::Property(global, name.clone().into()), value)
                        } else {
                            (Reference::Binding(name.clone()), JsValue::Undefined)
                        };
                        references[*slot] = Some(reference);
                        registers[*dst] = value;
                    }
                    Instruction::RequireResolved(slot) => match &references[*slot] {
                        Some(Reference::Binding(name)) if !env.has(name) => {
                            return Err(Throw::reference_error(
                                "assignment to an unresolved binding",
                            ));
                        }
                        Some(_) => {}
                        None => return Err(Throw::type_error("invalid VM reference lifetime")),
                    },
                    Instruction::GetMember(dst, object, key) => {
                        let key = key.resolve(i, &registers)?;
                        registers[*dst] = i.get_prop(&registers[*object], key)?;
                    }
                    Instruction::ResolveMember(slot, dst, object, key) => {
                        let key = key.resolve(i, &registers)?;
                        let receiver = registers[*object].clone();
                        registers[*dst] = i.get_prop(&receiver, key.clone())?;
                        references[*slot] = Some(Reference::Property(receiver, crate::value::PropertyKey::String(key)));
                    }
                    Instruction::StoreReference(slot, src) => {
                        let reference = references[*slot]
                            .take()
                            .ok_or_else(|| Throw::type_error("invalid VM reference lifetime"))?;
                        let mut value = registers[*src].clone();
                        if let Some(name) = reference_name(&reference) {
                            i.infer_function_name(&mut value, &name);
                        }
                        i.put_reference(reference, value, &env)?;
                    }
                    Instruction::NewObject(dst) => registers[*dst] = i.object_value(),
                    Instruction::SetObjectProperty(object, key, src) => {
                        let key = key.resolve(i, &registers)?;
                        i.set_prop(&registers[*object], key, registers[*src].clone())?;
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
                    Instruction::Unary(dst, op, src) => {
                        let value = &registers[*src];
                        registers[*dst] = match op {
                            UnOp::Neg => JsValue::num(-i.to_number(value)?),
                            UnOp::Pos => JsValue::num(i.to_number(value)?),
                            UnOp::Not => JsValue::Bool(!value.to_boolean()),
                            UnOp::BitNot => JsValue::num((!i.to_int32(value)?) as f64),
                            UnOp::Typeof => JsValue::str(i.typeof_value(value)),
                            UnOp::Void => JsValue::Undefined,
                            // The private compiler never emits these.
                            UnOp::Delete | UnOp::Await => {
                                return Err(Throw::type_error("invalid VM unary instruction"));
                            }
                        };
                    }
                    Instruction::Binary(dst, op, left, right) => {
                        registers[*dst] =
                            i.binary(*op, registers[*left].clone(), registers[*right].clone())?;
                    }
                    Instruction::Move(dst, src) => registers[*dst] = registers[*src].clone(),
                    Instruction::Jump(target) => *pc = *target,
                    Instruction::Branch(src, condition, target) => {
                        let value = &registers[*src];
                        if match condition {
                            Condition::Truthy => value.to_boolean(),
                            Condition::Falsy => !value.to_boolean(),
                            Condition::Nullish => value.is_null() || value.is_undefined(),
                        } {
                            *pc = *target;
                        }
                    }
                    Instruction::EnterScope => {
                        scopes.push(env.clone());
                        *env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
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
                        end: *end,
                        scope_depth: scopes.len(),
                        env: env.clone(),
                        phase: TryPhase::Body,
                        caught: None,
                        pending: None,
                        saved_completion: None,
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
                    Instruction::MakeFunction(dst, code) => {
                        registers[*dst] = code.instantiate(i, &env);
                    }
                    Instruction::Call(dst, callee, this, arguments) => {
                        let args = arguments.iter().map(|r| registers[*r].clone()).collect();
                        registers[*dst] =
                            i.call_value(&registers[*callee], registers[*this].clone(), args)?;
                    }
                    Instruction::Construct(dst, callee, arguments) => {
                        let args = arguments.iter().map(|r| registers[*r].clone()).collect();
                        registers[*dst] = i.construct(&registers[*callee], args)?;
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
                    Instruction::YieldDelegate(dst, _) => registers[*dst].clone(),
                    _ => unreachable!(),
                };
                return Ok(FrameOutcome::Yielded(value));
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

/// Continue a return or throw through active handlers. An instruction fuel
/// failure is handled outside this path so a catch cannot reset the VM budget.
fn route_abrupt(
    try_stack: &mut Vec<TryHandler>,
    scopes: &mut Vec<EnvHandle>,
    env: &mut EnvHandle,
    pc: &mut usize,
    i: &Interpreter,
    abrupt: Abrupt,
) -> Result<Option<JsValue>, Throw> {
    loop {
        let Some(handler) = try_stack.last_mut() else {
            return match abrupt {
                Abrupt::Return(value) => Ok(Some(value)),
                Abrupt::Throw(error) => Err(error),
            };
        };
        scopes.truncate(handler.scope_depth);
        *env = handler.env.clone();
        if handler.phase == TryPhase::Body {
            if let (Abrupt::Throw(error), Some(catch)) = (&abrupt, handler.catch) {
                handler.phase = TryPhase::Catch;
                handler.caught = Some(error.0.clone());
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
        try_stack.pop();
    }
}

impl FunctionCode {
    fn instantiate(&self, i: &Interpreter, env: &EnvHandle) -> JsValue {
        let name = self.0.name.clone();
        let mut object = JsObject::function(JsFunction {
            name: name.clone(),
            params: vec![],
            body: FunctionBody::Bytecode(self.clone()),
            is_arrow: self.0.is_arrow,
            is_async: false,
            is_generator: self.0.is_generator,
            env: env.clone(),
            home: None,
            proto_for_new: None,
        });
        object.proto = Some(i.function_proto.clone());
        let handle = Obj(Rc::new(RefCell::new(object)));
        if self.0.is_generator {
            handle.borrow_mut().constructible = false;
        }
        handle
            .borrow_mut()
            .set_own("name", JsValue::str(name.as_str()));
        if !self.0.is_arrow {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(i.object_proto.clone());
            if self.0.is_generator {
                prototype.set_own("next", Interpreter::native("next", generator_next, &i.function_proto));
                prototype.set_own("return", Interpreter::native("return", generator_return, &i.function_proto));
                prototype.set_own("throw", Interpreter::native("throw", generator_throw, &i.function_proto));
                prototype.set_own("@@iterator", Interpreter::native("[Symbol.iterator]", generator_iterator, &i.function_proto));
            }
            let prototype = Obj(Rc::new(RefCell::new(prototype)));
            handle
                .borrow_mut()
                .set_own("prototype", JsValue::Object(prototype.clone()));
            handle.borrow_mut().func.as_mut().unwrap().proto_for_new = Some(prototype);
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
        generator_proto: Option<Obj>,
    ) -> Result<JsValue, Throw> {
        let bound_this = if is_arrow {
            parent.resolve_this()
        } else {
            this
        };
        let env = EnvHandle::new(EnvKind::Function, Some(parent));
        env.bind_this(bound_this);
        for (index, name) in self.0.params.iter().enumerate() {
            env.declare_lexical(
                name,
                args.get(index).cloned().unwrap_or(JsValue::Undefined),
                true,
            )?;
        }
        let arguments = JsValue::object(JsObject::array());
        for (index, value) in args.iter().enumerate() {
            i.set_prop(&arguments, &index.to_string(), value.clone())?;
        }
        i.set_prop(&arguments, "length", JsValue::num(args.len() as f64))?;
        env.declare_var("arguments", arguments)?;
        if self.0.is_generator {
            let mut object = JsObject::ordinary();
            object.proto = generator_proto.or_else(|| Some(i.object_proto.clone()));
            object.exotic = crate::value::Exotic::Generator(Rc::new(RefCell::new(GeneratorState {
                code: self.clone(),
                env,
                frame: None,
                started: false,
                executing: false,
                completion: JsValue::Undefined,
            })));
            return Ok(JsValue::object(object));
        }
        self.0.program.execute(i, env, DEFAULT_INSTRUCTION_LIMIT)
    }
}

fn generator_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut result = JsObject::ordinary();
    result.proto = Some(i.object_proto.clone());
    result.set_own("value", value);
    result.set_own("done", JsValue::Bool(done));
    JsValue::object(result)
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

fn generator_iterator(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_state(this.clone())?;
    Ok(this)
}

fn generator_next(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_resume(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 0)
}

fn generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_resume(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 1)
}

fn generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_resume(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 2)
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
            state.code.0.program.start_frame(i, state.env.clone(), DEFAULT_INSTRUCTION_LIMIT)?
        };
        state.executing = true;
        (state.code.clone(), frame, state.completion.clone())
    };
    if let Some(dst) = frame.yield_dst.take() {
        if frame.delegate.is_some() {
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
    let outcome = code.0.program.resume_frame(i, &mut frame, usize::MAX);
    let completion = std::mem::replace(&mut i.completion, previous_completion);
    let mut state = state.borrow_mut();
    state.executing = false;
    state.completion = completion;
    match outcome {
        Ok(FrameOutcome::Yielded(value)) => {
            state.frame = Some(frame);
            Ok(generator_result(i, value, false))
        }
        Ok(FrameOutcome::Completed(value)) => Ok(generator_result(i, value, true)),
        Ok(FrameOutcome::Suspended) => unreachable!("unbounded generator slice suspended"),
        Err(error) => Err(error),
    }
}

fn consume(fuel: &mut usize) -> Result<(), Throw> {
    if *fuel == 0 {
        return Err(Throw::range_error("bytecode instruction limit exceeded"));
    }
    *fuel -= 1;
    Ok(())
}
