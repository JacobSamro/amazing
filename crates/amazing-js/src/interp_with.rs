// With-only binding adapter. Reference resolution and destructuring execution
// remain in the shared assignment-pattern algorithms.

fn has_local_object_environment(env: &EnvHandle) -> bool {
    let mut current = Some(env.clone());
    while let Some(env) = current {
        let frame = env.0.borrow();
        if frame.binding_object.is_some() { return true; }
        if !matches!(frame.kind, EnvKind::Block | EnvKind::Catch) { return false; }
        current = frame.parent.clone();
    }
    false
}

fn uses_object_var_assignment(env: &EnvHandle) -> bool {
    has_local_object_environment(env) || env.var_scope().kind() == EnvKind::Global
}

fn with_assignment_target(binding: &Binding) -> ast::Expr {
    let span = match binding {
        Binding::Ident(name) => name.span,
        Binding::Default(_, expression) => expression.span,
        _ => crate::tokens::Span::new(crate::tokens::SourceLocation::new(1, 1), crate::tokens::SourceLocation::new(1, 1)),
    };
    let kind = match binding {
        Binding::Ident(name) => ExprKind::Ident(name.clone()),
        Binding::Array(elements) => ExprKind::Array(elements.iter().map(|element|
            element.as_ref().map(|inner| with_assignment_target(inner))).collect()),
        Binding::Object(properties) => ExprKind::Object(properties.iter().map(|(key, inner)| {
            if matches!(key, ast::BindingKey::Static(name) if name == "...") {
                ast::ObjElem::Spread(Box::new(with_assignment_target(inner)))
            } else {
                let key = match key {
                    ast::BindingKey::Static(name) => ast::PropKey::Str(name.clone()),
                    ast::BindingKey::Computed(expression) => ast::PropKey::Computed(expression.clone()),
                };
                ast::ObjElem::Prop(ast::Property {
                    key, key_utf16: None, value: ast::PropValue::Init(with_assignment_target(inner)), span,
                })
            }
        }).collect()),
        Binding::Rest(inner) => ExprKind::Spread(Box::new(with_assignment_target(inner))),
        Binding::Default(inner, expression) => ExprKind::Assign {
            op: ast::AssignOp::Assign, target: Box::new(with_assignment_target(inner)), value: expression.clone(),
        },
    };
    ast::Expr::new(kind, span)
}
