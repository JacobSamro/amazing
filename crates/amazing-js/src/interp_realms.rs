// Realm state is swapped only on cross-realm entries. Agent queues, tokens,
// timers and the symbol registry deliberately remain on Interpreter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RealmHandle { agent: u64, realm: u64 }

struct RealmState {
    intrinsic_constructors: HashMap<String, (Obj, Obj)>,
    global_env: EnvHandle,
    global_obj: Obj,
    object_proto: Obj,
    function_proto: Obj,
    async_function_proto: Obj,
    generator_function_proto: Obj,
    async_generator_function_proto: Obj,
    generator_proto: Obj,
    async_generator_proto: Obj,
    array_proto: Obj,
    original_array_iterator: JsValue,
    string_proto: Obj,
    number_proto: Obj,
    boolean_proto: Obj,
    bigint_proto: Obj,
    symbol_proto: Obj,
    map_proto: Obj,
    set_proto: Obj,
    date_proto: Obj,
    promise_proto: Obj,
    intrinsic_promise_constructor: JsValue,
    regexp_proto: Obj,
    host_native_dispatch: Option<Rc<HostNativeDispatch>>,
    host_realm: u64,
    realm_origin: String,
    window_proxy: Option<Obj>,
    host_cache: HashMap<HostHandle, Weak<RefCell<JsObject>>>,
    put_forwards: Vec<(Weak<RefCell<JsObject>>, String, String)>,
    legacy_hooks: HashMap<u64, LegacyPlatformHooks>,
    legacy_iterator_proto: Obj,
    pair_hooks: HashMap<u64, PairIterableHooks>,
    pair_iterator_proto: Obj,
    regexp_string_iterator_proto: Obj,
    dom_host: Option<Rc<RefCell<dyn DomHost>>>,
    dom_listeners: Vec<DomListener>,
    location_url: String,
    pending_navigation: Option<String>,
    pending_history_delta: Option<i32>,
    pending_history_update: Option<(String, bool, JsValue)>,
    history_state: JsValue,
    last_event_default_prevented: bool,
}

impl RealmState {
    fn capture(i: &Interpreter) -> Self {
        Self {
            intrinsic_constructors: i.intrinsic_constructors.clone(),
            global_env: i.global_env.clone(),
            global_obj: i.global_obj.clone(),
            object_proto: i.object_proto.clone(),
            function_proto: i.function_proto.clone(),
            async_function_proto: i.async_function_proto.clone(),
            generator_function_proto: i.generator_function_proto.clone(),
            async_generator_function_proto: i.async_generator_function_proto.clone(),
            generator_proto: i.generator_proto.clone(),
            async_generator_proto: i.async_generator_proto.clone(),
            array_proto: i.array_proto.clone(),
            original_array_iterator: i.original_array_iterator.clone(),
            string_proto: i.string_proto.clone(),
            number_proto: i.number_proto.clone(),
            boolean_proto: i.boolean_proto.clone(),
            bigint_proto: i.bigint_proto.clone(),
            symbol_proto: i.symbol_proto.clone(),
            map_proto: i.map_proto.clone(),
            set_proto: i.set_proto.clone(),
            date_proto: i.date_proto.clone(),
            promise_proto: i.promise_proto.clone(),
            intrinsic_promise_constructor: i.intrinsic_promise_constructor.clone(),
            regexp_proto: i.regexp_proto.clone(),
            host_native_dispatch: i.host_native_dispatch.clone(),
            host_realm: i.host_realm.clone(),
            realm_origin: i.realm_origin.clone(),
            window_proxy: i.window_proxy.clone(),
            host_cache: i.host_cache.clone(),
            put_forwards: i.put_forwards.clone(),
            legacy_hooks: i.legacy_hooks.clone(),
            legacy_iterator_proto: i.legacy_iterator_proto.clone(),
            pair_hooks: i.pair_hooks.clone(),
            pair_iterator_proto: i.pair_iterator_proto.clone(),
            regexp_string_iterator_proto: i.regexp_string_iterator_proto.clone(),
            dom_host: i.dom_host.clone(),
            dom_listeners: i.dom_listeners.clone(),
            location_url: i.location_url.clone(),
            pending_navigation: i.pending_navigation.clone(),
            pending_history_delta: i.pending_history_delta.clone(),
            pending_history_update: i.pending_history_update.clone(),
            history_state: i.history_state.clone(),
            last_event_default_prevented: i.last_event_default_prevented.clone(),
        }
    }
    fn restore(self, i: &mut Interpreter) {
        i.intrinsic_constructors = self.intrinsic_constructors;
        i.global_env = self.global_env;
        i.global_obj = self.global_obj;
        i.object_proto = self.object_proto;
        i.function_proto = self.function_proto;
        i.async_function_proto = self.async_function_proto;
        i.generator_function_proto = self.generator_function_proto;
        i.async_generator_function_proto = self.async_generator_function_proto;
        i.generator_proto = self.generator_proto;
        i.async_generator_proto = self.async_generator_proto;
        i.array_proto = self.array_proto;
        i.original_array_iterator = self.original_array_iterator;
        i.string_proto = self.string_proto;
        i.number_proto = self.number_proto;
        i.boolean_proto = self.boolean_proto;
        i.bigint_proto = self.bigint_proto;
        i.symbol_proto = self.symbol_proto;
        i.map_proto = self.map_proto;
        i.set_proto = self.set_proto;
        i.date_proto = self.date_proto;
        i.promise_proto = self.promise_proto;
        i.intrinsic_promise_constructor = self.intrinsic_promise_constructor;
        i.regexp_proto = self.regexp_proto;
        i.host_native_dispatch = self.host_native_dispatch;
        i.host_realm = self.host_realm;
        i.realm_origin = self.realm_origin;
        i.window_proxy = self.window_proxy;
        i.host_cache = self.host_cache;
        i.put_forwards = self.put_forwards;
        i.legacy_hooks = self.legacy_hooks;
        i.legacy_iterator_proto = self.legacy_iterator_proto;
        i.pair_hooks = self.pair_hooks;
        i.pair_iterator_proto = self.pair_iterator_proto;
        i.regexp_string_iterator_proto = self.regexp_string_iterator_proto;
        i.dom_host = self.dom_host;
        i.dom_listeners = self.dom_listeners;
        i.location_url = self.location_url;
        i.pending_navigation = self.pending_navigation;
        i.pending_history_delta = self.pending_history_delta;
        i.pending_history_update = self.pending_history_update;
        i.history_state = self.history_state;
        i.last_event_default_prevented = self.last_event_default_prevented;
    }
}

impl Interpreter {
    pub fn binding_current_realm(&self) -> RealmHandle {
        RealmHandle { agent: self.agent_id, realm: self.host_realm }
    }

    pub fn binding_entry_realm(&self) -> RealmHandle {
        self.entry_realm.unwrap_or_else(|| self.binding_current_realm())
    }

    pub fn binding_incumbent_realm(&self) -> RealmHandle {
        self.realm_callers.last().copied().unwrap_or_else(|| self.binding_entry_realm())
    }

    fn check_realm(&self, realm: RealmHandle) -> Result<(), Throw> {
        if realm.agent != self.agent_id ||
            (realm.realm != self.host_realm && !self.realms.contains_key(&realm.realm)) {
            return Err(Throw::type_error("realm does not belong to this agent"));
        }
        Ok(())
    }

    fn switch_realm(&mut self, realm: RealmHandle) -> Result<(), Throw> {
        self.check_realm(realm)?;
        if realm.realm == self.host_realm { return Ok(()); }
        let next = self.realms.remove(&realm.realm).expect("validated realm");
        let previous = RealmState::capture(self);
        self.realms.insert(previous.host_realm, previous);
        next.restore(self);
        Ok(())
    }

    /// Enter a host-selected realm for installation, evaluation, or dispatch.
    /// Both success and abrupt completion restore the caller's realm.
    pub fn binding_with_realm<T>(
        &mut self, realm: RealmHandle,
        operation: impl FnOnce(&mut Interpreter) -> Result<T, Throw>,
    ) -> Result<T, Throw> {
        let previous = self.binding_current_realm();
        self.switch_realm(realm)?;
        let result = operation(self).map_err(|error| self.normalize_throw(error));
        self.switch_realm(previous).expect("caller realm remains registered");
        result
    }

    pub fn binding_eval_in_realm(&mut self, realm: RealmHandle, source: &str) -> Result<JsValue, Throw> {
        self.binding_with_realm(realm, |i| {
            let strict = std::mem::replace(&mut i.strict_eval_context, false);
            let in_eval = std::mem::replace(&mut i.in_eval, false);
            let direct = std::mem::replace(&mut i.direct_eval_call, false);
            let environment = i.direct_eval_env.take();
            let completion = i.completion.clone();
            let global = i.global_env.global_scope();
            let active_environment = std::mem::replace(&mut i.global_env, global);
            let result = i.run_script(source);
            i.global_env = active_environment;
            i.strict_eval_context = strict;
            i.in_eval = in_eval;
            i.direct_eval_call = direct;
            i.direct_eval_env = environment;
            i.completion = completion;
            result
        })
    }

    pub fn binding_call_in_realm(&mut self, realm: RealmHandle, function: &JsValue,
        receiver: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
        self.binding_with_realm(realm, |i| i.call_value(function, receiver, args))
    }

    /// The value of global `this`; installations still use binding_global_object.
    pub fn binding_global_this(&self) -> JsValue {
        self.window_proxy.as_ref().map(|proxy| JsValue::Object(proxy.clone()))
            .unwrap_or_else(|| JsValue::Object(self.global_obj.clone()))
    }

    /// Associate the embedding's Window handle with this realm's global target.
    /// WindowProxy receiver conversion then returns this same private handle.
    pub fn binding_brand_global(&mut self, handle: HostHandle) -> Result<(), Throw> {
        if self.global_obj.borrow().host_identity.is_some_and(|identity|
            identity.realm != self.host_realm || identity.handle != handle) {
            return Err(Throw::type_error("global already has another host identity"));
        }
        if self.host_cache.get(&handle).and_then(Weak::upgrade)
            .is_some_and(|object| !Rc::ptr_eq(&object, &self.global_obj.0)) {
            return Err(Throw::type_error("host handle already has another wrapper"));
        }
        self.global_obj.borrow_mut().host_identity = Some(crate::value::HostIdentity {
            realm: self.host_realm, handle,
        });
        self.host_cache.insert(handle, Rc::downgrade(&self.global_obj.0));
        Ok(())
    }

    pub fn binding_realm_global(&self, realm: RealmHandle) -> Result<JsValue, Throw> {
        self.check_realm(realm)?;
        let global = if realm.realm == self.host_realm { &self.global_obj }
            else { &self.realms[&realm.realm].global_obj };
        Ok(JsValue::Object(global.clone()))
    }

    /// Allocate fresh intrinsics in this agent using the embedding's global.
    /// No jobs, scheduler ownership, or symbol registry are transferred from
    /// the temporary intrinsic builder into the existing agent.
    pub fn binding_create_realm(&mut self, global: JsValue) -> Result<RealmHandle, Throw> {
        let JsValue::Object(global) = global else {
            return Err(Throw::type_error("realm global must be an object"));
        };
        if Rc::ptr_eq(&global.0, &self.global_obj.0) || self.realms.values().any(|r| Rc::ptr_eq(&r.global_obj.0, &global.0)) {
            return Err(Throw::type_error("global already belongs to a realm"));
        }
        let mut fresh = Interpreter::new();
        let keys = fresh.global_obj.borrow().own_property_keys();
        for key in keys {
            let property = fresh.global_obj.borrow().own_prop(&key).cloned().expect("own key");
            global.borrow_mut().define(key, property).map_err(|_| Throw::type_error("cannot initialize global"))?;
        }
        if global.borrow().proto.is_none() { global.borrow_mut().proto = Some(fresh.object_proto.clone()); }
        fresh.global_obj = global;
        fresh.global_env.set_global_object(fresh.global_obj.clone());
        let global_value = JsValue::Object(fresh.global_obj.clone());
        fresh.global_env.bind_this(global_value.clone());
        fresh.define_global("globalThis", global_value.clone());
        fresh.define_global("window", global_value);
        fresh.stamp_realm_objects();
        let realm = RealmHandle { agent: self.agent_id, realm: fresh.host_realm };
        self.realms.insert(realm.realm, RealmState::capture(&fresh));
        Ok(realm)
    }

    // Installation happens before page code. Traverse descriptors directly,
    // never getters, and stamp only newly allocated functions/prototypes.
    fn stamp_realm_objects(&mut self) {
        self.global_env.set_realm(self.host_realm);
        let mut pending = vec![self.global_obj.clone(), self.object_proto.clone(),
            self.function_proto.clone(), self.async_function_proto.clone(),
            self.generator_function_proto.clone(), self.async_generator_function_proto.clone(),
            self.generator_proto.clone(), self.async_generator_proto.clone(),
            self.legacy_iterator_proto.clone(), self.pair_iterator_proto.clone(),
            self.regexp_string_iterator_proto.clone()];
        let mut seen = std::collections::HashSet::new();
        while let Some(object) = pending.pop() {
            if !seen.insert(Rc::as_ptr(&object.0) as usize) { continue; }
            let mut object = object.borrow_mut();
            if object.realm.is_none() { object.realm = Some(self.host_realm); }
            if let Some(prototype) = &object.proto { pending.push(prototype.clone()); }
            for key in object.own_property_keys() {
                let property = object.own_prop(&key).expect("own key");
                for value in std::iter::once(&property.value)
                    .chain(property.getter.iter()).chain(property.setter.iter()) {
                    if let JsValue::Object(value) = value { pending.push(value.clone()); }
                }
            }
        }
    }
}

impl Interpreter {
    fn call_value_with_new_target(&mut self, function: &JsValue, receiver: JsValue,
        args: Vec<JsValue>, new_target: Option<JsValue>) -> Result<JsValue, Throw> {
        if Self::is_bound_function(function) {
            return self.invoke_bound_function(function, args, new_target);
        }
        // [[Call]] rejects class constructors before entering their realm.
        if new_target.is_none() && matches!(function, JsValue::Object(object) if object.borrow().class_constructor) {
            return Err(Throw::type_error("class constructor requires new"));
        }
        let caller = self.binding_current_realm();
        let id = match function { JsValue::Object(object) => object.borrow().realm, _ => None }
            .unwrap_or(self.host_realm);
        let target = RealmHandle { agent: self.agent_id, realm: id };
        self.check_realm(target)?;
        let entry = self.entry_realm;
        self.entry_realm.get_or_insert(target);
        self.realm_callers.push(if entry.is_some() { caller } else { target });
        let previous_source = if target != caller {
            let source = self.active_source_context();
            Some(std::mem::replace(&mut self.active_source_override, Some(source)))
        } else { None };
        let result = self.binding_with_realm(target, |i|
            i.call_value_with_new_target_inner(function, receiver, args, new_target, caller));
        if let Some(previous_source) = previous_source { self.active_source_override = previous_source; }
        self.realm_callers.pop();
        self.entry_realm = entry;
        result
    }

    fn foreign_host_realm(&self, object: &Obj) -> Option<RealmHandle> {
        let realm = object.borrow().host_identity?.realm;
        (realm != self.host_realm && self.realms.contains_key(&realm))
            .then_some(RealmHandle { agent: self.agent_id, realm })
    }

    fn object_prototype_for_function(&self, function: &JsValue) -> Obj {
        let id = match function { JsValue::Object(object) => object.borrow().realm, _ => None };
        id.and_then(|id| self.realms.get(&id).map(|realm| realm.object_proto.clone()))
            .unwrap_or_else(|| self.object_proto.clone())
    }
    // Keep immutable references: changing a global constructor binding must
    // not change GetPrototypeFromConstructor's intrinsic fallback.
    fn capture_intrinsic_constructors(&mut self) {
        for key in self.global_obj.borrow().own_property_keys() {
            let Some(name) = key.as_string() else { continue; };
            let Some(JsValue::Object(constructor)) = self.global_obj.borrow().get_own(&key) else { continue; };
            if constructor.borrow().func.is_none() { continue; }
            let prototype = constructor.borrow().get_own("prototype");
            if let Some(JsValue::Object(prototype)) = prototype {
                self.intrinsic_constructors.insert(name.to_utf8_lossy(), (constructor, prototype));
            }
        }
        // %AsyncFunction%, %GeneratorFunction% and %AsyncGeneratorFunction%
        // are deliberately never global bindings (is-not-a-global.js), so
        // the scan above never finds them. Register them by name anyway,
        // reached from their shared prototype's own "constructor" property,
        // so GetPrototypeFromConstructor's realm-scoped name lookup
        // (constructor_fallback_prototype) still resolves a subclassed
        // `new AsyncFunction()` etc. to the right realm's
        // %AsyncFunction.prototype% (proto-from-ctor-realm.js).
        for (name, proto) in [
            ("AsyncFunction", &self.async_function_proto),
            ("GeneratorFunction", &self.generator_function_proto),
            ("AsyncGeneratorFunction", &self.async_generator_function_proto),
        ] {
            if let Some(JsValue::Object(constructor)) = proto.borrow().get_own("constructor") {
                self.intrinsic_constructors.insert(name.to_string(), (constructor, proto.clone()));
            }
        }
    }

    fn constructor_fallback_prototype(&self, target: &JsValue, new_target: &JsValue) -> Obj {
        let JsValue::Object(target) = target else { return self.object_prototype_for_function(new_target); };
        let target_realm = target.borrow().realm.unwrap_or(self.host_realm);
        let target_intrinsics = self.realms.get(&target_realm)
            .map(|realm| &realm.intrinsic_constructors).unwrap_or(&self.intrinsic_constructors);
        let name = target_intrinsics.iter().find_map(|(name, (constructor, _))|
            Rc::ptr_eq(&constructor.0, &target.0).then_some(name));
        let Some(name) = name else { return self.object_prototype_for_function(new_target); };
        let new_realm = match new_target { JsValue::Object(object) => object.borrow().realm, _ => None };
        let intrinsics = new_realm.and_then(|realm| self.realms.get(&realm))
            .map(|realm| &realm.intrinsic_constructors).unwrap_or(&self.intrinsic_constructors);
        intrinsics.get(name).map(|(_, prototype)| prototype.clone())
            .unwrap_or_else(|| self.object_prototype_for_function(new_target))
    }

    fn complete_derived_constructor(&mut self, completion: Flow, environment: &EnvHandle, caller: RealmHandle) -> Result<JsValue, Throw> {
        // The function execution context has conceptually returned. Errors
        // created by [[Construct]] now use the caller's current realm.
        self.binding_with_realm(caller, |_| match completion {
            Flow::Return(value) if value.is_object() => Ok(value),
            Flow::Return(value) if !value.is_undefined() => Err(Throw::type_error("derived constructor returned a non-object")),
            _ => environment.resolve_this_checked(),
        })
    }

}
