// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Decorator contexts, callable/field/accessor replacement and initializer folding,
// adapted to Amazing's property gateways and per-decoration closure state.

#[derive(Clone)]
struct DecoratorDefinition { value: JsValue, receiver: JsValue }

struct DecoratedElement {
    key: FieldKey,
    storage: Option<FieldKey>,
    kind: &'static str,
    is_static: bool,
    decorators: Vec<DecoratorDefinition>,
    value: JsValue,
    get: JsValue,
    set: JsValue,
    init: Option<ast::Expr>,
    initializers: Vec<JsValue>,
    extra_initializers: Vec<JsValue>,
}

enum ClassDefinitionElement {
    Element(DecoratedElement),
    Block(ast::Block),
}

fn decorator_native(i: &Interpreter, name: &str, callback: crate::value::NativeFn,
    arity: usize, state: &Obj, env: &EnvHandle) -> JsValue {
    let value = Interpreter::native_arity(name, callback, &i.function_proto, arity);
    if let JsValue::Object(function) = &value {
        let mut function = function.borrow_mut();
        let function = function.func.as_mut().unwrap();
        function.home = Some(state.clone());
        function.env = env.clone();
    }
    value
}

fn decorator_key_value(key: &FieldKey) -> JsValue {
    match key {
        FieldKey::Public(PropertyKey::String(name)) => JsValue::str(name.clone()),
        FieldKey::Public(PropertyKey::Symbol(symbol)) => JsValue::Symbol(symbol.clone()),
        FieldKey::Private(name) => JsValue::str(name.text()),
    }
}

fn decorator_access_key(i: &Interpreter) -> Result<FieldKey, Throw> {
    let state = native_data(i)?;
    let key = state.borrow().get_own("key").unwrap();
    if state.borrow().get_own("private") == Some(JsValue::Bool(true)) {
        let function = i.native_stack.last().unwrap().borrow();
        let name = function.func.as_ref().unwrap().env.resolve_private("[[DecoratorAccess]]")
            .expect("private decorator access captures its brand");
        Ok(FieldKey::Private(name))
    } else {
        Ok(FieldKey::Public(match key {
            JsValue::Str(name) => PropertyKey::String(name),
            JsValue::Symbol(symbol) => PropertyKey::Symbol(symbol),
            _ => unreachable!("decorator access key is a property key"),
        }))
    }
}

fn decorator_access_get(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !object.is_object() { return Err(Throw::type_error("decorator access requires an object")); }
    match decorator_access_key(i)? {
        FieldKey::Public(key) => i.get_prop(&object, key),
        FieldKey::Private(name) => private_field_get(i, &object, &name),
    }
}

fn decorator_access_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !object.is_object() { return Err(Throw::type_error("decorator access requires an object")); }
    match decorator_access_key(i)? {
        FieldKey::Public(key) => {
            let JsValue::Object(target) = &object else { unreachable!() };
            if !i.set_with_receiver(target, &key, value, object.clone())? {
                return Err(Throw::type_error("decorator access could not set property"));
            }
        }
        FieldKey::Private(name) => private_field_set(i, &object, &name, value)?,
    }
    Ok(JsValue::Undefined)
}

fn decorator_access_has(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let Some(JsValue::Object(object)) = args.first() else {
        return Err(Throw::type_error("decorator access requires an object"));
    };
    let has = match decorator_access_key(i)? {
        FieldKey::Public(key) => i.default_has_property(object, &key)?,
        FieldKey::Private(name) => object.borrow().private_has(&name),
    };
    Ok(JsValue::Bool(has))
}

fn decorator_add_initializer(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let state = native_data(i)?;
    if state.borrow().get_own("finished") == Some(JsValue::Bool(true)) {
        return Err(Throw::type_error("decoration has already finished"));
    }
    let initializer = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&initializer).is_none() { return Err(Throw::type_error("initializer must be callable")); }
    let mut state = state.borrow_mut();
    let JsValue::Num(count) = state.get_own("count").unwrap() else { unreachable!() };
    state.set_own((count as usize).to_string(), initializer);
    state.set_own("count", JsValue::num(count + 1.0));
    Ok(JsValue::Undefined)
}

fn run_decorator_initializers(i: &mut Interpreter, owner: &JsValue, initializers: &[JsValue]) -> Result<(), Throw> {
    for initializer in initializers { i.call_value(initializer, owner.clone(), Vec::new())?; }
    Ok(())
}

impl Interpreter {
    fn evaluate_decorators(&mut self, decorators: &[ast::Expr], env: &EnvHandle) -> Result<Vec<DecoratorDefinition>, Throw> {
        let mut result = Vec::new();
        for decorator in decorators {
            let (value, receiver) = self.chain_callee(decorator, env)?
                .unwrap_or((JsValue::Undefined, JsValue::Undefined));
            result.push(DecoratorDefinition { value, receiver });
        }
        Ok(result)
    }

    fn decorator_context(&self, kind: &str, key: &FieldKey, is_static: bool, env: &EnvHandle) -> (JsValue, Obj) {
        let context = self.object_value();
        let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        state.borrow_mut().set_own("finished", JsValue::Bool(false));
        state.borrow_mut().set_own("count", JsValue::num(0.0));
        let JsValue::Object(object) = &context else { unreachable!() };
        object.borrow_mut().set_own("kind", JsValue::str(kind));
        if kind != "class" {
            let private = matches!(key, FieldKey::Private(_));
            let access_state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
            access_state.borrow_mut().set_own("key", decorator_key_value(key));
            access_state.borrow_mut().set_own("private", JsValue::Bool(private));
            let access_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
            if let FieldKey::Private(name) = key {
                access_env.set_class_private(std::collections::HashMap::from([("[[DecoratorAccess]]".into(), name.clone())]));
            }
            let access = self.object_value();
            let JsValue::Object(access_object) = &access else { unreachable!() };
            if matches!(kind, "field" | "accessor" | "method" | "getter") {
                access_object.borrow_mut().set_own("get", decorator_native(self, "", decorator_access_get, 1, &access_state, &access_env));
            }
            if matches!(kind, "field" | "accessor" | "setter") {
                access_object.borrow_mut().set_own("set", decorator_native(self, "", decorator_access_set, 2, &access_state, &access_env));
            }
            access_object.borrow_mut().set_own("has", decorator_native(self, "has", decorator_access_has, 1, &access_state, &access_env));
            object.borrow_mut().set_own("access", access);
            object.borrow_mut().set_own("static", JsValue::Bool(is_static));
            object.borrow_mut().set_own("private", JsValue::Bool(private));
        }
        object.borrow_mut().set_own("name", decorator_key_value(key));
        object.borrow_mut().set_own("addInitializer", decorator_native(self, "addInitializer", decorator_add_initializer, 1, &state, env));
        (context, state)
    }

    fn invoke_decorator(&mut self, decorator: &DecoratorDefinition, value: JsValue,
        kind: &str, key: &FieldKey, is_static: bool, env: &EnvHandle, extras: &mut Vec<JsValue>) -> Result<JsValue, Throw> {
        let (context, state) = self.decorator_context(kind, key, is_static, env);
        let result = self.call_value(&decorator.value, decorator.receiver.clone(), vec![value, context]);
        state.borrow_mut().set_own("finished", JsValue::Bool(true));
        let result = result?;
        let JsValue::Num(count) = state.borrow().get_own("count").unwrap() else { unreachable!() };
        for n in 0..count as usize { extras.push(state.borrow().get_own(n.to_string()).unwrap()); }
        Ok(result)
    }

    fn apply_element_decorators(&mut self, element: &mut DecoratedElement, env: &EnvHandle,
        method_extras: &mut Vec<JsValue>) -> Result<(), Throw> {
        let extras = if matches!(element.kind, "field" | "accessor") {
            &mut element.extra_initializers
        } else { method_extras };
        for decorator in element.decorators.iter().rev() {
            let value = match element.kind {
                "field" => JsValue::Undefined,
                "getter" => element.get.clone(),
                "setter" => element.set.clone(),
                "accessor" => {
                    let value = self.object_value();
                    let JsValue::Object(object) = &value else { unreachable!() };
                    object.borrow_mut().set_own("get", element.get.clone());
                    object.borrow_mut().set_own("set", element.set.clone());
                    value
                }
                _ => element.value.clone(),
            };
            let replacement = self.invoke_decorator(decorator, value, element.kind,
                &element.key, element.is_static, env, extras)?;
            if replacement.is_undefined() { continue; }
            if element.kind == "accessor" {
                if !replacement.is_object() { return Err(Throw::type_error("accessor decorator must return an object or undefined")); }
                for property in ["get", "set", "init"] {
                    let function = self.get_prop(&replacement, property)?;
                    if function.is_undefined() { continue; }
                    if callable_function(&function).is_none() { return Err(Throw::type_error("accessor decorator replacement must be callable")); }
                    match property {
                        "get" => element.get = function,
                        "set" => element.set = function,
                        _ => element.initializers.insert(0, function),
                    }
                }
            } else {
                if callable_function(&replacement).is_none() { return Err(Throw::type_error("decorator must return a function or undefined")); }
                match element.kind {
                    "field" => element.initializers.insert(0, replacement),
                    "getter" => element.get = replacement,
                    "setter" => element.set = replacement,
                    _ => element.value = replacement,
                }
            }
        }
        Ok(())
    }
}
