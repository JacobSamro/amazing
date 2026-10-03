use super::{
    CallArgument, ClassCode, ClassMethodCode, ClassSuper, CompileError, CompiledFunction, Condition, FunctionCode, HoistedFunction,
    Instruction, KeyOperand, Program, Register,
};
use crate::ast::{
    self, AssignOp, BinOp, Binding, Expr, ExprKind, LogicalOp, Stmt, StmtKind, UnOp, VarKind,
};
use crate::tokens::Span;
use crate::value::{JsString, JsValue};
use std::rc::Rc;

#[path = "compiler_patterns.rs"]
mod patterns;
use patterns::pattern_target;
#[path = "compiler_resources.rs"]
mod resources;

const MAX_REGISTERS: usize = 65_536;
const MAX_INSTRUCTIONS: usize = 262_144;
// Keep recursive compilation below the Windows test host's main-thread stack.
// Deeper valid syntax falls back atomically before instruction execution.
const MAX_DEPTH: usize = 64;

/// Opt-in measurement of actual runtime callable compilation attempts.
/// Append-only output lets Test262 host subprocesses share a single log.
fn record_compile_attempt(result: &Result<FunctionCode, CompileError>, params: &[ast::Param], body: &impl std::fmt::Debug) {
    if let Some(path) = std::env::var_os("AMAZING_JS_VM_COMPILE_LOG") {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let reason = match result { Ok(_) => "ok", Err(error) => error.message };
            // Format before appending so subprocesses cannot interleave the
            // reason and newline as separate writes.
            let _ = file.write_all(format!("{reason}\n").as_bytes());
        }
    }
    // Optional rejection context ranks the remaining fallback shapes without
    // changing the compact acceptance log used by existing gate evidence.
    if let Err(error) = result {
        if let Some(path) = std::env::var_os("AMAZING_JS_VM_REJECT_CONTEXT") {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(format!("{error:?}\tparams={params:?}\tbody={body:?}\n").as_bytes());
            }
        }
    }
}

fn has_use_strict_directive(statements: &[Stmt]) -> bool {
    statements.iter()
        .take_while(|stmt| matches!(&stmt.kind, StmtKind::Expression(Expr { kind: ExprKind::Str(_), .. })))
        .any(|stmt| matches!(&stmt.kind, StmtKind::Expression(Expr { kind: ExprKind::Str(value), .. }) if value == "use strict"))
}

/// Compile the entire AST before any binding mutation or JavaScript execution.
/// `is_module` forces strict mode even without a directive: module code is
/// always strict, per spec (matching `run_program`'s own runtime check of
/// `self.global_env.kind() == EnvKind::Module`, which this mirrors at compile
/// time since the compiler has no `Interpreter` to ask).
pub fn compile(ast: &ast::Program, is_module: bool) -> Result<Program, CompileError> {
    let mut c = Compiler::new(false);
    c.strict = is_module || has_use_strict_directive(&ast.body);
    c.hoist_functions(&ast.body)?;
    c.statements(&ast.body, ast.span)?;
    if c.program.code.len() > MAX_INSTRUCTIONS {
        return Err(c.error(ast.span, "instruction limit"));
    }
    Ok(c.program)
}

/// Compile a method body with the same path used for function declarations.
/// Callers retain the AST body when an instruction is not yet supported.
/// The sole caller is the tree walker's `method_value`, for both object-literal
/// methods and (non-VM-compiled-class) class methods; either way the result is
/// a method, not an ordinary function, so `is_method` must be set here --
/// `instantiate`'s own `!is_method || is_generator` guard is what actually
/// keeps a non-generator method from getting an own `prototype`. Relying on
/// deleting it afterward does not work now that `instantiate` gives that
/// property the spec-correct `configurable: false` descriptor.
pub(crate) fn compile_class_method(method: &ast::ClassMethod, strict: bool) -> Result<FunctionCode, CompileError> {
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    let result = compiler.compile_function(&ast::FunctionItem {
        name: None,
        params: method.params.clone(),
        body: method.body.clone(),
        is_async: method.is_async,
        is_generator: method.is_generator,
    }, method.span);
    record_compile_attempt(&result, &method.params, &method.body);
    let mut code = result?;
    Rc::get_mut(&mut code.0).expect("new method code").is_method = true;
    Ok(code)
}

pub(crate) fn compile_function_item(function: &ast::FunctionItem, span: Span, strict: bool)
    -> Result<FunctionCode, CompileError>
{
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    let result = compiler.compile_function(function, span);
    record_compile_attempt(&result, &function.params, &function.body);
    result
}

pub(crate) fn compile_arrow_item(arrow: &ast::ArrowFunc, span: Span, strict: bool)
    -> Result<FunctionCode, CompileError>
{
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    let result = compiler.compile_arrow(arrow, span);
    record_compile_attempt(&result, &arrow.params, &arrow.body);
    result
}

/// Compile an async AST body whose parameters have already been bound by the
/// interpreter. Compilation is atomic and does not execute JavaScript.
///
/// This is the VM half of the runtime async bridge: `interp.rs` binds the full
/// AST parameter list, then calls this and `FunctionCode::call_async_with_env`.
#[allow(dead_code)] // Consumed by the runtime async bridge in interp.rs.
pub(crate) fn compile_async_body(body: &ast::Block, strict: bool) -> Result<FunctionCode, CompileError> {
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    let result = compiler.compile_function(&ast::FunctionItem {
        name: None,
        params: vec![],
        body: body.clone(),
        is_async: true,
        is_generator: false,
    }, body.span);
    record_compile_attempt(&result, &[], body);
    result
}

/// Compile a generator AST body whose parameters have already been bound by
/// the interpreter. The generator/sync half of the runtime bridge, alongside
/// `compile_async_body`: `interp.rs` binds the full AST parameter list, then
/// calls this and `FunctionCode::start_generator_with_env`.
#[allow(dead_code)] // Consumed by the runtime generator bridge in interp.rs.
pub(crate) fn compile_generator_body(body: &ast::Block, strict: bool) -> Result<FunctionCode, CompileError> {
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    compiler.compile_function(&ast::FunctionItem {
        name: None,
        params: vec![],
        body: body.clone(),
        is_async: false,
        is_generator: true,
    }, body.span)
}

/// Compile an async generator AST body whose parameters have already been
/// bound by the interpreter -- the async-generator half of the runtime
/// bridge: `interp.rs` binds the full AST parameter list, then calls this
/// and `FunctionCode::start_generator_with_env`. The returned `FunctionCode`
/// has both `is_generator` and `is_async` set, so its VM frame suspends at
/// both `yield`/`yield*` and `await`, and `start_generator_with_env` routes
/// its `.next()`/`.return()`/`.throw()` to the async (Promise-returning,
/// request-queued) driver rather than the synchronous one -- see
/// `FunctionCode::instantiate`'s prototype-methods branch for the same
/// is_async/is_generator switch on a VM-compiled async generator function.
#[allow(dead_code)] // Consumed by the runtime generator bridge in interp.rs.
pub(crate) fn compile_async_generator_body(body: &ast::Block, strict: bool) -> Result<FunctionCode, CompileError> {
    let mut compiler = Compiler::new(false);
    compiler.strict = strict;
    compiler.compile_function(&ast::FunctionItem {
        name: None,
        params: vec![],
        body: body.clone(),
        is_async: true,
        is_generator: true,
    }, body.span)
}

struct Compiler {
    program: Program,
    depth: usize,
    is_generator: bool,
    is_async: bool,
    strict: bool,
    /// Enclosing loops/switches/labeled statements, innermost last (JSVM-10).
    /// `break`/`continue` search this from the top to resolve their target;
    /// see `find_break_target`/`find_continue_target`.
    loop_stack: Vec<LoopCtx>,
    /// Labels collected from a `Labeled` statement (possibly several:
    /// `a: b: for (...) {}`) not yet attached to the `LoopCtx` they belong
    /// to. Always empty except in the single step between compiling a
    /// `Labeled` statement and the loop/switch/plain-statement arm right
    /// below it that consumes (`std::mem::take`s) it -- see `StmtKind::Labeled`.
    pending_labels: Vec<String>,
    /// The frame's runtime `scopes` depth at the current emission point:
    /// `EnterScope`/`EnterCatch`/`BindFor` push, `LeaveScope` pops, and
    /// emission is statically balanced. A `break`/`continue` target records
    /// it so `ExitTo` can pop the scopes the jump leaves.
    scope_depth: usize,
    /// The innermost disposal scope captures each using declaration.
    resource_stack: Vec<usize>,
}

/// One entry per enclosing loop, `switch`, or plain labeled statement a
/// `break`/`continue` might target. `is_loop` is false for a `switch` or a
/// labeled statement wrapping something other than an iteration statement:
/// both are valid `break` targets but never a `continue` target (10.2.11 /
/// 14.9.1: `continue`'s label must directly label an enclosing
/// IterationStatement). `labels` holds every label attached to this
/// construct (`a: b: for (...) {}` attaches both to the same loop entry).
struct LoopCtx {
    labels: Vec<String>,
    /// `scope_depth` at both the break and the continue target.
    depth: usize,
    is_loop: bool,
    break_patches: Vec<usize>,
    continue_patches: Vec<usize>,
}

impl Compiler {
    fn new(is_function: bool) -> Self {
        Self {
            program: Program {
                code: vec![],
                statement_spans: Vec::new(),
                registers: 1,
                references: 0,
                spread_slots: 0,
                hoisted: vec![],
                functions: vec![],
                is_function,
                async_iterator_slots: 0,
                resource_slots: 0,
                for_in_slots: 0,
            },
            depth: 0,
            is_generator: false,
            is_async: false,
            strict: false,
            loop_stack: Vec::new(),
            pending_labels: Vec::new(),
            scope_depth: 0,
            resource_stack: Vec::new(),
        }
    }

    fn push_loop(&mut self, labels: Vec<String>, is_loop: bool) {
        self.loop_stack.push(LoopCtx { labels, is_loop, depth: self.scope_depth, break_patches: Vec::new(), continue_patches: Vec::new() });
    }

    /// Pop the innermost loop entry, patching its `break`s to `break_target`
    /// and (if it's a real loop) its `continue`s to `continue_target`. A
    /// non-loop entry (`switch`, plain labeled statement) has no
    /// `continue_target` to patch against -- `continue_patches` is always
    /// empty for those, since `find_continue_target` never resolves to one.
    fn pop_loop(&mut self, break_target: usize, continue_target: Option<usize>) {
        let ctx = self.loop_stack.pop().expect("push_loop/pop_loop must be paired");
        for idx in ctx.break_patches {
            self.patch_to(idx, break_target);
        }
        if let Some(continue_target) = continue_target {
            for idx in ctx.continue_patches {
                self.patch_to(idx, continue_target);
            }
        } else {
            debug_assert!(ctx.continue_patches.is_empty(), "non-loop entry collected a continue");
        }
    }

    fn find_break_target(&self, span: Span, label: &Option<ast::Ident>) -> Result<usize, CompileError> {
        match label {
            None => {
                if self.loop_stack.is_empty() {
                    return Err(self.error(span, "break outside loop/switch"));
                }
                Ok(self.loop_stack.len() - 1)
            }
            Some(l) => self.loop_stack.iter().rposition(|c| c.labels.iter().any(|x| x == &l.name))
                .ok_or_else(|| self.error(span, "undefined label")),
        }
    }

    fn find_continue_target(&self, span: Span, label: &Option<ast::Ident>) -> Result<usize, CompileError> {
        match label {
            None => self.loop_stack.iter().rposition(|c| c.is_loop)
                .ok_or_else(|| self.error(span, "continue outside loop")),
            Some(l) => self.loop_stack.iter().rposition(|c| c.is_loop && c.labels.iter().any(|x| x == &l.name))
                .ok_or_else(|| self.error(span, "undefined label")),
        }
    }

    // Parameters execute in the shared evaluator, against the same call-time
    // environment the VM prepares before creating a generator object.
    fn validate_parameters(&self, params: &[ast::Param]) -> Result<(), CompileError> {
        let mut probe = Self::new(true);
        probe.strict = self.strict;
        for param in params {
            probe.validate_binding_expressions(&param.binding)?;
            if let Some(default) = &param.default { probe.expression(default)?; }
        }
        Ok(())
    }

    fn validate_binding_expressions(&mut self, binding: &Binding) -> Result<(), CompileError> {
        match binding {
            Binding::Ident(_) => {}
            Binding::Array(elements) => {
                for element in elements.iter().flatten() { self.validate_binding_expressions(element)?; }
            }
            Binding::Object(properties) => {
                for (key, value) in properties {
                    if let ast::BindingKey::Computed(key) = key { self.expression(key)?; }
                    self.validate_binding_expressions(value)?;
                }
            }
            Binding::Rest(binding) => self.validate_binding_expressions(binding)?,
            Binding::Default(binding, default) => {
                self.validate_binding_expressions(binding)?;
                self.expression(default)?;
            }
        }
        Ok(())
    }

    fn binding_has_parameter_expression(binding: &Binding) -> bool {
        match binding {
            Binding::Ident(_) => false,
            Binding::Default(..) => true,
            Binding::Rest(inner) => Self::binding_has_parameter_expression(inner),
            Binding::Array(elements) => elements.iter().flatten().any(|inner| Self::binding_has_parameter_expression(inner)),
            Binding::Object(properties) => properties.iter().any(|(key, inner)|
                matches!(key, ast::BindingKey::Computed(_)) || Self::binding_has_parameter_expression(inner)),
        }
    }

    fn compile_function(
        &self,
        function: &ast::FunctionItem,
        span: Span,
    ) -> Result<FunctionCode, CompileError> {
        if !function.is_async && !function.is_generator
            && Self::has_scoped_function_declaration(&function.body.body, false)
        {
            return Err(self.error(span, "scoped function declaration"));
        }
        self.validate_parameters(&function.params)?;
        let params = function.params.clone();
        let mut child = Self::new(true);
        child.strict = self.strict || has_use_strict_directive(&function.body.body);
        child.is_generator = function.is_generator;
        child.is_async = function.is_async;
        child.hoist_functions(&function.body.body)?;
        child.statements(&function.body.body, function.body.span)?;
        let body_lexical_names = super::super::block_lexical_names(&function.body.body);
        let skip_implicit_arguments = !params.iter().any(|p| p.default.is_some() || Self::binding_has_parameter_expression(&p.binding))
            && (body_lexical_names.iter().any(|name| name == "arguments")
                || child.program.functions.iter().any(|function| function.name == "arguments"));
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
            body_lexical_names,
            skip_implicit_arguments,
            program: child.program,
            is_arrow: false,
            is_generator: function.is_generator,
            is_async: function.is_async,
            strict: child.strict,
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
        if !arrow.is_async && matches!(&arrow.body, ast::ArrowBody::Block(block)
            if Self::has_scoped_function_declaration(&block.body, false))
        {
            return Err(self.error(span, "scoped function declaration"));
        }
        self.validate_parameters(&arrow.params)?;
        let params = arrow.params.clone();
        let mut child = Self::new(true);
        child.strict = self.strict || matches!(&arrow.body, ast::ArrowBody::Block(block) if has_use_strict_directive(&block.body));
        child.is_async = arrow.is_async;
        match &arrow.body {
            ast::ArrowBody::Expr(expression) => {
                let value = child.expression(expression)?;
                child.emit(Instruction::Return(value));
            }
            ast::ArrowBody::Block(block) => {
                child.hoist_functions(&block.body)?;
                child.statements(&block.body, block.span)?;
            }
        }
        Ok(FunctionCode(Rc::new(CompiledFunction {
            name: String::new(),
            params,
            body_lexical_names: Vec::new(),
            skip_implicit_arguments: false,
            program: child.program,
            is_arrow: true,
            is_generator: false,
            is_async: arrow.is_async,
            strict: child.strict,
            is_method: false,
        })))
    }

    fn compile_method(
        &mut self,
        method: &ast::ClassMethod,
        class_method: bool,
    ) -> Result<FunctionCode, CompileError> {
        let function = ast::FunctionItem {
            name: None,
            params: method.params.clone(),
            body: method.body.clone(),
            is_async: method.is_async,
            is_generator: method.is_generator,
        };
        // A *class* method's body is always strict, per spec, regardless of
        // the enclosing (possibly sloppy) code -- matching the tree
        // walker's `method_value` (`class_method || ...`). This function is
        // also reused for plain object-literal methods (`{ f() {} }`), which
        // carry no such rule and must keep the enclosing scope's strictness.
        let previous_strict = self.strict;
        if class_method { self.strict = true; }
        let result = self.compile_function(&function, method.span);
        self.strict = previous_strict;
        let mut code = result?;
        Rc::get_mut(&mut code.0).expect("new method code").is_method = true;
        Ok(code)
    }

    fn compile_class(&mut self, class: &ast::ClassItem, span: Span) -> Result<ClassCode, CompileError> {
        if class_has_decorators(class) || class.members.iter().any(|member| matches!(
            member,
            ast::ClassMember::Field(_) | ast::ClassMember::StaticBlock(_)
        )) {
            return Err(self.error(span, "class field or static block"));
        }
        // A static method now compiles (installed on the constructor via
        // `is_static` below, not the prototype); a private method still
        // fails through `object_key`'s own `PropKey::Private` rejection per
        // member, which has the same whole-class-falls-back-to-tree-walker
        // effect as an upfront check without a separate one to keep in sync.
        // The super-class expression is compiled in the enclosing scope,
        // before the constructor: base and derived classes otherwise share
        // this compiler, and `super()`/`super.prop` inside the constructor
        // and methods are resolved at execution time from [[HomeObject]]
        // and the constructor's derived-constructor metadata (see MakeClass).
        // The heritage expression and every computed member key are always
        // evaluated in strict mode, per spec, even when the class sits
        // inside sloppy code (matching the tree walker's `eval_class`).
        self.emit(Instruction::EnterStrict);
        let super_class = class.super_class.as_ref().map(|expr| self.expression(expr)).transpose()?;
        self.emit(Instruction::ExitStrict);
        let constructor = class.members.iter().find_map(|member| match member {
            ast::ClassMember::Method(method) if method.kind == ast::MethodKind::Constructor => Some(method),
            _ => None,
        });
        let implicit_derived_ctor = super_class.is_some() && constructor.is_none();
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
        // A class constructor's body is always strict too (it is never
        // reached through `compile_method`, since it is not a `ClassMember`
        // method compiled by the loop below).
        let previous_strict = self.strict;
        self.strict = true;
        let constructor = self.compile_function(&function, span);
        self.strict = previous_strict;
        let constructor = constructor?;
        let mut methods = Vec::new();
        for member in &class.members {
            if let ast::ClassMember::Method(method) = member {
                if method.kind == ast::MethodKind::Constructor { continue; }
                self.emit(Instruction::EnterStrict);
                let key = self.object_key(&method.key, method.key_utf16.as_deref(), method.span)?;
                self.emit(Instruction::ExitStrict);
                let code = self.compile_method(method, true)?;
                methods.push(ClassMethodCode { key, kind: method.kind, code, is_static: method.is_static });
            }
        }
        let super_class = super_class.map(|register| ClassSuper { register, implicit: implicit_derived_ctor });
        Ok(ClassCode { constructor, methods, super_class, inferred_name: None })
    }

    // Nested block declarations need lexical instantiation and conditional Annex B
    // copies. Keep ordinary bodies on the existing interpreter until those are
    // represented in bytecode rather than hoisting them into the function scope.
    fn has_scoped_function_declaration(statements: &[Stmt], scoped: bool) -> bool {
        statements.iter().any(|statement| match &statement.kind {
            StmtKind::Function(_) => scoped,
            StmtKind::Block(block) => Self::has_scoped_function_declaration(&block.body, true),
            StmtKind::If { then, els, .. } =>
                Self::has_scoped_function_declaration(std::slice::from_ref(then.as_ref()), true)
                || els.as_ref().is_some_and(|els| Self::has_scoped_function_declaration(std::slice::from_ref(els.as_ref()), true)),
            StmtKind::While { body, .. } | StmtKind::DoWhile { body, .. }
            | StmtKind::For { body, .. } | StmtKind::ForIn { body, .. }
            | StmtKind::ForOf { body, .. } | StmtKind::ForAwaitOf { body, .. }
            | StmtKind::Labeled { body, .. } =>
                Self::has_scoped_function_declaration(std::slice::from_ref(body.as_ref()), true),
            StmtKind::Switch { cases, .. } => cases.iter().any(|case|
                Self::has_scoped_function_declaration(&case.body, true)),
            StmtKind::Try(statement) =>
                Self::has_scoped_function_declaration(&statement.block.body, true)
                || statement.catch.as_ref().is_some_and(|catch| Self::has_scoped_function_declaration(&catch.body.body, true))
                || statement.finally.as_ref().is_some_and(|block| Self::has_scoped_function_declaration(&block.body, true)),
            _ => false,
        })
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
                | StmtKind::For { body, .. }
                | StmtKind::ForIn { body, .. }
                | StmtKind::ForOf { body, .. }
                | StmtKind::ForAwaitOf { body, .. }
                | StmtKind::Labeled { body, .. } => {
                    self.hoist_functions(std::slice::from_ref(body.as_ref()))?;
                }
                StmtKind::Switch { cases, .. } => {
                    for case in cases {
                        self.hoist_functions(&case.body)?;
                    }
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
            return Err(self.error(span, RESOURCE_LIMIT));
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
        match &op {
            Instruction::EnterScope | Instruction::EnterCatch(_) | Instruction::EnterCatchValue(_) | Instruction::BindFor(..) => self.scope_depth += 1,
            Instruction::LeaveScope => self.scope_depth -= 1,
            _ => {}
        }
        self.program.code.push(op);
        index
    }

    fn patch(&mut self, index: usize) {
        let end = self.program.code.len();
        self.patch_to(index, end);
    }

    /// Like `patch`, but to an explicit, already-known target rather than
    /// the current end of the program (JSVM-10: `break`/`continue` patch to
    /// a loop's break/continue point, not to "here").
    fn patch_to(&mut self, index: usize, target: usize) {
        match &mut self.program.code[index] {
            Instruction::Jump(t) | Instruction::Branch(_, _, t) | Instruction::ExitTo(t, _) => *t = target,
            _ => unreachable!("compiler only patches branches"),
        }
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        self.enter(statement.span)?;
        let start = self.program.code.len();
        self.program.statement_spans.push((start, statement.span));
        let resources = matches!(&statement.kind, StmtKind::For { init: Some(ast::ForInit::Decl(d)), .. } if d.resource_kind.is_some())
            .then(|| self.begin_resources());
        let result = self.statement_inner(statement);
        let result = result.and_then(|()| match resources {
            Some((slot, handler)) => self.end_resources(slot, handler, statement.span),
            None => Ok(()),
        });
        // Give otherwise empty declarations/statements their own position:
        // a branch must not deliver the skipped statement's debugger hook
        // merely because its next live instruction shares the same offset.
        if result.is_ok() && self.program.code.len() == start {
            self.emit(Instruction::NoOp);
        }
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
                    if declaration.kind == VarKind::Var {
                        self.hoist_pattern(&binding.binding);
                        if binding.init.is_none() {
                            continue;
                        }
                    }
                    if declaration.kind == VarKind::Const && binding.init.is_none() {
                        return Err(self.error(binding.span, "const requires an initializer"));
                    }
                    if declaration.kind != VarKind::Var && (declaration.resource_kind.is_some() || !matches!(binding.binding, Binding::Ident(_))) {
                        self.reserve_pattern(&binding.binding, declaration.kind == VarKind::Let);
                    }
                    let value = match &binding.init {
                        Some(expression) => match &binding.binding {
                            Binding::Ident(name) => self.named_class_expression(expression, KeyOperand::Static(super::Interpreter::class_binding_name(&name.name)))?,
                            _ => self.expression(expression)?,
                        },
                        None => 0, // Register zero is immutable undefined.
                    };
                    if let (Binding::Ident(name), Some(init)) = (&binding.binding, &binding.init) {
                        self.emit(Instruction::NamedEvaluation(value, Rc::new(init.clone()), name.name.clone()));
                    }
                    if let Some(resource_kind) = declaration.resource_kind {
                        let slot = *self.resource_stack.last().ok_or_else(|| self.error(binding.span, "unscoped using declaration"))?;
                        self.emit(Instruction::ResourceCapture(slot, value, resource_kind == ast::ResourceKind::AwaitUsing));
                    }
                    self.binding_pattern(&binding.binding, value, declaration.kind, binding.span)?;
                }
            }
            StmtKind::Class(class) => {
                let name = class.name.as_ref().ok_or_else(|| self.error(statement.span, "anonymous class declaration"))?;
                // Class expressions already preserve fields, static blocks
                // and private elements through the shared evaluator whenever
                // the heritage/computed keys cannot suspend this frame. Use
                // that same atomic lowering for a declaration's value.
                let expression = Expr::new(ExprKind::Class(class.clone()), statement.span);
                let dst = self.expression(&expression)?;
                self.emit(Instruction::Declare(name.name.clone(), dst, VarKind::Let));
            }
            StmtKind::Block(block) => {
                self.emit(Instruction::Complete(0));
                self.emit(Instruction::EnterScope);
                self.statements(&block.body, block.span)?;
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
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                let start = self.program.code.len();
                let test = self.expression(test)?;
                let end = self.emit(Instruction::Branch(test, Condition::Falsy, 0));
                self.statement(body)?;
                self.emit(Instruction::Jump(start));
                self.patch(end);
                let break_target = self.program.code.len();
                self.pop_loop(break_target, Some(start));
            }
            StmtKind::DoWhile { body, test } => {
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                let start = self.program.code.len();
                self.statement(body)?;
                let continue_target = self.program.code.len();
                let condition = self.expression(test)?;
                self.emit(Instruction::Branch(condition, Condition::Truthy, start));
                let break_target = self.program.code.len();
                self.pop_loop(break_target, Some(continue_target));
            }
            StmtKind::For {
                init,
                test,
                update,
                body,
            } => {
                // `for (let/const ...;;)`: the head gets its own scope, and
                // each iteration of a `let` head gets a fresh copy of it
                // (CreatePerIterationEnvironment) so closures capture the
                // iteration's value. Break/continue targets sit inside that
                // scope; the break target is the closing `LeaveScope`.
                let lexical = match init {
                    Some(ast::ForInit::Decl(declaration)) if declaration.kind != VarKind::Var => {
                        Some(declaration)
                    }
                    _ => None,
                };
                let mut per_iteration = Vec::new();
                if let Some(declaration) = lexical {
                    self.emit(Instruction::EnterScope);
                    if declaration.kind == VarKind::Let {
                        for binding in &declaration.decls {
                            if let Binding::Ident(name) = &binding.binding {
                                per_iteration.push(name.name.clone());
                            }
                        }
                    }
                }
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                match init {
                    Some(ast::ForInit::Expr(expression)) => {
                        self.expression(expression)?;
                    }
                    Some(ast::ForInit::Decl(declaration)) => {
                        self.statement(&Stmt::new(
                            StmtKind::Var((**declaration).clone()),
                            statement.span,
                        ))?;
                    }
                    None => {}
                }
                if !per_iteration.is_empty() {
                    self.emit(Instruction::CopyScope(per_iteration.clone()));
                }
                let start = self.program.code.len();
                let end = if let Some(test) = test {
                    let value = self.expression(test)?;
                    Some(self.emit(Instruction::Branch(value, Condition::Falsy, 0)))
                } else {
                    None
                };
                self.statement(body)?;
                // `continue` must still run the update expression before
                // re-testing, so its target is here -- right after the body,
                // right before `update` -- not back at `start`.
                let continue_target = self.program.code.len();
                if !per_iteration.is_empty() {
                    self.emit(Instruction::CopyScope(per_iteration));
                }
                if let Some(update) = update {
                    self.expression(update)?;
                }
                self.emit(Instruction::Jump(start));
                if let Some(end) = end {
                    self.patch(end);
                }
                let break_target = self.program.code.len();
                self.pop_loop(break_target, Some(continue_target));
                if lexical.is_some() {
                    self.emit(Instruction::LeaveScope);
                }
            }
            StmtKind::ForOf { left, right, body } => {
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                let iterable = self.for_head_right(left, right)?;
                let slot = self.program.async_iterator_slots;
                self.program.async_iterator_slots += 1;
                self.emit(Instruction::IteratorStart(slot, iterable));
                let handler = self.emit(Instruction::PushTry { catch: None, finally: None, end: 0 });
                let start = self.program.code.len();
                let next = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorNext(next, slot));
                let done = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorDone(done, slot, next));
                let done_branch = self.emit(Instruction::Branch(done, Condition::Truthy, 0));
                let value = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorValue(value, next));
                self.for_iteration(left, value, body, statement.span)?;
                self.emit(Instruction::Jump(start));
                self.patch(done_branch);
                self.emit(Instruction::TryNormal);
                let finally = self.program.code.len();
                let closed = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorClose(closed, slot));
                self.emit(Instruction::AsyncIteratorCloseCheck(closed));
                self.emit(Instruction::EndFinally);
                let end = self.program.code.len();
                self.program.code[handler] = Instruction::PushTry {
                    catch: None, finally: Some(finally), end,
                };
                // `break`/`continue` both leave this try's protected region
                // (`route_abrupt` sees their target is `>= handler.end` --
                // `end` above -- for a break, or `< handler.end` for a
                // continue back to `start`, which is inside it) so
                // `IteratorClose` runs on a break automatically, and never
                // spuriously on a continue.
                self.pop_loop(end, Some(start));
            }
            StmtKind::ForIn { left, right, body } => {
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                let source = self.for_head_right(left, right)?;
                let slot = self.program.for_in_slots;
                self.program.for_in_slots += 1;
                self.emit(Instruction::ForInStart(slot, source));
                let start = self.program.code.len();
                let value = self.register(statement.span)?;
                let done = self.register(statement.span)?;
                self.emit(Instruction::ForInNext(value, done, slot));
                let end = self.emit(Instruction::Branch(done, Condition::Truthy, 0));
                self.for_iteration(left, value, body, statement.span)?;
                self.emit(Instruction::Jump(start));
                self.patch(end);
                let break_target = self.program.code.len();
                self.pop_loop(break_target, Some(start));
            }
            StmtKind::ForAwaitOf { left, right, body } if self.is_async => {
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, true);
                self.emit(Instruction::Complete(0));
                let iterable = self.for_head_right(left, right)?;
                let slot = self.program.async_iterator_slots;
                self.program.async_iterator_slots += 1;
                self.emit(Instruction::AsyncIteratorStart(slot, iterable));
                let handler = self.emit(Instruction::PushTry { catch: None, finally: None, end: 0 });
                let start = self.program.code.len();
                let next = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorNext(next, slot));
                let result = self.register(statement.span)?;
                self.emit(Instruction::Await(result, next));
                let done = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorDone(done, slot, result));
                let done_branch = self.emit(Instruction::Branch(done, Condition::Truthy, 0));
                let value = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorValue(value, result));
                let awaited = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorAwaitValue(awaited, slot, value));
                self.for_iteration(left, awaited, body, statement.span)?;
                self.emit(Instruction::Jump(start));
                self.patch(done_branch);
                self.emit(Instruction::TryNormal);
                let finally = self.program.code.len();
                let closed = self.register(statement.span)?;
                self.emit(Instruction::AsyncIteratorClose(closed, slot));
                let result = self.register(statement.span)?;
                self.emit(Instruction::Await(result, closed));
                self.emit(Instruction::AsyncIteratorCloseCheck(result));
                self.emit(Instruction::EndFinally);
                let end = self.program.code.len();
                self.program.code[handler] = Instruction::PushTry {
                    catch: None, finally: Some(finally), end,
                };
                self.pop_loop(end, Some(start));
            }
            StmtKind::Switch { disc, cases } => {
                self.emit(Instruction::Complete(0));
                let disc = self.expression(disc)?;
                let labels = std::mem::take(&mut self.pending_labels);
                self.push_loop(labels, false);
                self.emit(Instruction::EnterScope);
                let resources = cases.iter().flat_map(|case| &case.body)
                    .any(|s| matches!(&s.kind, StmtKind::Var(d) if d.resource_kind.is_some()))
                    .then(|| self.begin_resources());
                // Test each `case` in source order (Falsy => fall through to
                // the next test), first match wins; `default` (if any) has no
                // test of its own and is handled as a final unconditional
                // jump after every real case has been tried, matching spec
                // CaseBlockEvaluation's "try every case, then default" order
                // regardless of `default`'s syntactic position.
                let mut case_branches = Vec::new();
                let mut default_index = None;
                for (index, case) in cases.iter().enumerate() {
                    match &case.test {
                        Some(test) => {
                            let value = self.expression(test)?;
                            let matched = self.register(case.span)?;
                            self.emit(Instruction::Binary(matched, BinOp::StrictEq, disc, value));
                            case_branches.push((index, self.emit(Instruction::Branch(matched, Condition::Truthy, 0))));
                        }
                        None => default_index = Some(index),
                    }
                }
                let bodies_start = self.program.code.len();
                let default_jump = if default_index.is_some() {
                    Some(self.emit(Instruction::Jump(0)))
                } else {
                    None
                };
                let no_match_exit = if default_index.is_none() {
                    Some(self.emit(Instruction::Jump(0)))
                } else {
                    None
                };
                // Patch each `case` test's branch and (if present) the
                // fallback `Jump` to `default` now that every body's start
                // offset can be measured as it's compiled.
                let mut body_starts = Vec::with_capacity(cases.len());
                for (case_index, case) in cases.iter().enumerate() {
                    let body_start = self.program.code.len();
                    body_starts.push(body_start);
                    for statement in &case.body {
                        self.statement(statement)?;
                    }
                    let _ = case_index;
                }
                for (index, branch) in case_branches {
                    self.patch_to(branch, body_starts[index]);
                }
                if let (Some(default_jump), Some(default_index)) = (default_jump, default_index) {
                    self.patch_to(default_jump, body_starts[default_index]);
                }
                let _ = bodies_start;
                let normal_exit = self.program.code.len();
                if let Some((slot, handler)) = resources { self.end_resources(slot, handler, statement.span)?; }
                self.emit(Instruction::LeaveScope);
                let end = self.program.code.len();
                if let Some(no_match_exit) = no_match_exit {
                    self.patch_to(no_match_exit, normal_exit);
                }
                self.pop_loop(end, None);
            }
            StmtKind::Break { label } => {
                let target = self.find_break_target(statement.span, label)?;
                let jump = self.emit(Instruction::ExitTo(0, self.loop_stack[target].depth));
                self.loop_stack[target].break_patches.push(jump);
            }
            StmtKind::Continue { label } => {
                let target = self.find_continue_target(statement.span, label)?;
                let jump = self.emit(Instruction::ExitTo(0, self.loop_stack[target].depth));
                self.loop_stack[target].continue_patches.push(jump);
            }
            StmtKind::Labeled { label, body } => {
                self.pending_labels.push(label.name.clone());
                if is_loop_statement(&body.kind) {
                    // The loop arm above `take`s (and clears) `pending_labels`
                    // itself, attaching every accumulated label (there may be
                    // several: `a: b: for (...) {}`) straight to its own
                    // `LoopCtx` so `continue`/`break` with any of them resolve
                    // to it directly.
                    self.statement(body)?;
                } else {
                    let labels = std::mem::take(&mut self.pending_labels);
                    self.push_loop(labels, false);
                    self.statement(body)?;
                    let end = self.program.code.len();
                    self.pop_loop(end, None);
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
                self.statements(&statement.block.body, statement.block.span)?;
                self.emit(Instruction::LeaveScope);
                self.emit(Instruction::TryNormal);

                let catch = if let Some(catch) = &statement.catch {
                    let start = self.program.code.len();
                    if let Some(binding) = &catch.binding {
                        let value = self.register(catch.body.span)?;
                        self.emit(Instruction::EnterCatchValue(value));
                        self.reserve_pattern(binding, true);
                        self.binding_pattern(binding, value, VarKind::Let, catch.body.span)?;
                    } else { self.emit(Instruction::EnterCatch(None)); }
                    self.statements(&catch.body.body, catch.body.span)?;
                    self.emit(Instruction::LeaveScope);
                    self.emit(Instruction::TryNormal);
                    Some(start)
                } else {
                    None
                };

                let finally = if let Some(finally) = &statement.finally {
                    let start = self.program.code.len();
                    self.emit(Instruction::EnterScope);
                    self.statements(&finally.body, finally.span)?;
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
                    let value = self.expression(expression)?;
                    if self.is_async && self.is_generator {
                        // An async generator's `return expr` awaits the value.
                        let awaited = self.register(statement.span)?;
                        self.emit(Instruction::Await(awaited, value));
                        awaited
                    } else {
                        value
                    }
                } else {
                    0
                };
                self.emit(Instruction::Return(value));
            }
            _ => return Err(self.error(statement.span, unsupported_statement(&statement.kind))),
        }
        Ok(())
    }

    fn named_class_expression(&mut self, expression: &Expr, name: KeyOperand) -> Result<Register, CompileError> {
        self.enter(expression.span)?;
        let result = self.named_class_expression_inner(expression, name);
        self.depth -= 1;
        result
    }

    fn named_class_expression_inner(&mut self, expression: &Expr, name: KeyOperand) -> Result<Register, CompileError> {
        match &expression.kind {
            ExprKind::Paren(inner) => self.named_class_expression(inner, name),
            ExprKind::Class(class) if class.name.is_none() => {
                if !may_suspend(expression) {
                    let dst = self.register(expression.span)?;
                    self.emit(Instruction::TreeEvalNamedClass(dst, Rc::new(expression.clone()), name));
                    return Ok(dst);
                }
                let mut code = self.compile_class(class, expression.span)?;
                code.inferred_name = Some(name);
                let dst = self.register(expression.span)?;
                self.emit(Instruction::MakeClass(dst, code));
                Ok(dst)
            }
            _ => self.expression(expression),
        }
    }

    fn expression(&mut self, expression: &Expr) -> Result<Register, CompileError> {
        self.enter(expression.span)?;
        // The VM has no optional chains: hand the whole chain, from its
        // outermost link, to the tree walker so short-circuiting covers
        // every link (`o?.b[f()]` must not call `f`). Super reference operations
        // also need the home object's prototype and the actual this receiver.
        // The class adapter owns immutable inner names, descriptor validation,
        // fields/private elements and method metadata. Delegate the complete
        // class when its evaluation cannot suspend this bytecode frame.
        if (in_optional_chain(expression) || super_reference_operation(expression)
            || matches!(expression.kind, ExprKind::Class(_))) && !may_suspend(expression) {
            let result = self.register(expression.span).map(|dst| {
                self.emit(Instruction::TreeEval(dst, Rc::new(expression.clone())));
                dst
            });
            self.depth -= 1;
            return result;
        }
        let mark = (self.program.code.len(), self.scope_depth);
        let result = match self.expression_inner(expression) {
            // Direct eval runs against the evaluator's current environment,
            // including the shared parameter-binding environment.
            Err(error)
                if !matches!(error.message, RESOURCE_LIMIT | "register limit")
                    && !context_dependent(expression)
                    && !may_suspend(expression) =>
            {
                self.program.code.truncate(mark.0);
                self.scope_depth = mark.1;
                self.register(expression.span).map(|dst| {
                    self.emit(Instruction::TreeEval(dst, Rc::new(expression.clone())));
                    dst
                })
            }
            other => other,
        };
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
                let mut value = match argument {
                    Some(argument) => self.expression(argument)?,
                    None => 0,
                };
                if self.is_async {
                    // AsyncGeneratorYield(? Await(value)).
                    let awaited = self.register(expression.span)?;
                    self.emit(Instruction::Await(awaited, value));
                    value = awaited;
                }
                self.emit(Instruction::Yield(dst, value));
            }
            ExprKind::Yield { argument: Some(argument), delegate: true } if self.is_generator => {
                let value = self.expression(argument)?;
                self.emit(if self.is_async {
                    Instruction::YieldDelegateAsync(dst, value)
                } else {
                    Instruction::YieldDelegate(dst, value)
                });
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
                        // Object elements retain the parser's generic spread
                        // expression wrapper. It has no standalone value;
                        // evaluate its operand before copying properties.
                        let source = match &source.kind {
                            ExprKind::Spread(inner) => inner.as_ref(),
                            _ => source.as_ref(),
                        };
                        let source = self.expression(source)?;
                        self.emit(Instruction::SpreadObject(dst, source));
                        continue;
                    };
                    // A non-computed literal `__proto__: value` sets
                    // [[Prototype]] instead of creating an own property,
                    // matching the tree walker's `ExprKind::Object` evaluation.
                    let key_is_proto = matches!(&property.key, ast::PropKey::Ident(name) if name.name == "__proto__")
                        || matches!(&property.key, ast::PropKey::Str(s) if s == "__proto__");
                    let is_proto_literal = key_is_proto && matches!(&property.value, ast::PropValue::Init(_));
                    if is_proto_literal {
                        let ast::PropValue::Init(value) = &property.value else { unreachable!() };
                        let value = self.expression(value)?;
                        self.emit(Instruction::SetPrototype(dst, value));
                        continue;
                    }
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
                        ast::PropValue::Init(value) => {
                            if !matches!(&property.key, ast::PropKey::Computed(_))
                                && matches!(&key, KeyOperand::Static(name) if *name == "__proto__") {
                                self.expression(value)?
                            } else { self.named_class_expression(value, key.clone())? }
                        },
                        ast::PropValue::CoverInitializedName { .. } => {
                            return Err(self.error(property.span, "cover initialized name"));
                        }
                        ast::PropValue::Method(method) => {
                            let code = self.compile_method(method, false)?;
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
            ExprKind::NewTarget => {
                self.emit(Instruction::LoadNewTarget(dst));
            }
            ExprKind::Super => {
                // Bare `super` only appears as the object of a member
                // expression (`super.x`, `super[x]`); `super(...)` calls are
                // handled directly in the `Call` arm below. Resolve to
                // [[HomeObject]].[[GetPrototypeOf]](), matching the tree
                // walker's `ExprKind::Super` evaluation.
                self.emit(Instruction::LoadSuperHome(dst));
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
                let code = self.compile_function(function, expression.span)?;
                if let Some(name) = &function.name {
                    self.emit(Instruction::MakeNamedFunction(dst, code, name.name.clone()));
                } else {
                    self.emit(Instruction::MakeFunction(dst, code));
                }
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
                prop: ast::MemberProp::Private(name),
                optional: false,
            } => {
                let object = self.expression(obj)?;
                self.emit(Instruction::GetPrivateField(dst, object, name.name.clone()));
            }
            ExprKind::Member {
                obj,
                prop,
                optional: false,
            } => {
                let object = self.expression(obj)?;
                let key = self.member_key(object, prop, false)?;
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
            ExprKind::Binary { op: ast::BinOp::In, left, right }
                if matches!(&left.kind, ExprKind::PrivateName(_)) =>
            {
                let ExprKind::PrivateName(name) = &left.kind else { unreachable!() };
                let object = self.expression(right)?;
                self.emit(Instruction::HasPrivateField(dst, object, name.name.clone()));
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
            } if matches!(&callee.kind, ExprKind::Super) => {
                // `super(...)` in a derived constructor: construct the
                // super-constructor recorded on the active frame and use its
                // result to initialize `this`.
                let arguments = self.call_arguments(args)?;
                self.emit(Instruction::SuperCall(dst, arguments));
            }
            ExprKind::Call {
                callee,
                args,
                optional: false,
            } => {
                if matches!(&callee.kind,
                    ExprKind::Ident(name) if name.name == "eval") {
                    return Err(self.error(expression.span, "direct eval"));
                }
                let (callee, this) = match &callee.kind {
                    ExprKind::Member {
                        obj,
                        prop: ast::MemberProp::Private(name),
                        optional: false,
                    } => {
                        let object = self.expression(obj)?;
                        let function = self.register(callee.span)?;
                        self.emit(Instruction::GetPrivateField(function, object, name.name.clone()));
                        (function, object)
                    }
                    ExprKind::Member {
                        obj,
                        prop,
                        optional: false,
                    } => {
                        let object = self.expression(obj)?;
                        let key = self.member_key(object, prop, false)?;
                        let function = self.register(callee.span)?;
                        self.emit(Instruction::GetMember(function, object, key));
                        let this = if matches!(&obj.kind, ExprKind::Super) {
                            // `super.method(...)`: the property lookup starts
                            // at [[HomeObject]].[[GetPrototypeOf]]() (already
                            // in `object`), but the call receiver is the
                            // actual `this`, matching the tree walker.
                            let this = self.register(callee.span)?;
                            self.emit(Instruction::LoadThis(this));
                            this
                        } else {
                            object
                        };
                        (function, this)
                    }
                    _ if super::super::event_handler::identifier_name(callee).is_some() => {
                        let name = super::super::event_handler::identifier_name(callee).unwrap();
                        let function = self.register(callee.span)?;
                        let receiver = self.register(callee.span)?;
                        self.emit(Instruction::LoadCallName(function, receiver, name.to_owned()));
                        (function, receiver)
                    }
                    _ => (self.expression(callee)?, 0),
                };
                let arguments = self.call_arguments(args)?;
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
                if pattern_target(target) {
                    let value = self.expression(value)?;
                    self.assignment_pattern(target, value)?;
                    return Ok(value);
                }
                let slot = if let Some(name) = super::super::event_handler::identifier_name(target) {
                    let slot = self.program.references;
                    self.program.references += 1;
                    self.emit(Instruction::ResolveWriteName(slot, name.to_owned()));
                    slot
                } else if let Some((obj, name)) = private_target(target) {
                    // Plain `=` must not read: a setter-only private accessor
                    // has no getter, and reading it throws.
                    let object = self.expression(obj)?;
                    let slot = self.program.references;
                    self.program.references += 1;
                    self.emit(Instruction::ResolvePrivateWrite(slot, object, name.name.clone()));
                    slot
                } else if let Some((obj, prop)) = public_member_target(target) {
                    let object = self.expression(obj)?;
                    let key = self.member_key(object, prop, true)?;
                    let slot = self.program.references;
                    self.program.references += 1;
                    self.emit(Instruction::ResolveMemberWrite(slot, object, key));
                    slot
                } else { self.reference(target)?.0 };
                let value = if let Some(name) = super::super::event_handler::identifier_name(target) {
                    self.named_class_expression(value, KeyOperand::Static(name.into()))?
                } else { self.expression(value)? };
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
                    let right = if let Some(name) = super::super::event_handler::identifier_name(target) {
                        self.named_class_expression(value, KeyOperand::Static(name.into()))?
                    } else { self.expression(value)? };
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

    fn call_arguments(&mut self, args: &[Expr]) -> Result<Vec<CallArgument>, CompileError> {
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
        Ok(arguments)
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
                prop: ast::MemberProp::Private(name),
                optional: false,
            } => {
                let object = self.expression(obj)?;
                self.emit(Instruction::ResolvePrivate(slot, old, object, name.name.clone()));
            }
            ExprKind::Member {
                obj,
                prop,
                optional: false,
            } => {
                let object = self.expression(obj)?;
                let key = self.member_key(object, prop, true)?;
                self.emit(Instruction::ResolveMember(slot, old, object, key));
            }
            _ => return Err(self.error(target.span, "assignment target")),
        }
        Ok((slot, old))
    }

    /// `object` is the already-evaluated base of the member access.
    /// `require_coercible` matches an asymmetry that already exists in the
    /// tree walker: a plain read or a call target's property lookup
    /// (`eval_expr`'s `ExprKind::Member`, and the call-target arm of
    /// `call_expr`) calls `get_prop` directly, which is lenient -- a
    /// null/undefined base yields `undefined` rather than throwing -- while
    /// an assignment target (`resolve_reference`) throws TypeError instead.
    /// When strict, the check runs _after_ a computed key expression is
    /// evaluated for its own side effects (e.g. a call) but _before_ the raw
    /// value is converted with `ToPropertyKey` (which can itself run user
    /// code via `toString`/`valueOf`) -- the part the VM previously got
    /// backwards for compound assignment.
    fn member_key(
        &mut self,
        object: Register,
        prop: &ast::MemberProp,
        require_coercible: bool,
    ) -> Result<KeyOperand, CompileError> {
        match prop {
            ast::MemberProp::Ident(name) => {
                if require_coercible {
                    self.emit(Instruction::RequireObjectCoercible(object));
                }
                Ok(KeyOperand::Static(name.name.as_str().into()))
            }
            ast::MemberProp::Computed(expression) => {
                let register = self.expression(expression)?;
                if require_coercible {
                    self.emit(Instruction::RequireObjectCoercible(object));
                }
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

/// True for an iteration statement (JSVM-10): `StmtKind::Labeled` checks this
/// to decide whether to attach its label(s) directly to the loop's own
/// `LoopCtx` (enabling `continue label`) rather than pushing a separate,
/// `continue`-blind break-only wrapper.
fn is_loop_statement(kind: &StmtKind) -> bool {
    matches!(
        kind,
        StmtKind::While { .. }
            | StmtKind::DoWhile { .. }
            | StmtKind::For { .. }
            | StmtKind::ForIn { .. }
            | StmtKind::ForOf { .. }
            | StmtKind::ForAwaitOf { .. }
    )
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

fn public_member_target(mut target: &Expr) -> Option<(&Expr, &ast::MemberProp)> {
    while let ExprKind::Paren(inner) = &target.kind { target = inner; }
    match &target.kind {
        ExprKind::Member { obj, prop, optional: false } if !matches!(prop, ast::MemberProp::Private(_)) => Some((obj, prop)),
        _ => None,
    }
}

fn private_target(mut target: &Expr) -> Option<(&Expr, &ast::Ident)> {
    while let ExprKind::Paren(inner) = &target.kind {
        target = inner;
    }
    match &target.kind {
        ExprKind::Member { obj, prop: ast::MemberProp::Private(name), optional: false } => Some((obj, name)),
        _ => None,
    }
}

fn class_has_decorators(class: &ast::ClassItem) -> bool {
    !class.decorators.is_empty() || class.members.iter().any(|member| match member {
        ast::ClassMember::Method(method) => !method.decorators.is_empty(),
        ast::ClassMember::Field(field) => !field.decorators.is_empty(),
        ast::ClassMember::StaticBlock(_) => false,
    })
}

fn key_may_suspend(key: &ast::PropKey) -> bool {
    matches!(key, ast::PropKey::Computed(key) if may_suspend(key))
}

fn super_reference_operation(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => super_reference_operation(inner),
        ExprKind::Member { obj, .. } => matches!(obj.kind, ExprKind::Super),
        ExprKind::Call { callee, .. } => super_reference_operation(callee),
        ExprKind::Assign { target, .. } | ExprKind::Update { target, .. } => super_reference_operation(target),
        _ => false,
    }
}

fn in_optional_chain(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => in_optional_chain(inner),
        ExprKind::Member { obj, optional, .. } => *optional || in_optional_chain(obj),
        ExprKind::Call { callee, optional, .. } => *optional || in_optional_chain(callee),
        _ => false,
    }
}

/// Nodes whose meaning depends on their parent (a spread element, a bare
/// `super` or `#x`): delegating them alone would lose that context, so the
/// failure propagates and the parent is delegated instead.
fn context_dependent(expression: &Expr) -> bool {
    matches!(expression.kind, ExprKind::Spread(_) | ExprKind::Super | ExprKind::PrivateName(_))
}

const RESOURCE_LIMIT: &str = "compiler resource limit";

/// Whether evaluating `expression` could reach a `yield` or `await` of the
/// body being compiled. Nested function bodies are their own frames and do
/// not count; anything not understood here is assumed to suspend.
fn may_suspend(expression: &Expr) -> bool {
    let all = |items: &[Expr]| items.iter().any(may_suspend);
    match &expression.kind {
        ExprKind::Yield { .. } | ExprKind::Unary { op: UnOp::Await, .. } => true,
        ExprKind::Object(elements) => elements.iter().any(|element| match element {
            ast::ObjElem::Spread(inner) => may_suspend(inner),
            ast::ObjElem::Prop(property) => key_may_suspend(&property.key) || match &property.value {
                ast::PropValue::Shorthand(_) | ast::PropValue::Method(_) => false,
                ast::PropValue::CoverInitializedName { default, .. } => may_suspend(default),
                ast::PropValue::Init(value) => may_suspend(value),
            },
        }),
        // Field initializers, static blocks and method bodies are their own
        // scopes; only the heritage and computed keys run in this body.
        ExprKind::Class(class) => {
            all(&class.decorators) || class.super_class.as_ref().is_some_and(may_suspend)
                || class.members.iter().any(|member| match member {
                    ast::ClassMember::Method(method) => all(&method.decorators) || key_may_suspend(&method.key),
                    ast::ClassMember::Field(field) => all(&field.decorators) || key_may_suspend(&field.key),
                    ast::ClassMember::StaticBlock(_) => false,
                })
        }
        ExprKind::PrivateName(_) | ExprKind::RegExp(_) | ExprKind::Num(_) | ExprKind::BigInt(_)
        | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Null | ExprKind::Ident(_)
        | ExprKind::This | ExprKind::NewTarget | ExprKind::ImportMeta | ExprKind::Super
        | ExprKind::Function(_) | ExprKind::Arrow(_) => false,
        ExprKind::DynamicImport(args) | ExprKind::DynamicImportPhase { args, .. } => all(args),
        ExprKind::Template(template) => all(&template.exprs),
        ExprKind::TaggedTemplate { tag, template } => may_suspend(tag) || all(&template.exprs),
        ExprKind::Array(items) => items.iter().flatten().any(may_suspend),
        ExprKind::Spread(inner) | ExprKind::Paren(inner) => may_suspend(inner),
        ExprKind::Unary { operand, .. } => may_suspend(operand),
        ExprKind::Update { target, .. } => may_suspend(target),
        ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. } => {
            may_suspend(left) || may_suspend(right)
        }
        ExprKind::Assign { target, value, .. } => may_suspend(target) || may_suspend(value),
        ExprKind::Conditional { test, consequent, alternate } => {
            may_suspend(test) || may_suspend(consequent) || may_suspend(alternate)
        }
        ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
            may_suspend(callee) || all(args)
        }
        ExprKind::Member { obj, prop, .. } => {
            may_suspend(obj) || matches!(prop, ast::MemberProp::Computed(key) if may_suspend(key))
        }
        ExprKind::Sequence(items) => all(items),
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
