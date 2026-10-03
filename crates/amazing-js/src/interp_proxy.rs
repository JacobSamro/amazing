// Portions ported from boa-dev/boa (core/engine/src/builtins/proxy/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
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
// Invariants use the target's semantic internal methods, including mapped
// arguments and nested proxies, rather than its raw property storage.

#[derive(Debug)]
pub struct ProxyState {
    target: RefCell<JsValue>,
    handler: RefCell<JsValue>,
    revoked: Cell<bool>,
}

#[cfg(test)]
mod prototype_mutation_tests {
    use super::*;

    #[test]
    fn immutable_root_and_proxy_boundary_do_not_run_traps() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(r#"
                var calls = 0;
                var proxy = new Proxy({}, { getPrototypeOf() { calls++; return proxy; } });
                var o = {}, cycle = {};
                Object.setPrototypeOf(o, proxy);
                Object.setPrototypeOf(cycle, o);
                var rejected = Reflect.setPrototypeOf(Object.prototype, proxy) === false;
                var same = Reflect.setPrototypeOf(Object.prototype, null) === true;
                var failed = false;
                try { Object.setPrototypeOf(Object.prototype, proxy); }
                catch (e) { failed = e instanceof TypeError; }
                calls === 0 && rejected && same && failed &&
                    Object.getPrototypeOf(o) === proxy &&
                    Reflect.setPrototypeOf(o, cycle) === false;
            "# , backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn legacy_proto_accessor_observes_receiver_and_proxy_operations() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(r#"
                var desc = Object.getOwnPropertyDescriptor(Object.prototype, '__proto__');
                var o = {}, p = {}, changed = 0;
                o.__proto__ = p;
                var assigned = o.__proto__ === p;
                desc.set.call(o, 17);
                var ignored = Object.getPrototypeOf(o) === p;
                var proxy = new Proxy({}, {
                    getPrototypeOf() { return p; },
                    setPrototypeOf(target, value) { changed++; return false; }
                });
                var failed = false, nullish = false, frozen = false;
                try { desc.set.call(proxy, null); } catch (e) { failed = e instanceof TypeError; }
                try { desc.set.call(null, 17); } catch (e) { nullish = e instanceof TypeError; }
                Object.preventExtensions(o);
                desc.set.call(o, p);
                try { desc.set.call(o, null); } catch (e) { frozen = e instanceof TypeError; }
                assigned && ignored && failed && nullish && frozen && changed === 1 &&
                    desc.get.call(proxy) === p && desc.get.call(42) === Number.prototype &&
                    desc.set.call(42, null) === undefined &&
                    desc.get.name === 'get __proto__' && desc.get.length === 0 &&
                    desc.set.name === 'set __proto__' && desc.set.length === 1 &&
                    !desc.enumerable && desc.configurable;
            "# , backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }
}

#[cfg(test)]
mod invariant_tests {
    use super::*;

    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn descriptor_compatibility_and_conversion_order() {
        check(r#"
            function rejects(f) { try { f(); } catch(e) { return e instanceof TypeError; } return false; }
            var frozen = {}; Object.defineProperty(frozen,'x',{value:1});
            var writable = {}; Object.defineProperty(writable,'x',{value:1,writable:true});
            var configurable = {x:1};
            var sealed = {}; Object.preventExtensions(sealed);
            function proxy(t) { return new Proxy(t,{defineProperty(){return true;}}); }
            var ok = rejects(()=>Reflect.defineProperty(proxy(frozen),'x',{value:2})) &&
                rejects(()=>Reflect.defineProperty(proxy(frozen),'x',{writable:true})) &&
                rejects(()=>Reflect.defineProperty(proxy(frozen),'x',{get(){return 1;}})) &&
                rejects(()=>Reflect.defineProperty(proxy(writable),'x',{writable:false})) &&
                rejects(()=>Reflect.defineProperty(proxy(configurable),'x',{configurable:false})) &&
                rejects(()=>Reflect.defineProperty(proxy({}),'x',{configurable:false})) &&
                rejects(()=>Reflect.defineProperty(proxy(sealed),'x',{value:1})) &&
                Reflect.defineProperty(proxy(frozen),'x',{value:1}) &&
                Reflect.defineProperty(proxy({}),'x',{value:1,configurable:true});
            var log = '', count = 0, descriptor = {get value(){count++;log+='value;';return 7;}};
            var p = new Proxy({}, {get defineProperty(){log+='trap;';return undefined;}});
            Reflect.defineProperty(p,'x',descriptor);
            ok && log === 'value;trap;' && count === 1 && p.x === 7;
        "#);
    }

    #[test]
    fn reported_descriptors_and_keys_observe_target_invariant_order() {
        check(r#"
            function rejects(f) { try { f(); } catch(e) { return e instanceof TypeError; } return false; }
            var target = {}; Object.defineProperty(target,'x',{value:1,writable:true});
            var ok = rejects(()=>Object.getOwnPropertyDescriptor(new Proxy({}, {getOwnPropertyDescriptor(){return {};}}),'x')) &&
                rejects(()=>Object.getOwnPropertyDescriptor(new Proxy(target, {getOwnPropertyDescriptor(){return {value:1};}}),'x')) &&
                rejects(()=>Reflect.ownKeys(new Proxy(target,{ownKeys(){return [];}})));
            var closed = {x:1}; Object.preventExtensions(closed);
            ok = ok && rejects(()=>Reflect.ownKeys(new Proxy(closed,{ownKeys(){return [];}}))) &&
                rejects(()=>Reflect.ownKeys(new Proxy(closed,{ownKeys(){return ['x','y'];}}))) &&
                rejects(()=>Reflect.ownKeys(new Proxy({}, {ownKeys(){return ['x','x'];}}))) &&
                rejects(()=>Reflect.ownKeys(new Proxy({}, {ownKeys(){return [1];}}))) &&
                Reflect.ownKeys(new Proxy(target,{ownKeys(){return ['x','extra'];}})).length === 2;
            var log = '';
            var inner = new Proxy({x:1},{
                getOwnPropertyDescriptor(t,k){log+='desc;';return Reflect.getOwnPropertyDescriptor(t,k);},
                isExtensible(t){log+='ext;';return Reflect.isExtensible(t);},
                ownKeys(t){log+='keys;';return Reflect.ownKeys(t);}
            });
            var outer = new Proxy(inner,{getOwnPropertyDescriptor(){return {get value(){log+='value;';return 1;},configurable:true};}});
            Object.getOwnPropertyDescriptor(outer,'x');
            ok = ok && log === 'desc;ext;value;';
            log = ''; Reflect.ownKeys(new Proxy(inner,{ownKeys(){return ['x'];}}));
            ok && log === 'ext;keys;desc;';
        "#);
    }

    #[test]
    fn false_traps_short_circuit_and_mapped_descriptor_forwarding_is_live() {
        check(r#"
            var calls = 0;
            var inner = new Proxy({}, {isExtensible(){calls++;throw 1;},getOwnPropertyDescriptor(){calls++;throw 2;}});
            var p = new Proxy(inner,{setPrototypeOf(){return false;},defineProperty(){return false;},deleteProperty(){return false;},set(){return false;}});
            var ok = !Reflect.setPrototypeOf(p,null) && !Reflect.defineProperty(p,'x',{value:1}) &&
                !Reflect.deleteProperty(p,'x') && !Reflect.set(p,'x',1) && calls === 0;
            function direct(a) { a=4;return Reflect.getOwnPropertyDescriptor(arguments,'0').value===4; }
            function forwarded(a) { a=4;var p=new Proxy(arguments,{getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k);}});return Object.getOwnPropertyDescriptor(p,'0').value===4; }
            var revoker = Proxy.revocable({},{}).revoke;
            ok && direct(1) && forwarded(1) && Object.getOwnPropertyNames(revoker).join(',') === 'length,name';
        "#);
    }

    #[test]
    fn proxy_calls_and_prototype_fallback_use_the_required_realms() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            let proxy = i.binding_eval_in_realm(child, "new Proxy(function(){},{apply(t,v,a){return a;}})").unwrap();
            i.define_global("foreignProxy", proxy);
            assert_eq!(i.run_script_with_backend("foreignProxy(1).constructor === Array", backend).unwrap(), JsValue::Bool(true));
            let function = i.binding_eval_in_realm(child, "var f=function(){};f.prototype=null;f").unwrap();
            let expected = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
            i.define_global("foreignFunction", function);
            i.define_global("foreignPrototype", expected);
            assert_eq!(i.run_script_with_backend("Object.getPrototypeOf(Reflect.construct(Object,[],new Proxy(foreignFunction,{})))===foreignPrototype && Object.getPrototypeOf(Reflect.construct(Object,[],foreignFunction.bind(null)))===foreignPrototype", backend).unwrap(), JsValue::Bool(true));
            let revoked = i.binding_eval_in_realm(child, "var r=Proxy.revocable(function(){},{});r.revoke();r.proxy").unwrap();
            let error = i.call_value(&revoked, JsValue::Undefined, vec![]).unwrap_err();
            let expected = i.run_script("TypeError.prototype").unwrap();
            assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![error.0]).unwrap(), expected);
        }
    }
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
            accessor: true,
            value: JsValue::Undefined,
            writable: false,
            getter: fields.get.filter(|v| !v.is_undefined()),
            setter: fields.set.filter(|v| !v.is_undefined()),
            enumerable: fields.enumerable.unwrap_or(false),
            configurable: fields.configurable.unwrap_or(false),
        }
    } else {
        Property {
            accessor: false,
            value: fields.value.unwrap_or(JsValue::Undefined),
            writable: fields.writable.unwrap_or(false),
            getter: None,
            setter: None,
            enumerable: fields.enumerable.unwrap_or(false),
            configurable: fields.configurable.unwrap_or(false),
        }
    }
}

// IsCompatiblePropertyDescriptor, adapted to the runtime's ordinary descriptor record.
fn proxy_compatible_descriptor(fields: &StdDescriptorFields, current: Option<&Property>, extensible: bool) -> bool {
    let Some(current) = current else { return extensible; };
    if current.configurable { return true; }
    if fields.configurable == Some(true)
        || fields.enumerable.is_some_and(|enumerable| enumerable != current.enumerable) { return false; }
    let accessor = fields.get.is_some() || fields.set.is_some();
    let data = fields.value.is_some() || fields.writable.is_some();
    let old_accessor = current.is_accessor();
    if (accessor && !old_accessor) || (data && old_accessor) { return false; }
    if data && !current.writable {
        if fields.writable == Some(true)
            || fields.value.as_ref().is_some_and(|value| !std_same_value(value, &current.value)) { return false; }
    }
    if accessor {
        for (requested, existing) in [(&fields.get, &current.getter), (&fields.set, &current.setter)] {
            if let Some(requested) = requested {
                if !std_same_value(requested, existing.as_ref().unwrap_or(&JsValue::Undefined)) { return false; }
            }
        }
    }
    true
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
        // CreateBuiltinFunction establishes length before name.
        let name = object.own_prop("name").cloned();
        object.delete_own("name");
        if let Some(name) = name { object.define("name", name).unwrap(); }
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
        if let Some(result) = arguments_own_property(target, key) { return result; }
        if let Some(result)=self.namespace_own_property(target,key) {return result;}
        if let Some(descriptor) = binary_index_descriptor(target, key) { return Ok(descriptor); }
        match key.as_string() {
            Some(name) => self.legacy_own_property(target, name.clone(), false),
            None => Ok(target.borrow().own_prop(key).cloned()),
        }
    }

    fn default_has_property(&mut self, target: &Obj, key: &PropertyKey) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_has(target, key) { return result; }
        if let Some(result) = self.window_proxy_has(target, key) { return result; }
        if let Some(result)=self.namespace_has(target,key) {return result;}
        if let Some(descriptor) = binary_index_descriptor(target, key) { return Ok(descriptor.is_some()); }
        match key.as_string() {
            Some(name) => self.legacy_has_property(target, name.clone()),
            None => Ok(target.borrow().has_property(key)),
        }
    }

    fn default_delete_property(&mut self, target: &Obj, key: &PropertyKey) -> Result<bool, Throw> {
        if let Some(result) = self.proxy_delete(target, key) { return result; }
        if let Some(result) = self.window_proxy_delete(target, key) { return result; }
        if let Some(result)=self.namespace_delete(target,key) {return result;}
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
        // Object.prototype is an immutable-prototype exotic object, in every
        // realm. In particular, allowing a Proxy below it makes that Proxy's
        // ordinary handler inherit the Proxy itself and recurse on trap lookup.
        if Rc::ptr_eq(&target.0, &self.object_proto.0)
            || self.realms.values().any(|realm| Rc::ptr_eq(&target.0, &realm.object_proto.0))
            || is_module_namespace(target) {
            return Ok(false);
        }
        if !target.borrow().extensible { return Ok(false); }
        let mut cursor = prototype.clone();
        while let Some(candidate) = cursor {
            if Rc::ptr_eq(&candidate.0, &target.0) { return Ok(false); }
            // OrdinarySetPrototypeOf only follows ordinary [[GetPrototypeOf]]
            // methods. It must neither invoke a Proxy trap nor chase its result:
            // a trap can return itself forever, throw, or have observable effects.
            if matches!(candidate.borrow().exotic, Exotic::Proxy(_) | Exotic::ModuleNamespace(_) | Exotic::DeferredModuleNamespace(_))
                || self.is_window_proxy(&candidate) { break; }
            cursor = candidate.borrow().proto.clone();
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
                    if existing.is_data() && !existing.writable
                        && !std_same_value(&result, &existing.value) {
                        return Err(Throw::type_error("proxy get invariant violated: non-configurable, non-writable target property value mismatch"));
                    }
                    if existing.is_accessor() && existing.getter.is_none() && !result.is_undefined() {
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
                        if existing.is_data() && !existing.writable
                            && !std_same_value(&value, &existing.value) {
                            return Err(Throw::type_error("proxy set invariant violated: non-configurable, non-writable target property value mismatch"));
                        }
                        if existing.is_accessor() && existing.setter.is_none() {
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
            let extensible = self.default_is_extensible(&target_obj)?;
            let target_keys = self.default_own_property_keys(&target_obj)?;
            let mut non_configurable = Vec::new();
            let mut configurable = Vec::new();
            for target_key in target_keys {
                if self.default_get_own_property(&target_obj, &target_key)?
                    .is_some_and(|property| !property.configurable) {
                    non_configurable.push(target_key);
                } else { configurable.push(target_key); }
            }
            if extensible && non_configurable.is_empty() { return Ok(keys); }
            for required in non_configurable {
                if !seen.remove(&required) {
                    return Err(Throw::type_error("proxy ownKeys omitted a non-configurable target key"));
                }
            }
            if extensible { return Ok(keys); }
            for required in configurable {
                if !seen.remove(&required) {
                    return Err(Throw::type_error("proxy ownKeys omitted a non-extensible target key"));
                }
            }
            if !seen.is_empty() {
                return Err(Throw::type_error("proxy ownKeys added a non-extensible target key"));
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
            if !result.is_undefined() && !result.is_object() {
                return Err(Throw::type_error("proxy getOwnPropertyDescriptor trap must return an object or undefined"));
            }
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
            let extensible_target = self.default_is_extensible(&target_obj)?;
            let mut fields = self.std_descriptor_fields(&result)?;
            fields.enumerable.get_or_insert(false);
            fields.configurable.get_or_insert(false);
            if fields.get.is_some() || fields.set.is_some() {
                fields.get.get_or_insert(JsValue::Undefined);
                fields.set.get_or_insert(JsValue::Undefined);
            } else {
                fields.value.get_or_insert(JsValue::Undefined);
                fields.writable.get_or_insert(false);
            }
            if !proxy_compatible_descriptor(&fields, target_desc.as_ref(), extensible_target) {
                return Err(Throw::type_error("proxy getOwnPropertyDescriptor reported an incompatible descriptor"));
            }
            if fields.configurable == Some(false) {
                let Some(existing) = &target_desc else {
                    return Err(Throw::type_error("proxy cannot report a new non-configurable property"));
                };
                if existing.configurable || (fields.writable == Some(false) && existing.writable) {
                    return Err(Throw::type_error("proxy cannot report a target descriptor as frozen"));
                }
            }
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
        if !matches!(&object.borrow().exotic, Exotic::Proxy(_)) { return None; }
        Some((|| -> Result<bool, Throw> {
            // Convert once before revocation and GetMethod; forward the record.
            let fields = self.std_descriptor_fields(descriptor)?;
            let normalized = self.descriptor_fields_to_value(&fields);
            let (target, handler) = self.proxy_parts(object).unwrap()?;
            let target_obj = self.proxy_target_object(&target)?;
            let Some(trap) = self.proxy_trap(&handler, "defineProperty")? else {
                return self.define_own_property(&target_obj, key, &normalized);
            };
            let key_value = property_key_to_value(key);
            let result = self.call_value(&trap, handler.clone(), vec![target.clone(), key_value, normalized])?;
            if !result.to_boolean() { return Ok(false); }
            let current = self.default_get_own_property(&target_obj, key)?;
            let extensible = self.default_is_extensible(&target_obj)?;
            if !proxy_compatible_descriptor(&fields, current.as_ref(), extensible) {
                return Err(Throw::type_error("proxy defineProperty reported an incompatible descriptor"));
            }
            if fields.configurable == Some(false) && current.as_ref().is_none_or(|property| property.configurable) {
                return Err(Throw::type_error("proxy defineProperty cannot introduce a non-configurable property"));
            }
            if current.as_ref().is_some_and(|property| !property.configurable && property.writable)
                && fields.writable == Some(false) {
                return Err(Throw::type_error("proxy defineProperty cannot freeze a writable target property"));
            }
            Ok(true)
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
            if !result { return Ok(false); }
            if !self.default_is_extensible(&target_obj)? {
                let expected = self.default_get_prototype(&target_obj)?;
                if !std_same_value(&prototype_value, &expected) {
                    return Err(Throw::type_error("proxy setPrototypeOf invariant violated: must match a non-extensible target's prototype"));
                }
            }
            Ok(true)
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
        if object.borrow().func.is_none() { return Err(Throw::type_error("proxy is not callable")); }
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
