//! Post-parse validation of cover initialized names. The object grammar must
//! accept `{x = value}` while deciding whether its parent is an assignment
//! pattern, but that spelling is invalid as an ordinary object literal.
use super::*;

fn invalid(span: Span) -> SyntaxError {
    SyntaxError::new(
        "cover initialized name outside assignment pattern",
        span.start.line,
        span.start.column,
    )
}

pub(super) fn validate_program(body: &[Stmt]) -> Result<(), SyntaxError> {
    for stmt in body {
        statement(stmt)?;
    }
    Ok(())
}

fn statement(stmt: &Stmt) -> Result<(), SyntaxError> {
    match &stmt.kind {
        StmtKind::Var(decl) => {
            for item in &decl.decls {
                binding(&item.binding)?;
                if let Some(init) = &item.init {
                    expression(init, false)?;
                }
            }
        }
        StmtKind::Function(function) => function_item(function)?,
        StmtKind::Class(class) => class_item(class)?,
        StmtKind::Block(block) => validate_program(&block.body)?,
        StmtKind::Expression(expr) | StmtKind::Throw(expr) => expression(expr, false)?,
        StmtKind::If { cond, then, els } => {
            expression(cond, false)?;
            statement(then)?;
            if let Some(els) = els {
                statement(els)?;
            }
        }
        StmtKind::DoWhile { body, test } | StmtKind::While { test, body } => {
            statement(body)?;
            expression(test, false)?;
        }
        StmtKind::With { object, body } => {
            expression(object, false)?;
            statement(body)?;
        }
        StmtKind::For {
            init,
            test,
            update,
            body,
        } => {
            if let Some(init) = init {
                match init {
                    ForInit::Expr(expr) => expression(expr, false)?,
                    ForInit::Decl(decl) => {
                        for item in &decl.decls {
                            binding(&item.binding)?;
                            if let Some(init) = &item.init {
                                expression(init, false)?;
                            }
                        }
                    }
                }
            }
            if let Some(test) = test {
                expression(test, false)?;
            }
            if let Some(update) = update {
                expression(update, false)?;
            }
            statement(body)?;
        }
        StmtKind::ForIn { left, right, body }
        | StmtKind::ForOf { left, right, body }
        | StmtKind::ForAwaitOf { left, right, body } => {
            match left {
                ForLeft::Target(expr) => expression(expr, true)?,
                ForLeft::Decl(decl) => {
                    for item in &decl.decls {
                        binding(&item.binding)?;
                    }
                }
            }
            expression(right, false)?;
            statement(body)?;
        }
        StmtKind::Switch { disc, cases } => {
            expression(disc, false)?;
            for case in cases {
                if let Some(test) = &case.test {
                    expression(test, false)?;
                }
                validate_program(&case.body)?;
            }
        }
        StmtKind::Return(expr) => {
            if let Some(expr) = expr {
                expression(expr, false)?;
            }
        }
        StmtKind::Labeled { body, .. } => statement(body)?,
        StmtKind::Try(try_stmt) => {
            validate_program(&try_stmt.block.body)?;
            if let Some(catch) = &try_stmt.catch {
                validate_program(&catch.body.body)?;
            }
            if let Some(finally) = &try_stmt.finally {
                validate_program(&finally.body)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn binding(target: &Binding) -> Result<(), SyntaxError> {
    match target {
        Binding::Array(items) => {
            for item in items.iter().flatten() {
                binding(item)?;
            }
        }
        Binding::Object(items) => {
            for (key, value) in items {
                if let BindingKey::Computed(expr) = key {
                    expression(expr, false)?;
                }
                binding(value)?;
            }
        }
        Binding::Rest(inner) => binding(inner)?,
        Binding::Default(inner, expr) => {
            binding(inner)?;
            expression(expr, false)?;
        }
        Binding::Ident(_) => {}
    }
    Ok(())
}

fn function_item(function: &FunctionItem) -> Result<(), SyntaxError> {
    for param in &function.params {
        binding(&param.binding)?;
        if let Some(default) = &param.default {
            expression(default, false)?;
        }
    }
    validate_program(&function.body.body)
}

fn class_item(class: &ClassItem) -> Result<(), SyntaxError> {
    if let Some(super_class) = &class.super_class {
        expression(super_class, false)?;
    }
    for member in &class.members {
        match member {
            ClassMember::Method(method) => {
                if let PropKey::Computed(expr) = &method.key {
                    expression(expr, false)?;
                }
                for param in &method.params {
                    binding(&param.binding)?;
                    if let Some(default) = &param.default {
                        expression(default, false)?;
                    }
                }
                validate_program(&method.body.body)?;
            }
            ClassMember::Field(field) => {
                if let PropKey::Computed(expr) = &field.key {
                    expression(expr, false)?;
                }
                if let Some(init) = &field.init {
                    expression(init, false)?;
                }
            }
            ClassMember::StaticBlock(block) => validate_program(&block.body)?,
        }
    }
    Ok(())
}

fn expression(expr: &Expr, pattern: bool) -> Result<(), SyntaxError> {
    match &expr.kind {
        ExprKind::Object(items) => {
            for item in items {
                match item {
                    ObjElem::Spread(inner) => expression(inner, pattern)?,
                    ObjElem::Prop(property) => {
                        if let PropKey::Computed(key) = &property.key {
                            expression(key, false)?;
                        }
                        match &property.value {
                            PropValue::CoverInitializedName { default, .. } => {
                                if !pattern {
                                    return Err(invalid(property.span));
                                }
                                expression(default, false)?;
                            }
                            PropValue::Init(value) => expression(value, pattern)?,
                            PropValue::Method(method) => validate_program(&method.body.body)?,
                            PropValue::Shorthand(_) => {}
                        }
                    }
                }
            }
        }
        ExprKind::Array(items) => {
            for item in items.iter().flatten() {
                expression(item, pattern)?;
            }
        }
        ExprKind::Paren(inner) | ExprKind::Spread(inner) => expression(inner, pattern)?,
        ExprKind::Assign { target, value, .. } => {
            expression(target, true)?;
            expression(value, false)?;
        }
        ExprKind::Unary { operand, .. } => expression(operand, false)?,
        ExprKind::Update { target, .. } => expression(target, false)?,
        ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. } => {
            expression(left, false)?;
            expression(right, false)?;
        }
        ExprKind::Conditional {
            test,
            consequent,
            alternate,
        } => {
            expression(test, false)?;
            expression(consequent, false)?;
            expression(alternate, false)?;
        }
        ExprKind::Call { callee, args, .. } | ExprKind::New { callee, args } => {
            expression(callee, false)?;
            for arg in args {
                expression(arg, false)?;
            }
        }
        ExprKind::DynamicImport(args) => {
            for arg in args {
                expression(arg, false)?;
            }
        }
        ExprKind::DynamicImportPhase { args, .. } => {
            for arg in args {
                expression(arg, false)?;
            }
        }
        ExprKind::Member { obj, prop, .. } => {
            expression(obj, false)?;
            if let MemberProp::Computed(key) = prop {
                expression(key, false)?;
            }
        }
        ExprKind::Sequence(items) => {
            for item in items {
                expression(item, false)?;
            }
        }
        ExprKind::Template(template) => {
            for expr in &template.exprs {
                expression(expr, false)?;
            }
        }
        ExprKind::TaggedTemplate { tag, template } => {
            expression(tag, false)?;
            for expr in &template.exprs {
                expression(expr, false)?;
            }
        }
        ExprKind::Yield { argument, .. } => {
            if let Some(argument) = argument {
                expression(argument, false)?;
            }
        }
        ExprKind::Function(function) => function_item(function)?,
        ExprKind::Arrow(arrow) => {
            for param in &arrow.params {
                binding(&param.binding)?;
                if let Some(default) = &param.default {
                    expression(default, false)?;
                }
            }
            match &arrow.body {
                ArrowBody::Expr(expr) => expression(expr, false)?,
                ArrowBody::Block(block) => validate_program(&block.body)?,
            }
        }
        ExprKind::Class(class) => class_item(class)?,
        _ => {}
    }
    Ok(())
}
