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

impl Interpreter {
    fn bind_pattern(
        &mut self, binding: &Binding, value: JsValue,
        kind: ast::VarKind, env: &EnvHandle,
    ) -> Result<(), Throw> {
        self.bind_pattern_mode(binding, value, Some(kind), env)
    }

    fn bind_pattern_mode(
        &mut self,
        binding: &Binding,
        value: JsValue,
        kind: Option<ast::VarKind>,
        env: &EnvHandle,
    ) -> Result<(), Throw> {
        match binding {
            Binding::Ident(identifier) => match kind {
                None => env.initialize_parameter(&identifier.name, value)?,
                Some(ast::VarKind::Var) => {
                    if !env.assign(&identifier.name, value.clone())? {
                        self.global_env.declare_var(&identifier.name, value)?;
                    }
                }
                Some(ast::VarKind::Let) => env.declare_lexical(&identifier.name, value, true)?,
                Some(ast::VarKind::Const) => env.declare_lexical(&identifier.name, value, false)?,
            },
            Binding::Default(inner, expression) => {
                let value = if value.is_undefined() {
                    let mut value = self.eval_expr(expression, env)?;
                    if let Binding::Ident(name) = inner.as_ref() {
                        self.infer_function_name(&mut value, &name.name);
                    }
                    value
                } else {
                    value
                };
                self.bind_pattern_mode(inner, value, kind, env)?;
            }
            Binding::Rest(inner) => self.bind_pattern_mode(inner, value, kind, env)?,
            Binding::Array(elements) => {
                let method = self.get_prop(&value, WellKnownSymbol::Iterator.symbol())?;
                if callable_function(&method).is_none() {
                    return Err(Throw::type_error("value is not iterable"));
                }
                let iterator = self.call_value(&method, value, Vec::new())?;
                if !iterator.is_object() {
                    return Err(Throw::type_error("iterator is not an object"));
                }
                let next = self.get_prop(&iterator, "next")?;
                if callable_function(&next).is_none() {
                    return Err(Throw::type_error("iterator next is not callable"));
                }
                let mut exhausted = false;
                for element in elements {
                    if let Some(Binding::Rest(rest)) = element.as_deref() {
                        let mut values = Vec::new();
                        if !exhausted {
                            for _ in 0..100_000 {
                                match self.binding_iterator_step(&iterator, &next) {
                                    Ok(Some(value)) => values.push(value),
                                    Ok(None) => { exhausted = true; break; }
                                    Err(error) => { self.close_iterator(&iterator); return Err(error); }
                                }
                            }
                        }
                        if !exhausted {
                            self.close_iterator(&iterator);
                            return Err(Throw::range_error("iterator exceeded the iteration limit"));
                        }
                        let rest_value = array_values(self, values)?;
                        self.bind_pattern_mode(rest, rest_value, kind, env)?;
                    } else {
                        let item = if exhausted { JsValue::Undefined } else {
                            match self.binding_iterator_step(&iterator, &next) {
                                Ok(Some(value)) => value,
                                Ok(None) => { exhausted = true; JsValue::Undefined }
                                Err(error) => { self.close_iterator(&iterator); return Err(error); }
                            }
                        };
                        if let Some(element) = element {
                            if let Err(error) = self.bind_pattern_mode(element, item, kind, env) {
                                if !exhausted { self.close_iterator(&iterator); }
                                return Err(error);
                            }
                        }
                    }
                }
                if !exhausted { self.close_iterator(&iterator); }
            }
            Binding::Object(properties) => {
                let source = match value {
                    JsValue::Object(_) | JsValue::Str(_) => value,
                    JsValue::Null | JsValue::Undefined => {
                        return Err(Throw::type_error("cannot destructure nullish value"))
                    }
                    other => other,
                };
                let mut used = Vec::new();
                for (key, property) in properties {
                    match key {
                        ast::BindingKey::Static(name) if name == "..." => {
                            let rest = self.object_rest(&source, &used)?;
                            self.bind_pattern_mode(property, rest, kind, env)?;
                        }
                        ast::BindingKey::Static(name) => {
                            used.push(name.clone().into());
                            let item = self.get_prop(&source, name)?;
                            self.bind_pattern_mode(property, item, kind, env)?;
                        }
                        ast::BindingKey::Computed(expression) => {
                            let computed_value = self.eval_expr(expression, env)?;
                            let computed = self.to_property_key(&computed_value)?;
                            used.push(computed.clone());
                            let item = self.get_prop(&source, &computed)?;
                            self.bind_pattern_mode(property, item, kind, env)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
