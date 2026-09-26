#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowProxyOperation {
    Get, Set, Has, Delete, OwnKeys, GetOwnProperty, DefineOwnProperty,
    GetPrototype, SetPrototype, IsExtensible, PreventExtensions,
}

#[derive(Clone, Debug)]
pub struct WindowProxyAccess {
    pub current: RealmHandle,
    pub entry: RealmHandle,
    pub incumbent: RealmHandle,
    pub target: RealmHandle,
    pub target_origin: String,
    pub operation: WindowProxyOperation,
    pub key: Option<JsValue>,
}

pub type WindowProxyAccessHook = dyn Fn(&mut Interpreter, WindowProxyAccess) -> Result<(), Throw>;

#[derive(Clone, Debug)]
pub struct WindowProxyState { target: RealmHandle }

impl Interpreter {
    pub fn binding_set_window_proxy_access_hook(&mut self, hook: Rc<WindowProxyAccessHook>) {
        self.window_access = Some(hook);
    }

    pub fn binding_set_realm_origin(&mut self, realm: RealmHandle, origin: &str) -> Result<(), Throw> {
        self.binding_with_realm(realm, |i| { i.realm_origin = origin.to_owned(); Ok(()) })
    }

    /// Create the realm's stable global-this proxy before installing Window
    /// bindings or running page code. Host Window getters use binding_global_this.
    pub fn binding_create_window_proxy(&mut self, realm: RealmHandle) -> Result<JsValue, Throw> {
        self.binding_with_realm(realm, |i| {
            if let Some(proxy) = &i.window_proxy { return Ok(JsValue::Object(proxy.clone())); }
            let mut object = JsObject::ordinary();
            object.exotic = Exotic::WindowProxy(Rc::new(RefCell::new(WindowProxyState { target: realm })));
            let proxy = Obj(Rc::new(RefCell::new(object)));
            i.install_window_proxy(proxy.clone());
            Ok(JsValue::Object(proxy))
        })
    }

    fn install_window_proxy(&mut self, proxy: Obj) {
        self.window_proxy = Some(proxy.clone());
        let value = JsValue::Object(proxy);
        self.global_env.bind_this(value.clone());
        for name in ["globalThis", "window", "self"] { self.define_global(name, value.clone()); }
    }

    /// Navigate an existing proxy to a prepared same-agent Window realm.
    /// References to the proxy keep identity; old realm objects remain usable.
    pub fn binding_retarget_window_proxy(&mut self, proxy: &JsValue, realm: RealmHandle) -> Result<(), Throw> {
        self.check_realm(realm)?;
        let JsValue::Object(proxy) = proxy else { return Err(Throw::type_error("expected WindowProxy")); };
        let state = match &proxy.borrow().exotic {
            Exotic::WindowProxy(state) => state.clone(),
            _ => return Err(Throw::type_error("expected WindowProxy")),
        };
        self.check_realm(state.borrow().target)?;
        self.binding_with_realm(realm, |i| {
            if i.window_proxy.as_ref().is_some_and(|existing| !Rc::ptr_eq(&existing.0, &proxy.0)) {
                return Err(Throw::type_error("target realm already has a WindowProxy"));
            }
            i.install_window_proxy(proxy.clone());
            Ok(())
        })?;
        state.borrow_mut().target = realm;
        Ok(())
    }

    fn window_proxy_target(&mut self, object: &Obj, operation: WindowProxyOperation,
        key: Option<&PropertyKey>) -> Option<Result<(RealmHandle, Obj), Throw>> {
        let state = match &object.borrow().exotic {
            Exotic::WindowProxy(state) => state.clone(),
            _ => return None,
        };
        Some((|| {
            for _ in 0..16 {
                let target = state.borrow().target;
                self.check_realm(target)?;
                let target_origin = if target.realm == self.host_realm { self.realm_origin.clone() }
                    else { self.realms[&target.realm].realm_origin.clone() };
                let access = WindowProxyAccess {
                    current: self.binding_current_realm(), entry: self.binding_entry_realm(),
                    incumbent: self.binding_incumbent_realm(), target, target_origin: target_origin.clone(), operation,
                    key: key.map(|key| match key { PropertyKey::String(s) => JsValue::Str(s.clone()),
                        PropertyKey::Symbol(s) => JsValue::Symbol(s.clone()) }),
                };
                if let Some(hook) = self.window_access.clone() { hook(self, access)?; }
                else if self.realm_origin != target_origin {
                    return Err(Throw::type_error("cross-origin WindowProxy access requires host permission"));
                }
                // A host hook may navigate the proxy. Repeat the operation against
                // the now-current target, with a fresh policy check if it changed.
                if state.borrow().target != target {
                    continue;
                }
                let JsValue::Object(global) = self.binding_realm_global(target)? else { unreachable!() };
                return Ok((target, global));
            }
            Err(Throw::type_error("WindowProxy target keeps changing during access"))
        })())
    }

    fn window_proxy_define(&mut self, object: &Obj, key: &PropertyKey, descriptor: &JsValue)
        -> Option<Result<bool, Throw>> {
        self.window_proxy_target(object, WindowProxyOperation::DefineOwnProperty, Some(key))
            .map(|target| target.and_then(|(realm, global)| self.binding_with_realm(realm,
                |i| if array_index_key(key).is_some() { Ok(false) }
                    else { i.define_own_property(&global, key, descriptor) })))
    }

    fn window_proxy_set(&mut self, object: &Obj, key: &PropertyKey, value: JsValue, receiver: JsValue)
        -> Option<Result<bool, Throw>> {
        self.window_proxy_target(object, WindowProxyOperation::Set, Some(key))
            .map(|target| target.and_then(|(realm, global)| self.binding_with_realm(realm,
                |i| if array_index_key(key).is_some() { Ok(false) }
                    else { i.set_with_receiver(&global, key, value, receiver) })))
    }
}

impl Interpreter {
    fn window_proxy_own_property(&mut self, object: &Obj, key: &PropertyKey)
        -> Option<Result<Option<Property>, Throw>> {
        self.window_proxy_target(object, WindowProxyOperation::GetOwnProperty, Some(key))
            .map(|target| target.and_then(|(realm, global)| self.binding_with_realm(realm, |i| {
                match key { PropertyKey::String(name) => i.legacy_own_property(&global, name, false),
                    PropertyKey::Symbol(_) => Ok(global.borrow().own_prop(key).cloned()) }
            })))
    }

    fn window_proxy_has(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        self.window_proxy_target(object, WindowProxyOperation::Has, Some(key))
            .map(|target| target.and_then(|(realm, global)| self.binding_with_realm(realm, |i| {
                match key { PropertyKey::String(name) => i.legacy_has_property(&global, name),
                    PropertyKey::Symbol(_) => Ok(global.borrow().has_property(key)) }
            })))
    }

    fn window_proxy_delete(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        self.window_proxy_target(object, WindowProxyOperation::Delete, Some(key))
            .map(|target| target.and_then(|(realm, global)| self.binding_with_realm(realm, |i| {
                if array_index_key(key).is_some() {
                    return Ok(match key.as_string() {
                        Some(name) => i.legacy_own_property(&global, name, false)?.is_none(),
                        None => true,
                    });
                }
                if let Some(name) = key.as_string() {
                    if let Some(deleted) = i.legacy_delete(&global, name)? { return Ok(deleted); }
                }
                let deleted = global.borrow_mut().delete_own(key);
                Ok(deleted)
            })))
    }
}

impl Interpreter {
    fn is_window_proxy(&self, object: &Obj) -> bool {
        matches!(object.borrow().exotic, Exotic::WindowProxy(_))
    }

    fn window_proxy_prototype(&mut self, object: &Obj) -> Option<Result<JsValue, Throw>> {
        let target = match &object.borrow().exotic {
            Exotic::WindowProxy(state) => state.borrow().target,
            _ => return None,
        };
        if let Err(error) = self.check_realm(target) { return Some(Err(error)); }
        let origin = if target.realm == self.host_realm { &self.realm_origin }
            else { &self.realms[&target.realm].realm_origin };
        if self.window_access.is_none() && origin != &self.realm_origin {
            return Some(Ok(JsValue::Null));
        }
        self.window_proxy_target(object, WindowProxyOperation::GetPrototype, None)
            .map(|target| target.map(|(_, global)| {
                let prototype = global.borrow().proto.clone();
                prototype.map(JsValue::Object).unwrap_or(JsValue::Null)
            }))
    }

    fn window_proxy_set_prototype(&mut self, object: &Obj, prototype: &Option<Obj>)
        -> Option<Result<bool, Throw>> {
        self.window_proxy_prototype(object).map(|result| result.map(|current|
            current == prototype.clone().map(JsValue::Object).unwrap_or(JsValue::Null)))
    }

    fn window_proxy_descriptor(&mut self, object: &Obj, key: &PropertyKey)
        -> Option<Result<JsValue, Throw>> {
        self.window_proxy_own_property(object, key).map(|result| result.map(|property| {
            let Some(property) = property else { return JsValue::Undefined; };
            let mut descriptor = JsObject::ordinary();
            descriptor.proto = Some(self.object_proto.clone());
            if property.getter.is_some() || property.setter.is_some() {
                descriptor.set_own("get", property.getter.unwrap_or(JsValue::Undefined));
                descriptor.set_own("set", property.setter.unwrap_or(JsValue::Undefined));
            } else {
                descriptor.set_own("value", property.value);
                descriptor.set_own("writable", JsValue::Bool(property.writable));
            }
            descriptor.set_own("enumerable", JsValue::Bool(property.enumerable));
            descriptor.set_own("configurable", JsValue::Bool(property.configurable));
            JsValue::object(descriptor)
        }))
    }
}
