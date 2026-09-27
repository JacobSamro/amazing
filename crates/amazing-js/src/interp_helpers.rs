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
        Reference::Binding(name) => Some(name.clone()),
        Reference::Property(_, key) => key.as_string().map(|name| name.to_utf8_lossy()),
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

/// A lazily-stepped iterator for array-pattern `BindingInitialization`
/// (§8.6.2/§8.6.3): unlike `Interpreter::iterate`'s eager collect-everything
/// (right for spread and for-of, which always want every value), a pattern
/// without a trailing rest element must pull only as many values as it has
/// elements, then perform IteratorClose (§7.4.11) exactly once — an iterator
/// that legitimately never signals `done` (e.g. an infinite generator bound
/// to `[x]`) must not be drained to completion first.
enum PatternIter {
    /// An array whose `[Symbol.iterator]` is still the untouched intrinsic:
    /// index access is spec-equivalent, and %ArrayIteratorPrototype% has no
    /// `return` method, so IteratorClose on it is always a no-op — `close`
    /// is simply never called for this variant.
    Fast { value: JsValue, index: usize, len: usize },
    Protocol { iterator: JsValue, next: JsValue, done: bool },
}

impl PatternIter {
    fn new(i: &mut Interpreter, value: &JsValue) -> Result<PatternIter, Throw> {
        if let JsValue::Object(object) = value {
            if matches!(object.borrow().exotic, Exotic::Array) && i.has_original_array_iterator(value)? {
                let len = match object.borrow().get_own("length") {
                    Some(JsValue::Num(n)) => n as usize,
                    _ => 0,
                };
                return Ok(PatternIter::Fast { value: value.clone(), index: 0, len });
            }
        }
        let method = i.get_prop(value, WellKnownSymbol::Iterator.symbol())?;
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("value is not iterable"));
        }
        let iterator = i.call_value(&method, value.clone(), Vec::new())?;
        let next = i.get_prop(&iterator, "next")?;
        if callable_function(&next).is_none() {
            return Err(Throw::type_error("iterator's next method is not callable"));
        }
        Ok(PatternIter::Protocol { iterator, next, done: false })
    }

    /// `Ok(None)` is IteratorStep returning done — never advances past it.
    fn step(&mut self, i: &mut Interpreter) -> Result<Option<JsValue>, Throw> {
        match self {
            PatternIter::Fast { value, index, len } => {
                if *index >= *len {
                    return Ok(None);
                }
                let v = i.get_prop(value, &index.to_string())?;
                *index += 1;
                Ok(Some(v))
            }
            PatternIter::Protocol { iterator, next, done } => {
                if *done {
                    return Ok(None);
                }
                let result = match i.call_value(next, iterator.clone(), Vec::new()) {
                    Ok(result) => result,
                    Err(error) => {
                        *done = true;
                        return Err(error);
                    }
                };
                let result_value = match result {
                    JsValue::Object(_) => result,
                    _ => {
                        *done = true;
                        return Err(Throw::type_error("iterator result is not an object"));
                    }
                };
                // IteratorComplete/IteratorValue (§7.4.6/§7.4.7): the full
                // [[Get]], not a raw own-property probe, so a `done`/`value`
                // accessor that throws is observed instead of silently
                // reading as absent (an iterator whose result objects define
                // `value` as a getter would otherwise loop forever).
                let is_done = match i.get_prop(&result_value, "done") {
                    Ok(value) => value.to_boolean(),
                    Err(error) => {
                        *done = true;
                        return Err(error);
                    }
                };
                if is_done {
                    *done = true;
                    return Ok(None);
                }
                match i.get_prop(&result_value, "value") {
                    Ok(value) => Ok(Some(value)),
                    Err(error) => {
                        *done = true;
                        Err(error)
                    }
                }
            }
        }
    }

    /// Pulls every remaining value for a trailing rest element; the
    /// iterator is naturally exhausted afterward, so `close` need not run.
    fn collect_rest(&mut self, i: &mut Interpreter) -> Result<Vec<JsValue>, Throw> {
        let mut values = Vec::new();
        while let Some(value) = self.step(i)? {
            values.push(value);
        }
        Ok(values)
    }

    /// IteratorClose: a no-op once already done (naturally exhausted, a
    /// rest element drained it, or `step` itself already failed).
    fn close(&mut self, i: &mut Interpreter) {
        if let PatternIter::Protocol { iterator, done, .. } = self {
            if !*done {
                i.close_iterator(iterator);
                *done = true;
            }
        }
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
                    let mut v = self.eval_expr(expression, env)?;
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
                            iter.step(self)?;
                        }
                    }
                    Ok(())
                })();
                iter.close(self);
                result?;
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
