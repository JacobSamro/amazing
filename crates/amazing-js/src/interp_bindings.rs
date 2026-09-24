//! Backend-neutral operations used by an embedding's binding adapter.
//!
//! These methods expose JavaScript semantics, not AST bodies or VM registers.
//! DOM types and WebIDL policy remain in the driver and amazing-bindings.

use super::*;
use crate::value::HostIdentity;

/// Attributes of an own property installed by native bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindingPropertyFlags {
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
}

impl BindingPropertyFlags {
    pub const METHOD: Self = Self { writable: true, enumerable: false, configurable: true };
    pub const CONSTANT: Self = Self { writable: false, enumerable: true, configurable: false };
}

impl Interpreter {
    /// The active constructor's `newTarget`, including through a derived
    /// `super()` call. Ordinary calls and nested non-constructor callbacks
    /// have no `newTarget` of their own.
    pub fn current_new_target(&self) -> Option<JsValue> {
        self.frames.last().and_then(|frame| frame.new_target.clone())
    }

    /// Produce the existing DOM facade for an element already allocated by the
    /// binding layer. Its style and dataset objects remain live for that node.
    pub fn binding_make_legacy_dom_element(&self, node: u32) -> JsValue {
        self.make_dom_element(node)
    }

    pub fn binding_replace_location_object(&mut self, location: JsValue) -> Result<(), Throw> {
        self.global_obj.borrow_mut().set_own("[[LocationObject]]", location.clone());
        let location_property = { self.global_obj.borrow().own_prop("location").cloned() };
        if let Some(mut property) = location_property {
            property.configurable = false;
            self.global_obj.borrow_mut().define("location", property)
                .map_err(|_| Throw::type_error("Cannot install Window.location"))?;
        }
        self.global_env.assign("location", location).map(|_| ())
    }

    pub fn binding_location_href(&self) -> &str { &self.location_url }

    pub fn binding_location_navigate(&mut self, href: &str, _replace: bool) {
        self.location_url = href.to_owned();
        self.pending_navigation = Some(href.to_owned());
    }

    pub fn binding_location_reload(&mut self) {
        self.pending_navigation = Some(self.location_url.clone());
    }

    /// Set the instance-owned native dispatcher before installing host functions.
    /// The dispatcher must not capture unrooted JS values in its Rust closure.
    pub fn set_host_native_dispatcher(&mut self, dispatch: Rc<HostNativeDispatch>) {
        self.host_native_dispatch = Some(dispatch);
    }

    pub fn binding_global_object(&self) -> JsValue {
        JsValue::Object(self.global_obj.clone())
    }

    /// Register WebIDL [PutForwards] on an interface prototype or global
    /// object. The mapping is private runtime state and applies to inherited
    /// receivers even when a legacy wrapper has an own read-only data property.
    pub fn binding_install_put_forwards(
        &mut self, prototype: &JsValue, attribute: &str, forward_to: &str,
    ) -> Result<(), Throw> {
        let JsValue::Object(owner) = prototype else {
            return Err(Throw::type_error("PutForwards owner must be an object"));
        };
        if attribute == forward_to {
            return Err(Throw::type_error("PutForwards cannot target itself"));
        }
        self.put_forwards.retain(|(weak, name, _)|
            weak.upgrade().is_some() && !(weak.upgrade().is_some_and(|live|
                Rc::ptr_eq(&live, &owner.0)) && name == attribute));
        self.put_forwards.push((Rc::downgrade(&owner.0), attribute.into(), forward_to.into()));
        Ok(())
    }

    pub(super) fn binding_put_forwards_target(&self, receiver: &Obj, attribute: &str) -> Option<String> {
        let mut current = Some(receiver.clone());
        while let Some(object) = current {
            for (weak, name, target) in self.put_forwards.iter().rev() {
                if name == attribute && weak.upgrade().is_some_and(|live| Rc::ptr_eq(&live, &object.0)) {
                    return Some(target.clone());
                }
            }
            current = object.borrow().proto.clone();
        }
        None
    }

    /// `None` selects a null prototype; pass `binding_global_object`'s
    /// prototype or another object when inheritance is required.
    pub fn binding_create_object(&self, prototype: Option<&JsValue>) -> Result<JsValue, Throw> {
        let mut object = JsObject::ordinary();
        object.proto = match prototype {
            Some(JsValue::Object(proto)) => Some(proto.clone()),
            Some(_) => return Err(Throw::type_error("prototype must be an object or null")),
            None => None,
        };
        Ok(JsValue::object(object))
    }

    pub fn binding_object_prototype(&self) -> JsValue {
        JsValue::Object(self.object_proto.clone())
    }

    /// Return the canonical live wrapper for this arena-local host handle.
    /// The cache is weak, so a detached and otherwise unreachable wrapper can
    /// be collected. The driver validates arena, slot and generation against
    /// its DOM before using a handle returned by `binding_host_handle`.
    pub fn binding_wrap_host(
        &mut self,
        handle: HostHandle,
        prototype: &JsValue,
    ) -> Result<JsValue, Throw> {
        if let Some(existing) = self.host_cache.get(&handle).and_then(Weak::upgrade) {
            return Ok(JsValue::Object(Obj(existing)));
        }
        let JsValue::Object(prototype) = prototype else {
            return Err(Throw::type_error("host wrapper prototype must be an object"));
        };
        let mut object = JsObject::ordinary();
        object.proto = Some(prototype.clone());
        object.host_identity = Some(HostIdentity { realm: self.host_realm, handle });
        let wrapper = JsValue::object(object);
        if let JsValue::Object(object) = &wrapper {
            self.host_cache.insert(handle, Rc::downgrade(&object.0));
        }
        Ok(wrapper)
    }

    /// Inspect a private brand. A writable JS property cannot forge it.
    pub fn binding_host_handle(&self, value: &JsValue, brand: u64) -> Option<HostHandle> {
        let JsValue::Object(object) = value else { return None; };
        let identity = object.borrow().host_identity?;
        (identity.realm == self.host_realm && identity.handle.brand == brand)
            .then_some(identity.handle)
    }

    /// Create a callable whose behavior is selected by an interpreter-local ID.
    /// The dispatcher receives the original receiver and `newTarget`.
    pub fn binding_create_native_function(
        &self,
        id: HostFunctionId,
        name: &str,
        length: u8,
        policy: HostCallPolicy,
    ) -> JsValue {
        let mut object = JsObject::function(JsFunction {
            name: name.into(),
            params: Vec::new(),
            body: FunctionBody::HostNative(id, policy, self.host_realm),
            is_arrow: false,
            is_async: false,
            is_generator: false,
            env: self.global_env.clone(),
            home: None,
            proto_for_new: None,
        });
        object.proto = Some(self.function_proto.clone());
        let _ = object.define("name", Property {
            value: JsValue::str(name),
            writable: false,
            enumerable: false,
            configurable: true,
            getter: None,
            setter: None,
        });
        let _ = object.define("length", Property {
            value: JsValue::num(length as f64),
            writable: false,
            enumerable: false,
            configurable: true,
            getter: None,
            setter: None,
        });
        if policy != HostCallPolicy::CallOnly {
            let mut proto = JsObject::ordinary();
            proto.proto = Some(self.object_proto.clone());
            object.set_own("prototype", JsValue::object(proto));
        }
        JsValue::object(object)
    }

    fn binding_owner(object: &JsValue) -> Result<Obj, Throw> {
        match object {
            JsValue::Object(object) => Ok(object.clone()),
            _ => Err(Throw::type_error("property owner must be an object")),
        }
    }

    fn binding_can_define(owner: &Obj, name: &str) -> Result<(), Throw> {
        let object = owner.borrow();
        if object.own_prop(name).is_some_and(|prop| !prop.configurable) {
            return Err(Throw::type_error("cannot redefine a non-configurable property"));
        }
        if object.own_prop(name).is_none() && !object.extensible {
            return Err(Throw::type_error("object is not extensible"));
        }
        Ok(())
    }

    pub fn binding_define_data_property(
        &mut self,
        owner: &JsValue,
        name: &str,
        value: JsValue,
        flags: BindingPropertyFlags,
    ) -> Result<(), Throw> {
        let owner = Self::binding_owner(owner)?;
        let property = Property { value, writable: flags.writable, enumerable: flags.enumerable, configurable: flags.configurable, getter: None, setter: None };
        if self.legacy_define(&owner, name, &property)? { return Ok(()); }
        Self::binding_can_define(&owner, name)?;
        owner.borrow_mut().define(name, property).map_err(|_| Throw::type_error("could not define property"))
    }

    /// Define a real symbol-keyed WebIDL data property without a string alias.
    pub fn binding_define_symbol_data_property(
        &mut self,
        owner: &JsValue,
        symbol: crate::value::JsSymbol,
        value: JsValue,
        flags: BindingPropertyFlags,
    ) -> Result<(), Throw> {
        let owner = Self::binding_owner(owner)?;
        if owner.borrow().own_prop(&symbol).is_some_and(|prop| !prop.configurable) {
            return Err(Throw::type_error("cannot redefine a non-configurable property"));
        }
        if owner.borrow().own_prop(&symbol).is_none() && !owner.borrow().extensible {
            return Err(Throw::type_error("object is not extensible"));
        }
        let property = Property {
            value, writable: flags.writable, enumerable: flags.enumerable,
            configurable: flags.configurable, getter: None, setter: None,
        };
        owner.borrow_mut().define(symbol, property)
            .map_err(|_| Throw::type_error("could not define symbol property"))
    }

    /// Accessors are stored as runtime callables, independent of their body
    /// kind. A reentrant getter/setter runs after the object borrow is released.
    pub fn binding_define_accessor_property(
        &mut self,
        owner: &JsValue,
        name: &str,
        getter: Option<&JsValue>,
        setter: Option<&JsValue>,
        enumerable: bool,
        configurable: bool,
    ) -> Result<(), Throw> {
        fn callable(value: Option<&JsValue>) -> Result<Option<Box<JsFunction>>, Throw> {
            value.map(|value| match value {
                JsValue::Object(object) => object.borrow().func.clone()
                    .map(Box::new).ok_or_else(|| Throw::type_error("accessor is not callable")),
                _ => Err(Throw::type_error("accessor is not callable")),
            }).transpose()
        }
        let getter = callable(getter)?;
        let setter = callable(setter)?;
        let owner = Self::binding_owner(owner)?;
        let property = Property { value: JsValue::Undefined, writable: false, enumerable, configurable, getter, setter };
        if self.legacy_define(&owner, name, &property)? { return Ok(()); }
        Self::binding_can_define(&owner, name)?;
        owner.borrow_mut().define(name, property).map_err(|_| Throw::type_error("could not define property"))
    }

    pub fn binding_get_property(&mut self, object: &JsValue, name: &str) -> Result<JsValue, Throw> {
        self.get_prop(object, name)
    }

    pub fn binding_get_own_property(&mut self, object: &JsValue, name: &str) -> Result<Option<Property>, Throw> {
        let owner = Self::binding_owner(object)?;
        self.legacy_own_property(&owner, name, false)
    }

    pub fn binding_own_property_keys(&mut self, object: &JsValue) -> Result<Vec<String>, Throw> {
        let owner = Self::binding_owner(object)?;
        Ok(self.legacy_own_keys(&owner, false)?.into_iter().map(|key| key.to_utf8_lossy()).collect())
    }

    pub fn binding_own_property_key_units(&mut self, object: &JsValue) -> Result<Vec<Vec<u16>>, Throw> {
        let owner = Self::binding_owner(object)?;
        Ok(self.legacy_own_keys(&owner, false)?.into_iter().map(|key| key.units().to_vec()).collect())
    }

    pub fn binding_delete_property(&mut self, object: &JsValue, name: &str) -> Result<bool, Throw> {
        let owner = Self::binding_owner(object)?;
        match self.legacy_delete(&owner, name)? {
            Some(deleted) => Ok(deleted),
            None => {
                let deleted = owner.borrow_mut().delete_own(name);
                Ok(deleted)
            }
        }
    }

    pub fn binding_set_property(
        &mut self,
        object: &JsValue,
        name: &str,
        value: JsValue,
    ) -> Result<(), Throw> {
        // The legacy assignment helper handles own descriptors but does not
        // walk inherited setters. Installed interface members live on a
        // prototype, so honor the first inherited descriptor here.
        if let JsValue::Object(receiver) = object {
            if self.legacy_set(receiver, name, value.clone())? { return Ok(()); }
            if receiver.borrow().own_prop(name).is_none() {
                let mut cursor = receiver.borrow().proto.clone();
                while let Some(prototype) = cursor {
                    let descriptor = prototype.borrow().own_prop(name).cloned();
                    if let Some(descriptor) = descriptor {
                        if let Some(setter) = descriptor.setter {
                            let callable = JsValue::object(JsObject::function(*setter));
                            self.call_value(&callable, object.clone(), vec![value])?;
                            return Ok(());
                        }
                        if descriptor.getter.is_some() || !descriptor.writable {
                            return Err(Throw::type_error("property has no writable setter"));
                        }
                        break;
                    }
                    cursor = prototype.borrow().proto.clone();
                }
            }
        }
        self.set_prop(object, name, value)
    }

    pub fn binding_construct(&mut self, function: &JsValue, args: Vec<JsValue>)
        -> Result<JsValue, Throw> {
        self.construct(function, args)
    }

    /// Invoke a callable with the supplied receiver. The call path clones the
    /// native dispatcher before entry, so a callback can reenter this runtime.
    pub fn binding_call(
        &mut self,
        function: &JsValue,
        this: JsValue,
        args: Vec<JsValue>,
    ) -> Result<JsValue, Throw> {
        self.call_value(function, this, args)
    }

    /// Create a realm array with the usual Array exotic and prototype.
    pub fn binding_create_array(&mut self, values: Vec<JsValue>) -> Result<JsValue, Throw> {
        array_values(self, values)
    }

    /// ECMAScript SameValue: NaN equals NaN, but positive and negative zero differ.
    pub fn binding_same_value(&self, left: &JsValue, right: &JsValue) -> bool {
        match (left, right) {
            (JsValue::Num(a), JsValue::Num(b)) if a.is_nan() && b.is_nan() => true,
            (JsValue::Num(a), JsValue::Num(b)) if *a == 0.0 && *b == 0.0 => {
                a.to_bits() == b.to_bits()
            }
            _ => left == right,
        }
    }

    pub fn binding_to_boolean(&self, value: &JsValue) -> bool {
        value.to_boolean()
    }

    pub fn binding_to_number(&mut self, value: &JsValue) -> Result<f64, Throw> {
        self.to_number(value)
    }

    /// Create and inspect WebIDL DOMString values without UTF-8 substitution.
    pub fn binding_string(&self, units: &[u16]) -> JsValue {
        JsValue::str_units(units.to_vec())
    }

    pub fn binding_string_units(&self, value: &JsValue) -> Option<Vec<u16>> {
        value.as_str().map(|s| s.units().to_vec())
    }

    pub fn binding_to_dom_string(&mut self, value: &JsValue) -> Result<Vec<u16>, Throw> {
        Ok(self.to_js_string(value)?.units().to_vec())
    }

    /// Match the entire UTF-16 input for a host form-pattern check. Invalid
    /// patterns are reported separately from execution/resource failures.
    pub fn binding_regex_full_match(
        &mut self,
        pattern: &[u16],
        input: &[u16],
        flags: &str,
    ) -> Result<Option<bool>, Throw> {
        // Anchoring the alternatives as a group lets the matcher backtrack
        // to a full match (for example, `a|ab` against `ab`).
        let mut full_pattern = Vec::with_capacity(pattern.len() + 6);
        full_pattern.extend("^(?:".encode_utf16());
        full_pattern.extend_from_slice(pattern);
        full_pattern.extend(")$".encode_utf16());
        let regex = match amazing_regex::Regex::new_utf16(&full_pattern, flags) {
            Ok(regex) => regex,
            Err(amazing_regex::Error::Syntax(_)) => return Ok(None),
            Err(error) => return Err(regexp_error(error)),
        };
        let matched = regex.exec_utf16(input, 0).map_err(regexp_error)?;
        Ok(Some(matched.is_some_and(|match_| match_.range == (0..input.len()))))
    }

    pub fn binding_is_nullish(&self, value: &JsValue) -> bool {
        matches!(value, JsValue::Null | JsValue::Undefined)
    }

    pub fn binding_is_callable(&self, value: &JsValue) -> bool {
        matches!(value, JsValue::Object(object) if object.borrow().func.is_some())
    }

    /// Preserve arbitrary thrown JS values, including object identity.
    pub fn binding_throw(&self, value: JsValue) -> Throw {
        Throw(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn lone_surrogate_round_trips_through_fake_dom_host() {
        let mut runtime = Interpreter::new();
        let host_string = Rc::new(RefCell::new(Vec::<u16>::new()));
        let host_slot = host_string.clone();
        runtime.set_host_native_dispatcher(Rc::new(move |runtime, _, _, args, _| {
            let units = runtime.binding_to_dom_string(args.first().unwrap_or(&JsValue::Undefined))?;
            *host_slot.borrow_mut() = units;
            Ok(runtime.binding_string(&host_slot.borrow()))
        }));
        let echo = runtime.binding_create_native_function(
            HostFunctionId(99), "hostEcho", 1, HostCallPolicy::CallOnly,
        );
        runtime.binding_define_data_property(
            &runtime.binding_global_object(), "hostEcho", echo, BindingPropertyFlags::METHOD,
        ).unwrap();
        let result = runtime.run_script(r#"
            var lone = '\uD800';
            var echoed = hostEcho(lone);
            echoed === lone && echoed.length === 1 && echoed.charCodeAt(0) === 0xD800 &&
            echoed.isWellFormed() === false && echoed.toWellFormed() === '\uFFFD'
        "#).unwrap();
        assert_eq!(result, JsValue::Bool(true));
        assert_eq!(*host_string.borrow(), vec![0xD800]);
    }

    #[test]
    fn utf16_strings_survive_properties_templates_and_regexp_exec() {
        let mut runtime = Interpreter::new();
        let result = runtime.run_script(r#"
            var lone = String.fromCharCode(0xD800);
            var object = {'\uD800': 7};
            var pair = '\uD83D\uDE00';
            object[lone] === 7 && Object.keys(object)[0] === lone &&
            lone.charAt(0) === lone && lone.slice(0, 1) === lone &&
            lone.substring(0, 1) === lone && ('a' + lone + 'b').indexOf(lone) === 1 &&
            String.prototype.charAt.call(lone, 0) === lone &&
            String.prototype.charAt.length === 1 &&
            String.prototype.charAt.prototype === undefined &&
            String.prototype.split.prototype === undefined &&
            pair.length === 2 && pair.codePointAt(0) === 0x1F600 &&
            `a${lone}`.charCodeAt(1) === 0xD800 &&
            'e\u0301'.normalize() === '\u00E9' &&
            '\u00E9'.normalize('NFD') === 'e\u0301' &&
            /./u.exec(lone)[0] === lone
        "#).unwrap();
        assert_eq!(result, JsValue::Bool(true));
        assert_eq!(runtime.run_script("var pieces = []; for (var part of 'a\\uD801\\uDC28b') pieces.push(part); pieces.length === 3 && pieces[1] === '\\uD801\\uDC28'").unwrap(), JsValue::Bool(true));
        assert_eq!(runtime.run_script("'\\u0100'.normalize('NFD') === 'A\\u0304'").unwrap(), JsValue::Bool(true));
        assert_eq!(runtime.run_script("var rejected = false; try { ''.normalize('NFE'); } catch (e) { rejected = e instanceof RangeError; } rejected").unwrap(), JsValue::Bool(true));
        assert_eq!(runtime.run_script("var rejected = false; try { new (String.prototype.charAt)(); } catch (e) { rejected = e instanceof TypeError; } rejected").unwrap(), JsValue::Bool(true));
        assert_eq!(runtime.run_script("var re = new RegExp('\\uD800', 'u'); re.source === '\\uD800' && re.test('\\uD800')").unwrap(), JsValue::Bool(true));
        assert_eq!(runtime.run_script("var rv = new RegExp('a', 'gv'); rv.unicodeSets && rv.flags === 'gv' && !/a/u.unicodeSets").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn array_callbacks_skip_sparse_holes() {
        let mut runtime = Interpreter::new();
        assert_eq!(runtime.run_script("var sparse = []; sparse[10000] = 'x'; var calls = 0; sparse.forEach(function(value) { calls++; }); calls === 1 && sparse.filter(function(value) { return true; }).length === 1").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn host_dispatch_is_per_instance_reentrant_and_preserves_new_target() {
        let mut interpreter = Interpreter::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let calls_for_dispatch = calls.clone();
        interpreter.set_host_native_dispatcher(Rc::new(move |runtime, id, this, args, new_target| {
            calls_for_dispatch.borrow_mut().push((id.0, new_target.is_some()));
            assert_eq!(runtime.current_new_target(), new_target);
            // Nested entry must not borrow the dispatch slot or the caller frame.
            assert_eq!(runtime.run_script("1 + 2")?, JsValue::num(3.0));
            if new_target.is_some() {
                runtime.binding_set_property(&this, "value", args[0].clone())?;
                Ok(this)
            } else {
                Ok(args[0].clone())
            }
        }));
        let ordinary = interpreter.binding_create_native_function(
            HostFunctionId(7), "read", 1, HostCallPolicy::CallOnly,
        );
        let mut foreign = Interpreter::new();
        foreign.set_host_native_dispatcher(Rc::new(|_, _, _, _, _| Ok(JsValue::num(99.0))));
        assert!(foreign.call_value(&ordinary, JsValue::Undefined, vec![]).is_err());
        let constructor = interpreter.binding_create_native_function(
            HostFunctionId(8), "Host", 1, HostCallPolicy::ConstructOnly,
        );
        let global = interpreter.binding_global_object();
        interpreter.binding_define_data_property(
            &global, "read", ordinary, BindingPropertyFlags::METHOD,
        ).unwrap();
        interpreter.binding_define_data_property(
            &global, "Host", constructor.clone(), BindingPropertyFlags::METHOD,
        ).unwrap();
        assert_eq!(interpreter.run_script("read(4)").unwrap(), JsValue::num(4.0));
        assert!(interpreter.run_script("Host(4)").is_err());
        let result = interpreter.binding_construct(&constructor, vec![JsValue::num(9.0)]).unwrap();
        assert_eq!(interpreter.binding_get_property(&result, "value").unwrap(), JsValue::num(9.0));
        assert_eq!(&*calls.borrow(), &[(7, false), (8, true)]);
    }

    #[test]
    fn derived_host_super_binds_the_canonical_upgrade_wrapper() {
        let mut interpreter = Interpreter::new();
        let handle = HostHandle { arena: 1, slot: 42, generation: 0, brand: 9 };
        let constructor = interpreter.binding_create_native_function(
            HostFunctionId(42), "HTMLElement", 0, HostCallPolicy::ConstructOnly,
        );
        let prototype = interpreter.binding_get_property(&constructor, "prototype").unwrap();
        let wrapper = interpreter.binding_wrap_host(handle, &prototype).unwrap();
        let prototype_for_host = prototype.clone();
        interpreter.set_host_native_dispatcher(Rc::new(move |runtime, id, this, _, new_target| {
            assert_eq!(id, HostFunctionId(42));
            assert!(this.is_object());
            assert!(new_target.is_some());
            assert_eq!(runtime.current_new_target(), new_target);
            runtime.binding_wrap_host(handle, &prototype_for_host)
        }));
        interpreter.binding_define_data_property(
            &interpreter.binding_global_object(), "HTMLElement", constructor,
            BindingPropertyFlags::METHOD,
        ).unwrap();
        let upgraded = interpreter.run_script(
            "class X extends HTMLElement { constructor(){ super(); } } new X()",
        ).unwrap();
        assert_eq!(upgraded, wrapper);
        assert_eq!(interpreter.binding_host_handle(&upgraded, 9), Some(handle));
    }

    #[test]
    fn reflect_construct_preserves_explicit_new_target_in_native_entry() {
        let mut interpreter = Interpreter::new();
        interpreter.set_host_native_dispatcher(Rc::new(|runtime, _, this, args, new_target| {
            assert_eq!(runtime.current_new_target(), new_target);
            assert_eq!(runtime.binding_get_property(&new_target.unwrap(), "name")?, JsValue::str("Derived"));
            runtime.binding_set_property(&this, "value", args[0].clone())?;
            Ok(this)
        }));
        let host = interpreter.binding_create_native_function(
            HostFunctionId(99), "Host", 1, HostCallPolicy::ConstructOnly,
        );
        interpreter.binding_define_data_property(
            &interpreter.binding_global_object(), "Host", host, BindingPropertyFlags::METHOD,
        ).unwrap();
        assert_eq!(interpreter.run_script(
            "function Derived() {} var instance = Reflect.construct(Host, [7], Derived); instance instanceof Derived && instance.value === 7",
        ).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn regex_full_match_facade_distinguishes_invalid_patterns() {
        let mut interpreter = Interpreter::new();
        let pattern: Vec<u16> = "a+".encode_utf16().collect();
        assert_eq!(interpreter.binding_regex_full_match(&pattern, &['a' as u16, 'a' as u16], "v").unwrap(), Some(true));
        assert_eq!(interpreter.binding_regex_full_match(&pattern, &['b' as u16, 'a' as u16], "v").unwrap(), Some(false));
        assert_eq!(interpreter.binding_regex_full_match(&"a|ab".encode_utf16().collect::<Vec<_>>(), &['a' as u16, 'b' as u16], "v").unwrap(), Some(true));
        assert_eq!(interpreter.binding_regex_full_match(&['[' as u16], &[], "v").unwrap(), None);
        assert_eq!(interpreter.binding_regex_full_match(&[0xd800], &[0xd800], "").unwrap(), Some(true));
    }

    #[test]
    fn native_accessor_reenters_without_an_object_borrow() {
        let mut interpreter = Interpreter::new();
        let stored = Rc::new(Cell::new(2.0));
        let state = stored.clone();
        interpreter.set_host_native_dispatcher(Rc::new(move |runtime, id, _, args, _| {
            assert_eq!(runtime.run_script("6 * 7")?, JsValue::num(42.0));
            if id.0 == 1 {
                Ok(JsValue::num(state.get()))
            } else {
                if let JsValue::Num(n) = args[0] { state.set(n); }
                Ok(JsValue::Undefined)
            }
        }));
        let prototype = interpreter.binding_object_prototype();
        let object = interpreter.binding_create_object(Some(&prototype)).unwrap();
        let get = interpreter.binding_create_native_function(
            HostFunctionId(1), "get value", 0, HostCallPolicy::CallOnly,
        );
        let set = interpreter.binding_create_native_function(
            HostFunctionId(2), "set value", 1, HostCallPolicy::CallOnly,
        );
        interpreter.binding_define_accessor_property(
            &object, "value", Some(&get), Some(&set), true, true,
        ).unwrap();
        assert_eq!(interpreter.binding_get_property(&object, "value").unwrap(), JsValue::num(2.0));
        interpreter.binding_set_property(&object, "value", JsValue::num(5.0)).unwrap();
        assert_eq!(interpreter.binding_get_property(&object, "value").unwrap(), JsValue::num(5.0));
        interpreter.binding_define_accessor_property(
            &prototype, "inherited", Some(&get), Some(&set), true, true,
        ).unwrap();
        interpreter.binding_set_property(&object, "inherited", JsValue::num(8.0)).unwrap();
        assert_eq!(interpreter.binding_get_property(&object, "inherited").unwrap(), JsValue::num(8.0));
        if let JsValue::Object(object) = &object {
            assert!(object.borrow().get_own("inherited").is_none());
        }
        assert_eq!(stored.get(), 8.0);
    }

    #[test]
    fn property_flags_and_thrown_object_identity_are_preserved() {
        let mut interpreter = Interpreter::new();
        let object = interpreter.binding_create_object(None).unwrap();
        interpreter.binding_define_data_property(
            &object, "constant", JsValue::num(3.0), BindingPropertyFlags::CONSTANT,
        ).unwrap();
        assert!(interpreter.binding_set_property(&object, "constant", JsValue::num(4.0)).is_err());
        assert!(interpreter.binding_define_data_property(
            &object, "constant", JsValue::num(4.0), BindingPropertyFlags::METHOD,
        ).is_err());
        let thrown = interpreter.binding_throw(object.clone());
        assert_eq!(thrown.0, object);
    }

    #[test]
    fn symbol_binding_property_is_visible_by_identity_and_respects_flags() {
        let mut interpreter = Interpreter::new();
        let object = interpreter.binding_create_object(None).unwrap();
        let tag = WellKnownSymbol::ToStringTag.symbol();
        interpreter.binding_define_symbol_data_property(
            &object, tag.clone(), JsValue::str("Bound"),
            BindingPropertyFlags { writable: false, enumerable: false, configurable: false },
        ).unwrap();
        let JsValue::Object(owner) = &object else { unreachable!() };
        assert_eq!(owner.borrow().get_own(&tag), Some(JsValue::str("Bound")));
        assert!(owner.borrow().get_own("Symbol(Symbol.toStringTag)").is_none());
        assert!(owner.borrow().own_keys_enumerable().is_empty());
        assert!(interpreter.binding_define_symbol_data_property(
            &object, tag, JsValue::str("Changed"), BindingPropertyFlags::METHOD,
        ).is_err());
    }

    #[test]
    fn host_wrappers_are_canonical_weak_private_and_realm_scoped() {
        let mut interpreter = Interpreter::new();
        let prototype = interpreter.binding_object_prototype();
        let handle = HostHandle { arena: 9, slot: u32::MAX as u64 + 4, generation: u32::MAX as u64 + 1, brand: 17 };
        let first = interpreter.binding_wrap_host(handle, &prototype).unwrap();
        let again = interpreter.binding_wrap_host(handle, &prototype).unwrap();
        assert_eq!(first, again);
        assert_eq!(interpreter.binding_host_handle(&first, 17), Some(handle));
        assert_eq!(interpreter.binding_host_handle(&first, 18), None);
        let forged = interpreter.binding_create_object(Some(&prototype)).unwrap();
        interpreter.binding_define_data_property(
            &forged, "host_identity", JsValue::num(4.0), BindingPropertyFlags::METHOD,
        ).unwrap();
        assert_eq!(interpreter.binding_host_handle(&forged, 17), None);
        assert_eq!(Interpreter::new().binding_host_handle(&first, 17), None);
        let weak = if let JsValue::Object(object) = &first {
            Rc::downgrade(&object.0)
        } else { unreachable!() };
        drop(first);
        drop(again);
        assert!(weak.upgrade().is_none());
        let replacement = interpreter.binding_wrap_host(handle, &prototype).unwrap();
        assert_eq!(interpreter.binding_host_handle(&replacement, 17), Some(handle));
    }

    #[test]
    fn binding_call_reenters_with_receiver_and_preserves_thrown_identity() {
        let mut interpreter = Interpreter::new();
        interpreter.set_host_native_dispatcher(Rc::new(|runtime, id, this, args, _| {
            if id.0 == 1 {
                runtime.binding_call(&args[0], this, Vec::new())
            } else {
                Ok(JsValue::num(5.0))
            }
        }));
        let global = interpreter.binding_global_object();
        let leaf = interpreter.binding_create_native_function(
            HostFunctionId(2), "leaf", 0, HostCallPolicy::CallOnly,
        );
        interpreter.binding_define_data_property(
            &global, "leaf", leaf, BindingPropertyFlags::METHOD,
        ).unwrap();
        let invoke = interpreter.binding_create_native_function(
            HostFunctionId(1), "invoke", 1, HostCallPolicy::CallOnly,
        );
        let receiver = interpreter.binding_create_object(None).unwrap();
        interpreter.binding_define_data_property(
            &receiver, "x", JsValue::num(7.0), BindingPropertyFlags::METHOD,
        ).unwrap();
        let callback = interpreter.run_script("function callback(){ return this.x + leaf(); } callback").unwrap();
        assert_eq!(interpreter.binding_call(&invoke, receiver.clone(), vec![callback]).unwrap(), JsValue::num(12.0));

        let thrown = interpreter.binding_create_object(None).unwrap();
        interpreter.binding_define_data_property(
            &global, "marker", thrown.clone(), BindingPropertyFlags::METHOD,
        ).unwrap();
        let failing = interpreter.run_script("function failing(){ throw marker; } failing").unwrap();
        let error = interpreter.binding_call(&invoke, receiver, vec![failing]).unwrap_err();
        assert_eq!(error.0, thrown);
    }

    #[test]
    fn binding_array_and_same_value_follow_realm_semantics() {
        let mut interpreter = Interpreter::new();
        let array = interpreter.binding_create_array(vec![JsValue::num(1.0), JsValue::num(2.0)]).unwrap();
        let global = interpreter.binding_global_object();
        interpreter.binding_define_data_property(
            &global, "nativeArray", array.clone(), BindingPropertyFlags::METHOD,
        ).unwrap();
        assert_eq!(interpreter.run_script("Array.isArray(nativeArray) && nativeArray.join('-') === '1-2'").unwrap(), JsValue::Bool(true));
        assert!(interpreter.binding_same_value(&JsValue::num(f64::NAN), &JsValue::num(f64::NAN)));
        assert!(!interpreter.binding_same_value(&JsValue::num(0.0), &JsValue::num(-0.0)));
        assert!(interpreter.binding_same_value(&array, &array.clone()));
        let other = interpreter.binding_create_array(Vec::new()).unwrap();
        assert!(!interpreter.binding_same_value(&array, &other));
    }
}
