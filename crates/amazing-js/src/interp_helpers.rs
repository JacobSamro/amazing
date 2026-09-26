fn has_use_strict_directive(statements: &[ast::Stmt]) -> bool {
    statements.iter()
        .take_while(|stmt| matches!(&stmt.kind, StmtKind::Expression(ast::Expr { kind: ExprKind::Str(_), .. })))
        .any(|stmt| matches!(&stmt.kind, StmtKind::Expression(ast::Expr { kind: ExprKind::Str(value), .. }) if value == "use strict"))
}

// Interpreter binding and reference helpers.


fn reference_name(reference: &Reference) -> Option<String> {
    match reference {
        Reference::Binding(name) => Some(name.clone()),
        Reference::Private(_, _) => None,
        Reference::Property(_, key) => key.as_string().map(|name| name.to_utf8_lossy()),
    }
}

fn binding_names(binding: &Binding) -> Vec<String> {
    match binding {
        Binding::Ident(identifier) => vec![identifier.name.clone()],
        Binding::Array(elements) => elements
            .iter()
            .flatten()
            .flat_map(|element| binding_names(element))
            .collect(),
        Binding::Object(properties) => properties
            .iter()
            .flat_map(|(_, property)| binding_names(property))
            .collect(),
        Binding::Rest(inner) | Binding::Default(inner, _) => binding_names(inner),
    }
}

fn is_destructuring_expr(expression: &ast::Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => is_destructuring_expr(inner),
        ExprKind::Array(_) | ExprKind::Object(_) => true,
        _ => false,
    }
}

fn for_iteration_env(left: &ForLeft, env: &EnvHandle) -> EnvHandle {
    match left {
        ForLeft::Decl(declaration) if declaration.kind != ast::VarKind::Var => {
            EnvHandle::new(EnvKind::Block, Some(env.clone()))
        }
        _ => env.clone(),
    }
}
