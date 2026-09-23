//! Live WebIDL legacy platform properties and value iterators.

use super::*;

pub type IndexCount = dyn Fn(HostHandle) -> Result<u32, Throw>;
pub type SupportedNames = dyn Fn(HostHandle) -> Result<Vec<Vec<u16>>, Throw>;
pub type IndexedGet = dyn Fn(&mut Interpreter, HostHandle, u32) -> Result<JsValue, Throw>;
pub type NamedGet = dyn Fn(&mut Interpreter, HostHandle, &[u16]) -> Result<JsValue, Throw>;
pub type IndexedSet = dyn Fn(&mut Interpreter, HostHandle, u32, JsValue) -> Result<(), Throw>;
pub type NamedSet = dyn Fn(&mut Interpreter, HostHandle, &[u16], JsValue) -> Result<(), Throw>;
pub type NamedDelete = dyn Fn(&mut Interpreter, HostHandle, &[u16]) -> Result<bool, Throw>;

/// Callbacks are owned by the embedding. Each is invoked without a runtime
/// object borrow; the embedding may then take a short native DOM borrow.
#[derive(Clone)]
pub struct LegacyPlatformHooks {
    pub supported_index_count: Rc<IndexCount>,
    pub supported_names: Option<Rc<SupportedNames>>,
    pub indexed_getter: Rc<IndexedGet>,
    pub named_getter: Option<Rc<NamedGet>>,
    pub indexed_setter: Option<Rc<IndexedSet>>,
    pub named_setter: Option<Rc<NamedSet>>,
    pub named_deleter: Option<Rc<NamedDelete>>,
    pub unenumerable_names: bool,
    pub override_builtins: bool,
}

fn array_index(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index.to_string() == key && index != u32::MAX).then_some(index)
}

fn property(value: JsValue, writable: bool, enumerable: bool) -> Property {
    Property { value, writable, enumerable, configurable: true, getter: None, setter: None }
}

impl Interpreter {
    /// Register one interface's live property hooks. A wrapper is matched by
    /// its private realm and brand, never by a JavaScript-visible property.
    pub fn binding_define_legacy_platform_object(&mut self, brand: u64, hooks: LegacyPlatformHooks) {
        self.legacy_hooks.insert(brand, hooks);
    }

    pub(super) fn legacy_for(&self, object: &Obj) -> Option<(HostHandle, LegacyPlatformHooks)> {
        let identity = object.borrow().host_identity?;
        if identity.realm != self.host_realm { return None; }
        self.legacy_hooks.get(&identity.handle.brand).cloned().map(|hooks| (identity.handle, hooks))
    }

    fn supported_name(&self, hooks: &LegacyPlatformHooks, handle: HostHandle, key: &str) -> Result<bool, Throw> {
        let Some(names) = &hooks.supported_names else { return Ok(false); };
        let units: Vec<u16> = key.encode_utf16().collect();
        Ok(names(handle)?.iter().any(|name| name == &units))
    }

    fn named_visible(&self, object: &Obj, key: &str, hooks: &LegacyPlatformHooks) -> bool {
        let own = object.borrow().own_prop(key).cloned();
        if own.as_ref().is_some_and(|p| !p.configurable) { return false; }
        if hooks.override_builtins { return true; }
        if own.is_some() { return false; }
        let mut proto = object.borrow().proto.clone();
        while let Some(p) = proto {
            let borrowed = p.borrow();
            if borrowed.own_prop(key).is_some() { return false; }
            proto = borrowed.proto.clone();
        }
        true
    }

    pub(super) fn legacy_own_property(&mut self, object: &Obj, key: &str, ignore_named: bool) -> Result<Option<Property>, Throw> {
        let Some((handle, hooks)) = self.legacy_for(object) else {
            let ordinary = object.borrow().own_prop(key).cloned();
            return Ok(ordinary);
        };
        if let Some(index) = array_index(key) {
            if index < (hooks.supported_index_count)(handle)? {
                let value = (hooks.indexed_getter)(self, handle, index)?;
                return Ok(Some(property(value, hooks.indexed_setter.is_some(), true)));
            }
        }
        if !ignore_named && hooks.named_getter.is_some() && self.supported_name(&hooks, handle, key)? && self.named_visible(object, key, &hooks) {
            let value = (hooks.named_getter.as_ref().unwrap())(self, handle, &key.encode_utf16().collect::<Vec<_>>())?;
            return Ok(Some(property(value, hooks.named_setter.is_some(), !hooks.unenumerable_names)));
        }
        let ordinary = object.borrow().own_prop(key).cloned();
        Ok(ordinary)
    }

    pub(super) fn legacy_has_property(&self, object: &Obj, key: &str) -> Result<bool, Throw> {
        if let Some((handle, hooks)) = self.legacy_for(object) {
            if let Some(index) = array_index(key) {
                if index < (hooks.supported_index_count)(handle)? { return Ok(true); }
            }
            if self.supported_name(&hooks, handle, key)? && self.named_visible(object, key, &hooks) { return Ok(true); }
        }
        let ordinary = object.borrow().has_property(key);
        Ok(ordinary)
    }

    pub(super) fn legacy_set(&mut self, object: &Obj, key: &str, value: JsValue) -> Result<bool, Throw> {
        let Some((handle, hooks)) = self.legacy_for(object) else { return Ok(false); };
        if let Some(index) = array_index(key) {
            if let Some(setter) = hooks.indexed_setter { setter(self, handle, index, value)?; return Ok(true); }
            return Err(Throw::type_error("indexed property has no setter"));
        }
        if let Some(setter) = hooks.named_setter {
            setter(self, handle, &key.encode_utf16().collect::<Vec<_>>(), value)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn legacy_define(&mut self, object: &Obj, key: &str, prop: &Property) -> Result<bool, Throw> {
        let Some((handle, hooks)) = self.legacy_for(object) else { return Ok(false); };
        if let Some(index) = array_index(key) {
            if prop.getter.is_some() || prop.setter.is_some() { return Err(Throw::type_error("indexed property requires a data descriptor")); }
            let Some(setter) = hooks.indexed_setter else { return Err(Throw::type_error("indexed property has no setter")); };
            setter(self, handle, index, prop.value.clone())?;
            return Ok(true);
        }
        let ordinary = object.borrow().own_prop(key).cloned();
        let unforgeable = ordinary.as_ref().is_some_and(|property| !property.configurable);
        if hooks.supported_names.is_some() && !unforgeable
            && (hooks.override_builtins || ordinary.is_none()) {
            let supported = self.supported_name(&hooks, handle, key)?;
            if supported && hooks.named_setter.is_none() { return Err(Throw::type_error("named property has no setter")); }
            if let Some(setter) = hooks.named_setter {
                if prop.getter.is_some() || prop.setter.is_some() { return Err(Throw::type_error("named property requires a data descriptor")); }
                setter(self, handle, &key.encode_utf16().collect::<Vec<_>>(), prop.value.clone())?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn legacy_delete(&mut self, object: &Obj, key: &str) -> Result<Option<bool>, Throw> {
        let Some((handle, hooks)) = self.legacy_for(object) else { return Ok(None); };
        if let Some(index) = array_index(key) { return Ok(Some(index >= (hooks.supported_index_count)(handle)?)); }
        if self.supported_name(&hooks, handle, key)? && self.named_visible(object, key, &hooks) {
            return match hooks.named_deleter { Some(deleter) => deleter(self, handle, &key.encode_utf16().collect::<Vec<_>>()).map(Some), None => Ok(Some(false)) };
        }
        Ok(None)
    }

    pub(super) fn legacy_own_keys(&mut self, object: &Obj, enumerable_only: bool) -> Result<Vec<String>, Throw> {
        let Some((handle, hooks)) = self.legacy_for(object) else {
            let ordinary = if enumerable_only { object.borrow().own_keys_enumerable() } else { object.borrow().own_keys() };
            return Ok(ordinary);
        };
        let mut keys: Vec<String> = (0..(hooks.supported_index_count)(handle)?).map(|i| i.to_string()).collect();
        if hooks.named_getter.is_some() && (!enumerable_only || !hooks.unenumerable_names) {
            if let Some(names) = &hooks.supported_names {
                for units in names(handle)? {
                    let name = String::from_utf16(&units).map_err(|_| Throw::type_error("UTF-16 property key needs JsString"))?;
                    if self.named_visible(object, &name, &hooks) && !keys.contains(&name) { keys.push(name); }
                }
            }
        }
        let ordinary = if enumerable_only { object.borrow().own_keys_enumerable() } else { object.borrow().own_keys() };
        for key in ordinary {
            if enumerable_only && hooks.unenumerable_names && hooks.named_getter.is_some()
                && self.supported_name(&hooks, handle, &key)? && self.named_visible(object, &key, &hooks) {
                continue;
            }
            if !keys.contains(&key) { keys.push(key); }
        }
        Ok(keys)
    }

    /// Install the value-iterable methods on an interface prototype. The
    /// wrapper's private brand selects the live index hooks at each step.
    pub fn binding_define_iterable(&mut self, brand: u64, prototype: &JsValue) -> Result<(), Throw> {
        if !self.legacy_hooks.contains_key(&brand) { return Err(Throw::type_error("iterable needs indexed hooks")); }
        let JsValue::Object(prototype) = prototype else { return Err(Throw::type_error("iterable prototype must be an object")); };
        let values = Self::native("values", legacy_values, &self.function_proto);
        let for_each = self.array_proto.borrow().get_own("forEach")
            .ok_or_else(|| Throw::type_error("Array.prototype.forEach is missing"))?;
        for (name, function) in [
            ("@@iterator", values.clone()), ("values", values),
            ("keys", Self::native("keys", legacy_keys, &self.function_proto)),
            ("entries", Self::native("entries", legacy_entries, &self.function_proto)),
            ("forEach", for_each),
        ] {
            prototype.borrow_mut().define(name, Property { value: function, writable: true, enumerable: false, configurable: true, getter: None, setter: None })
                .map_err(|_| Throw::type_error("cannot install iterable method"))?;
        }
        let next = Self::native("next", legacy_iterator_next, &self.function_proto);
        self.legacy_iterator_proto.borrow_mut().define("next", Property { value: next, writable: true, enumerable: false, configurable: true, getter: None, setter: None })
            .map_err(|_| Throw::type_error("cannot install iterator next"))?;
        let self_iterator = Self::native("[Symbol.iterator]", legacy_iterator_self, &self.function_proto);
        self.legacy_iterator_proto.borrow_mut().define("@@iterator", Property { value: self_iterator, writable: true, enumerable: false, configurable: true, getter: None, setter: None })
            .map_err(|_| Throw::type_error("cannot install iterator symbol"))?;
        self.legacy_iterator_proto.borrow_mut().define("@@toStringTag", Property { value: JsValue::str("Array Iterator"), writable: false, enumerable: false, configurable: true, getter: None, setter: None })
            .map_err(|_| Throw::type_error("cannot install iterator tag"))?;
        Ok(())
    }

    fn legacy_iterator(&self, this: JsValue, kind: LegacyIteratorKind) -> Result<JsValue, Throw> {
        let JsValue::Object(target) = this else { return Err(Throw::type_error("iterable receiver is not an object")); };
        if self.legacy_for(&target).is_none() { return Err(Throw::type_error("incompatible iterable receiver")); }
        let mut object = JsObject::ordinary();
        object.proto = Some(self.legacy_iterator_proto.clone());
        object.exotic = Exotic::LegacyIterator(LegacyIteratorState { target, next_index: 0, kind });
        Ok(JsValue::object(object))
    }
}

fn legacy_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.legacy_iterator(this, LegacyIteratorKind::Keys)
}
fn legacy_values(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.legacy_iterator(this, LegacyIteratorKind::Values)
}
fn legacy_entries(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    i.legacy_iterator(this, LegacyIteratorKind::Entries)
}
fn legacy_iterator_self(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let valid = matches!(&this, JsValue::Object(o) if matches!(o.borrow().exotic, Exotic::LegacyIterator(_)));
    if valid { Ok(this) } else { Err(Throw::type_error("incompatible iterator receiver")) }
}
fn iterator_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut result = JsObject::ordinary();
    result.proto = Some(i.object_proto.clone());
    result.set_own("value", value);
    result.set_own("done", JsValue::Bool(done));
    JsValue::object(result)
}
fn legacy_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(iterator) = this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let state = match &iterator.borrow().exotic {
        Exotic::LegacyIterator(state) => state.clone(),
        _ => return Err(Throw::type_error("incompatible iterator receiver")),
    };
    let (handle, hooks) = i.legacy_for(&state.target).ok_or_else(|| Throw::type_error("stale iterable receiver"))?;
    let count = (hooks.supported_index_count)(handle)?;
    if state.next_index >= count { return Ok(iterator_result(i, JsValue::Undefined, true)); }
    if let Exotic::LegacyIterator(state) = &mut iterator.borrow_mut().exotic { state.next_index += 1; }
    let index = JsValue::num(state.next_index as f64);
    let value = match state.kind {
        LegacyIteratorKind::Keys => index,
        LegacyIteratorKind::Values => i.get_prop(&JsValue::Object(state.target), &state.next_index.to_string())?,
        LegacyIteratorKind::Entries => {
            let value = i.get_prop(&JsValue::Object(state.target), &state.next_index.to_string())?;
            array_values(i, vec![index, value])?
        }
    };
    Ok(iterator_result(i, value, false))
}
#[cfg(test)]
mod tests {
    use super::*;

    fn fake_hooks(values: Rc<RefCell<Vec<JsValue>>>, names: Rc<RefCell<Vec<String>>>, override_builtins: bool, unenumerable_names: bool) -> LegacyPlatformHooks {
        let count_values = values.clone();
        let get_values = values.clone();
        let get_names = names.clone();
        LegacyPlatformHooks {
            supported_index_count: Rc::new(move |_| Ok(count_values.borrow().len() as u32)),
            supported_names: Some(Rc::new(move |_| Ok(get_names.borrow().iter().map(|name| name.encode_utf16().collect()).collect()))),
            indexed_getter: Rc::new(move |_, _, index| Ok(get_values.borrow()[index as usize].clone())),
            named_getter: Some(Rc::new(|_, _, name| Ok(JsValue::str(String::from_utf16(name).unwrap())))),
            indexed_setter: None,
            named_setter: None,
            named_deleter: None,
            unenumerable_names,
            override_builtins,
        }
    }

    fn fixture(override_builtins: bool, unenumerable_names: bool) -> (Interpreter, JsValue, JsValue, Rc<RefCell<Vec<JsValue>>>, Rc<RefCell<Vec<String>>>) {
        let values = Rc::new(RefCell::new(vec![JsValue::num(10.0), JsValue::num(20.0)]));
        let names = Rc::new(RefCell::new(vec!["item".into(), "values".into()]));
        let mut i = Interpreter::new();
        let prototype = i.binding_create_object(Some(&i.binding_object_prototype())).unwrap();
        i.binding_define_legacy_platform_object(7, fake_hooks(values.clone(), names.clone(), override_builtins, unenumerable_names));
        let object = i.binding_wrap_host(HostHandle { arena: 1, slot: 2, generation: 3, brand: 7 }, &prototype).unwrap();
        (i, object, prototype, values, names)
    }

    #[test]
    fn live_indexed_and_named_properties_are_not_materialized() {
        let (mut i, object, _, values, names) = fixture(false, false);
        assert_eq!(i.binding_get_property(&object, "0").unwrap(), JsValue::num(10.0));
        assert_eq!(i.binding_get_property(&object, "item").unwrap(), JsValue::str("item"));
        assert_eq!(i.legacy_own_keys(&match &object { JsValue::Object(o) => o.clone(), _ => unreachable!() }, false).unwrap(), vec!["0", "1", "item", "values"]);
        values.borrow_mut().remove(0);
        names.borrow_mut().remove(0);
        assert_eq!(i.binding_get_property(&object, "0").unwrap(), JsValue::num(20.0));
        assert_eq!(i.binding_get_property(&object, "1").unwrap(), JsValue::Undefined);
        assert_eq!(i.binding_get_property(&object, "item").unwrap(), JsValue::Undefined);
        assert_eq!(i.legacy_own_keys(&match &object { JsValue::Object(o) => o.clone(), _ => unreachable!() }, false).unwrap(), vec!["0", "values"]);
        if let JsValue::Object(o) = object { assert!(o.borrow().own_keys().is_empty()); }
    }

    #[test]
    fn visibility_and_unenumerable_names_follow_hook_flags() {
        let (mut i, object, prototype, _, _) = fixture(false, true);
        i.binding_define_data_property(&prototype, "values", JsValue::num(99.0), BindingPropertyFlags::METHOD).unwrap();
        assert_eq!(i.binding_get_property(&object, "values").unwrap(), JsValue::num(99.0));
        let JsValue::Object(o) = &object else { unreachable!() };
        assert_eq!(i.legacy_own_keys(o, false).unwrap(), vec!["0", "1", "item"]);
        assert_eq!(i.legacy_own_keys(o, true).unwrap(), vec!["0", "1"]);
        assert_eq!(i.legacy_delete(o, "0").unwrap(), Some(false));
        assert_eq!(i.legacy_delete(o, "item").unwrap(), Some(false));
        assert!(i.binding_set_property(&object, "8", JsValue::num(1.0)).is_err());
        i.binding_set_property(&object, "item", JsValue::num(7.0)).unwrap();
        assert_eq!(i.binding_get_property(&object, "item").unwrap(), JsValue::num(7.0));
        let (mut i, object, prototype, _, _) = fixture(true, false);
        i.binding_define_data_property(&prototype, "values", JsValue::num(99.0), BindingPropertyFlags::METHOD).unwrap();
        assert_eq!(i.binding_get_property(&object, "values").unwrap(), JsValue::str("values"));
        i.binding_set_property(&object, "item", JsValue::num(7.0)).unwrap();
        assert_eq!(i.binding_get_property(&object, "item").unwrap(), JsValue::str("item"));
        let (mut i, object, _, _, _) = fixture(true, true);
        i.binding_set_property(&object, "item", JsValue::num(7.0)).unwrap();
        let JsValue::Object(o) = &object else { unreachable!() };
        assert_eq!(i.legacy_own_keys(o, true).unwrap(), vec!["0", "1"]);
    }

    #[test]
    fn iterator_and_for_each_read_live_indices() {
        let (mut i, object, prototype, values, _) = fixture(false, false);
        i.binding_define_iterable(7, &prototype).unwrap();
        let method = i.binding_get_property(&object, "@@iterator").unwrap();
        let iterator = i.binding_call(&method, object.clone(), Vec::new()).unwrap();
        let next = i.binding_get_property(&iterator, "next").unwrap();
        let first = i.binding_call(&next, iterator.clone(), Vec::new()).unwrap();
        assert_eq!(i.binding_get_property(&first, "value").unwrap(), JsValue::num(10.0));
        values.borrow_mut()[1] = JsValue::num(42.0);
        let second = i.binding_call(&next, iterator.clone(), Vec::new()).unwrap();
        assert_eq!(i.binding_get_property(&second, "value").unwrap(), JsValue::num(42.0));
        let third = i.binding_call(&next, iterator, Vec::new()).unwrap();
        assert_eq!(i.binding_get_property(&third, "done").unwrap(), JsValue::Bool(true));
        let keys = i.binding_get_property(&object, "keys").unwrap();
        let key_iter = i.binding_call(&keys, object.clone(), Vec::new()).unwrap();
        let next = i.binding_get_property(&key_iter, "next").unwrap();
        let result = i.binding_call(&next, key_iter, Vec::new()).unwrap();
        assert_eq!(i.binding_get_property(&result, "value").unwrap(), JsValue::num(0.0));
        let entries = i.binding_get_property(&object, "entries").unwrap();
        let entry_iter = i.binding_call(&entries, object.clone(), Vec::new()).unwrap();
        let next = i.binding_get_property(&entry_iter, "next").unwrap();
        let result = i.binding_call(&next, entry_iter, Vec::new()).unwrap();
        let pair = i.binding_get_property(&result, "value").unwrap();
        assert_eq!(i.binding_get_property(&pair, "0").unwrap(), JsValue::num(0.0));
        assert_eq!(i.binding_get_property(&pair, "1").unwrap(), JsValue::num(10.0));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let capture = seen.clone();
        i.set_host_native_dispatcher(Rc::new(move |_, _, _, args, _| {
            capture.borrow_mut().push(args);
            Ok(JsValue::Undefined)
        }));
        let callback = i.binding_create_native_function(HostFunctionId(1), "collect", 3, HostCallPolicy::CallOnly);
        let for_each = i.binding_get_property(&object, "forEach").unwrap();
        let array_for_each = i.array_proto.borrow().get_own("forEach").unwrap();
        assert!(i.binding_same_value(&for_each, &array_for_each));
        i.binding_call(&for_each, object.clone(), vec![callback]).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0][0], JsValue::num(10.0));
        assert_eq!(seen[1][0], JsValue::num(42.0));
    }

    #[test]
    fn optional_setters_and_deleter_receive_native_keys() {
        let values = Rc::new(RefCell::new(vec![JsValue::num(1.0)]));
        let names = Rc::new(RefCell::new(vec!["named".to_string()]));
        let mut hooks = fake_hooks(values.clone(), names.clone(), false, false);
        let indexed = values.clone();
        hooks.indexed_setter = Some(Rc::new(move |_, _, index, value| {
            let mut values = indexed.borrow_mut();
            if index as usize == values.len() { values.push(value); }
            else { values[index as usize] = value; }
            Ok(())
        }));
        let named = names.clone();
        hooks.named_setter = Some(Rc::new(move |_, _, key, _| {
            let key = String::from_utf16(key).unwrap();
            if !named.borrow().contains(&key) { named.borrow_mut().push(key); }
            Ok(())
        }));
        let deleted = names.clone();
        hooks.named_deleter = Some(Rc::new(move |_, _, key| {
            let key = String::from_utf16(key).unwrap();
            deleted.borrow_mut().retain(|name| name != &key);
            Ok(true)
        }));
        let mut i = Interpreter::new();
        let prototype = i.binding_object_prototype();
        i.binding_define_legacy_platform_object(9, hooks);
        let object = i.binding_wrap_host(HostHandle { arena: 4, slot: 5, generation: 6, brand: 9 }, &prototype).unwrap();
        i.binding_set_property(&object, "0", JsValue::num(3.0)).unwrap();
        assert_eq!(values.borrow()[0], JsValue::num(3.0));
        i.binding_define_data_property(&object, "1", JsValue::num(4.0), BindingPropertyFlags::METHOD).unwrap();
        assert_eq!(values.borrow()[1], JsValue::num(4.0));
        i.binding_set_property(&object, "newName", JsValue::num(5.0)).unwrap();
        assert!(names.borrow().contains(&"newName".to_string()));
        assert!(i.binding_delete_property(&object, "named").unwrap());
        assert!(!names.borrow().contains(&"named".to_string()));
    }

    #[test]
    fn unforgeable_own_property_bypasses_override_named_setter() {
        let (mut i, object, _, values, names) = fixture(true, false);
        let invoked = Rc::new(RefCell::new(0));
        let record = invoked.clone();
        let mut hooks = fake_hooks(values, names, true, false);
        hooks.named_setter = Some(Rc::new(move |_, _, _, _| { *record.borrow_mut() += 1; Ok(()) }));
        i.binding_define_legacy_platform_object(7, hooks);
        let JsValue::Object(owner) = &object else { unreachable!() };
        owner.borrow_mut().define("item", Property { value: JsValue::num(3.0), writable: false, enumerable: true, configurable: false, getter: None, setter: None }).unwrap();
        assert!(i.binding_define_data_property(&object, "item", JsValue::num(4.0), BindingPropertyFlags::METHOD).is_err());
        assert_eq!(*invoked.borrow(), 0);
    }
}
