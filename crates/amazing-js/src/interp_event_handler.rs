//! Host-facing HTML handler compilation and live object environment lookup.
use super::*;

impl Interpreter {
    /// Compile a handler in this interpreter's global realm without executing it.
    ///
    /// `scope_chain` is outermost first: document, optional form owner, element.
    /// Window handlers pass an empty slice. Objects stay live: later property
    /// changes and prototype/accessor behavior are observed at invocation time.
    /// The caller owns lazy compilation, listener registration and error reporting.
    pub fn compile_event_handler(
        &mut self,
        name: &str,
        parameters: &[&str],
        body: &str,
        scope_chain: &[JsValue],
    ) -> Result<JsValue, Throw> {
        let function = crate::parser::parse_event_handler(name, parameters, body)
            .map_err(|error| Throw::syntax_error(error.to_string()))?;
        let mut env = self.global_env.clone();
        for value in scope_chain {
            let JsValue::Object(object) = value else {
                return Err(Throw::type_error("event-handler scope must be an object"));
            };
            env = EnvHandle::new_object(object.clone(), env);
        }
        // Do not inherit the compiling script's strictness or module environment.
        let strict = has_use_strict_directive(&function.body.body);
        let mut object = JsObject::function(JsFunction {
            name: name.to_owned(), params: function.params,
            body: FunctionBody::Block(function.body), env,
            is_arrow: false, is_async: false, is_generator: false,
            home: None, proto_for_new: None,
        });
        object.proto = Some(self.function_proto.clone());
        object.strict_function = strict;
        object.constructible = false;
        Ok(JsValue::Object(Obj(Rc::new(RefCell::new(object)))))
    }

    /// Find the first binding, preserving object scopes between lexical frames.
    pub(super) fn scoped_reference(&mut self, env: &EnvHandle, name: &str) -> Result<Option<Reference>, Throw> {
        let mut current = Some(env.clone());
        while let Some(frame) = current {
            if frame.local_cell(name).is_some() {
                if let Some(object) = frame.global_binding_object(name) {
                    if self.default_has_property(&object, &PropertyKey::from(name))? {
                        return Ok(Some(Reference::ObjectBinding(object, name.to_owned(), false)));
                    }
                } else { return Ok(Some(Reference::Binding(name.to_owned(), frame.clone()))); }
            }
            let (object, parent) = {
                let frame = frame.0.borrow();
                (frame.binding_object.clone(), frame.parent.clone())
            };
            if let Some(object) = object {
                if self.legacy_has_property(&object, name)? {
                    let value = JsValue::Object(object);
                    let unscopables = self.get_prop(&value,
                        &PropertyKey::Symbol(WellKnownSymbol::Unscopables.symbol()))?;
                    if !unscopables.is_object() || !self.get_prop(&unscopables, name)?.to_boolean() {
                        return Ok(Some(Reference::ObjectBinding(match value { JsValue::Object(object) => object, _ => unreachable!() }, name.to_owned(), true)));
                    }
                }
            }
            current = parent;
        }
        let global = env.realm_global_object().unwrap_or_else(|| self.global_obj.clone());
        if self.default_has_property(&global, &PropertyKey::from(name))? {
            Ok(Some(Reference::ObjectBinding(global, name.to_owned(), false)))
        } else { Ok(None) }
    }

    pub(super) fn name_reference(&mut self, env: &EnvHandle, name: &str) -> Result<Reference, Throw> {
        Ok(self.scoped_reference(env, name)?.unwrap_or_else(||
            Reference::Unresolvable(name.to_owned(), env.realm_global_object().unwrap_or_else(|| self.global_obj.clone()))))
    }

    pub(super) fn name_reference_value(&mut self, env: &EnvHandle, name: &str) -> Result<(Reference, JsValue), Throw> {
        let mut reference = self.name_reference(env, name)?;
        let value = self.reference_value(&mut reference)?;
        Ok((reference, value))
    }

    pub(super) fn scoped_call(&mut self, env: &EnvHandle, name: &str) -> Result<(JsValue, JsValue), Throw> {
        let mut reference = self.name_reference(env, name)?;
        let receiver = match &reference {
            Reference::ObjectBinding(object, _, true) => JsValue::Object(object.clone()),
            _ => JsValue::Undefined,
        };
        let value = self.reference_value(&mut reference)?;
        Ok((value, receiver))
    }

}

pub(super) fn identifier_name(expression: &ast::Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Ident(identifier) => Some(&identifier.name),
        ExprKind::Paren(inner) => identifier_name(inner),
        _ => None,
    }
}

impl Interpreter {
    pub(super) fn delete_scoped_name(&mut self, env: &EnvHandle, name: &str) -> Result<JsValue, Throw> {
        let deleted = match self.scoped_reference(env, name)? {
            None => true,
            Some(Reference::ObjectBinding(object, key, with)) => {
                if with {
                    if let Some(result) = self.legacy_delete(&object, name)? { result }
                    else { object.borrow_mut().delete_own(&key) }
                } else {
                    let deleted = self.default_delete_property(&object, &PropertyKey::from(key.as_str()))?;
                    if deleted { env.global_scope().remove_object_binding(name); }
                    deleted
                }
            }
            Some(Reference::Binding(binding, _))
                if env.shares_binding_frame(&self.global_env, &binding) => {
                // Builtins have a mirrored global environment entry. Their
                // object descriptor, not a lexical-var rule, controls deletion.
                let configurable = self.global_obj.borrow().own_prop(name)
                    .is_some_and(|property| property.configurable);
                configurable && self.global_env.remove_object_binding(name)
                    && self.global_obj.borrow_mut().delete_own(name)
            }
            Some(_) => false,
        };
        Ok(JsValue::Bool(deleted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(interpreter: &mut Interpreter) -> Vec<JsValue> {
        interpreter.run_script("var outer = 100; var doc = {x: 1}; var form = {x: 2}; var element = {x: 3};").unwrap();
        ["doc", "form", "element"].iter()
            .map(|name| interpreter.global_env.resolve(name).unwrap()).collect()
    }

    #[test]
    fn handler_scope_precedence_is_live_and_parameters_stay_local() {
        let mut i = Interpreter::new();
        let scope = scopes(&mut i);
        let handler = i.compile_event_handler("onclick", &["event"],
            "x += event; return x + outer;", &scope).unwrap();
        assert_eq!(i.call_value(&handler, scope[2].clone(), vec![JsValue::num(4.0)]).unwrap(), JsValue::num(107.0));
        assert_eq!(i.run_script("element.x").unwrap(), JsValue::num(7.0));
        i.run_script("delete element.x; form.x = 10;").unwrap();
        assert_eq!(i.call_value(&handler, scope[2].clone(), vec![JsValue::num(1.0)]).unwrap(), JsValue::num(111.0));
        let handler = i.compile_event_handler("onclick", &["event"],
            "var x = event; return x;", &scope).unwrap();
        assert_eq!(i.call_value(&handler, JsValue::Undefined, vec![JsValue::num(42.0)]).unwrap(), JsValue::num(42.0));
        assert_eq!(i.run_script("form.x").unwrap(), JsValue::num(11.0));
    }

    #[test]
    fn handler_accessors_unscopables_and_method_receiver_use_object_records() {
        let mut i = Interpreter::new();
        let scope = scopes(&mut i);
        i.run_script("element.method = function() { return this === element; }; Object.defineProperty(element, 'access', {get: function() { return this.x; }, set: function(v) { this.x = v; }}); element[Symbol.unscopables] = {x: true};").unwrap();
        let handler = i.compile_event_handler("onclick", &["event"],
            "access = event; return method() && (method)() && access === 8 && x === 2 && ({x}).x === 2;", &scope).unwrap();
        assert_eq!(i.call_value(&handler, JsValue::Undefined, vec![JsValue::num(8.0)]).unwrap(), JsValue::Bool(true));
        let handler = i.compile_event_handler("onclick", &["event"],
            "return function* () { x++; yield method(); yield x; };", &scope).unwrap();
        let generator = i.call_value(&handler, JsValue::Undefined, vec![]).unwrap();
        i.global_env.declare_var("handlerGenerator", generator).unwrap();
        assert_eq!(i.run_script("var iterator = handlerGenerator(); iterator.next().value;").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("iterator.next().value;").unwrap(), JsValue::num(3.0));
    }

    #[test]
    fn handler_plain_and_destructuring_writes_do_not_read_the_scope_getter() {
        let mut i = Interpreter::new();
        let scope = scopes(&mut i);
        i.run_script("Object.defineProperty(element, 'sink', {get: function() { throw new Error('read during write'); }, set: function(value) { this.saved = value; }});").unwrap();
        for body in ["sink = event; return element.saved;",
            "({sink} = {sink:event}); [sink] = [event]; return element.saved;"] {
            let handler = i.compile_event_handler("onclick", &["event"], body, &scope).unwrap();
            assert_eq!(i.call_value(&handler, JsValue::Undefined, vec![JsValue::num(8.0)]).unwrap(), JsValue::num(8.0));
        }
        let handler = i.compile_event_handler("onclick", &["event"],
            "return function* () { sink = event; yield element.saved; };", &scope).unwrap();
        let generator = i.call_value(&handler, JsValue::Undefined, vec![JsValue::num(9.0)]).unwrap();
        i.global_env.declare_var("scopeWriter", generator).unwrap();
        assert_eq!(i.run_script("scopeWriter().next().value").unwrap(), JsValue::num(9.0));
    }

    #[test]
    fn handler_metadata_receiver_and_compilation_are_independent_of_caller() {
        let mut i = Interpreter::new();
        let scope = scopes(&mut i);
        i.strict_eval_context = true;
        let handler = i.compile_event_handler("onerror", &["event", "source", "lineno", "colno", "error"],
            "return [this, event, source, lineno, colno, error]; // eof", &[]).unwrap();
        assert_eq!(i.get_prop(&handler, "name").unwrap(), JsValue::str("onerror"));
        assert_eq!(i.get_prop(&handler, "length").unwrap(), JsValue::num(5.0));
        let JsValue::Object(object) = &handler else { panic!("function") };
        assert!(!object.borrow().strict_function);
        assert!(!object.borrow().constructible);
        assert!(i.construct(&handler, vec![]).is_err());
        assert!(i.strict_eval_context);
        let result = i.call_value(&handler, scope[2].clone(), vec![JsValue::num(1.0), JsValue::num(2.0), JsValue::num(3.0), JsValue::num(4.0), JsValue::num(5.0)]).unwrap();
        assert_eq!(i.get_prop(&result, "0").unwrap(), scope[2]);
        assert_eq!(i.get_prop(&result, "5").unwrap(), JsValue::num(5.0));
        assert!(i.compile_event_handler("onclick", &["event"], "} globalThis.injected = true; {", &[]).is_err());
        assert!(!i.global_obj.borrow().has_property("injected"));
    }

    #[test]
    fn handler_closures_delete_and_eval_keep_the_object_chain() {
        let mut i = Interpreter::new();
        let scope = scopes(&mut i);
        let handler = i.compile_event_handler("onclick", &["event"],
            "return function() { delete x; return eval('x'); };", &scope).unwrap();
        let closure = i.call_value(&handler, JsValue::Undefined, vec![]).unwrap();
        assert_eq!(i.call_value(&closure, JsValue::Undefined, vec![]).unwrap(), JsValue::num(2.0));
        assert_eq!(i.call_value(&closure, JsValue::Undefined, vec![]).unwrap(), JsValue::num(1.0));
        let handler = i.compile_event_handler("onclick", &["event"], "return typeof missing;", &scope).unwrap();
        assert_eq!(i.call_value(&handler, JsValue::Undefined, vec![]).unwrap(), JsValue::str("undefined"));
    }
}

#[cfg(test)]
mod global_binding_tests {
    use super::*;

    #[test]
    fn deletion_distinguishes_global_properties_from_declared_bindings() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            // Delete itself can fall back to the interpreter, while assignment
            // and the subsequent global read exercise both execution paths.
            i.run_script_with_backend("implicit = 1; implicit = 2;", backend).unwrap();
            assert_eq!(i.run_script("globalThis.implicit").unwrap(), JsValue::num(2.0));
            assert_eq!(i.run_script("delete implicit").unwrap(), JsValue::Bool(true));
            assert_eq!(i.run_script("typeof implicit").unwrap(), JsValue::str("undefined"));
            assert_eq!(i.run_script("delete JSON").unwrap(), JsValue::Bool(true));
            assert_eq!(i.run_script("typeof JSON").unwrap(), JsValue::str("undefined"));
            assert_eq!(i.run_script("var permanent = 1; delete permanent").unwrap(), JsValue::Bool(false));
            assert_eq!(i.run_script("delete undefined").unwrap(), JsValue::Bool(false));
            i.run_script_with_backend(
                "Object.preventExtensions(globalThis); rejectedImplicit = 1;", backend).unwrap();
            assert_eq!(i.run_script("typeof rejectedImplicit").unwrap(), JsValue::str("undefined"));
        }
    }
}
