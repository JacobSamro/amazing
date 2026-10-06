// Portions ported and adapted from lucid-softworks/lumen
// (crates/lumen/src/builtins/shadowrealm.rs) @ f6c9860d44fa52249cfd647122edea184d32c863,
// MIT, Copyright (c) 2026 Lucid Softworks limited.
// Safe same-agent RealmHandle entries replace upstream's unsafe sub-interpreter
// pointers. Native jobs/reactions replace eager imports and microtask drains.

impl Interpreter {
    fn install_shadow_realm(&mut self) {
        let mut prototype = JsObject::ordinary();
        prototype.proto = Some(self.object_proto.clone());
        let prototype = Obj(Rc::new(RefCell::new(prototype)));
        for (name, arity, implementation) in [
            ("evaluate", 1, native_shadow_evaluate as crate::value::NativeFn),
            ("importValue", 2, native_shadow_import_value as crate::value::NativeFn),
        ] {
            let method = Self::native_arity(name, implementation, &self.function_proto, arity);
            prototype.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(),
            symbol_data(JsValue::str("ShadowRealm"), false, true)).unwrap();
        let constructor = Self::native_constructor_arity("ShadowRealm", native_shadow_constructor, &self.function_proto, 0);
        let JsValue::Object(object) = &constructor else { unreachable!() };
        object.borrow_mut().delete_own("prototype");
        object.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
        prototype.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
        self.define_global("ShadowRealm", constructor);
    }

    fn shadow_realm_receiver(&self, receiver: &JsValue) -> Result<RealmHandle, Throw> {
        let JsValue::Object(object) = receiver else { return Err(Throw::type_error("incompatible ShadowRealm receiver")); };
        let realm = match &object.borrow().exotic {
            Exotic::ShadowRealm(realm) => *realm,
            _ => return Err(Throw::type_error("incompatible ShadowRealm receiver")),
        };
        self.check_realm(realm)?;
        Ok(realm)
    }

    fn shadow_wrapped_value(&mut self, destination: RealmHandle, value: JsValue) -> Result<JsValue, Throw> {
        let JsValue::Object(target) = &value else { return Ok(value); };
        if target.borrow().func.is_none() { return Err(Throw::type_error("ShadowRealm cannot transfer a non-callable object")); }
        self.check_realm(destination)?;
        let prototype = if destination == self.binding_current_realm() { self.function_proto.clone() }
            else { self.realms.get(&destination.realm).expect("checked realm").function_proto.clone() };
        let wrapped = Self::native("", native_shadow_wrapped_call, &prototype);
        let JsValue::Object(object) = &wrapped else { unreachable!() };
        let mut storage = JsObject::ordinary();
        storage.set_own("target", value.clone());
        {
            let mut object = object.borrow_mut();
            object.realm = Some(destination.realm);
            object.func.as_mut().unwrap().home = Some(Obj(Rc::new(RefCell::new(storage))));
        }
        // CopyNameAndLength must use semantic HasOwnProperty/Get in this order.
        // Metadata failures are copied without reading any thrown error property.
        let metadata = (|| {
            let mut length = 0.0;
            if self.default_get_own_property(target, &PropertyKey::from("length"))?.is_some() {
                if let JsValue::Num(number) = self.get_prop(&value, "length")? {
                    if number == f64::INFINITY { length = number; }
                    else if number.is_finite() && number > 0.0 { length = number.trunc(); }
                }
            }
            self.define_bound_metadata(object, "length", JsValue::num(length))?;
            let name = self.get_prop(&value, "name")?;
            self.define_bound_metadata(object, "name", match name { JsValue::Str(_) => name, _ => JsValue::str("") })
        })();
        metadata.map_err(|_| Throw::type_error("ShadowRealm callable metadata failed"))?;
        Ok(wrapped)
    }

    // Shadow contexts have no caller Function or ScriptOrModule. Moving the
    // suspended frames preserves their real strong slots and restores them on
    // every language completion; physical call admission depth is unchanged.
    fn shadow_context<T>(&mut self, operation: impl FnOnce(&mut Self) -> Result<T, Throw>) -> Result<T, Throw> {
        let frames = std::mem::take(&mut self.frames);
        let grammar = self.eval_grammar_override.take();
        // Host module referrers belong to the actual receiving realm. A
        // null ScriptOrModule must not erase that realm's saved locator.
        let source = self.active_source_override.replace(environment_source_context(&self.global_env));
        let result = operation(self);
        self.frames = frames;
        self.eval_grammar_override = grammar;
        self.active_source_override = source;
        result
    }

    fn shadow_eval_program(&mut self, program: &ast::Program) -> Result<JsValue, Throw> {
        let strict = has_use_strict_directive(&program.body);
        let env = EnvHandle::new(if strict { EnvKind::Var } else { EnvKind::Block }, Some(self.global_env.global_scope()));
        // EvalDeclarationInstantiation is the sole lexical binding producer.
        // Reserving these names here would duplicate its complete lexical plan.
        let old_env = self.direct_eval_env.replace(env);
        let old_eval = std::mem::replace(&mut self.in_eval, true);
        let old_strict = std::mem::replace(&mut self.strict_eval_context, strict);
        let old_direct = std::mem::replace(&mut self.direct_eval_call, false);
        let old_grammar = self.eval_grammar_override.replace((self.frames.len(), EvalGrammar::default()));
        let old_completion = self.completion.clone();
        // This executes the actual eval declaration path, not Script GDI.
        // Whole qualification requires the genuine EDI/Catch foundation.
        self.completion = None;
        let environment = self.direct_eval_env.take().expect("fresh ShadowRealm environment");
        let result = (|| {
            self.instantiate_eval_declarations(&program.body, &environment)?;
            self.in_eval = false;
            match self.exec_stmts(&program.body, &environment)? {
                Flow::Normal => Ok(self.completion.clone().unwrap_or(JsValue::Undefined)),
                _ => Err(Throw::type_error("invalid ShadowRealm completion")),
            }
        })();
        self.direct_eval_env = old_env;
        self.in_eval = old_eval;
        self.strict_eval_context = old_strict;
        self.direct_eval_call = old_direct;
        self.eval_grammar_override = old_grammar;
        self.completion = old_completion;
        result
    }
}

fn native_shadow_constructor(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if i.current_new_target().is_none() { return Err(Throw::type_error("ShadowRealm requires new")); }
    let JsValue::Object(instance) = this else { return Err(Throw::type_error("missing ShadowRealm allocation")); };
    let referrer = i.active_source_context();
    // Leave the fresh ordinary global's prototype unset: the actual child
    // intrinsic builder supplies its Object.prototype. Using object_value()
    // here would retain the creator's mutable Object.prototype across realms.
    let realm = i.binding_create_realm(JsValue::object(JsObject::ordinary()))?;
    let state = i.realms.get_mut(&realm.realm).expect("fresh registered realm");
    if let Some(referrer) = referrer { state.global_env = state.global_env.with_source_context(&referrer); }
    // Core policy exposes fresh ECMAScript intrinsics and its own job helpers,
    // not browser Window/storage/network bindings. No author traps are invoked.
    for name in ["window", "location", "history", "localStorage", "sessionStorage",
        "fetch", "setTimeout", "clearTimeout", "setInterval", "clearInterval",
        "requestAnimationFrame", "cancelAnimationFrame"] {
        state.global_obj.borrow_mut().delete_own(name);
        state.global_env.remove_object_binding(name);
    }
    // Realm creation installs actual fresh intrinsics without exposing another
    // interpreter or sharing caller global bindings. No new script is evaluated.
    instance.borrow_mut().exotic = Exotic::ShadowRealm(realm);
    Ok(JsValue::Object(instance))
}

fn native_shadow_evaluate(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let realm = i.shadow_realm_receiver(&this)?;
    let Some(JsValue::Str(source)) = args.first() else { return Err(Throw::type_error("ShadowRealm.evaluate requires a String")); };
    // The genuine Script goal rejects top-level NewTarget/Super/Return. Syntax
    // failures remain caller SyntaxError; runtime failures become caller TypeError.
    let program = crate::parse_script_utf16(source.units()).map_err(|error| Throw::syntax_error(error.to_string()))?;
    let caller = i.binding_current_realm();
    let result = i.binding_with_realm(realm, |i| i.shadow_context(|i| i.shadow_eval_program(&program)));
    let value = result.map_err(|_| Throw::type_error("ShadowRealm evaluation failed"))?;
    i.shadow_wrapped_value(caller, value)
}

fn native_shadow_wrapped_call(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().and_then(|function| function.home.clone()))
        .and_then(|home| home.borrow().get_own("target")).ok_or_else(|| Throw::type_error("missing wrapped target"))?;
    i.shadow_context(|i| {
    let caller = i.binding_current_realm();
    let target_realm = i.get_function_realm(&target)?;
    let mut wrapped = Vec::with_capacity(args.len());
    for argument in args { wrapped.push(i.shadow_wrapped_value(target_realm, argument)?); }
    let receiver = i.shadow_wrapped_value(target_realm, this)?;
    let result = i.call_value(&target, receiver, wrapped)
        .map_err(|_| Throw::type_error("ShadowRealm wrapped call failed"))?;
    i.shadow_wrapped_value(caller, result)
    })
}

fn native_shadow_import_value(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let realm = i.shadow_realm_receiver(&this)?;
    let specifier = i.to_js_string(args.first().unwrap_or(&JsValue::Undefined))?;
    let Some(JsValue::Str(export_name)) = args.get(1) else { return Err(Throw::type_error("ShadowRealm exportName requires a String")); };
    // Existing loader jobs capture the actual eval realm and adopt pending/TLA
    // promises. Hosts must key their genuine module registry by this realm.
    let inner = i.binding_with_realm(realm, |i| {
        let env = i.global_env.global_scope();
        i.dynamic_import_values(JsValue::Str(specifier), JsValue::Undefined, &env, crate::module::ImportPhase::Evaluation)
    })?;
    let JsValue::Object(inner) = inner else { unreachable!("intrinsic import promise") };
    let fulfilled = Interpreter::native_arity("", native_shadow_export, &i.function_proto, 1);
    if let JsValue::Object(object) = &fulfilled {
        let mut data = JsObject::ordinary(); data.set_own("name", JsValue::Str(export_name.clone()));
        object.borrow_mut().func.as_mut().unwrap().home = Some(Obj(Rc::new(RefCell::new(data))));
    }
    let rejected = Interpreter::native_arity("", native_shadow_import_rejected, &i.function_proto, 1);
    let constructor = i.intrinsic_promise_constructor.clone();
    let capability = new_promise_capability(i, &constructor)?;
    perform_promise_then(i, &inner, Some(fulfilled), Some(rejected), Some(capability.clone()))?;
    Ok(capability_promise(&capability))
}

fn native_shadow_export(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let name = i.native_stack.last().and_then(|object| object.borrow().func.as_ref().and_then(|function| function.home.clone()))
        .and_then(|data| data.borrow().get_own("name")).ok_or_else(|| Throw::type_error("missing export name"))?;
    let JsValue::Str(name) = name else { unreachable!() };
    let Some(JsValue::Object(namespace)) = args.first() else { return Err(Throw::type_error("invalid module namespace")); };
    if i.default_get_own_property(namespace, &PropertyKey::from(&name))?.is_none() { return Err(Throw::type_error("module has no requested export")); }
    let value = i.get_prop(&JsValue::Object(namespace.clone()), &PropertyKey::from(&name))?;
    i.shadow_wrapped_value(i.binding_current_realm(), value)
}

fn native_shadow_import_rejected(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Err(Throw::type_error("ShadowRealm module import failed"))
}

#[cfg(test)]
#[path = "runtime_shadow_realm_tests.rs"]
mod runtime_shadow_realm_tests;

#[cfg(test)]
#[path = "runtime_shadow_context_tests.rs"]
mod runtime_shadow_context_tests;
