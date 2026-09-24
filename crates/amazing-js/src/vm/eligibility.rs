//! Constructs whose tree-walker semantics are not yet represented in bytecode.
use crate::ast::{self, ArrowBody, Binding, ClassMember, ExprKind, ForInit, ForLeft, MemberProp, ObjElem, PropKey, PropValue, StmtKind};

pub(super) fn eligible(program: &ast::Program) -> bool {
    !program.body.iter().any(statement_needs_fallback)
}

fn block_needs_fallback(block: &ast::Block) -> bool {
    block.body.iter().any(statement_needs_fallback)
}

fn binding_needs_fallback(binding: &Binding) -> bool {
    match binding {
        Binding::Ident(_) => false,
        Binding::Array(items) => items.iter().flatten().any(|item| binding_needs_fallback(item)),
        Binding::Object(items) => items.iter().any(|(key, value)|
            matches!(key, ast::BindingKey::Computed(expr) if expression_needs_fallback(expr))
                || binding_needs_fallback(value)),
        Binding::Rest(inner) => binding_needs_fallback(inner),
        Binding::Default(inner, expr) => binding_needs_fallback(inner) || expression_needs_fallback(expr),
    }
}

fn declaration_needs_fallback(decl: &ast::VarDecl) -> bool {
    decl.decls.iter().any(|item| binding_needs_fallback(&item.binding)
        || item.init.as_ref().is_some_and(expression_needs_fallback))
}

fn for_left_needs_fallback(left: &ForLeft) -> bool {
    match left {
        ForLeft::Target(expr) => expression_needs_fallback(expr),
        ForLeft::Decl(decl) => declaration_needs_fallback(decl),
    }
}

fn function_needs_fallback(function: &ast::FunctionItem) -> bool {
    function.params.iter().any(|param| binding_needs_fallback(&param.binding)
        || param.default.as_ref().is_some_and(expression_needs_fallback))
        || block_needs_fallback(&function.body)
}

fn method_needs_fallback(method: &ast::ClassMethod) -> bool {
    key_needs_fallback(&method.key)
        || method.params.iter().any(|param| binding_needs_fallback(&param.binding)
            || param.default.as_ref().is_some_and(expression_needs_fallback))
        || block_needs_fallback(&method.body)
}

fn class_needs_fallback(class: &ast::ClassItem) -> bool {
    class.super_class.as_ref().is_some_and(expression_needs_fallback)
        || class.members.iter().any(|member| match member {
            ClassMember::Method(method) => method_needs_fallback(method),
            ClassMember::StaticBlock(block) => block_needs_fallback(block),
            ClassMember::Field(field) => key_needs_fallback(&field.key)
                || field.init.as_ref().is_some_and(expression_needs_fallback),
        })
}

fn key_needs_fallback(key: &PropKey) -> bool {
    matches!(key, PropKey::Computed(expr) if expression_needs_fallback(expr))
}

fn statement_needs_fallback(statement: &ast::Stmt) -> bool {
    match &statement.kind {
        StmtKind::Var(decl) => declaration_needs_fallback(decl),
        StmtKind::Function(function) => function_needs_fallback(function),
        StmtKind::Class(class) => class_needs_fallback(class),
        StmtKind::Block(block) => block_needs_fallback(block),
        StmtKind::Expression(expr) | StmtKind::Throw(expr) => expression_needs_fallback(expr),
        StmtKind::If { cond, then, els } => expression_needs_fallback(cond)
            || statement_needs_fallback(then)
            || els.as_ref().is_some_and(|stmt| statement_needs_fallback(stmt)),
        StmtKind::DoWhile { body, test } | StmtKind::While { body, test } =>
            statement_needs_fallback(body) || expression_needs_fallback(test),
        StmtKind::With { object, body } => expression_needs_fallback(object) || statement_needs_fallback(body),
        StmtKind::For { init, test, update, body } =>
            init.as_ref().is_some_and(|init| match init {
                ForInit::Decl(decl) => declaration_needs_fallback(decl),
                ForInit::Expr(expr) => expression_needs_fallback(expr),
            }) || test.as_ref().is_some_and(expression_needs_fallback)
                || update.as_ref().is_some_and(expression_needs_fallback)
                || statement_needs_fallback(body),
        StmtKind::ForIn { left, right, body }
        | StmtKind::ForOf { left, right, body }
        | StmtKind::ForAwaitOf { left, right, body } =>
            for_left_needs_fallback(left) || expression_needs_fallback(right)
                || statement_needs_fallback(body),
        StmtKind::Switch { disc, cases } => expression_needs_fallback(disc)
            || cases.iter().any(|case| case.test.as_ref().is_some_and(expression_needs_fallback)
                || case.body.iter().any(statement_needs_fallback)),
        StmtKind::Return(expr) => expr.as_ref().is_some_and(expression_needs_fallback),
        StmtKind::Labeled { body, .. } => statement_needs_fallback(body),
        StmtKind::Try(try_stmt) => block_needs_fallback(&try_stmt.block)
            || try_stmt.catch.as_ref().is_some_and(|catch| {
                catch.binding.as_ref().is_some_and(|binding| !matches!(binding, Binding::Ident(_)))
                    || block_needs_fallback(&catch.body)
            }) || try_stmt.finally.as_ref().is_some_and(block_needs_fallback),
        _ => false,
    }
}

fn direct_eval(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ExprKind::Ident(name) => name.name == "eval",
        ExprKind::Paren(inner) => direct_eval(inner),
        _ => false,
    }
}

fn expression_needs_fallback(expr: &ast::Expr) -> bool {
    match &expr.kind {
        ExprKind::Call { callee, args, .. } => direct_eval(callee)
            || expression_needs_fallback(callee) || args.iter().any(expression_needs_fallback),
        ExprKind::New { callee, args } => expression_needs_fallback(callee)
            || args.iter().any(expression_needs_fallback),
        ExprKind::Function(function) => function_needs_fallback(function),
        ExprKind::Arrow(arrow) => arrow.params.iter().any(|param| binding_needs_fallback(&param.binding)
            || param.default.as_ref().is_some_and(expression_needs_fallback))
            || match &arrow.body {
                ArrowBody::Expr(expr) => expression_needs_fallback(expr),
                ArrowBody::Block(block) => block_needs_fallback(block),
            },
        ExprKind::Class(class) => class_needs_fallback(class),
        ExprKind::Unary { operand, .. } | ExprKind::Update { target: operand, .. }
        | ExprKind::Spread(operand) | ExprKind::Paren(operand) => expression_needs_fallback(operand),
        ExprKind::Binary { left, right, .. } | ExprKind::Logical { left, right, .. }
        | ExprKind::Assign { target: left, value: right, .. } =>
            expression_needs_fallback(left) || expression_needs_fallback(right),
        ExprKind::Conditional { test, consequent, alternate } =>
            expression_needs_fallback(test) || expression_needs_fallback(consequent)
                || expression_needs_fallback(alternate),
        ExprKind::Member { obj, prop, .. } => expression_needs_fallback(obj)
            || matches!(prop, MemberProp::Computed(expr) if expression_needs_fallback(expr)),
        ExprKind::Sequence(exprs) | ExprKind::DynamicImport(exprs) =>
            exprs.iter().any(expression_needs_fallback),
        ExprKind::DynamicImportPhase { args, .. } => args.iter().any(expression_needs_fallback),
        ExprKind::Template(template) => template.exprs.iter().any(expression_needs_fallback),
        ExprKind::TaggedTemplate { tag, template } => expression_needs_fallback(tag)
            || template.exprs.iter().any(expression_needs_fallback),
        ExprKind::Array(items) => items.iter().flatten().any(expression_needs_fallback),
        ExprKind::Object(items) => items.iter().any(|item| match item {
            ObjElem::Spread(expr) => expression_needs_fallback(expr),
            ObjElem::Prop(property) => key_needs_fallback(&property.key)
                || match &property.value {
                    PropValue::Shorthand(_) => false,
                    PropValue::CoverInitializedName { default, .. } => expression_needs_fallback(default),
                    PropValue::Init(expr) => expression_needs_fallback(expr),
                    PropValue::Method(method) => method_needs_fallback(method),
                },
        }),
        ExprKind::Yield { argument, .. } => argument.as_ref().is_some_and(|expr| expression_needs_fallback(expr)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_script;

    #[test]
    fn direct_eval_and_pattern_catch_require_fallback() {
        for source in [
            "eval('var x=1')",
            "function f(){ return (eval)('1') }",
            "var o={run(){ eval('1') }}",
            "try { throw [1] } catch ([x]) { x }",
        ] {
            assert!(!eligible(&parse_script(source).unwrap()), "{source}");
        }
        for source in ["var o={x:1}; o.x", "function f(x){return x+1} f(2)", "try { 1 } catch (e) { e }"] {
            assert!(eligible(&parse_script(source).unwrap()), "{source}");
        }
    }
}
