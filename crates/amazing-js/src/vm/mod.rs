//! Experimental register VM. Compilation rejects unsupported syntax atomically;
//! execution never falls back to AST statement/expression evaluation.
#![forbid(unsafe_code)]

mod compiler;
#[cfg(test)]
mod tests;

use super::{adopt_promise_result, callable_function, for_iteration_env, native_promise_reject, native_promise_resolve, new_promise, promise_callback, reference_name, settle_promise, Interpreter, Reference};
use crate::ast::{BinOp, ForLeft, UnOp, VarKind};
use crate::env::{EnvHandle, EnvKind};
use crate::tokens::{Numeric, Span};
use crate::value::{Exotic, FunctionBody, JsFunction, JsObject, JsString, JsValue, Obj, PromiseFate, Property, PropertyKey, Throw, WellKnownSymbol};
use std::cell::RefCell;
use std::rc::Rc;

pub use compiler::compile;
pub(crate) use compiler::compile_class_method;
pub(crate) use compiler::compile_function_item;
#[allow(unused_imports)] // Re-exported for the runtime async bridge in interp.rs.
pub(crate) use compiler::compile_async_body;

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
}

#[derive(Debug)]
struct ClassCode {
    constructor: FunctionCode,
    inferred_name: Option<KeyOperand>,
    methods: Vec<ClassMethodCode>,
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
    NewObject(Register),
    SetObjectProperty(Register, KeyOperand, Register),
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
    MakeClass(Register, ClassCode),
    Call(Register, Register, Register, Vec<CallArgument>),
    Construct(Register, Register, Vec<CallArgument>),
    Throw(Register),
    Yield(Register, Register),
    YieldDelegate(Register, Register),
    Await(Register, Register),
    AsyncIteratorStart(usize, Register),
    AsyncIteratorNext(Register, usize),
    AsyncIteratorDone(Register, usize, Register),
    AsyncIteratorValue(Register, Register),
    AsyncIteratorClose(Register, usize),
    AsyncIteratorCloseCheck(Register),
    BindFor(ForLeft, Register),
    Update(Register, usize, Register, crate::ast::UpdateOp, bool),
}

#[derive(Debug, Clone)]
pub struct FunctionCode(Rc<CompiledFunction>);

#[derive(Debug)]
struct CompiledFunction {
    name: String,
    params: Vec<crate::ast::Param>,
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
    registers: usize,
    references: usize,
    spread_slots: usize,
    hoisted: Vec<String>,
    functions: Vec<HoistedFunction>,
    is_function: bool,
    async_iterator_slots: usize,
}

/// Mutable execution state owned by a heap allocation. The try stack is empty
/// until exception handlers are compiled, but belongs to the frame so it can
/// survive an instruction-boundary suspension with the other VM state.
struct VmFrame {
    registers: Vec<JsValue>,
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
    await_dst: Option<Register>,
    async_iterators: Vec<Option<AsyncIteratorState>>,
}

enum AsyncIteratorState {
    Values { values: Vec<JsValue>, index: usize, exhausted: bool },
    Iterator { value: JsValue, next: JsValue, exhausted: bool },
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
            registers: vec![JsValue::Undefined; self.registers],
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
            await_dst: None,
            async_iterators: (0..self.async_iterator_slots).map(|_| None).collect(),
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
            await_dst,
            async_iterators,
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
                    Instruction::RegExp(dst, regex) => registers[*dst] = i.make_regexp(regex.clone())?,
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
                    Instruction::LoadThis(dst) => registers[*dst] = env.resolve_this(),
                    Instruction::ResolveName(slot, dst, name) => {
                        let (reference, value) = i.name_reference_value(&env, name)?;
                        references[*slot] = Some(reference);
                        registers[*dst] = value;
                    }
                    Instruction::ResolveWriteName(slot, name) => {
                        references[*slot] = Some(i.name_reference(&env, name)?);
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
                        let key = key.resolve(&registers)?;
                        registers[*dst] = i.get_prop(&registers[*object], key)?;
                    }
                    Instruction::ResolveMember(slot, dst, object, key) => {
                        let key = key.resolve(&registers)?;
                        let receiver = registers[*object].clone();
                        registers[*dst] = i.get_prop(&receiver, key.clone())?;
                        references[*slot] = Some(Reference::Property(receiver, key));
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
                    Instruction::Update(dst, slot, old, op, prefix) => {
                        let reference = references[*slot]
                            .take()
                            .ok_or_else(|| Throw::type_error("invalid VM update reference"))?;
                        let previous = registers[*old].clone();
                        let next = i.update_value(*op, previous.clone())?;
                        i.put_reference(reference, next.clone(), &env)?;
                        registers[*dst] = if *prefix { next } else { previous };
                    }
                    Instruction::NewObject(dst) => registers[*dst] = i.object_value(),
                    Instruction::SetObjectProperty(object, key, src) => {
                        let key = key.resolve(&registers)?;
                        let JsValue::Object(target) = &registers[*object] else { unreachable!() };
                        target.borrow_mut().define(&key, Property::data(registers[*src].clone()))
                            .map_err(|_| Throw::type_error("could not define object property"))?;
                    }
                    Instruction::SetObjectMethod(object, key, code, kind) => {
                        let key = key.resolve(&registers)?;
                        let JsValue::Object(target) = &registers[*object] else { unreachable!() };
                        let method = code.instantiate(i, env);
                        let JsValue::Object(callable) = &method else { unreachable!() };
                        callable.borrow_mut().func.as_mut().unwrap().home = Some(target.clone());
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
                    Instruction::Unary(dst, op, src) => {
                        registers[*dst] = i.unary_value(*op, registers[*src].clone())?;
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
                    Instruction::MakeFunction(dst, code) => {
                        registers[*dst] = code.instantiate(i, &env);
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
                        prototype.borrow_mut().set_own("constructor", constructor.clone());
                        callable.borrow_mut().func.as_mut().unwrap().home = Some(prototype.clone());
                        for method in &class.methods {
                            let key = method.key.resolve(&registers)?;
                            let mut value = method.code.instantiate(i, &env);
                            let name = match &key {
                                PropertyKey::String(name) => name.to_utf8_lossy(),
                                PropertyKey::Symbol(symbol) => symbol.descriptive_string(),
                            };
                            let name = match method.kind {
                                crate::ast::MethodKind::Get => format!("get {name}"),
                                crate::ast::MethodKind::Set => format!("set {name}"),
                                _ => name,
                            };
                            i.infer_function_name(&mut value, &name);
                            let JsValue::Object(function) = &value else { unreachable!() };
                            function.borrow_mut().func.as_mut().unwrap().home = Some(prototype.clone());
                            i.define_class_method(&JsValue::Object(prototype.clone()), &key,
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
                    Instruction::Await(dst, _) => {
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
                            AsyncIteratorState::Iterator { value, next, exhausted: false }
                        } else if let Some(value) = i.custom_iterator(&iterable)? {
                            let next = i.get_prop(&value, "next")?;
                            AsyncIteratorState::Iterator { value, next, exhausted: false }
                        } else {
                            AsyncIteratorState::Values { values: i.iterate(&iterable)?, index: 0, exhausted: false }
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
                    Instruction::AsyncIteratorClose(dst, slot) => {
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
                    Instruction::BindFor(left, source) => {
                        let previous = env.clone();
                        let iteration = for_iteration_env(left, env);
                        i.bind_for(left, registers[*source].clone(), &iteration)?;
                        scopes.push(previous);
                        *env = iteration;
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
            if await_dst.is_some() {
                let Instruction::Await(_, src) = op else { unreachable!() };
                return Ok(FrameOutcome::Awaited(registers[*src].clone()));
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
    mut abrupt: Abrupt,
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
        handle.borrow_mut().define("name", Property {
            value: JsValue::str(name.as_str()), writable: false, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).expect("function name property");
        if !self.0.is_arrow && (!self.0.is_async || self.0.is_generator)
            && (!self.0.is_method || self.0.is_generator) {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(i.object_proto.clone());
            if self.0.is_generator {
                let mut methods = JsObject::ordinary();
                methods.proto = Some(i.object_proto.clone());
                methods.set_own("next", Interpreter::native("next", generator_next, &i.function_proto));
                methods.set_own("return", Interpreter::native("return", generator_return, &i.function_proto));
                methods.set_own("throw", Interpreter::native("throw", generator_throw, &i.function_proto));
                let iterator_symbol = if self.0.is_async {
                    WellKnownSymbol::AsyncIterator
                } else {
                    WellKnownSymbol::Iterator
                };
                methods.set_own(iterator_symbol.symbol(), Interpreter::native(
                    if self.0.is_async { "[Symbol.asyncIterator]" } else { "[Symbol.iterator]" },
                    generator_iterator, &i.function_proto));
                prototype.proto = Some(Obj(Rc::new(RefCell::new(methods))));
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
        generator_function: Option<Obj>,
    ) -> Result<JsValue, Throw> {
        let bound_this = if is_arrow {
            parent.resolve_this()
        } else {
            this
        };
        let env = EnvHandle::new(EnvKind::Function, Some(parent));
        env.bind_this(bound_this);
        i.bind_params(&env, &self.0.params, args, is_arrow)?;
        if self.0.is_generator {
            // Parameter defaults can replace the function's prototype.
            let generator_proto = generator_function.and_then(|function|
                match function.borrow().get_own("prototype") {
                    Some(JsValue::Object(proto)) => Some(proto),
                    _ => None,
                });
            let mut object = JsObject::ordinary();
            object.proto = generator_proto.or_else(|| Some(i.object_proto.clone()));
            object.exotic = crate::value::Exotic::Generator(Rc::new(RefCell::new(GeneratorState {
                code: self.clone(),
                env,
                frame: None,
                started: false,
                executing: false,
                completion: JsValue::Undefined,
                async_promise: None,
            })));
            return Ok(JsValue::object(object));
        }
        if self.0.is_async {
            return self.call_async(i, env);
        }
        self.0.program.execute(i, env, DEFAULT_INSTRUCTION_LIMIT)
    }

    /// Start the resumable body in the environment prepared by AST parameter binding.
    #[allow(dead_code)] // Consumed by the runtime async bridge in interp.rs.
    pub(crate) fn call_async_with_env(&self, i: &mut Interpreter, env: EnvHandle) -> Result<JsValue, Throw> {
        self.call_async(i, env)
    }

    fn call_async(&self, i: &mut Interpreter, env: EnvHandle) -> Result<JsValue, Throw> {
        let promise = new_promise(i, PromiseFate::Pending);
        let JsValue::Object(target) = &promise else { unreachable!() };
        let frame = match self.0.program.start_frame(i, env.clone(), DEFAULT_INSTRUCTION_LIMIT) {
            Ok(frame) => Some(frame),
            Err(Throw(error)) => {
                settle_promise(target, PromiseFate::Rejected(error), i)?;
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
            let awaited = async_resolve_value(i, value);
            let fulfilled = async_handler(i, state.clone(), false);
            let rejected = async_handler(i, state, true);
            let then = i.get_prop(&awaited, "then")?;
            i.call_value(&then, awaited, vec![fulfilled, rejected])?;
        }
        Ok(FrameOutcome::Completed(value)) => {
            async_complete(i, &target, value)?;
        }
        Ok(FrameOutcome::Yielded(_)) => unreachable!("async function yielded"),
        Ok(FrameOutcome::Suspended) => unreachable!("unbounded async slice suspended"),
        Err(Throw(error)) => settle_promise(&target, PromiseFate::Rejected(error), i)?,
    }
    Ok(())
}

fn async_resolve_value(i: &mut Interpreter, value: JsValue) -> JsValue {
    if matches!(&value, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Promise(_))) {
        return value;
    }
    let promise = new_promise(i, PromiseFate::Pending);
    let JsValue::Object(target) = &promise else { unreachable!() };
    let then = if value.is_object() {
        match i.get_prop(&value, "then") {
            Ok(then) => then,
            Err(Throw(error)) => {
                let _ = settle_promise(target, PromiseFate::Rejected(error), i);
                return promise;
            }
        }
    } else {
        JsValue::Undefined
    };
    if callable_function(&then).is_some() {
        async_enqueue_thenable(i, target, value, then);
    } else {
        let _ = settle_promise(target, PromiseFate::Fulfilled(value), i);
    }
    promise
}

fn async_complete(i: &mut Interpreter, target: &Obj, value: JsValue) -> Result<(), Throw> {
    if matches!(&value, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Promise(_))) {
        if adopt_promise_result(target, &value, i)? {
            return Ok(());
        }
    }
    if value.is_object() {
        let then = match i.get_prop(&value, "then") {
            Ok(then) => then,
            Err(Throw(error)) => return settle_promise(target, PromiseFate::Rejected(error), i),
        };
        if callable_function(&then).is_some() {
            async_enqueue_thenable(i, target, value, then);
            return Ok(());
        }
    }
    settle_promise(target, PromiseFate::Fulfilled(value), i)
}

fn async_enqueue_thenable(i: &mut Interpreter, target: &Obj, value: JsValue, then: JsValue) {
    let job = Interpreter::native("PromiseResolveThenableJob", async_thenable_job, &i.function_proto);
    if let JsValue::Object(object) = &job {
        let mut object = object.borrow_mut();
        object.set_own("[[Then]]", then);
        object.set_own("[[Thenable]]", value);
        object.set_own("[[Target]]", JsValue::Object(target.clone()));
    }
    if let Err(Throw(error)) = i.queue_microtask(job) {
        let _ = settle_promise(target, PromiseFate::Rejected(error), i);
    }
}

fn async_thenable_job(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let native = i.native_stack.last().cloned().ok_or_else(|| Throw::type_error("missing thenable job"))?;
    let (then, thenable, target) = {
        let object = native.borrow();
        let then = object.get_own("[[Then]]").ok_or_else(|| Throw::type_error("missing thenable method"))?;
        let thenable = object.get_own("[[Thenable]]").ok_or_else(|| Throw::type_error("missing thenable receiver"))?;
        let Some(JsValue::Object(target)) = object.get_own("[[Target]]") else {
            return Err(Throw::type_error("missing thenable promise"));
        };
        (then, thenable, target)
    };
    let resolve = promise_callback(i, "resolve", &target, native_promise_resolve);
    let reject = promise_callback(i, "reject", &target, native_promise_reject);
    if let Err(Throw(error)) = i.call_value(&then, thenable, vec![resolve, reject]) {
        settle_promise(&target, PromiseFate::Rejected(error), i)?;
    }
    Ok(JsValue::Undefined)
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
    generator_call(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 0)
}

fn generator_return(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    generator_call(i, this, args.into_iter().next().unwrap_or(JsValue::Undefined), 1)
}

fn generator_throw(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
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
        Err(Throw(error)) => settle_promise(target, PromiseFate::Rejected(error), i)?,
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
    let mut outcome = resume_frame_with_strictness(i, &code, &mut frame);
    if code.0.is_async {
        for _ in 0..1_000 {
            let value = match &outcome {
                Ok(FrameOutcome::Awaited(value)) => value.clone(),
                _ => break,
            };
            let destination = frame.await_dst.take().expect("await instruction destination");
            match i.await_iteration_value(value) {
                Ok(value) => frame.registers[destination] = value,
                Err(error) => frame.pending_resume = Some(Abrupt::Throw(error)),
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

fn consume(fuel: &mut usize) -> Result<(), Throw> {
    if *fuel == 0 {
        return Err(Throw::range_error("bytecode instruction limit exceeded"));
    }
    *fuel -= 1;
    Ok(())
}
