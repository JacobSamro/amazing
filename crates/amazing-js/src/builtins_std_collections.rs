// Portions ported from boa-dev/boa (core/engine/src/builtins/{map,set}/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Strong collection algorithms use existing actor-owned live iterator cursors;
// current ECMA-262 refreshes intersection size after every Has callback.
// Keyed collections and the ES2025 Set operations.

fn std_collection_method(i: &Interpreter, name: &str, length: u32, native: crate::value::NativeFn) -> JsValue {
    let method = symbol_method(i, name, native);
    symbol_method_length(&method, length);
    method
}

fn std_collection_key(value: &JsValue) -> MapKey {
    if let JsValue::Num(number) = value { if *number == 0.0 { return MapKey::Num(0.0); } }
    map_key(value)
}

fn std_collection_map(value: &JsValue) -> Result<Obj, Throw> { require_map(value) }
fn std_collection_set(value: &JsValue) -> Result<Obj, Throw> { require_set(value) }

fn std_collection_map_put(map: &Obj, key: JsValue, value: JsValue) {
    let key = std_collection_key(&key);
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic {
        if let Some((_, old)) = state.entries.iter_mut().find(|(candidate, _)| candidate.same(&key)) {
            *old = value;
        } else { state.entries.push((key, value)); }
    }
}

fn std_collection_set_add(set: &Obj, value: JsValue) {
    let key = std_collection_key(&value);
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic {
        if !state.entries.iter().any(|candidate| candidate.same(&key)) { state.entries.push(key); }
    }
}

fn std_collection_map_new(i: &Interpreter) -> Obj {
    Obj(Rc::new(RefCell::new({ let mut object = JsObject::ordinary();
        object.exotic = Exotic::Map(MapState::default()); object.proto = Some(i.map_proto.clone()); object })))
}

fn std_collection_set_new(i: &Interpreter) -> Obj {
    Obj(Rc::new(RefCell::new({ let mut object = JsObject::ordinary();
        object.exotic = Exotic::Set(SetState::default()); object.proto = Some(i.set_proto.clone()); object })))
}

impl Interpreter {
    fn install_std_collections(&mut self) {
        self.install_std_collection_iterators();
        let map = Self::native_constructor("Map", std_map_constructor, &self.function_proto);
        let set = Self::native_constructor("Set", std_set_constructor, &self.function_proto);
        for (constructor, proto, name) in [
            (&map, &self.map_proto, "Map"), (&set, &self.set_proto, "Set"),
        ] {
            if let JsValue::Object(object) = constructor {
                object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                object.borrow_mut().define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
                object.borrow_mut().define("prototype", symbol_data(JsValue::Object(proto.clone()), false, false)).unwrap();
            }
            proto.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(name), false, true)).unwrap();
        }
        for (constructor, proto, native) in [(&map, &self.map_proto, std_map_size as crate::value::NativeFn), (&set, &self.set_proto, std_set_size as crate::value::NativeFn)] {
            let getter = std_collection_method(self, "get size", 0, native);
            let species = std_collection_method(self, "get [Symbol.species]", 0, std_collection_species);
            if let JsValue::Object(constructor) = constructor {
                constructor.borrow_mut().define(WellKnownSymbol::Species.symbol(), Property {
                    accessor: true, value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                    getter: Some(species), setter: None,
                }).unwrap();
            }
            proto.borrow_mut().define("size", Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: Some(getter), setter: None,
            }).unwrap();
        }
        self.define_global("Map", map.clone());
        self.define_global("Set", set.clone());
        self.global_obj.borrow_mut().define("Map", symbol_data(map.clone(), true, true)).unwrap();
        self.global_obj.borrow_mut().define("Set", symbol_data(set.clone(), true, true)).unwrap();
        for (name, length, function) in [
            ("get", 1, std_map_get as crate::value::NativeFn),
            ("getOrInsert", 2, std_map_get_or_insert as crate::value::NativeFn),
            ("getOrInsertComputed", 2, std_map_get_or_insert_computed as crate::value::NativeFn),
            ("set", 2, std_map_set as crate::value::NativeFn),
            ("has", 1, std_map_has as crate::value::NativeFn),
            ("delete", 1, std_map_delete as crate::value::NativeFn),
            ("clear", 0, std_map_clear as crate::value::NativeFn),
            ("keys", 0, std_map_keys as crate::value::NativeFn),
            ("values", 0, std_map_values as crate::value::NativeFn),
            ("forEach", 1, std_map_for_each as crate::value::NativeFn),
        ] {
            self.map_proto.borrow_mut().define(name, symbol_data(std_collection_method(self, name, length, function), true, true)).unwrap();
        }
        let map_entries = std_collection_method(self, "entries", 0, std_map_entries);
        self.map_proto.borrow_mut().define("entries", symbol_data(map_entries.clone(), true, true)).unwrap();
        self.map_proto.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), symbol_data(map_entries, true, true)).unwrap();
        for (name, length, function) in [
            ("add", 1, std_set_add as crate::value::NativeFn),
            ("has", 1, std_set_has as crate::value::NativeFn),
            ("delete", 1, std_set_delete as crate::value::NativeFn),
            ("clear", 0, std_set_clear as crate::value::NativeFn),
            ("entries", 0, std_set_entries as crate::value::NativeFn),
            ("forEach", 1, std_set_for_each as crate::value::NativeFn),
            ("union", 1, std_set_union as crate::value::NativeFn),
            ("intersection", 1, std_set_intersection as crate::value::NativeFn),
            ("difference", 1, std_set_difference as crate::value::NativeFn),
            ("symmetricDifference", 1, std_set_symmetric_difference as crate::value::NativeFn),
            ("isSubsetOf", 1, std_set_is_subset_of as crate::value::NativeFn),
            ("isSupersetOf", 1, std_set_is_superset_of as crate::value::NativeFn),
            ("isDisjointFrom", 1, std_set_is_disjoint_from as crate::value::NativeFn),
        ] {
            self.set_proto.borrow_mut().define(name, symbol_data(std_collection_method(self, name, length, function), true, true)).unwrap();
        }
        let set_values = std_collection_method(self, "values", 0, std_set_values);
        self.set_proto.borrow_mut().define("values", symbol_data(set_values.clone(), true, true)).unwrap();
        self.set_proto.borrow_mut().define("keys", symbol_data(set_values.clone(), true, true)).unwrap();
        self.set_proto.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), symbol_data(set_values, true, true)).unwrap();
        if let JsValue::Object(constructor) = map {
            constructor.borrow_mut().define("groupBy", symbol_data(std_collection_method(self, "groupBy", 2, std_map_group_by), true, true)).unwrap();
        }
        self.install_std_weak_collections();
        self.install_disposable_stack();
    }
}

fn std_collection_species(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> { Ok(this) }
fn std_map_size(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    Ok(JsValue::num(if let Exotic::Map(state) = &map.borrow().exotic { state.entries.len() as f64 } else { unreachable!() }))
}
fn std_set_size(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    Ok(JsValue::num(if let Exotic::Set(state) = &set.borrow().exotic { state.entries.len() as f64 } else { unreachable!() }))
}

fn std_collection_constructor_prototype(i: &mut Interpreter) -> Result<Obj, Throw> {
    let target = i.current_new_target().ok_or_else(|| Throw::type_error("collection constructor requires new"))?;
    match i.get_prop(&target, "prototype")? {
        JsValue::Object(prototype) => Ok(prototype),
        _ => {
            let constructor = JsValue::Object(i.native_stack.last().cloned()
                .ok_or_else(|| Throw::type_error("missing collection constructor context"))?);
            i.constructor_fallback_prototype(&constructor, &target)
        }
    }
}
fn std_collection_sync_iterator(i: &mut Interpreter, source: JsValue) -> Result<StdIteratorRecord, Throw> {
    let method = i.get_prop(&source, WellKnownSymbol::Iterator.symbol())?;
    if !std_callable(&method) { return Err(Throw::type_error("collection input is not iterable")); }
    let iterator = i.call_value(&method, source, Vec::new())?;
    if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
    let next = i.get_prop(&iterator, "next")?;
    Ok(StdIteratorRecord { iterator, next })
}
fn std_map_constructor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let prototype = std_collection_constructor_prototype(i)?;
    let map = std_collection_map_new(i);
    map.borrow_mut().proto = Some(prototype);
    let source = std_reflect_arg(&args, 0);
    if source.is_null() || source.is_undefined() { return Ok(JsValue::Object(map)); }
    let adder = i.get_prop(&JsValue::Object(map.clone()), "set")?;
    if !std_callable(&adder) { return Err(Throw::type_error("Map set is not callable")); }
    let record = std_collection_sync_iterator(i, source)?;
    while let Some(entry) = std_iterator_step(i, &record)? {
        let result = (|| {
            if !entry.is_object() { return Err(Throw::type_error("Map iterator value is not an object")); }
            let key = i.get_prop(&entry, "0")?;
            let value = i.get_prop(&entry, "1")?;
            i.call_value(&adder, JsValue::Object(map.clone()), vec![key, value])
        })();
        if let Err(error) = result { return Err(std_iterator_abrupt_close(i, &record, error)); }
    }
    Ok(JsValue::Object(map))
}
fn std_set_constructor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let prototype = std_collection_constructor_prototype(i)?;
    let set = std_collection_set_new(i);
    set.borrow_mut().proto = Some(prototype);
    let source = std_reflect_arg(&args, 0);
    if source.is_null() || source.is_undefined() { return Ok(JsValue::Object(set)); }
    let adder = i.get_prop(&JsValue::Object(set.clone()), "add")?;
    if !std_callable(&adder) { return Err(Throw::type_error("Set add is not callable")); }
    let record = std_collection_sync_iterator(i, source)?;
    while let Some(value) = std_iterator_step(i, &record)? {
        if let Err(error) = i.call_value(&adder, JsValue::Object(set.clone()), vec![value]) {
            return Err(std_iterator_abrupt_close(i, &record, error));
        }
    }
    Ok(JsValue::Object(set))
}
fn std_collection_map_lookup(map: &Obj, key: &JsValue) -> Option<JsValue> {
    let key = std_collection_key(key);
    if let Exotic::Map(state) = &map.borrow().exotic {
        state.entries.iter().find(|(candidate, _)| candidate.same(&key)).map(|(_, value)| value.clone())
    } else { unreachable!() }
}
fn std_map_get_or_insert(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let key = key_value(&std_collection_key(&std_reflect_arg(&args, 0)));
    if let Some(value) = std_collection_map_lookup(&map, &key) { return Ok(value); }
    let value = std_reflect_arg(&args, 1);
    std_collection_map_put(&map, key, value.clone());
    Ok(value)
}
fn std_map_get_or_insert_computed(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let callback = std_reflect_arg(&args, 1);
    if !std_callable(&callback) { return Err(Throw::type_error("Map upsert callback is not callable")); }
    let key = key_value(&std_collection_key(&std_reflect_arg(&args, 0)));
    if let Some(value) = std_collection_map_lookup(&map, &key) { return Ok(value); }
    let value = i.call_value(&callback, JsValue::Undefined, vec![key.clone()])?;
    // Re-lookup after callback mutation, retaining an existing entry's order.
    std_collection_map_put(&map, key, value.clone());
    Ok(value)
}

fn std_map_get(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    let object = map.borrow();
    Ok(if let Exotic::Map(state) = &object.exotic {
        state.entries.iter().find(|(candidate, _)| candidate.same(&key)).map(|(_, value)| value.clone()).unwrap_or(JsValue::Undefined)
    } else { JsValue::Undefined })
}
fn std_map_set(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    std_collection_map_put(&map, std_reflect_arg(&args, 0), std_reflect_arg(&args, 1));
    Ok(this)
}
fn std_map_has(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    Ok(JsValue::Bool(matches!(&map.borrow().exotic, Exotic::Map(state) if state.entries.iter().any(|(candidate, _)| candidate.same(&key)))))
}
fn std_map_delete(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    let mut object = map.borrow_mut();
    let removed = if let Exotic::Map(state) = &mut object.exotic {
        if let Some(index) = state.entries.iter().position(|(candidate, _)| candidate.same(&key)) {
            state.entries.remove(index); Some(index)
        } else { None }
    } else { None };
    drop(object);
    if let Some(index) = removed { std_collection_iter_removed(i, &map, Some(index)); }
    Ok(JsValue::Bool(removed.is_some()))
}
fn std_map_clear(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic { state.entries.clear(); }
    std_collection_iter_removed(i, &map, None);
    Ok(JsValue::Undefined)
}
fn std_map_keys(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    Ok(i.std_collection_iterator(map, StdCollectionIterKind::MapKeys))
}
fn std_map_values(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    Ok(i.std_collection_iterator(map, StdCollectionIterKind::MapValues))
}
fn std_map_entries(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    Ok(i.std_collection_iterator(map, StdCollectionIterKind::MapEntries))
}
fn std_map_for_each(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_collection_map(&this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let this_arg = std_reflect_arg(&args, 1);
    let cursor = i.std_collection_iterator(target, StdCollectionIterKind::MapEntries);
    let result = (|| {
        while let Some((key, value)) = std_collection_cursor_entry(i, &cursor)? {
            i.call_value(&callback, this_arg.clone(), vec![value, key, this.clone()])?;
        }
        Ok(JsValue::Undefined)
    })();
    std_collection_cursor_release(i, &cursor);
    result
}
fn std_map_group_by(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_reflect_arg(&args, 0);
    if source.is_null() || source.is_undefined() { return Err(Throw::type_error("groupBy input is null or undefined")); }
    let callback = std_reflect_arg(&args, 1);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let record = std_collection_sync_iterator(i, source)?;
    let map = std_collection_map_new(i);
    let mut index = 0u64;
    loop {
        if index >= (1u64 << 53) - 1 {
            return Err(std_iterator_abrupt_close(i, &record, Throw::type_error("groupBy maximum index exceeded")));
        }
        let Some(value) = std_iterator_step(i, &record)? else { break };
        let key = match i.call_value(&callback, JsValue::Undefined, vec![value.clone(), JsValue::num(index as f64)]) {
            Ok(key) => key,
            Err(error) => return Err(std_iterator_abrupt_close(i, &record, error)),
        };
        let group = match std_collection_map_lookup(&map, &key) { Some(group) => group, None => array_values(i, Vec::new())? };
        let length = i.array_length(&group);
        array_create_data_property(i, &group, length, value)?;
        std_collection_map_put(&map, key, group);
        index += 1;
    }
    Ok(JsValue::Object(map))
}

fn std_set_add(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    std_collection_set_add(&set, std_reflect_arg(&args, 0));
    Ok(this)
}
fn std_set_has(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    Ok(JsValue::Bool(matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key)))))
}
fn std_set_delete(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    let mut object = set.borrow_mut();
    let removed = if let Exotic::Set(state) = &mut object.exotic {
        if let Some(index) = state.entries.iter().position(|candidate| candidate.same(&key)) { state.entries.remove(index); Some(index) } else { None }
    } else { None };
    drop(object);
    if let Some(index) = removed { std_collection_iter_removed(i, &set, Some(index)); }
    Ok(JsValue::Bool(removed.is_some()))
}
fn std_set_clear(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic { state.entries.clear(); }
    std_collection_iter_removed(i, &set, None);
    Ok(JsValue::Undefined)
}
fn std_set_values(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    Ok(i.std_collection_iterator(set, StdCollectionIterKind::SetValues))
}
fn std_set_entries(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    Ok(i.std_collection_iterator(set, StdCollectionIterKind::SetEntries))
}
fn std_set_for_each(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_collection_set(&this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let this_arg = std_reflect_arg(&args, 1);
    let cursor = i.std_collection_iterator(target, StdCollectionIterKind::SetValues);
    let result = (|| {
        while let Some((key, value)) = std_collection_cursor_entry(i, &cursor)? {
            i.call_value(&callback, this_arg.clone(), vec![value, key, this.clone()])?;
        }
        Ok(JsValue::Undefined)
    })();
    std_collection_cursor_release(i, &cursor);
    result
}
struct StdSetRecord { object: JsValue, size: f64, has: JsValue, keys: JsValue }

fn std_set_record(i: &mut Interpreter, value: JsValue) -> Result<StdSetRecord, Throw> {
    if !value.is_object() { return Err(Throw::type_error("set-like argument must be an object")); }
    let raw_size = i.get_prop(&value, "size")?;
    let number_size = i.to_number(&raw_size)?;
    if number_size.is_nan() { return Err(Throw::type_error("set-like size is NaN")); }
    let size = number_size.trunc();
    if size < 0.0 { return Err(Throw::range_error("set-like size is negative")); }
    let has = i.get_prop(&value, "has")?;
    if !std_callable(&has) { return Err(Throw::type_error("set-like has is not callable")); }
    let keys = i.get_prop(&value, "keys")?;
    if !std_callable(&keys) { return Err(Throw::type_error("set-like keys is not callable")); }
    Ok(StdSetRecord { object: value, size, has, keys })
}
fn std_set_record_has(i: &mut Interpreter, record: &StdSetRecord, value: JsValue) -> Result<bool, Throw> {
    Ok(i.call_value(&record.has, record.object.clone(), vec![value])?.to_boolean())
}
fn std_set_record_iterator(i: &mut Interpreter, record: &StdSetRecord) -> Result<StdIteratorRecord, Throw> {
    let iterator = i.call_value(&record.keys, record.object.clone(), Vec::new())?;
    i.std_iterator_direct(iterator)
}
fn std_set_snapshot(set: &Obj) -> Vec<JsValue> {
    if let Exotic::Set(state) = &set.borrow().exotic { state.entries.iter().map(key_value).collect() } else { unreachable!() }
}
fn std_set_size_value(set: &Obj) -> usize {
    if let Exotic::Set(state) = &set.borrow().exotic { state.entries.len() } else { unreachable!() }
}
fn std_set_contains(set: &Obj, value: &JsValue) -> bool {
    let key = std_collection_key(value);
    matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key)))
}
fn std_set_remove_result(set: &Obj, value: &JsValue) {
    let key = std_collection_key(value);
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic { state.entries.retain(|candidate| !candidate.same(&key)); }
}
fn std_set_new_from(i: &Interpreter, values: impl IntoIterator<Item = JsValue>) -> Obj {
    let set = std_collection_set_new(i);
    for value in values { std_collection_set_add(&set, value); }
    set
}
// Cursor notifications preserve append/delete/clear semantics across arbitrary
// reentrant Has callbacks. No callback is run while registry/storage is borrowed.
fn std_set_live_visit(
    i: &mut Interpreter, set: Obj,
    mut visit: impl FnMut(&mut Interpreter, JsValue) -> Result<bool, Throw>,
) -> Result<bool, Throw> {
    let cursor = i.std_collection_iterator(set, StdCollectionIterKind::SetValues);
    let result = (|| {
        while let Some((value, _)) = std_collection_cursor_entry(i, &cursor)? {
            if !visit(i, value)? { return Ok(false); }
        }
        Ok(true)
    })();
    std_collection_cursor_release(i, &cursor);
    result
}
fn std_set_union(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    // GetIteratorFromMethod can mutate the receiver before its data is copied.
    let iterator = std_set_record_iterator(i, &record)?;
    let result = std_set_new_from(i, std_set_snapshot(&set));
    while let Some(value) = std_iterator_step(i, &iterator)? { std_collection_set_add(&result, value); }
    Ok(JsValue::Object(result))
}
fn std_set_intersection(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let result = std_collection_set_new(i);
    if std_set_size_value(&set) as f64 <= record.size {
        std_set_live_visit(i, set, |i, value| {
            if std_set_record_has(i, &record, value.clone())? { std_collection_set_add(&result, value); }
            Ok(true)
        })?;
    } else {
        let iterator = std_set_record_iterator(i, &record)?;
        while let Some(value) = std_iterator_step(i, &iterator)? {
            if std_set_contains(&set, &value) { std_collection_set_add(&result, value); }
        }
    }
    Ok(JsValue::Object(result))
}
fn std_set_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    // Unlike intersection, difference walks the original fixed copied data.
    let values = std_set_snapshot(&set);
    let result = std_set_new_from(i, values.clone());
    if values.len() as f64 <= record.size {
        for value in values {
            if std_set_record_has(i, &record, value.clone())? { std_set_remove_result(&result, &value); }
        }
    } else {
        let iterator = std_set_record_iterator(i, &record)?;
        while let Some(value) = std_iterator_step(i, &iterator)? { std_set_remove_result(&result, &value); }
    }
    Ok(JsValue::Object(result))
}
fn std_set_symmetric_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let iterator = std_set_record_iterator(i, &record)?;
    let result = std_set_new_from(i, std_set_snapshot(&set));
    while let Some(value) = std_iterator_step(i, &iterator)? {
        if std_set_contains(&set, &value) { std_set_remove_result(&result, &value); }
        else { std_collection_set_add(&result, value); }
    }
    Ok(JsValue::Object(result))
}
fn std_set_is_subset_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    if std_set_size_value(&set) as f64 > record.size { return Ok(JsValue::Bool(false)); }
    Ok(JsValue::Bool(std_set_live_visit(i, set, |i, value| std_set_record_has(i, &record, value))?))
}
fn std_set_is_superset_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    if (std_set_size_value(&set) as f64) < record.size { return Ok(JsValue::Bool(false)); }
    let iterator = std_set_record_iterator(i, &record)?;
    while let Some(value) = std_iterator_step(i, &iterator)? {
        if !std_set_contains(&set, &value) { std_iterator_close(i, &iterator)?; return Ok(JsValue::Bool(false)); }
    }
    Ok(JsValue::Bool(true))
}
fn std_set_is_disjoint_from(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    if std_set_size_value(&set) as f64 <= record.size {
        Ok(JsValue::Bool(std_set_live_visit(i, set, |i, value| Ok(!std_set_record_has(i, &record, value)?))?))
    } else {
        let iterator = std_set_record_iterator(i, &record)?;
        while let Some(value) = std_iterator_step(i, &iterator)? {
            if std_set_contains(&set, &value) { std_iterator_close(i, &iterator)?; return Ok(JsValue::Bool(false)); }
        }
        Ok(JsValue::Bool(true))
    }
}

type StdWeakObject = Weak<RefCell<JsObject>>;
/// CanBeHeldWeakly keys: objects (held weakly) and Symbols not created by Symbol.for.
#[derive(Clone)]
enum StdWeakKey { Object(StdWeakObject), Symbol(JsSymbol) }
struct StdWeakMapData { owner: StdWeakObject, entries: Vec<(StdWeakKey, JsValue)> }
struct StdWeakSetData { owner: StdWeakObject, entries: Vec<StdWeakKey> }
struct StdWeakRefData { owner: StdWeakObject, target: StdWeakKey }
struct StdFinalizationData {
    owner: StdWeakObject,
    _callback: JsValue,
    registrations: Vec<(StdWeakKey, JsValue, Option<JsValue>)>,
}
thread_local! {
    static STD_WEAK_MAPS: RefCell<HashMap<usize, StdWeakMapData>> = RefCell::new(HashMap::new());
    static STD_WEAK_SETS: RefCell<HashMap<usize, StdWeakSetData>> = RefCell::new(HashMap::new());
    static STD_WEAK_REFS: RefCell<HashMap<usize, StdWeakRefData>> = RefCell::new(HashMap::new());
    static STD_FINALIZATION: RefCell<HashMap<usize, StdFinalizationData>> = RefCell::new(HashMap::new());
}

fn std_weak_id(object: &Obj) -> usize { Rc::as_ptr(&object.0) as usize }
fn std_weak_key(i: &Interpreter, value: &JsValue) -> Option<StdWeakKey> {
    match value {
        JsValue::Object(object) => Some(StdWeakKey::Object(Rc::downgrade(&object.0))),
        JsValue::Symbol(symbol) if !i.symbol_registry.values().any(|registered| registered == symbol) =>
            Some(StdWeakKey::Symbol(symbol.clone())),
        _ => None,
    }
}
fn std_weak_alive(key: &StdWeakKey) -> bool {
    match key { StdWeakKey::Object(object) => object.upgrade().is_some(), StdWeakKey::Symbol(_) => true }
}
fn std_weak_key_same(a: &StdWeakKey, b: &StdWeakKey) -> bool {
    match (a, b) {
        (StdWeakKey::Object(a), StdWeakKey::Object(b)) =>
            a.upgrade().is_some_and(|a| b.upgrade().is_some_and(|b| Rc::ptr_eq(&a, &b))),
        (StdWeakKey::Symbol(a), StdWeakKey::Symbol(b)) => a == b,
        _ => false,
    }
}
fn std_weak_same(a: &StdWeakObject, b: &Obj) -> bool {
    a.upgrade().is_some_and(|object| Rc::ptr_eq(&object, &b.0))
}
fn std_weak_value(key: &StdWeakKey) -> JsValue {
    match key {
        StdWeakKey::Object(object) => object.upgrade().map(|o| JsValue::Object(Obj(o))).unwrap_or(JsValue::Undefined),
        StdWeakKey::Symbol(symbol) => JsValue::Symbol(symbol.clone()),
    }
}

impl Interpreter {
    fn install_std_weak_collections(&mut self) {
        for (name, constructor, methods) in [
            ("WeakMap", std_weak_map_constructor as crate::value::NativeFn,
                vec![("get", 1, std_weak_map_get as crate::value::NativeFn), ("set", 2, std_weak_map_set),
                     ("has", 1, std_weak_map_has), ("delete", 1, std_weak_map_delete),
                     ("getOrInsert", 2, std_weak_map_get_or_insert), ("getOrInsertComputed", 2, std_weak_map_get_or_insert_computed)]),
            ("WeakSet", std_weak_set_constructor as crate::value::NativeFn,
                vec![("add", 1, std_weak_set_add as crate::value::NativeFn), ("has", 1, std_weak_set_has),
                     ("delete", 1, std_weak_set_delete)]),
            ("WeakRef", std_weak_ref_constructor as crate::value::NativeFn,
                vec![("deref", 0, std_weak_ref_deref as crate::value::NativeFn)]),
            ("FinalizationRegistry", std_finalization_constructor as crate::value::NativeFn,
                vec![("register", 2, std_finalization_register as crate::value::NativeFn),
                     ("unregister", 1, std_finalization_unregister)]),
        ] {
            let mut prototype = JsObject::ordinary();
            prototype.proto = Some(self.object_proto.clone());
            let prototype = Obj(Rc::new(RefCell::new(prototype)));
            let constructor = Self::native_constructor(name, constructor, &self.function_proto);
            if let JsValue::Object(object) = &constructor {
                object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                object.borrow_mut().define("length", symbol_data(JsValue::num(if name == "FinalizationRegistry" || name == "WeakRef" { 1.0 } else { 0.0 }), false, true)).unwrap();
                object.borrow_mut().define("prototype", symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
            }
            prototype.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(name), false, true)).unwrap();
            for (method_name, length, native) in methods {
                prototype.borrow_mut().define(method_name,
                    symbol_data(std_collection_method(self, method_name, length, native), true, true)).unwrap();
            }
            self.define_global(name, constructor);
            if let Some(value) = self.global_env.resolve(name) {
                self.global_obj.borrow_mut().define(name, symbol_data(value, true, true)).unwrap();
            }
        }
    }
}

fn std_weak_map_owner(this: &JsValue) -> Result<Obj, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible WeakMap receiver")); };
    let valid = STD_WEAK_MAPS.with(|registry| registry.borrow().get(&std_weak_id(object))
        .is_some_and(|data| std_weak_same(&data.owner, object)));
    if valid { Ok(object.clone()) } else { Err(Throw::type_error("incompatible WeakMap receiver")) }
}
fn std_weak_set_owner(this: &JsValue) -> Result<Obj, Throw> {
    let JsValue::Object(object) = this else { return Err(Throw::type_error("incompatible WeakSet receiver")); };
    let valid = STD_WEAK_SETS.with(|registry| registry.borrow().get(&std_weak_id(object))
        .is_some_and(|data| std_weak_same(&data.owner, object)));
    if valid { Ok(object.clone()) } else { Err(Throw::type_error("incompatible WeakSet receiver")) }
}

fn std_weak_map_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("WeakMap requires new")); };
    STD_WEAK_MAPS.with(|registry| registry.borrow_mut().insert(std_weak_id(owner),
        StdWeakMapData { owner: Rc::downgrade(&owner.0), entries: Vec::new() }));
    let source = std_reflect_arg(&args, 0);
    if !source.is_null() && !source.is_undefined() {
        let adder = i.get_prop(&this, "set")?;
        if !std_callable(&adder) { return Err(Throw::type_error("WeakMap set is not callable")); }
        let record = std_collection_sync_iterator(i, source)?;
        while let Some(entry) = std_iterator_step(i, &record)? {
            let result = (|| {
                if !entry.is_object() { return Err(Throw::type_error("WeakMap iterator value is not an object")); }
                let key = i.get_prop(&entry, "0")?;
                let value = i.get_prop(&entry, "1")?;
                i.call_value(&adder, this.clone(), vec![key, value])
            })();
            if let Err(error) = result { return Err(std_iterator_abrupt_close(i, &record, error)); }
        }
    }
    Ok(this)
}

fn std_weak_map_lookup(owner: &Obj, key: &StdWeakKey) -> Option<JsValue> {
    STD_WEAK_MAPS.with(|registry| registry.borrow().get(&std_weak_id(owner)).and_then(|data|
        data.entries.iter().find(|(candidate, _)| std_weak_key_same(candidate, key)).map(|(_, value)| value.clone())))
}
fn std_weak_map_put(owner: &Obj, key: StdWeakKey, value: JsValue) {
    STD_WEAK_MAPS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(owner)).unwrap();
        data.entries.retain(|(candidate, _)| std_weak_alive(candidate));
        if let Some((_, old)) = data.entries.iter_mut().find(|(candidate, _)| std_weak_key_same(candidate, &key)) {
            *old = value;
        } else { data.entries.push((key, value)); }
    });
}
fn std_weak_map_get(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(i, &std_reflect_arg(&args, 0)) else { return Ok(JsValue::Undefined); };
    Ok(std_weak_map_lookup(&owner, &key).unwrap_or(JsValue::Undefined))
}
fn std_weak_map_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let key = std_weak_key(i, &std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakMap key cannot be held weakly"))?;
    std_weak_map_put(&owner, key, std_reflect_arg(&args, 1));
    Ok(this)
}
fn std_weak_map_get_or_insert(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let key = std_weak_key(i, &std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakMap key cannot be held weakly"))?;
    if let Some(value) = std_weak_map_lookup(&owner, &key) { return Ok(value); }
    let value = std_reflect_arg(&args, 1);
    std_weak_map_put(&owner, key, value.clone());
    Ok(value)
}
fn std_weak_map_get_or_insert_computed(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let key_value = std_reflect_arg(&args, 0);
    let key = std_weak_key(i, &key_value).ok_or_else(|| Throw::type_error("WeakMap key cannot be held weakly"))?;
    let callback = std_reflect_arg(&args, 1);
    if !std_callable(&callback) { return Err(Throw::type_error("WeakMap upsert callback is not callable")); }
    if let Some(value) = std_weak_map_lookup(&owner, &key) { return Ok(value); }
    let value = i.call_value(&callback, JsValue::Undefined, vec![key_value])?;
    std_weak_map_put(&owner, key, value.clone());
    Ok(value)
}
fn std_weak_map_has(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(i, &std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    Ok(JsValue::Bool(STD_WEAK_MAPS.with(|registry| registry.borrow().get(&std_weak_id(&owner))
        .is_some_and(|data| data.entries.iter().any(|(candidate, _)| std_weak_key_same(candidate, &key))))))
}
fn std_weak_map_delete(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(i, &std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    let deleted = STD_WEAK_MAPS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.entries.len();
        data.entries.retain(|(candidate, _)| !std_weak_key_same(candidate, &key));
        data.entries.len() != old
    });
    Ok(JsValue::Bool(deleted))
}

fn std_weak_set_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("WeakSet requires new")); };
    STD_WEAK_SETS.with(|registry| registry.borrow_mut().insert(std_weak_id(owner),
        StdWeakSetData { owner: Rc::downgrade(&owner.0), entries: Vec::new() }));
    let source = std_reflect_arg(&args, 0);
    if !source.is_null() && !source.is_undefined() {
        let adder = i.get_prop(&this, "add")?;
        if !std_callable(&adder) { return Err(Throw::type_error("WeakSet add is not callable")); }
        let record = std_collection_sync_iterator(i, source)?;
        while let Some(value) = std_iterator_step(i, &record)? {
            if let Err(error) = i.call_value(&adder, this.clone(), vec![value]) {
                return Err(std_iterator_abrupt_close(i, &record, error));
            }
        }
    }
    Ok(this)
}
fn std_weak_set_add(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let key = std_weak_key(i, &std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakSet value cannot be held weakly"))?;
    STD_WEAK_SETS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        data.entries.retain(std_weak_alive);
        if !data.entries.iter().any(|candidate| std_weak_key_same(candidate, &key)) {
            data.entries.push(key);
        }
    });
    Ok(this)
}
fn std_weak_set_has(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let Some(key) = std_weak_key(i, &std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    Ok(JsValue::Bool(STD_WEAK_SETS.with(|registry| registry.borrow().get(&std_weak_id(&owner))
        .is_some_and(|data| data.entries.iter().any(|candidate| std_weak_key_same(candidate, &key))))))
}
fn std_weak_set_delete(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let Some(key) = std_weak_key(i, &std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    let deleted = STD_WEAK_SETS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.entries.len();
        data.entries.retain(|candidate| !std_weak_key_same(candidate, &key));
        data.entries.len() != old
    });
    Ok(JsValue::Bool(deleted))
}

fn std_weak_ref_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("WeakRef requires new")); };
    let target = std_weak_key(i, &std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakRef target cannot be held weakly"))?;
    STD_WEAK_REFS.with(|registry| registry.borrow_mut().insert(std_weak_id(owner),
        StdWeakRefData { owner: Rc::downgrade(&owner.0), target }));
    Ok(this)
}
fn std_weak_ref_deref(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible WeakRef receiver")); };
    STD_WEAK_REFS.with(|registry| registry.borrow().get(&std_weak_id(owner))
        .filter(|data| std_weak_same(&data.owner, owner))
        .map(|data| std_weak_value(&data.target))
        .ok_or_else(|| Throw::type_error("incompatible WeakRef receiver")))
}

fn std_finalization_constructor(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("FinalizationRegistry requires new")); };
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("cleanup callback is not callable")); }
    STD_FINALIZATION.with(|registry| registry.borrow_mut().insert(std_weak_id(owner),
        StdFinalizationData { owner: Rc::downgrade(&owner.0), _callback: callback, registrations: Vec::new() }));
    Ok(this)
}
fn std_finalization_owner(this: &JsValue) -> Result<Obj, Throw> {
    let JsValue::Object(owner) = this else { return Err(Throw::type_error("incompatible FinalizationRegistry receiver")); };
    let valid = STD_FINALIZATION.with(|registry| registry.borrow().get(&std_weak_id(owner))
        .is_some_and(|data| std_weak_same(&data.owner, owner)));
    if valid { Ok(owner.clone()) } else { Err(Throw::type_error("incompatible FinalizationRegistry receiver")) }
}
fn std_finalization_register(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_finalization_owner(&this)?;
    let target_value = std_reflect_arg(&args, 0);
    let target = std_weak_key(i, &target_value).ok_or_else(|| Throw::type_error("target cannot be held weakly"))?;
    let held = std_reflect_arg(&args, 1);
    if std_same_value(&target_value, &held) {
        return Err(Throw::type_error("target and holdings must differ"));
    }
    let token = args.get(2).filter(|value| !value.is_undefined()).cloned();
    if token.as_ref().is_some_and(|value| std_weak_key(i, value).is_none()) {
        return Err(Throw::type_error("unregister token cannot be held weakly"));
    }
    STD_FINALIZATION.with(|registry| registry.borrow_mut().get_mut(&std_weak_id(&owner)).unwrap()
        .registrations.push((target, held, token)));
    Ok(JsValue::Undefined)
}
fn std_finalization_unregister(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_finalization_owner(&this)?;
    let token = std_reflect_arg(&args, 0);
    if std_weak_key(i, &token).is_none() { return Err(Throw::type_error("unregister token cannot be held weakly")); }
    let removed = STD_FINALIZATION.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.registrations.len();
        data.registrations.retain(|(_, _, candidate)| !candidate.as_ref().is_some_and(|value| std_same_value(value, &token)));
        data.registrations.len() != old
    });
    Ok(JsValue::Bool(removed))
}

#[cfg(test)]
mod strong_collection_protocol_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script_with_backend(source, backend).unwrap_or_else(|e|panic!("{backend:?}: {}\n{source}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn weak_collections_accept_unregistered_symbols_and_upsert() {
        check(r#"
          var s=Symbol('s'),reg=Symbol.for('r'),o={},ok=true;
          var wm=new WeakMap([[s,1]]),ws=new WeakSet([s]);
          ok=ok && wm.get(s)===1 && wm.has(s) && ws.has(s) && wm.delete(s) && !wm.has(s);
          ok=ok && wm.getOrInsert(o,5)===5 && wm.getOrInsert(o,6)===5 && wm.getOrInsertComputed(s,function(k){return k===s?7:0;})===7;
          for(var f of [function(){wm.set(reg,1);},function(){ws.add(reg);},function(){new WeakRef(reg);},function(){wm.getOrInsert(1,1);}]){
            try{f();ok=false;}catch(e){ok=ok && e instanceof TypeError;}
          }
          ok=ok && wm.has(Symbol.iterator)===false && new WeakRef(Symbol.iterator).deref()===Symbol.iterator && WeakRef.length===1;
          ok;
        "#);
    }
    #[test]
    fn installed_species_size_brands_and_upsert_metadata_are_genuine() {
        check(r#"
          var valid=true,m=new Map,s=new Set;
          for(var C of [Map,Set]){
            var d=Object.getOwnPropertyDescriptor(C,Symbol.species),g=d.get;
            valid=valid && d.enumerable===false && d.configurable===true && d.set===undefined && g.length===0 && g.name==='get [Symbol.species]' && C[Symbol.species]===C && g.call(7)===7;
            try{new g;valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }
          var mg=Object.getOwnPropertyDescriptor(Map.prototype,'size').get,sg=Object.getOwnPropertyDescriptor(Set.prototype,'size').get;
          valid=valid && mg.call(m)===0 && sg.call(s)===0;
          for(var entry of [[mg,s],[sg,m],[mg,new Proxy(m,{})],[sg,{}],[mg,Map.prototype],[sg,Set.prototype]]){
            try{entry[0].call(entry[1]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }
          for(var name of ['getOrInsert','getOrInsertComputed']){
            var d=Object.getOwnPropertyDescriptor(Map.prototype,name),f=d.value;
            valid=valid && f.length===2 && f.name===name && d.writable && !d.enumerable && d.configurable && !Object.hasOwn(f,'prototype');
            try{new f;valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }valid;
        "#);
    }
    #[test]
    fn upsert_canonicalizes_keys_validates_before_lookup_and_rechecks_mutation() {
        check(r#"
          var valid=true,m=new Map,object={},symbol=Symbol(),calls=0;
          for(var key of [undefined,null,true,false,1,NaN,2n,'x',object,symbol]){
            var value={};valid=valid && m.getOrInsert(key,value)===value && m.getOrInsert(key,7)===value;
            valid=valid && m.getOrInsertComputed(key,function(){calls++;throw 'unused';})===value;
          }
          valid=valid && calls===0;
          try{m.getOrInsertComputed(object,3);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var a=new Map,cb=function(k){'use strict';valid=valid && this===undefined && arguments.length===1 && Object.is(k,0);a.set('first',1);a.set(-0,'callback');return'final';};
          valid=valid && a.getOrInsertComputed(-0,cb)==='final' && a.get(0)==='final' && Array.from(a.keys()).join(',')==='first,0';
          var error={};try{a.getOrInsertComputed('bad',function(){throw error;});valid=false;}catch(e){valid=valid && e===error && !a.has('bad');}
          a.set('gone',1);a.getOrInsertComputed('new',function(k){a.clear();a.set(k,2);a.set('after',3);return 4;});
          valid=valid && Array.from(a.keys()).join(',')==='new,after' && a.get('new')===4;
          var b=new Map;b.getOrInsert(-0,1);valid=valid && Object.is(b.keys().next().value,0);valid;
        "#);
    }
    #[test]
    fn constructors_allocate_once_then_stream_cached_next_and_adder() {
        check(r#"
          var valid=true;
          for(var C of [Map,Set]){
            var map=C===Map,log=[],count=0,n=0,adder=map?'set':'add',proto={};
            Object.defineProperty(proto,adder,{get(){log.push('adder');return function(k,v){log.push('add:'+k);valid=valid && Object.getPrototypeOf(this)===proto;};}});
            var nt=new Proxy(function(){},{get(t,k,r){if(k==='prototype'){count++;log.push('proto');return proto;}return Reflect.get(t,k,r);}});
            var input={get [Symbol.iterator](){log.push('iterator');return function(){log.push('call');return{get next(){log.push('next');return function(){var index=n++;log.push('step:'+index);return index===2?{done:true}:{value:map?[index,index+1]:index};};}};};}};
            var result=Reflect.construct(C,[input],nt);
            valid=valid && count===1 && Object.getPrototypeOf(result)===proto && log.join(',')==='proto,adder,iterator,call,next,step:0,add:0,step:1,add:1,step:2';
            var bound=C.bind(null),proxy=new Proxy(C,{});valid=valid && new bound instanceof C && new proxy instanceof C;
            class Sub extends C{};var sub=new Sub(map?[[1,2]]:[1]);valid=valid && sub instanceof Sub && sub instanceof C && sub.size===1;
            var bare={};try{C.call(bare);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            var gets=0,emptyProto=new Proxy({}, {get(){gets++;throw 'unused adder';}}),emptyNT=new Proxy(function(){},{get(t,k,r){return k==='prototype'?emptyProto:Reflect.get(t,k,r);}});
            valid=valid && Object.getPrototypeOf(Reflect.construct(C,[null],emptyNT))===emptyProto && gets===0;
          }valid;
        "#);
    }
    #[test]
    fn constructor_close_preserves_entry_or_adder_errors_and_step_failures_do_not_close() {
        check(r#"
          var valid=true;
          for(var mode of [0,1,2,3,4,5,6]){
            var log=[],error={},entry={get 0(){if(mode===1)throw error;return 1;},get 1(){if(mode===2)throw error;return 2;}},proto={set(){log.push('add');if(mode===3)throw error;}};
            var nt=new Proxy(function(){},{get(t,k,r){return k==='prototype'?proto:Reflect.get(t,k,r);}});
            var input={[Symbol.iterator](){return{next(){log.push('next');if(mode===4)throw error;return{get done(){if(mode===5)throw error;return false;},get value(){if(mode===6)throw error;return mode===0?1:entry;}};},return(){log.push('close');throw'ignored';}};}};
            try{Reflect.construct(Map,[input],nt);valid=false;}catch(e){valid=valid && (mode===0?e instanceof TypeError:e===error);}
            valid=valid && log.join(',')===(mode===3?'next,add,close':mode<=2?'next,close':'next');
          }
          var error={},closed=0,proto={add(){throw error;}},nt=new Proxy(function(){},{get(t,k,r){return k==='prototype'?proto:Reflect.get(t,k,r);}}),input={[Symbol.iterator](){return{next(){return{value:1};},return(){closed++;return 1;}};}};
          try{Reflect.construct(Set,[input],nt);valid=false;}catch(e){valid=valid && e===error && closed===1;}
          for(var C of [Map,Set]){
            var log=[],proto={};Object.defineProperty(proto,C===Map?'set':'add',{get(){log.push('adder');return 0;}});
            var nt=new Proxy(function(){},{get(t,k,r){return k==='prototype'?proto:Reflect.get(t,k,r);}}),input={get [Symbol.iterator](){log.push('iterator');throw'unused';}};
            try{Reflect.construct(C,[input],nt);valid=false;}catch(e){valid=valid && e instanceof TypeError && log.join(',')==='adder';}
          }valid;
        "#);
    }
    #[test]
    fn foreach_live_walks_reinsertion_clear_updates_and_reentrant_iterations() {
        check(r#"
          var valid=true;
          for(var C of [Map,Set]){
            var map=C===Map,target=new C(map?[[1,10],[2,20],[3,30]]:[1,2,3]),seen=[],changed=false,receiver={};
            target.forEach(function(v,k,t){'use strict';valid=valid && this===receiver && t===target && arguments.length===3;seen.push(k);
              if(k===1 && !changed){changed=true;target.delete(1);target.delete(2);if(map){target.set(1,11);target.set(4,40);}else{target.add(1);target.add(4);}}
            },receiver);valid=valid && seen.join(',')==='1,3,1,4';
            var target=new C(map?[[1,10],[2,20]]:[1,2]),seen=[],changed=false;
            target.forEach(function(v,k){seen.push(k);if(!changed){changed=true;target.clear();if(map)target.set(9,90);else target.add(9);}});
            valid=valid && seen.join(',')==='1,9';
            var error={},before=0;try{target.forEach(function(){before++;throw error;});valid=false;}catch(e){valid=valid && e===error && before===1;}
            var outer=0,inner=0;target.forEach(function(){outer++;target.forEach(function(){inner++;});});valid=valid && outer===1 && inner===1;
          }
          var m=new Map([[1,10],[2,20]]),seen=[];m.forEach(function(v,k){seen.push(v);if(k===1)m.set(2,21);});valid=valid && seen.join(',')==='10,21';valid;
        "#);
    }
    #[test]
    fn set_records_have_exact_conversion_get_order_and_streaming_early_close() {
        check(r#"
          var valid=true,methods=['union','intersection','difference','symmetricDifference','isSubsetOf','isSupersetOf','isDisjointFrom'];
          for(var method of methods){
            var log=[],other={get size(){log.push('size');return{valueOf(){log.push('number');return-0.5;}};},get has(){log.push('has');return function(){return false;};},get keys(){log.push('keys');return function(){log.push('call');return{next(){return{done:true};}};};}};
            new Set()[method](other);valid=valid && log.slice(0,4).join(',')==='size,number,has,keys';
            var reads=0,bad={size:0,has:3,get keys(){reads++;throw'unused';}};try{new Set()[method](bad);valid=false;}catch(e){valid=valid && e instanceof TypeError && reads===0;}
          }
          for(var method of ['isSupersetOf','isDisjointFrom'])for(var mode of [0,1,2]){
            var log=[],error={},other={size:0,has(){throw'unused';},keys(){log.push('keys');return{next(){log.push('next');return{value:method==='isSupersetOf'?9:1};},return(){log.push('close');if(mode===1)throw error;return mode===2?1:{};}};}};
            try{var result=new Set([1,2])[method](other);valid=valid && mode===0 && result===false;}catch(e){valid=valid && (mode===1?e===error:mode===2&&e instanceof TypeError);}
            valid=valid && log.join(',')==='keys,next,close';
          }valid;
        "#);
    }
    #[test]
    fn set_operations_preserve_live_membership_fixed_copy_and_keys_acquisition_order() {
        check(r#"
          var valid=true;
          for(var method of ['intersection','isSubsetOf','isDisjointFrom']){
            var set=new Set([1,2]),seen=[],changed=false,other={size:100,has(k){seen.push(k);if(!changed){changed=true;set.delete(1);set.delete(2);set.add(3);set.add(1);}return method==='isDisjointFrom'?false:true;},keys(){throw'unused';}};
            var result=set[method](other);valid=valid && seen.join(',')==='1,3,1' && (method==='intersection'?Array.from(result).join(',')==='1,3':result===true);
          }
          var set=new Set([1,2]),seen=[],other={size:100,has(k){seen.push(k);set.clear();set.add(9);return k===1;},keys(){throw'unused';}};
          valid=valid && Array.from(set.difference(other)).join(',')==='2' && seen.join(',')==='1,2';
          for(var method of ['union','symmetricDifference','difference']){
            var set=new Set([1]),other={size:0,has(){throw'unused';},keys(){set.add(2);return{next(){return{done:true};}};}};
            valid=valid && Array.from(set[method](other)).join(',')===(method==='difference'?'1':'1,2');
          }
          for(var method of ['union','intersection','symmetricDifference']){
            var n=0,other={size:0,has(){return false;},keys(){return{next(){return n++<2?{value:-0}:{done:true};}};}};
            var result=new Set([0,1])[method](other);valid=valid && Array.from(result).every(function(k){return !Object.is(k,-0);});
          }valid;
        "#);
    }
    #[test]
    fn grouping_streams_callbacks_cached_next_and_own_data_arrays() {
        check(r#"
          var valid=true,log=[],n=0,gets=0,source={[Symbol.iterator](){return{get next(){gets++;return function(){log.push('step:'+n);return n<3?{value:n++}:{done:true};};},return(){log.push('close');throw'ignored';}};}};
          var result=Map.groupBy(source,function(v,k){'use strict';valid=valid && this===undefined && arguments.length===2 && v===k;log.push('group:'+k);return v%2;});
          valid=valid && gets===1 && log.join(',')==='step:0,group:0,step:1,group:1,step:2,group:2,step:3' && result.get(0).join(',')==='0,2' && result.get(1)[0]===1;
          var error={},closed=0,source={[Symbol.iterator](){return{next(){return{value:1};},return(){closed++;throw'ignored';}};}};
          try{Map.groupBy(source,function(){throw error;});valid=false;}catch(e){valid=valid && e===error && closed===1;}
          var input=[1,2,3],desc=Object.getOwnPropertyDescriptor(Array.prototype,'0'),writes=0;
          Object.defineProperty(Array.prototype,'0',{set(){writes++;throw'inherited';},configurable:true});
          try{var grouped=Map.groupBy(input,function(){return 0;});}finally{if(desc)Object.defineProperty(Array.prototype,'0',desc);else delete Array.prototype[0];}
          valid=valid && writes===0 && grouped.get(0).join(',')==='1,2,3';valid;
        "#);
    }
    #[test]
    fn collection_iterators_distinguish_brands_live_mutation_and_data_entry_creation() {
        check(r#"
          var valid=true,m=new Map([[1,10],[2,20]]),s=new Set([1,2]),mi=m.entries(),si=s.entries(),mn=mi.next,sn=si.next;
          for(var entry of [[mn,si],[sn,mi],[mn,new Proxy(mi,{})],[sn,{}]]){try{entry[0].call(entry[1]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          valid=valid && Object.getPrototypeOf(Object.getPrototypeOf(mi))===Iterator.prototype && !Object.hasOwn(Object.getPrototypeOf(mi),Symbol.iterator);
          var first=mi.next().value;m.delete(1);m.set(3,30);valid=valid && first.join(',')==='1,10' && mi.next().value[0]===2 && mi.next().value[0]===3 && mi.next().done;
          m.set(4,40);valid=valid && mi.next().done;
          var desc=Object.getOwnPropertyDescriptor(Array.prototype,'0'),writes=0;
          Object.defineProperty(Array.prototype,'0',{set(){writes++;throw'inherited';},configurable:true});
          try{var me=m.entries().next().value,se=s.entries().next().value;}finally{if(desc)Object.defineProperty(Array.prototype,'0',desc);else delete Array.prototype[0];}
          valid=valid && writes===0 && Object.hasOwn(me,'0') && Object.hasOwn(se,'0') && me.length===2 && se.length===2;valid;
        "#);
    }
    #[test]
    fn foreign_constructor_fallback_errors_and_iterator_resume_realms_are_independent() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            let object=i.binding_create_object(None).unwrap();let realm=i.binding_create_realm(object).unwrap();
            let values=i.binding_eval_in_realm(realm,"[Map,Set,Map.prototype.entries,Set.prototype.values,Map.prototype.entries.call(new Map).next,Set.prototype.values.call(new Set).next,Object.prototype,Array.prototype,TypeError.prototype,(function(){var f=function(){};f.prototype=null;return f;})()]" ).unwrap();
            for (index,name) in ["ForeignMap","ForeignSet","foreignEntries","foreignValues","foreignMapNext","foreignSetNext","foreignObject","foreignArray","foreignType","ForeignNT"].into_iter().enumerate(){
                let value=i.get_prop(&values,index.to_string()).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            let source=r#"
              var valid=true;
              for(var C of [Map,Set]){
                var result=Reflect.construct(C,[],ForeignNT);valid=valid && Object.getPrototypeOf(result)===(C===Map?ForeignMap.prototype:ForeignSet.prototype);
                var bound=ForeignNT.bind(null),proxy=new Proxy(ForeignNT,{});bound.prototype=null;var r=Reflect.construct(C,[],bound);valid=valid && Object.getPrototypeOf(r)===(C===Map?ForeignMap.prototype:ForeignSet.prototype);
                var r=Reflect.construct(C,[],proxy);valid=valid && Object.getPrototypeOf(r)===(C===Map?ForeignMap.prototype:ForeignSet.prototype);
              }
              for(var tuple of [[new Map([[1,2]]),foreignEntries,foreignMapNext],[new Set([1]),foreignValues,foreignSetNext]]){
                var target=tuple[0],factory=tuple[1],next=tuple[2],localIter=target[Symbol.iterator](),localNext=localIter.next;
                var r=next.call(localIter);valid=valid && Object.getPrototypeOf(r)===Object.prototype;
                if(target instanceof Map)valid=valid && Object.getPrototypeOf(r.value)===Array.prototype;
                r=next.call(localIter);valid=valid && r.done && Object.getPrototypeOf(r)===Object.prototype;
                r=next.call(localIter);valid=valid && r.done && Object.getPrototypeOf(r)===foreignObject;
                var foreignIter=factory.call(target);r=localNext.call(foreignIter);valid=valid && Object.getPrototypeOf(r)===foreignObject;
                if(target instanceof Map)valid=valid && Object.getPrototypeOf(r.value)===foreignArray;
                r=localNext.call(foreignIter);valid=valid && r.done && Object.getPrototypeOf(r)===foreignObject;
                r=localNext.call(foreignIter);valid=valid && r.done && Object.getPrototypeOf(r)===Object.prototype;
                try{next.call({});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
              }
              for(var C of [ForeignMap,ForeignSet]){try{C();valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}}
              var revoked=Proxy.revocable(ForeignNT,{});revoked.revoke();try{Reflect.construct(Map,[],revoked.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
              var error={},nt=new Proxy(function(){},{get(t,k,r){if(k==='prototype')throw error;return Reflect.get(t,k,r);}});try{Reflect.construct(Set,[],nt);valid=false;}catch(e){valid=valid && e===error;}valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
}
