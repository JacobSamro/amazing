// IteratorRecord and array-pattern stepping portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// `is_anonymous_function_definition` and `property_key_as_fn_name` below are ported/adapted
// from lucid-softworks/lumen (crates/lumen/src/eval.rs, IsAnonymousFunctionDefinition/
// SetFunctionName helpers) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.

fn has_use_strict_directive(statements: &[ast::Stmt]) -> bool {
    statements.iter()
        .take_while(|stmt| matches!(&stmt.kind, StmtKind::Expression(ast::Expr { kind: ExprKind::Str(_), .. })))
        .any(|stmt| matches!(&stmt.kind, StmtKind::Expression(ast::Expr { kind: ExprKind::Str(value), .. }) if value == "use strict"))
}

// Interpreter binding and reference helpers.


fn reference_name(reference: &Reference) -> Option<String> {
    match reference {
        Reference::Binding(name, _) | Reference::ObjectBinding(_, name, _) | Reference::Unresolvable(name, _) => Some(name.clone()),
        Reference::RawProperty(_, key) | Reference::RawSuperProperty(_, key, _) => match key {
            JsValue::Str(name) => Some(name.to_utf8_lossy()), _ => None,
        },
        Reference::Property(_, key) | Reference::SuperProperty(_, key, _) => key.as_string().map(|name| name.to_utf8_lossy()),
        Reference::Private(_, name) => Some(format!("#{}", name.text())),
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

/// IsAnonymousFunctionDefinition (spec 8.6.2): true only for a bare, nameless function/arrow/
/// class expression -- a `CoverParenthesizedExpression` is transparent (`(function(){})` is
/// still anonymous-eligible; `(0, function(){})` is not, since unwrapping its parens reaches a
/// Sequence expression, not a function), but every other kind of wrapping (a call, a ternary,
/// another operator) is not. Every NamedEvaluation call site below (JSRT-19) checks this on the
/// *source expression* before naming a value -- naming unconditionally on "the produced value
/// happens to be a nameless function" would over-fire for exactly the excluded cases.
fn is_anonymous_function_definition(expression: &ast::Expr) -> bool {
    match &expression.kind {
        ExprKind::Paren(inner) => is_anonymous_function_definition(inner),
        ExprKind::Function(f) => f.name.is_none(),
        ExprKind::Arrow(_) => true,
        ExprKind::Class(c) => c.name.is_none(),
        _ => false,
    }
}

/// The name argument SetFunctionName expects for a given property key (spec 7.3.9): a string
/// key as-is, a numeric key's string form (already folded into `PropertyKey::String` by the
/// time it reaches here), a symbol key with a description as `[description]`, or `""` for a
/// symbol with no description.
fn property_key_as_fn_name(key: &PropertyKey) -> String {
    match key {
        PropertyKey::String(s) => s.to_utf8_lossy(),
        PropertyKey::Symbol(s) => match s.description() {
            Some(d) => format!("[{}]", d.to_utf8_lossy()),
            None => String::new(),
        },
    }
}

/// IteratorRecord used by array binding and assignment patterns. Even intrinsic
/// arrays use the actual iterator so prototype hooks and live length stay visible.
struct PatternIter {
    iterator: JsValue,
    next: JsValue,
    done: bool,
}

impl PatternIter {
    fn new(i: &mut Interpreter, value: &JsValue) -> Result<Self, Throw> {
        let method = i.get_prop(value, WellKnownSymbol::Iterator.symbol())?;
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("value is not iterable"));
        }
        let iterator = i.call_value(&method, value.clone(), Vec::new())?;
        if !iterator.is_object() {
            return Err(Throw::type_error("iterator is not an object"));
        }
        // GetIterator caches next; callability is checked only by IteratorNext.
        let next = i.get_prop(&iterator, "next")?;
        Ok(Self { iterator, next, done: false })
    }

    fn step_result(&mut self, i: &mut Interpreter) -> Result<Option<JsValue>, Throw> {
        if self.done { return Ok(None); }
        let result = (|| {
            let result = i.call_value(&self.next, self.iterator.clone(), Vec::new())?;
            if !result.is_object() {
                return Err(Throw::type_error("iterator result is not an object"));
            }
            if i.get_prop(&result, "done")?.to_boolean() {
                self.done = true;
                Ok(None)
            } else {
                Ok(Some(result))
            }
        })();
        if result.is_err() { self.done = true; }
        result
    }

    fn step(&mut self, i: &mut Interpreter) -> Result<Option<JsValue>, Throw> {
        let Some(result) = self.step_result(i)? else { return Ok(None); };
        match i.get_prop(&result, "value") {
            Ok(value) => Ok(Some(value)),
            Err(error) => { self.done = true; Err(error) }
        }
    }

    /// Elision performs IteratorStep without observing IteratorValue.
    fn skip(&mut self, i: &mut Interpreter) -> Result<(), Throw> {
        self.step_result(i).map(|_| ())
    }

    fn collect_rest(&mut self, i: &mut Interpreter) -> Result<Vec<JsValue>, Throw> {
        let mut values = Vec::new();
        while let Some(value) = self.step(i)? { values.push(value); }
        Ok(values)
    }

    /// Callers combine completions: an existing throw wins over close failure.
    fn close(&mut self, i: &mut Interpreter) -> Result<(), Throw> {
        if self.done { return Ok(()); }
        self.done = true;
        i.close_iterator(&self.iterator)
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
        if kind == Some(ast::VarKind::Var) && has_local_object_environment(env) {
            let target = with_assignment_target(binding);
            return self.assign_pattern_expr(&target, &value, env);
        }
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
                    let mut v = match inner.as_ref() {
                        Binding::Ident(identifier) =>
                            self.eval_named_class_expr(expression, env, &identifier.name.clone().into())?,
                        _ => self.eval_expr(expression, env)?,
                    };
                    if let Binding::Ident(identifier) = inner.as_ref() {
                        self.named_evaluation(expression, &mut v, &identifier.name);
                    }
                    v
                } else {
                    value
                };
                self.bind_pattern_mode(inner, value, kind, env)?;
            }
            Binding::Rest(inner) => self.bind_pattern_mode(inner, value, kind, env)?,
            Binding::Array(elements) => {
                // GetIterator (§7.4.2) must throw when `value` has no usable
                // iterator, and a pattern without a trailing rest element
                // must pull only as many values as it declares (§8.6.3),
                // closing the iterator afterward rather than draining an
                // iterator that may never signal done — see `PatternIter`.
                let mut iter = PatternIter::new(self, &value)?;
                let result = (|| -> Result<(), Throw> {
                    for element in elements {
                        if let Some(element) = element {
                            if let Binding::Rest(rest) = element.as_ref() {
                                let items = iter.collect_rest(self)?;
                                let rest_value = array_values(self, items)?;
                                self.bind_pattern_mode(rest, rest_value, kind, env)?;
                            } else {
                                let item = iter.step(self)?.unwrap_or(JsValue::Undefined);
                                self.bind_pattern_mode(element, item, kind, env)?;
                            }
                        } else {
                            iter.skip(self)?;
                        }
                    }
                    Ok(())
                })();
                let closed = iter.close(self);
                // `result.and(closed)`: an already-abrupt `result` wins outright (`closed` is
                // discarded, matching 7.4.6 step 2); a normal `result` defers to whatever
                // `closed` says (its own `Err`, if any, becomes the new completion).
                result.and(closed)?;
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
