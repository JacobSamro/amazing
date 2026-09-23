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
use crate::value::{FunctionBody, JsFunction, JsObject, JsValue, Obj, Throw};
use std::cell::RefCell;
use std::rc::Rc;

pub use compiler::compile;

pub const DEFAULT_INSTRUCTION_LIMIT: usize = 1_000_000;
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
enum Instruction {
    Constant(Register, JsValue),
    Numeric(Register, Numeric),
    LoadName(Register, String, bool),
    LoadThis(Register),
    ResolveName(usize, Register, String),
    StoreReference(usize, Register),
    Declare(String, Register, VarKind),
    Unary(Register, UnOp, Register),
    Binary(Register, BinOp, Register, Register),
    Move(Register, Register),
    Jump(usize),
    Branch(Register, Condition, usize),
    EnterScope,
    LeaveScope,
    Complete(Register),
    Return(Register),
    MakeFunction(Register, FunctionCode),
    Call(Register, Register, Vec<Register>),
    Throw(Register),
}

#[derive(Debug, Clone)]
pub struct FunctionCode(Rc<CompiledFunction>);

#[derive(Debug)]
struct CompiledFunction {
    name: String,
    params: Vec<String>,
    program: Program,
    is_arrow: bool,
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

    fn execute(
        &self,
        i: &mut Interpreter,
        mut env: EnvHandle,
        mut fuel: usize,
    ) -> Result<JsValue, Throw> {
        let mut registers = vec![JsValue::Undefined; self.registers];
        let mut references: Vec<Option<Reference>> = (0..self.references).map(|_| None).collect();
        let mut scopes = Vec::new();
        for name in &self.hoisted {
            consume(&mut fuel)?;
            env.declare_var(name, JsValue::Undefined)?;
        }
        for function in &self.functions {
            consume(&mut fuel)?;
            let value = function.code.instantiate(i, &env);
            env.declare_var(&function.name, value)?;
        }
        let mut pc = 0;
        while let Some(op) = self.code.get(pc) {
            consume(&mut fuel)?;
            pc += 1;
            match op {
                Instruction::Constant(dst, value) => registers[*dst] = value.clone(),
                Instruction::Numeric(dst, number) => registers[*dst] = i.numeric(number)?,
                Instruction::LoadName(dst, name, optional) => {
                    registers[*dst] = match i.identifier_value(&env, name)? {
                        Some(value) => value,
                        None if *optional => JsValue::Undefined,
                        None => {
                            return Err(Throw::reference_error(format!("{name} is not defined")));
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
                Instruction::Declare(name, src, kind) => match kind {
                    VarKind::Var => {
                        if !env.assign(name, registers[*src].clone())? {
                            i.global_env.declare_var(name, registers[*src].clone())?;
                        }
                    }
                    VarKind::Let | VarKind::Const => {
                        env.declare_lexical(name, registers[*src].clone(), *kind == VarKind::Let)?;
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
                Instruction::Jump(target) => pc = *target,
                Instruction::Branch(src, condition, target) => {
                    let value = &registers[*src];
                    if match condition {
                        Condition::Truthy => value.to_boolean(),
                        Condition::Falsy => !value.to_boolean(),
                        Condition::Nullish => value.is_null() || value.is_undefined(),
                    } {
                        pc = *target;
                    }
                }
                Instruction::EnterScope => {
                    scopes.push(env.clone());
                    env = EnvHandle::new(EnvKind::Block, Some(env));
                }
                Instruction::LeaveScope => {
                    env = scopes
                        .pop()
                        .ok_or_else(|| Throw::type_error("invalid VM scope"))?;
                }
                Instruction::Complete(src) => i.completion = registers[*src].clone(),
                Instruction::Return(src) => return Ok(registers[*src].clone()),
                Instruction::MakeFunction(dst, code) => {
                    registers[*dst] = code.instantiate(i, &env);
                }
                Instruction::Call(dst, callee, arguments) => {
                    let callable = match &registers[*callee] {
                        JsValue::Object(object) => object
                            .borrow()
                            .func
                            .as_ref()
                            .is_some_and(|f| matches!(&f.body, FunctionBody::Bytecode(_))),
                        _ => false,
                    };
                    if !callable {
                        return Err(Throw::type_error("VM call target is not bytecode"));
                    }
                    let args = arguments.iter().map(|r| registers[*r].clone()).collect();
                    registers[*dst] =
                        i.call_value(&registers[*callee], JsValue::Undefined, args)?;
                }
                Instruction::Throw(src) => return Err(Throw(registers[*src].clone())),
            }
        }
        if self.is_function {
            Ok(JsValue::Undefined)
        } else {
            if i.external_scheduling.is_none() {
                i.drain_jobs()?;
            }
            Ok(i.completion.clone())
        }
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
            is_generator: false,
            env: env.clone(),
            home: None,
            proto_for_new: None,
        });
        object.proto = Some(i.function_proto.clone());
        let handle = Obj(Rc::new(RefCell::new(object)));
        handle
            .borrow_mut()
            .set_own("name", JsValue::str(name.as_str()));
        if !self.0.is_arrow {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(i.object_proto.clone());
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
        self.0.program.execute(i, env, DEFAULT_INSTRUCTION_LIMIT)
    }
}

fn consume(fuel: &mut usize) -> Result<(), Throw> {
    if *fuel == 0 {
        return Err(Throw::range_error("bytecode instruction limit exceeded"));
    }
    *fuel -= 1;
    Ok(())
}
