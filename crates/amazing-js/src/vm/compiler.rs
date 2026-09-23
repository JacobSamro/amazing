use super::{
    CallArgument, ClassCode, ClassMethodCode, CompileError, CompiledFunction, Condition, FunctionCode, HoistedFunction, Instruction,
    KeyOperand, Program, Register,
};
use crate::ast::{
    self, AssignOp, Binding, Expr, ExprKind, LogicalOp, Stmt, StmtKind, UnOp, VarKind,
};
use crate::tokens::Span;
use crate::value::{JsString, JsValue};
use std::rc::Rc;

const MAX_REGISTERS: usize = 65_536;
const MAX_INSTRUCTIONS: usize = 262_144;
// Keep recursive compilation below the Windows test host's main-thread stack.
// Deeper valid syntax falls back atomically before instruction execution.
const MAX_DEPTH: usize = 64;

/// Compile the entire AST before any binding mutation or JavaScript execution.
pub fn compile(ast: &ast::Program) -> Result<Program, CompileError> {
    let mut c = Compiler::new(false);
    c.hoist_functions(&ast.body)?;
    for statement in &ast.body {
        c.statement(statement)?;
    }
    if c.program.code.len() > MAX_INSTRUCTIONS {
        return Err(c.error(ast.span, "instruction limit"));
    }
    Ok(c.program)
}

struct Compiler {
    program: Program,
    depth: usize,
    is_generator: bool,
    is_async: bool,
}

impl Compiler {
    fn new(is_function: bool) -> Self {
        Self {
            program: Program {
                code: vec![],
                registers: 1,
                references: 0,
                spread_slots: 0,
                hoisted: vec![],
                functions: vec![],
                is_function,
            },
            depth: 0,
            is_generator: false,
            is_async: false,
        }
    }

    fn compile_function(
        &self,
        function: &ast::FunctionItem,
        span: Span,
    ) -> Result<FunctionCode, CompileError> {
        if function.is_async && function.is_generator {
            return Err(self.error(span, "async generator function"));
        }
        let mut params = Vec::new();
        for param in &function.params {
            if param.default.is_some() || param.rest {
                return Err(self.error(param.span, "parameter pattern"));
            }
            let Binding::Ident(name) = &param.binding else {
                return Err(self.error(param.span, "parameter pattern"));
            };
            params.push(name.name.clone());
        }
        let mut child = Self::new(true);
        child.is_generator = function.is_generator;
        child.is_async = function.is_async;
        child.hoist_functions(&function.body.body)?;
        for statement in &function.body.body {
            child.statement(statement)?;
        }
        if child.program.code.len() > MAX_INSTRUCTIONS {
            return Err(self.error(span, "instruction limit"));
        }
        Ok(FunctionCode(Rc::new(CompiledFunction {
            name: function
                .name
                .as_ref()
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            params,
            program: child.program,
            is_arrow: false,
            is_generator: function.is_generator,
            is_async: function.is_async,
            is_method: false,
        })))
    }

    fn compile_arrow(
        &self,
        arrow: &ast::ArrowFunc,
        span: Span,
    ) -> Result<FunctionCode, CompileError> {
        if arrow.is_generator {
            return Err(self.error(span, "generator arrow"));
        }
        let mut params = Vec::new();
        for param in &arrow.params {
            if param.default.is_some() || param.rest {
                return Err(self.error(param.span, "parameter pattern"));
            }
            let Binding::Ident(name) = &param.binding else {
                return Err(self.error(param.span, "parameter pattern"));
            };
            params.push(name.name.clone());
        }
        let mut child = Self::new(true);
        child.is_async = arrow.is_async;
        match &arrow.body {
            ast::ArrowBody::Expr(expression) => {
                let value = child.expression(expression)?;
                child.emit(Instruction::Return(value));
            }
            ast::ArrowBody::Block(block) => {
                child.hoist_functions(&block.body)?;
                for statement in &block.body {
                    child.statement(statement)?;
                }
            }
        }
        Ok(FunctionCode(Rc::new(CompiledFunction {
            name: String::new(),
            params,
            program: child.program,
            is_arrow: true,
            is_generator: false,
            is_async: arrow.is_async,
            is_method: false,
        })))
    }

    fn compile_method(
        &self,
        method: &ast::ClassMethod,
    ) -> Result<FunctionCode, CompileError> {
        let function = ast::FunctionItem {
            name: None,
            params: method.params.clone(),
            body: method.body.clone(),
            is_async: method.is_async,
            is_generator: method.is_generator,
        };
        let mut code = self.compile_function(&function, method.span)?;
        Rc::get_mut(&mut code.0).expect("new method code").is_method = true;
        Ok(code)
    }

    fn compile_class(&mut self, class: &ast::ClassItem, span: Span) -> Result<ClassCode, CompileError> {
        if class.super_class.is_some() {
            return Err(self.error(span, "derived class"));
        }
        let constructor = class.members.iter().find_map(|member| match member {
            ast::ClassMember::Method(method) if method.kind == ast::MethodKind::Constructor => Some(method),
            _ => None,
        });
        let function = ast::FunctionItem {
            name: class.name.clone(),
            params: constructor.map_or_else(Vec::new, |method| method.params.clone()),
            body: constructor.map_or_else(
                || ast::Block { body: Vec::new(), span },
                |method| method.body.clone(),
            ),
            is_async: false,
            is_generator: false,
        };
        let constructor = self.compile_function(&function, span)?;
        let mut methods = Vec::new();
        for member in &class.members {
            if let ast::ClassMember::Method(method) = member {
                if method.kind == ast::MethodKind::Constructor { continue; }
                let key = self.object_key(&method.key, method.key_utf16.as_deref(), method.span)?;
                let code = self.compile_method(method)?;
                methods.push(ClassMethodCode { key, kind: method.kind, code });
            }
        }
        Ok(ClassCode { constructor, methods })
    }

    fn hoist_functions(&mut self, statements: &[Stmt]) -> Result<(), CompileError> {
        for statement in statements {
            match &statement.kind {
                StmtKind::Function(function) => {
                    let Some(name) = &function.name else {
                        return Err(self.error(statement.span, "anonymous declaration"));
                    };
                    let code = self.compile_function(function, statement.span)?;
                    self.program.functions.push(HoistedFunction {
                        name: name.name.clone(),
                        code,
                    });
                }
                StmtKind::Block(block) => self.hoist_functions(&block.body)?,
                StmtKind::If { then, els, .. } => {
                    self.hoist_functions(std::slice::from_ref(then.as_ref()))?;
                    if let Some(els) = els {
                        self.hoist_functions(std::slice::from_ref(els.as_ref()))?;
                    }
                }
                StmtKind::While { body, .. }
                | StmtKind::DoWhile { body, .. }
                | StmtKind::For { body, .. } => {
                    self.hoist_functions(std::slice::from_ref(body.as_ref()))?;
                }
                StmtKind::Try(statement) => {
                    self.hoist_functions(&statement.block.body)?;
                    if let Some(catch) = &statement.catch {
                        self.hoist_functions(&catch.body.body)?;
                    }
                    if let Some(finally) = &statement.finally {
                        self.hoist_functions(&finally.body)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn error(&self, span: Span, message: &'static str) -> CompileError {
        CompileError { span, message }
    }

    fn enter(&mut self, span: Span) -> Result<(), CompileError> {
        if self.depth >= MAX_DEPTH || self.program.code.len() >= MAX_INSTRUCTIONS {
            return Err(self.error(span, "compiler resource limit"));
        }
        self.depth += 1;
        Ok(())
    }

    fn register(&mut self, span: Span) -> Result<Register, CompileError> {
        let register = self.program.registers;
        if register >= MAX_REGISTERS {
            return Err(self.error(span, "register limit"));
        }
        self.program.registers += 1;
        Ok(register)
    }

    fn emit(&mut self, op: Instruction) -> usize {
        let index = self.program.code.len();
        self.program.code.push(op);
        index
    }

    fn patch(&mut self, index: usize) {
        let end = self.program.code.len();
        match &mut self.program.code[index] {
            Instruction::Jump(target) | Instruction::Branch(_, _, target) => *target = end,
            _ => unreachable!("compiler only patches branches"),
        }
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        self.enter(statement.span)?;
        let result = self.statement_inner(statement);
        self.depth -= 1;
        result
    }

    fn statement_inner(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match &statement.kind {
            StmtKind::Empty | StmtKind::Debugger | StmtKind::Function(_) => {}
            StmtKind::Expression(expression) => {
                let value = self.expression(expression)?;
                self.emit(Instruction::Complete(value));
            }
            StmtKind::Var(declaration) => {
                for binding in &declaration.decls {
                    if self.program.code.len() >= MAX_INSTRUCTIONS {
                        return Err(self.error(binding.span, "instruction limit"));
                    }
                    let Binding::Ident(name) = &binding.binding else {
                        return Err(self.error(binding.span, "binding pattern"));
                    };
                    if declaration.kind == VarKind::Var {
                        self.program.hoisted.push(name.name.clone());
                    }
                    if declaration.kind == VarKind::Const && binding.init.is_none() {
                        return Err(self.error(binding.span, "const requires an initializer"));
                    }
                    let value = match &binding.init {
                        Some(expression) => self.expression(expression)?,
                        None => 0, // Register zero is immutable undefined.
                    };
                    self.emit(Instruction::Declare(
                        name.name.clone(),
                        value,
                        declaration.kind,
                    ));
                }
            }
            StmtKind::Class(class) => {
                let name = class.name.as_ref().ok_or_else(|| self.error(statement.span, "anonymous class declaration"))?;
                let code = self.compile_class(class, statement.span)?;
                let dst = self.register(statement.span)?;
                self.emit(Instruction::MakeClass(dst, code));
                self.emit(Instruction::Declare(name.name.clone(), dst, VarKind::Let));
            }
            StmtKind::Block(block) => {
                self.emit(Instruction::Complete(0));
                self.emit(Instruction::EnterScope);
                for child in &block.body {
                    self.statement(child)?;
                }
                self.emit(Instruction::LeaveScope);
            }
            StmtKind::If { cond, then, els } => {
                self.emit(Instruction::Complete(0));
                let test = self.expression(cond)?;
                let branch = self.emit(Instruction::Branch(test, Condition::Falsy, 0));
                self.statement(then)?;
                let end = self.emit(Instruction::Jump(0));
                self.patch(branch);
                if let Some(els) = els {
                    self.statement(els)?;
                }
                self.patch(end);
            }
            StmtKind::While { test, body } => {
                self.emit(Instruction::Complete(0));
                let start = self.program.code.len();
                let test = self.expression(test)?;
                let end = self.emit(Instruction::Branch(test, Condition::Falsy, 0));
                self.statement(body)?;
                self.emit(Instruction::Jump(start));
                self.patch(end);
            }
            StmtKind::DoWhile { body, test } => {
                self.emit(Instruction::Complete(0));
                let start = self.program.code.len();
                self.statement(body)?;
                let condition = self.expression(test)?;
                self.emit(Instruction::Branch(condition, Condition::Truthy, start));
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                self.emit(Instruction::Complete(0));
                match init {
                    Some(ast::ForInit::Expr(expression)) => {
                        self.expression(expression)?;
                    }
                    Some(ast::ForInit::Decl(declaration)) if declaration.kind == VarKind::Var => {
                        self.statement(&Stmt::new(
                            StmtKind::Var((**declaration).clone()),
                            statement.span,
                        ))?;
                    }
                    Some(ast::ForInit::Decl(_)) => {
                        return Err(self.error(statement.span, "lexical for binding"))
                    }
                    None => {}
                }
                let start = self.program.code.len();
                let end = if let Some(test) = test {
                    let value = self.expression(test)?;
                    Some(self.emit(Instruction::Branch(value, Condition::Falsy, 0)))
                } else {
                    None
                };
                self.statement(body)?;
                if let Some(update) = update {
                    self.expression(update)?;
                }
                self.emit(Instruction::Jump(start));
                if let Some(end) = end {
                    self.patch(end);
                }
            }
            StmtKind::Throw(expression) => {
                let value = self.expression(expression)?;
                self.emit(Instruction::Throw(value));
            }
            StmtKind::Try(statement) => {
                self.emit(Instruction::Complete(0));
                let handler = self.emit(Instruction::PushTry {
                    catch: None,
                    finally: None,
                    end: 0,
                });
                self.emit(Instruction::EnterScope);
                for child in &statement.block.body {
                    self.statement(child)?;
                }
                self.emit(Instruction::LeaveScope);
                self.emit(Instruction::TryNormal);

                let catch = if let Some(catch) = &statement.catch {
                    let start = self.program.code.len();
                    self.emit(Instruction::EnterCatch(
                        catch.param.as_ref().map(|param| param.name.clone()),
                    ));
                    for child in &catch.body.body {
                        self.statement(child)?;
                    }
                    self.emit(Instruction::LeaveScope);
                    self.emit(Instruction::TryNormal);
                    Some(start)
                } else {
                    None
                };

                let finally = if let Some(finally) = &statement.finally {
                    let start = self.program.code.len();
                    self.emit(Instruction::EnterScope);
                    for child in &finally.body {
                        self.statement(child)?;
                    }
                    self.emit(Instruction::LeaveScope);
                    self.emit(Instruction::EndFinally);
                    Some(start)
                } else {
                    None
                };
                let end = self.program.code.len();
                self.program.code[handler] = Instruction::PushTry {
                    catch,
                    finally,
                    end,
                };
            }
            StmtKind::Return(expression) if self.program.is_function => {
                let value = if let Some(expression) = expression {
                    self.expression(expression)?
                } else {
                    0
                };
                self.emit(Instruction::Return(value));
            }
            _ => return Err(self.error(statement.span, unsupported_statement(&statement.kind))),
        }
        Ok(())
    }

    fn expression(&mut self, expression: &Expr) -> Result<Register, CompileError> {
        self.enter(expression.span)?;
        let result = self.expression_inner(expression);
        self.depth -= 1;
        result
    }

    fn expression_inner(&mut self, expression: &Expr) -> Result<Register, CompileError> {
        let dst = self.register(expression.span)?;
        match &expression.kind {
            ExprKind::Num(number) => {
                self.emit(Instruction::Numeric(dst, number.clone()));
            }
            ExprKind::RegExp(regex) => {
                self.emit(Instruction::RegExp(dst, regex.clone()));
            }
            ExprKind::Str(value) => {
                let value = expression
                    .string_utf16
                    .as_ref()
                    .map(|units| JsString::from_units(units.clone()))
                    .unwrap_or_else(|| value.as_str().into());
                self.emit(Instruction::Constant(dst, JsValue::Str(value)));
            }
            ExprKind::Bool(value) => {
                self.emit(Instruction::Constant(dst, JsValue::Bool(*value)));
            }
            ExprKind::Null => {
                self.emit(Instruction::Constant(dst, JsValue::Null));
            }
            ExprKind::Yield { argument, delegate: false } if self.is_generator => {
                let value = match argument {
                    Some(argument) => self.expression(argument)?,
                    None => 0,
                };
                self.emit(Instruction::Yield(dst, value));
            }
            ExprKind::Yield { argument: Some(argument), delegate: true } if self.is_generator => {
                let value = self.expression(argument)?;
                self.emit(Instruction::YieldDelegate(dst, value));
            }
            ExprKind::Unary { op: UnOp::Await, operand } if self.is_async => {
                let value = self.expression(operand)?;
                self.emit(Instruction::Await(dst, value));
            }
            ExprKind::Array(elements) => {
                self.emit(Instruction::NewArray(dst));
                if elements.iter().any(|element| element.as_ref().is_some_and(|element| matches!(element.kind, ExprKind::Spread(_)))) {
                    let index = self.register(expression.span)?;
                    self.emit(Instruction::Constant(index, JsValue::num(0.0)));
                    for element in elements {
                        match element {
                            Some(Expr { kind: ExprKind::Spread(source), .. }) => {
                                let source = self.expression(source)?;
                                self.emit(Instruction::AppendArraySpread(dst, index, source));
                            }
                            Some(element) => {
                                let source = self.expression(element)?;
                                self.emit(Instruction::AppendArrayElement(dst, index, Some(source)));
                            }
                            None => { self.emit(Instruction::AppendArrayElement(dst, index, None)); }
                        }
                    }
                    self.emit(Instruction::SetArrayLengthFrom(dst, index));
                } else {
                    for (index, element) in elements.iter().enumerate() {
                        if let Some(element) = element {
                            let value = self.expression(element)?;
                            self.emit(Instruction::SetArrayElement(dst, index, value));
                        }
                    }
                    self.emit(Instruction::SetArrayLength(dst, elements.len()));
                }
            }
            ExprKind::Object(elements) => {
                self.emit(Instruction::NewObject(dst));
                for element in elements {
                    let ast::ObjElem::Prop(property) = element else {
                        let ast::ObjElem::Spread(source) = element else { unreachable!() };
                        let source = self.expression(source)?;
                        self.emit(Instruction::SpreadObject(dst, source));
                        continue;
                    };
                    let key = self.object_key(
                        &property.key,
                        property.key_utf16.as_deref(),
                        property.span,
                    )?;
                    let value = match &property.value {
                        ast::PropValue::Shorthand(name) => {
                            let value = self.register(property.span)?;
                            self.emit(Instruction::LoadName(value, name.name.clone(), false));
                            value
                        }
                        ast::PropValue::Init(value) => self.expression(value)?,
                        ast::PropValue::Method(method) => {
                            let code = self.compile_method(method)?;
                            self.emit(Instruction::SetObjectMethod(dst, key, code, method.kind));
                            continue;
                        }
                    };
                    self.emit(Instruction::SetObjectProperty(dst, key, value));
                }
            }
            ExprKind::This => {
                self.emit(Instruction::LoadThis(dst));
            }
            ExprKind::Template(template) => {
                self.emit(Instruction::Constant(dst, JsValue::str("")));
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let expression = template.exprs.get(index)
                        .map(|expression| self.expression(expression)).transpose()?;
                    let units = template.quasis_utf16.get(index).cloned()
                        .unwrap_or_else(|| quasi.encode_utf16().collect());
                    self.emit(Instruction::TemplateAppend(dst, units, expression));
                }
            }
            ExprKind::Function(function) => {
                if function.is_generator && function.name.is_some() {
                    return Err(self.error(expression.span, "named generator expression binding"));
                }
                let code = self.compile_function(function, expression.span)?;
                self.emit(Instruction::MakeFunction(dst, code));
            }
            ExprKind::Class(class) => {
                let code = self.compile_class(class, expression.span)?;
                self.emit(Instruction::MakeClass(dst, code));
            }
            ExprKind::Arrow(arrow) => {
                let code = self.compile_arrow(arrow, expression.span)?;
                self.emit(Instruction::MakeFunction(dst, code));
            }
            ExprKind::Ident(name) => {
                self.emit(Instruction::LoadName(dst, name.name.clone(), false));
            }
            ExprKind::Member {
                obj,
                prop,
                optional: false,
            } => {
                let object = self.expression(obj)?;
                let key = self.member_key(prop)?;
                self.emit(Instruction::GetMember(dst, object, key));
            }
            ExprKind::Paren(inner) => return self.expression(inner),
            ExprKind::Sequence(items) => {
                let mut last = 0;
                for item in items {
                    last = self.expression(item)?;
                }
                return Ok(last);
            }
            ExprKind::Unary { op, operand } if !matches!(op, UnOp::Await | UnOp::Delete) => {
                // Match v1's special handling of typeof x and typeof (x).
                let name = match &operand.kind {
                    ExprKind::Ident(name) => Some(name),
                    ExprKind::Paren(inner) => match &inner.kind {
                        ExprKind::Ident(name) => Some(name),
                        _ => None,
                    },
                    _ => None,
                };
                let src = if let (UnOp::Typeof, Some(name)) = (op, name) {
                    let src = self.register(operand.span)?;
                    self.emit(Instruction::LoadName(src, name.name.clone(), true));
                    src
                } else {
                    self.expression(operand)?
                };
                self.emit(Instruction::Unary(dst, *op, src));
            }
            ExprKind::Binary { op, left, right } => {
                let left = self.expression(left)?;
                let right = self.expression(right)?;
                self.emit(Instruction::Binary(dst, *op, left, right));
            }
            ExprKind::Call {
                callee,
                args,
                optional: false,
            } => {
                let (callee, this) = match &callee.kind {
                    ExprKind::Member {
                        obj,
                        prop,
                        optional: false,
                    } => {
                        let object = self.expression(obj)?;
                        let key = self.member_key(prop)?;
                        let function = self.register(callee.span)?;
                        self.emit(Instruction::GetMember(function, object, key));
                        (function, object)
                    }
                    _ => (self.expression(callee)?, 0),
                };
                let mut arguments = Vec::new();
                for argument in args {
                    match &argument.kind {
                        ExprKind::Spread(source) => {
                            let source = self.expression(source)?;
                            let slot = self.program.spread_slots;
                            self.program.spread_slots += 1;
                            self.emit(Instruction::CaptureSpread(slot, source));
                            arguments.push(CallArgument::Spread(slot));
                        }
                        _ => arguments.push(CallArgument::Value(self.expression(argument)?)),
                    }
                }
                self.emit(Instruction::Call(dst, callee, this, arguments));
            }
            ExprKind::New { callee, args } => {
                let callee = self.expression(callee)?;
                let mut arguments = Vec::new();
                for argument in args {
                    match &argument.kind {
                        ExprKind::Spread(source) => {
                            let source = self.expression(source)?;
                            let slot = self.program.spread_slots;
                            self.program.spread_slots += 1;
                            self.emit(Instruction::CaptureSpread(slot, source));
                            arguments.push(CallArgument::Spread(slot));
                        }
                        _ => arguments.push(CallArgument::Value(self.expression(argument)?)),
                    }
                }
                self.emit(Instruction::Construct(dst, callee, arguments));
            }
            ExprKind::Logical { op, left, right } => {
                let left = self.expression(left)?;
                self.emit(Instruction::Move(dst, left));
                let condition = match op {
                    LogicalOp::And => Condition::Truthy,
                    LogicalOp::Or => Condition::Falsy,
                    LogicalOp::Coalesce => Condition::Nullish,
                };
                let rhs = self.emit(Instruction::Branch(left, condition, 0));
                let end = self.emit(Instruction::Jump(0));
                self.patch(rhs);
                let right = self.expression(right)?;
                self.emit(Instruction::Move(dst, right));
                self.patch(end);
            }
            ExprKind::Conditional {
                test,
                consequent,
                alternate,
            } => {
                let test = self.expression(test)?;
                let branch = self.emit(Instruction::Branch(test, Condition::Falsy, 0));
                let yes = self.expression(consequent)?;
                self.emit(Instruction::Move(dst, yes));
                let end = self.emit(Instruction::Jump(0));
                self.patch(branch);
                let no = self.expression(alternate)?;
                self.emit(Instruction::Move(dst, no));
                self.patch(end);
            }
            ExprKind::Assign {
                op: AssignOp::Assign,
                target,
                value,
            } => {
                let (slot, _) = self.reference(target)?;
                let value = self.expression(value)?;
                self.emit(Instruction::StoreReference(slot, value));
                return Ok(value);
            }
            ExprKind::Assign { op, target, value } => {
                let (slot, old) = self.reference(target)?;
                self.emit(Instruction::RequireResolved(slot));
                let binary = match op {
                    AssignOp::Add => Some(ast::BinOp::Add),
                    AssignOp::Sub => Some(ast::BinOp::Sub),
                    AssignOp::Mul => Some(ast::BinOp::Mul),
                    AssignOp::Div => Some(ast::BinOp::Div),
                    AssignOp::Rem => Some(ast::BinOp::Rem),
                    AssignOp::Pow => Some(ast::BinOp::Pow),
                    AssignOp::Shl => Some(ast::BinOp::Shl),
                    AssignOp::Shr => Some(ast::BinOp::Shr),
                    AssignOp::UShr => Some(ast::BinOp::UShr),
                    AssignOp::BitAnd => Some(ast::BinOp::BitAnd),
                    AssignOp::BitOr => Some(ast::BinOp::BitOr),
                    AssignOp::BitXor => Some(ast::BinOp::BitXor),
                    _ => None,
                };
                if let Some(binary) = binary {
                    let right = self.expression(value)?;
                    self.emit(Instruction::Binary(dst, binary, old, right));
                    self.emit(Instruction::StoreReference(slot, dst));
                } else {
                    self.emit(Instruction::Move(dst, old));
                    let condition = match op {
                        AssignOp::And => Condition::Truthy,
                        AssignOp::Or => Condition::Falsy,
                        AssignOp::Nullish => Condition::Nullish,
                        _ => return Err(self.error(expression.span, "assignment operator")),
                    };
                    let rhs = self.emit(Instruction::Branch(old, condition, 0));
                    let end = self.emit(Instruction::Jump(0));
                    self.patch(rhs);
                    let right = self.expression(value)?;
                    self.emit(Instruction::Move(dst, right));
                    self.emit(Instruction::StoreReference(slot, dst));
                    self.patch(end);
                }
            }
            ExprKind::Update { op, prefix, target } => {
                let (slot, old) = self.reference(target)?;
                self.emit(Instruction::Update(dst, slot, old, *op, *prefix));
            }
            _ => return Err(self.error(expression.span, unsupported_expression(&expression.kind))),
        }
        Ok(dst)
    }

    fn reference(&mut self, expression: &Expr) -> Result<(usize, Register), CompileError> {
        let mut target = expression;
        while let ExprKind::Paren(inner) = &target.kind {
            target = inner;
        }
        let slot = self.program.references;
        self.program.references += 1;
        let old = self.register(expression.span)?;
        match &target.kind {
            ExprKind::Ident(name) => {
                self.emit(Instruction::ResolveName(slot, old, name.name.clone()));
            }
            ExprKind::Member {
                obj,
                prop,
                optional: false,
            } => {
                let object = self.expression(obj)?;
                let key = self.member_key(prop)?;
                self.emit(Instruction::ResolveMember(slot, old, object, key));
            }
            _ => return Err(self.error(target.span, "assignment target")),
        }
        Ok((slot, old))
    }

    fn member_key(&mut self, prop: &ast::MemberProp) -> Result<KeyOperand, CompileError> {
        match prop {
            ast::MemberProp::Ident(name) => Ok(KeyOperand::Static(name.name.as_str().into())),
            ast::MemberProp::Computed(expression) => {
                let register = self.expression(expression)?;
                self.emit(Instruction::ToKey(register));
                Ok(KeyOperand::Dynamic(register))
            }
            ast::MemberProp::Private(name) => Err(self.error(name.span, "private member")),
        }
    }

    fn object_key(
        &mut self,
        key: &ast::PropKey,
        utf16: Option<&[u16]>,
        span: Span,
    ) -> Result<KeyOperand, CompileError> {
        if let Some(units) = utf16 {
            return Ok(KeyOperand::Static(JsString::from_units(units.to_vec())));
        }
        match key {
            ast::PropKey::Ident(name) => Ok(KeyOperand::Static(name.name.as_str().into())),
            ast::PropKey::Str(value) => Ok(KeyOperand::Static(value.as_str().into())),
            ast::PropKey::Num(number) => {
                let register = self.register(span)?;
                self.emit(Instruction::Numeric(register, number.clone()));
                self.emit(Instruction::ToKey(register));
                Ok(KeyOperand::Dynamic(register))
            }
            ast::PropKey::Computed(expression) => {
                let register = self.expression(expression)?;
                self.emit(Instruction::ToKey(register));
                Ok(KeyOperand::Dynamic(register))
            }
            ast::PropKey::Private(name) => Err(self.error(name.span, "private object key")),
        }
    }
}

fn unsupported_statement(kind: &StmtKind) -> &'static str {
    match kind {
        StmtKind::ForIn { .. } => "for-in",
        StmtKind::ForOf { .. } => "for-of",
        StmtKind::Switch { .. } => "switch",
        StmtKind::Try(_) => "try",
        StmtKind::Break { .. } => "break",
        StmtKind::Continue { .. } => "continue",
        StmtKind::Labeled { .. } => "labeled statement",
        StmtKind::Class(_) => "class",
        _ => "statement kind",
    }
}

fn unsupported_expression(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Array(_) => "array literal",
        ExprKind::Object(_) => "object literal",
        ExprKind::Member { .. } => "member access",
        ExprKind::New { .. } => "new",
        ExprKind::Template(_) => "template literal",
        ExprKind::RegExp(_) => "regexp literal",
        ExprKind::Call { optional: true, .. } => "optional call",
        ExprKind::Spread(_) => "spread",
        ExprKind::Class(_) => "class",
        ExprKind::Yield { .. } => "yield",
        ExprKind::Super => "super",
        ExprKind::PrivateName(_) => "private name",
        ExprKind::Unary {
            op: UnOp::Delete, ..
        } => "delete",
        ExprKind::Unary {
            op: UnOp::Await, ..
        } => "await",
        _ => "expression kind",
    }
}
