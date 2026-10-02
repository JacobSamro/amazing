// Proxy exotic objects (ECMA-262 10.5) and their trap dispatch.
//
// Every fundamental object operation checks proxy_* first, mirroring the
// WindowProxy hook pattern in interp_window_proxy.rs: `None` means "not a
// Proxy, proceed ordinarily"; `Some(result)` means "handled here".
//
// A trap that is absent (undefined/null on the handler) falls back to the
// target's own default behavior. Fallbacks for [[Get]]/[[Set]]/[[OwnKeys]]/
// [[DefineOwnProperty]] just re-invoke the shared gateway (get_prop_receiver,
// set_with_receiver, own_property_keys, define_own_property) on the target,
// which already checks proxy_* first, so a proxy wrapping another proxy
// recurses correctly. The remaining fundamental operations ([[HasProperty]],
// [[Delete]], [[GetOwnProperty]], [[GetPrototypeOf]], [[SetPrototypeOf]],
// [[IsExtensible]], [[PreventExtensions]]) have no such shared gateway in
// this interpreter; their default_* helpers below duplicate the small amount
// of ordinary-object logic already inlined at each existing call site
// (binary index fast path, then legacy platform hooks, then plain storage),
// and are themselves proxy-aware so nested proxies keep working.
//
// Known gaps, accepted for this ticket: invariant enforcement covers the
// highest-value checks per trap (revocation, non-callable traps, the
// non-configurable/non-extensible-target agreements) but not the complete
// ECMA-262 invariant list -- e.g. the ownKeys "extensible target must report
// every non-configurable target key" superset rule is not enforced.
// Object.freeze/seal/isFrozen/isSealed treat a Proxy's own extensibility
// through the isExtensible/preventExtensions traps but do not route each
// property through the target's defineProperty trap. A Proxy inserted
// mid-prototype-chain is only honored at the well-known entry points below
// (Object/Reflect getPrototypeOf/setPrototypeOf, `in`, property access);
// a handful of raw `.proto` field walks elsewhere do not call through it.

#[derive(Debug)]
pub struct ProxyState {
    target: RefCell<JsValue>,
    handler: RefCell<JsValue>,
    revoked: Cell<bool>,
}

fn property_key_to_value(key: &PropertyKey) -> JsValue {
    match key {
        PropertyKey::String(s) => JsValue::Str(s.clone()),
        PropertyKey::Symbol(s) => JsValue::Symbol(s.clone()),
    }
}

fn complete_property_descriptor(fields: StdDescriptorFields) -> Property {
    if fields.get.is_some() || fields.set.is_some() {
        Property {
            value: JsValue::Undefined,
            writable: false,
            getter: fields.get.filter(|v| !v.is_undefined()),
            setter: fields.set.filter(|v| !v.is_undefined()),
            enumerable: fields.enumerable.unwrap_or(false),
            configurable: fields.configurable.unwrap_or(false),
        }
    } else {
        Property {
            value: fields.value.unwrap_or(JsValue::Undefined),
            writable: fields.writable.unwrap_or(false),
            getter: None,
            setter: None,
            enumerable: fields.enumerable.unwrap_or(false),
            configurable: fields.configurable.unwrap_or(false),
        }
    }
}

fn proxy_unreachable_call(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    // call_value_with_new_target_inner / construct_with_new_target check
    // Exotic::Proxy and dispatch to proxy_call/proxy_construct before ever
    // reaching a function body. This exists only so a Proxy over a callable
    // target reports func.is_some() (typeof/IsCallable/IsConstructor read
    // that field directly), never so it can actually run.
    Err(Throw::type_error("unreachable: proxy call/construct must be intercepted before dispatch"))
}

fn proxy_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Proxy requires new")); }
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let handler = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !target.is_object() || !handler.is_object() {
        return Err(Throw::type_error("Proxy target and handler must be objects"));
    }
    Ok(JsValue::Object(i.new_proxy_object(target, handler)))
}

fn proxy_revoke_call(i: &mut Interpreter, _this: JsValue, _args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if let Some(object) = i.native_stack.last().cloned() {
        let state = object.borrow().proxy_revoke.clone();
        if let Some(state) = state {
            state.revoked.set(true);
            *state.target.borrow_mut() = JsValue::Undefined;
            *state.handler.borrow_mut() = JsValue::Undefined;
        }
    }
    Ok(JsValue::Undefined)
}

fn proxy_revocable(i: &mut Interpreter, _this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = args.first().cloned().unwrap_or(JsValue::Undefined);
    let handler = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if !target.is_object() || !handler.is_object() {
        return Err(Throw::type_error("Proxy target and handler must be objects"));
    }
    let proxy = i.new_proxy_object(target, handler);
    let state = match &proxy.borrow().exotic {
        Exotic::Proxy(state) => state.clone(),
        _ => unreachable!("new_proxy_object always sets Exotic::Proxy"),
    };
    let revoke = i.new_proxy_revoke_function(state);
    let mut result = JsObject::ordinary();
    result.proto = Some(i.object_proto.clone());
    result.set_own("proxy", JsValue::Object(proxy));
    result.set_own("revoke", revoke);
    Ok(JsValue::object(result))
}

impl Interpreter {
    fn install_proxy(&mut self) {
        let constructor = Self::native_constructor_arity("Proxy", proxy_constructor, &self.function_proto, 2);
        if let JsValue::Object(object) = &constructor {
            // Proxy has no own "prototype" property, unlike ordinary
            // constructors (ECMA-262 has no Proxy.prototype).
            object.borrow_mut().delete_own("prototype");
        }
        self.define_global("Proxy", constructor.clone());
        self.global_obj.borrow_mut().define("Proxy", symbol_data(constructor.clone(), true, true)).unwrap();
        if let JsValue::Object(object) = &constructor {
            let revocable = symbol_method(self, "revocable", proxy_revocable as crate::value::NativeFn);
            symbol_method_length(&revocable, 2);
            object.borrow_mut().define("revocable", symbol_data(revocable, true, true)).unwrap();
        }
    }

    fn new_proxy_object(&self, target: JsValue, handler: JsValue) -> Obj {
        let state = Rc::new(ProxyState {
            target: RefCell::new(target.clone()),
            handler: RefCell::new(handler),
            revoked: Cell::new(false),
        });
        let mut object = JsObject::ordinary();
        object.realm = self.object_proto.borrow().realm;
        object.proto = Some(self.object_proto.clone());
        let (callable, constructible) = match &target {
            JsValue::Object(target_object) => {
                let borrowed = target_object.borrow();
                (borrowed.func.is_some(), borrowed.func.is_some() && borrowed.constructible)
            }
            _ => (false, false),
        };
        object.constructible = constructible;
        if callable {
            object.func = Some(JsFunction {
                name: "".into(), params: Vec::new(), body: FunctionBody::Native(proxy_unreachable_call),
                is_arrow: false, is_async: false, is_generator: false,
                env: EnvHandle::new(EnvKind::Global, None), home: None, proto_for_new: None,
            });
        }
        object.exotic = Exotic::Proxy(state);
        Obj(Rc::new(RefCell::new(object)))
    }

    fn new_proxy_revoke_function(&self, state: Rc<ProxyState>) -> JsValue {
        let mut object = JsObject::function(JsFunction {
            name: "".into(), params: Vec::new(), body: FunctionBody::Native(proxy_revoke_call),
            is_arrow: false, is_async: false, is_generator: false,
            env: EnvHandle::new(EnvKind::Global, None), home: None, proto_for_new: None,
        });
        object.proto = Some(self.function_proto.clone());
        object.constructible = false;
        object.proxy_revoke = Some(state);
        JsValue::Object(Obj(Rc::new(RefCell::new(object))))
    }

    // Shared plumbing -----------------------------------------------------

    fn proxy_parts(&self, object: &Obj) -> Option<Result<(JsValue, JsValue), Throw>> {
        let state = match &object.borrow().exotic {
            Exotic::Proxy(state) => state.clone(),
            _ => return None,
        };
        if state.revoked.get() {
            return Some(Err(Throw::type_error("cannot perform operation on a revoked proxy")));
        }
        Some(Ok((state.target.borrow().clone(), state.handler.borrow().clone())))
    }

    fn proxy_target_object(&self, target: &JsValue) -> Result<Obj, Throw> {
        match target {
            JsValue::Object(object) => Ok(object.clone()),
            _ => Err(Throw::type_error("proxy target must be an object")),
        }
    }

    fn proxy_trap(&mut self, handler: &JsValue, name: &str) -> Result<Option<JsValue>, Throw> {
        let trap = self.get_prop(handler, name)?;
        if trap.is_undefined() || trap.is_null() { return Ok(None); }
        if callable_function(&trap).is_none() {
            return Err(Throw::type_error("proxy trap is not callable"));
        }
        Ok(Some(trap))
    }

    fn descriptor_fields_to_value(&self, fields: &StdDescriptorFields) -> JsValue {
        let mut object = JsObject::ordinary();
        object.proto = Some(self.object_proto.clone());
        if let Some(value) = &fields.value { object.set_own("value", value.clone()); }
        if let Some(writable) = fields.writable { object.set_own("writable", JsValue::Bool(writable)); }
        if let Some(get) = &fields.get { object.set_own("get", get.clone()); }
        if let Some(set) = &fields.set { object.set_own("set", set.clone()); }
        if let Some(enumerable) = fields.enumerable { object.set_own("enumerable", JsValue::Bool(enumerable)); }
        if let Some(configurable) = fields.configurable { object.set_own("configurable", JsValue::Bool(configurable)); }
        JsValue::object(object)
    }

    // Ordinary-object fallbacks, themselves proxy-aware -------------------

    fn default_get_own_property(&mut self, target: &Obj, key: &PropertyKey) -> Result<Option<Property>, Throw> {
        if let Some(result) = self.proxy_get_own_property(target, key) { return result; }
        if let Some(result) = self.window_proxy_own_property(target, key) { return result; }
        if let Some(descriptor) = binary_index_descriptor(target, key) { return Ok(descriptor); }
        match key.as_string() {
            Some(name) => self.legacy_own_property(target, name.clone(), false),
            None => Ok(target.borrow().own_prop(key).cloned()),
        }
    }

    fn default_has_property(&mut self, target: &Obj, key: &PropertyKey) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_has(target, key) { return result; }
        if let Some(result) = self.window_proxy_has(target, key) { return result; }
        if let Some(descriptor) = binary_index_descriptor(target, key) { return Ok(descriptor.is_some()); }
        match key.as_string() {
            Some(name) => self.legacy_has_property(target, name.clone()),
            None => Ok(target.borrow().has_property(key)),
        }
    }

    fn default_delete_property(&mut self, target: &Obj, key: &PropertyKey) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_delete(target, key) { return result; }
        if let Some(result) = self.window_proxy_delete(target, key) { return result; }
        if let Some(deleted) = binary_delete_index(target, key) { return Ok(deleted); }
        if let Some(name) = key.as_string() {
            if let Some(deleted) = self.legacy_delete(target, name.clone())? { return Ok(deleted); }
        }
        Ok(target.borrow_mut().delete_own(key))
    }

    fn default_own_property_keys(&mut self, target: &Obj) -> Result<Vec<PropertyKey>, Throw> {
        self.own_property_keys(&JsValue::Object(target.clone()))
    }

    pub(super) fn default_get_prototype(&mut self, target: &Obj) -> Result<JsValue, Throw> {
        if let Some(result) = self.proxy_get_prototype(target) { return result; }
        if let Some(result) = self.window_proxy_prototype(target) { return result; }
        Ok(target.borrow().proto.clone().map(JsValue::Object).unwrap_or(JsValue::Null))
    }

    pub(super) fn default_set_prototype(&mut self, target: &Obj, prototype: &Option<Obj>) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_set_prototype(target, prototype) { return result; }
        if let Some(result) = self.window_proxy_set_prototype(target, prototype) { return result; }
        let current = target.borrow().proto.clone();
        if current.as_ref().zip(prototype.as_ref()).is_some_and(|(a, b)| Rc::ptr_eq(&a.0, &b.0))
            || (current.is_none() && prototype.is_none()) { return Ok(true); }
        if !target.borrow().extensible { return Ok(false); }
        let mut cursor = prototype.clone();
        while let Some(candidate) = cursor {
            if Rc::ptr_eq(&candidate.0, &target.0) { return Ok(false); }
            cursor = if let Some(result) = self.proxy_get_prototype(&candidate) {
                match result? { JsValue::Object(p) => Some(p), _ => None }
            } else { candidate.borrow().proto.clone() };
        }
        target.borrow_mut().proto = prototype.clone();
        Ok(true)
    }

    pub(super) fn default_is_extensible(&mut self, target: &Obj) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_is_extensible(target) { return result; }
        if self.is_window_proxy(target) { return Ok(true); }
        if self.legacy_for(target).is_some() { return Ok(true); }
        Ok(target.borrow().extensible)
    }

    pub(super) fn default_prevent_extensions(&mut self, target: &Obj) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_prevent_extensions(target) { return result; }
        if self.is_window_proxy(target) { return Ok(false); }
        if self.legacy_for(target).is_some() { return Ok(false); }
        target.borrow_mut().extensible = false;
        Ok(true)
    }

    // Trap dispatch ---------------------------------------------------------

    fn proxy_get(&mut self, object: &Obj, key: &PropertyKey, receiver: JsValue) -> Option<Result<JsValue, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<JsValue, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "get")? else {
                return self.get_prop_receiver(&target, key, receiver);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value, receiver])?;
            if let Some(existing) = self.default_get_own_property(&target_obj, key)? {
                if !existing.configurable {
                    if existing.getter.is_none() && existing.setter.is_none() && !existing.writable
                        && !std_same_value(&result, &existing.value) {
                        return Err(Throw::type_error("proxy get invariant violated: non-configurable, non-writable target property value mismatch"));
                    }
                    if existing.setter.is_some() && existing.getter.is_none() && !result.is_undefined() {
                        return Err(Throw::type_error("proxy get invariant violated: non-configurable accessor with no getter must report undefined"));
                    }
                }
            }
            Ok(result)
        })())
    }

    fn proxy_set(&mut self, object: &Obj, key: &PropertyKey, value: JsValue, receiver: JsValue) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "set")? else {
                return self.set_with_receiver(&target_obj, key, value, receiver);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value, value.clone(), receiver])?.to_boolean();
            if result {
                if let Some(existing) = self.default_get_own_property(&target_obj, key)? {
                    if !existing.configurable {
                        if existing.getter.is_none() && existing.setter.is_none() && !existing.writable
                            && !std_same_value(&value, &existing.value) {
                            return Err(Throw::type_error("proxy set invariant violated: non-configurable, non-writable target property value mismatch"));
                        }
                        if existing.getter.is_some() && existing.setter.is_none() {
                            return Err(Throw::type_error("proxy set invariant violated: non-configurable accessor with no setter cannot be set"));
                        }
                    }
                }
            }
            Ok(result)
        })())
    }

    fn proxy_has(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "has")? else {
                return self.default_has_property(&target_obj, key);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value])?.to_boolean();
            if !result {
                if let Some(existing) = self.default_get_own_property(&target_obj, key)? {
                    if !existing.configurable {
                        return Err(Throw::type_error("proxy has invariant violated: non-configurable target property reported absent"));
                    }
                    if !self.default_is_extensible(&target_obj)? {
                        return Err(Throw::type_error("proxy has invariant violated: non-extensible target property reported absent"));
                    }
                }
            }
            Ok(result)
        })())
    }

    fn proxy_delete(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "deleteProperty")? else {
                return self.default_delete_property(&target_obj, key);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value])?.to_boolean();
            if result {
                if let Some(existing) = self.default_get_own_property(&target_obj, key)? {
                    if !existing.configurable {
                        return Err(Throw::type_error("proxy deleteProperty invariant violated: cannot delete a non-configurable target property"));
                    }
                    if !self.default_is_extensible(&target_obj)? {
                        return Err(Throw::type_error("proxy deleteProperty invariant violated: cannot delete a property on a non-extensible target"));
                    }
                }
            }
            Ok(result)
        })())
    }

    fn proxy_own_keys(&mut self, object: &Obj) -> Option<Result<Vec<PropertyKey>, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<Vec<PropertyKey>, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "ownKeys")? else {
                return self.default_own_property_keys(&target_obj);
            };
            let result = self.call_value(&trap, handler.clone(), vec![target.clone()])?;
            let values = self.create_list_from_array_like(&result)?;
            let mut seen = HashSet::new();
            let mut keys = Vec::with_capacity(values.len());
            for value in values {
                let key = match &value {
                    JsValue::Str(s) => PropertyKey::String(s.clone()),
                    JsValue::Symbol(s) => PropertyKey::Symbol(s.clone()),
                    _ => return Err(Throw::type_error("proxy ownKeys trap result must contain only strings and symbols")),
                };
                if !seen.insert(key.clone()) {
                    return Err(Throw::type_error("proxy ownKeys trap result must not contain duplicate keys"));
                }
                keys.push(key);
            }
            Ok(keys)
        })())
    }

    fn proxy_get_own_property(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<Option<Property>, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<Option<Property>, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "getOwnPropertyDescriptor")? else {
                return self.default_get_own_property(&target_obj, key);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value])?;
            let target_desc = self.default_get_own_property(&target_obj, key)?;
            if result.is_undefined() {
                if let Some(existing) = &target_desc {
                    if !existing.configurable {
                        return Err(Throw::type_error("proxy getOwnPropertyDescriptor invariant violated: non-configurable target property reported missing"));
                    }
                    if !self.default_is_extensible(&target_obj)? {
                        return Err(Throw::type_error("proxy getOwnPropertyDescriptor invariant violated: non-extensible target reported property missing"));
                    }
                }
                return Ok(None);
            }
            if !result.is_object() {
                return Err(Throw::type_error("proxy getOwnPropertyDescriptor trap must return an object or undefined"));
            }
            let fields = self.std_descriptor_fields(&result)?;
            let extensible_target = self.default_is_extensible(&target_obj)?;
            let property = complete_property_descriptor(fields);
            match &target_desc {
                Some(existing) if !existing.configurable && property.configurable => {
                    return Err(Throw::type_error("proxy getOwnPropertyDescriptor invariant violated: cannot report a non-configurable target property as configurable"));
                }
                None if !extensible_target => {
                    return Err(Throw::type_error("proxy getOwnPropertyDescriptor invariant violated: cannot report a new property on a non-extensible target"));
                }
                _ => {}
            }
            Ok(Some(property))
        })())
    }

    fn proxy_define(&mut self, object: &Obj, key: &PropertyKey, descriptor: &JsValue) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "defineProperty")? else {
                return self.define_own_property(&target_obj, key, descriptor);
            };
            let fields = self.std_descriptor_fields(descriptor)?;
            let normalized = self.descriptor_fields_to_value(&fields);
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value, normalized])?;
            Ok(result.to_boolean())
        })())
    }

    fn proxy_get_prototype(&mut self, object: &Obj) -> Option<Result<JsValue, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<JsValue, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "getPrototypeOf")? else {
                return self.default_get_prototype(&target_obj);
            };
            let result = self.call_value(&trap, handler.clone(), vec![target.clone()])?;
            if !matches!(result, JsValue::Object(_) | JsValue::Null) {
                return Err(Throw::type_error("proxy getPrototypeOf trap must return an object or null"));
            }
            if !self.default_is_extensible(&target_obj)? {
                let expected = self.default_get_prototype(&target_obj)?;
                if !std_same_value(&result, &expected) {
                    return Err(Throw::type_error("proxy getPrototypeOf invariant violated: must match a non-extensible target's prototype"));
                }
            }
            Ok(result)
        })())
    }

    fn proxy_set_prototype(&mut self, object: &Obj, prototype: &Option<Obj>) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "setPrototypeOf")? else {
                return self.default_set_prototype(&target_obj, prototype);
            };
            let prototype_value = prototype.clone().map(JsValue::Object).unwrap_or(JsValue::Null);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), prototype_value.clone()])?.to_boolean();
            if result { return Ok(true); }
            if !self.default_is_extensible(&target_obj)? {
                let expected = self.default_get_prototype(&target_obj)?;
                if !std_same_value(&prototype_value, &expected) {
                    return Err(Throw::type_error("proxy setPrototypeOf invariant violated: must match a non-extensible target's prototype"));
                }
            }
            Ok(false)
        })())
    }

    fn proxy_is_extensible(&mut self, object: &Obj) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "isExtensible")? else {
                return self.default_is_extensible(&target_obj);
            };
            let result = self.call_value(&trap, handler.clone(), vec![target.clone()])?.to_boolean();
            let target_extensible = self.default_is_extensible(&target_obj)?;
            if result != target_extensible {
                return Err(Throw::type_error("proxy isExtensible invariant violated: result must match the target"));
            }
            Ok(result)
        })())
    }

    fn proxy_prevent_extensions(&mut self, object: &Obj) -> Option<Result<bool, Throw>> {
        let (target, handler) = match self.proxy_parts(object)? { Ok(v) => v, Err(e) => return Some(Err(e)) };
        Some((|| -> Result<bool, Throw> {
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "preventExtensions")? else {
                return self.default_prevent_extensions(&target_obj);
            };
            let result = self.call_value(&trap, handler.clone(), vec![target.clone()])?.to_boolean();
            if result && self.default_is_extensible(&target_obj)? {
                return Err(Throw::type_error("proxy preventExtensions invariant violated: target is still extensible"));
            }
            Ok(result)
        })())
    }

    // [[Call]] / [[Construct]] -- invoked directly from
    // call_value_with_new_target_inner / construct_with_new_target, which
    // check Exotic::Proxy before doing anything callable-specific.

    fn proxy_call(&mut self, object: &Obj, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        let (target, handler) = self.proxy_parts(object).expect("caller checked Exotic::Proxy")?;
        let trap = self.proxy_trap(&handler, "apply")?;
        match trap {
            Some(trap) => {
                let arguments = array_values(self, args)?;
                self.call_value(&trap, handler, vec![target, this, arguments])
            }
            None => self.call_value(&target, this, args),
        }
    }

    fn proxy_construct(&mut self, object: &Obj, args: Vec<JsValue>, new_target: JsValue) -> Result<JsValue, Throw> {
        let (target, handler) = self.proxy_parts(object).expect("caller checked Exotic::Proxy")?;
        if !matches!(&target, JsValue::Object(o) if o.borrow().func.is_some() && o.borrow().constructible) {
            return Err(Throw::type_error("proxy target is not a constructor"));
        }
        let trap = self.proxy_trap(&handler, "construct")?;
        match trap {
            Some(trap) => {
                let arguments = array_values(self, args)?;
                let result = self.call_value(&trap, handler, vec![target, arguments, new_target])?;
                if !result.is_object() {
                    return Err(Throw::type_error("proxy construct trap must return an object"));
                }
                Ok(result)
            }
            None => self.construct_with_new_target(&target, args, new_target),
        }
    }
}
