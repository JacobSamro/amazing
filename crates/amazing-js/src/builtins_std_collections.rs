// Keyed collections and the ES2025 Set operations.

fn std_collection_method(i: &Interpreter, name: &str, length: u32, native: crate::value::NativeFn) -> JsValue {
    let method = symbol_method(i, name, native);
    symbol_method_length(&method, length);
    method
}

fn std_collection_key(value: &JsValue) -> MapKey {
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
        let map = Self::native("Map", std_map_constructor, &self.function_proto);
        let set = Self::native("Set", std_set_constructor, &self.function_proto);
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
        for proto in [&self.map_proto, &self.set_proto] {
            let getter = std_collection_method(self, "get size", 0, std_collection_size);
            proto.borrow_mut().define("size", Property {
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: callable_function(&getter).map(Box::new), setter: None,
            }).unwrap();
        }
        self.define_global("Map", map.clone());
        self.define_global("Set", set.clone());
        self.global_obj.borrow_mut().define("Map", symbol_data(map.clone(), true, true)).unwrap();
        self.global_obj.borrow_mut().define("Set", symbol_data(set.clone(), true, true)).unwrap();
        for (name, length, function) in [
            ("get", 1, std_map_get as crate::value::NativeFn),
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
    }
}

fn std_collection_size(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    match &this {
        JsValue::Object(object) => match &object.borrow().exotic {
            Exotic::Map(state) => Ok(JsValue::num(state.entries.len() as f64)),
            Exotic::Set(state) => Ok(JsValue::num(state.entries.len() as f64)),
            _ => Err(Throw::type_error("incompatible collection receiver")),
        },
        _ => Err(Throw::type_error("incompatible collection receiver")),
    }
}

fn std_map_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Map requires new")); }
    let map = std_collection_map_new(i);
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            let adder = i.get_prop(&JsValue::Object(map.clone()), "set")?;
            if !std_callable(&adder) { return Err(Throw::type_error("Map set is not callable")); }
            for entry in i.iterate(source)? {
                if !entry.is_object() { return Err(Throw::type_error("Map iterator value is not an object")); }
                let key = i.get_prop(&entry, "0")?;
                let value = i.get_prop(&entry, "1")?;
                i.call_value(&adder, JsValue::Object(map.clone()), vec![key, value])?;
            }
        }
    }
    Ok(JsValue::Object(map))
}

fn std_set_constructor(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Set requires new")); }
    let set = std_collection_set_new(i);
    if let Some(source) = args.first() {
        if !source.is_null() && !source.is_undefined() {
            let adder = i.get_prop(&JsValue::Object(set.clone()), "add")?;
            if !std_callable(&adder) { return Err(Throw::type_error("Set add is not callable")); }
            for value in i.iterate(source)? {
                i.call_value(&adder, JsValue::Object(set.clone()), vec![value])?;
            }
        }
    }
    Ok(JsValue::Object(set))
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
fn std_map_delete(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    let mut object = map.borrow_mut();
    let removed = if let Exotic::Map(state) = &mut object.exotic {
        if let Some(index) = state.entries.iter().position(|(candidate, _)| candidate.same(&key)) {
            state.entries.remove(index); Some(index)
        } else { None }
    } else { None };
    drop(object);
    if let Some(index) = removed { std_collection_iter_removed(&map, Some(index)); }
    Ok(JsValue::Bool(removed.is_some()))
}
fn std_map_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let map = std_collection_map(&this)?;
    if let Exotic::Map(state) = &mut map.borrow_mut().exotic { state.entries.clear(); }
    std_collection_iter_removed(&map, None);
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
    let map = std_collection_map(&this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let this_arg = std_reflect_arg(&args, 1);
    let mut index = 0;
    loop {
        let next = if let Exotic::Map(state) = &map.borrow().exotic { state.entries.get(index).map(|(key, value)| (key_value(key), value.clone())) } else { None };
        let Some((key, value)) = next else { break };
        i.call_value(&callback, this_arg.clone(), vec![value, key, this.clone()])?;
        index += 1;
    }
    Ok(JsValue::Undefined)
}
fn std_map_group_by(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = std_reflect_arg(&args, 0);
    let callback = std_reflect_arg(&args, 1);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let map = std_collection_map_new(i);
    for (index, value) in i.iterate(&source)?.into_iter().enumerate() {
        let key = i.call_value(&callback, JsValue::Undefined, vec![value.clone(), JsValue::num(index as f64)])?;
        let old = std_map_get(i, JsValue::Object(map.clone()), vec![key.clone()])?;
        let group = if old.is_undefined() { array_values(i, Vec::new())? } else { old };
        let length = i.array_length(&group);
        i.set_prop(&group, length.to_string(), value)?;
        std_collection_map_put(&map, key, group);
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
fn std_set_delete(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let key = std_collection_key(&std_reflect_arg(&args, 0));
    let mut object = set.borrow_mut();
    let removed = if let Exotic::Set(state) = &mut object.exotic {
        if let Some(index) = state.entries.iter().position(|candidate| candidate.same(&key)) { state.entries.remove(index); Some(index) } else { None }
    } else { None };
    drop(object);
    if let Some(index) = removed { std_collection_iter_removed(&set, Some(index)); }
    Ok(JsValue::Bool(removed.is_some()))
}
fn std_set_clear(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    if let Exotic::Set(state) = &mut set.borrow_mut().exotic { state.entries.clear(); }
    std_collection_iter_removed(&set, None);
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
    let set = std_collection_set(&this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("callback is not callable")); }
    let this_arg = std_reflect_arg(&args, 1);
    let mut index = 0;
    loop {
        let next = if let Exotic::Set(state) = &set.borrow().exotic { state.entries.get(index).map(key_value) } else { None };
        let Some(value) = next else { break };
        i.call_value(&callback, this_arg.clone(), vec![value.clone(), value, this.clone()])?;
        index += 1;
    }
    Ok(JsValue::Undefined)
}

struct StdSetRecord { object: JsValue, size: f64, has: JsValue, keys: JsValue }

fn std_set_record(i: &mut Interpreter, value: JsValue) -> Result<StdSetRecord, Throw> {
    if !value.is_object() { return Err(Throw::type_error("set-like argument must be an object")); }
    let size_value = i.get_prop(&value, "size")?;
    let size = i.to_number(&size_value)?;
    if size.is_nan() { return Err(Throw::type_error("set-like size is NaN")); }
    if size < 0.0 { return Err(Throw::range_error("set-like size is negative")); }
    let has = i.get_prop(&value, "has")?;
    let keys = i.get_prop(&value, "keys")?;
    if !std_callable(&has) || !std_callable(&keys) { return Err(Throw::type_error("set-like methods must be callable")); }
    Ok(StdSetRecord { object: value, size: size.trunc(), has, keys })
}

fn std_set_record_has(i: &mut Interpreter, record: &StdSetRecord, value: JsValue) -> Result<bool, Throw> {
    Ok(i.call_value(&record.has, record.object.clone(), vec![value])?.to_boolean())
}

fn std_set_record_values(i: &mut Interpreter, record: &StdSetRecord) -> Result<Vec<JsValue>, Throw> {
    let iterator = i.call_value(&record.keys, record.object.clone(), Vec::new())?;
    let iterator = i.std_iterator_direct(iterator)?;
    let mut values = Vec::new();
    while let Some(value) = std_iterator_step(i, &iterator)? { values.push(value); }
    Ok(values)
}

fn std_set_snapshot(set: &Obj) -> Vec<JsValue> {
    if let Exotic::Set(state) = &set.borrow().exotic { state.entries.iter().map(key_value).collect() } else { Vec::new() }
}

fn std_set_new_from(i: &Interpreter, values: impl IntoIterator<Item = JsValue>) -> Obj {
    let set = std_collection_set_new(i);
    for value in values { std_collection_set_add(&set, value); }
    set
}

fn std_set_union(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let result = std_set_new_from(i, std_set_snapshot(&set));
    for value in std_set_record_values(i, &record)? { std_collection_set_add(&result, value); }
    Ok(JsValue::Object(result))
}

fn std_set_intersection(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let values = std_set_snapshot(&set);
    let result = std_collection_set_new(i);
    if values.len() as f64 <= record.size {
        for value in values { if std_set_record_has(i, &record, value.clone())? { std_collection_set_add(&result, value); } }
    } else {
        for value in std_set_record_values(i, &record)? {
            let key = std_collection_key(&value);
            if matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key))) {
                std_collection_set_add(&result, value);
            }
        }
    }
    Ok(JsValue::Object(result))
}

fn std_set_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let values = std_set_snapshot(&set);
    let result = std_set_new_from(i, values.clone());
    if values.len() as f64 <= record.size {
        for value in values { if std_set_record_has(i, &record, value.clone())? {
            let key = std_collection_key(&value);
            if let Exotic::Set(state) = &mut result.borrow_mut().exotic { state.entries.retain(|candidate| !candidate.same(&key)); }
        } }
    } else {
        for value in std_set_record_values(i, &record)? {
            let key = std_collection_key(&value);
            if let Exotic::Set(state) = &mut result.borrow_mut().exotic { state.entries.retain(|candidate| !candidate.same(&key)); }
        }
    }
    Ok(JsValue::Object(result))
}

fn std_set_symmetric_difference(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let result = std_set_new_from(i, std_set_snapshot(&set));
    for value in std_set_record_values(i, &record)? {
        let key = std_collection_key(&value);
        let originally_present = matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key)));
        if originally_present {
            if let Exotic::Set(state) = &mut result.borrow_mut().exotic { state.entries.retain(|candidate| !candidate.same(&key)); }
        } else { std_collection_set_add(&result, value); }
    }
    Ok(JsValue::Object(result))
}

fn std_set_is_subset_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let values = std_set_snapshot(&set);
    if values.len() as f64 > record.size { return Ok(JsValue::Bool(false)); }
    for value in values { if !std_set_record_has(i, &record, value)? { return Ok(JsValue::Bool(false)); } }
    Ok(JsValue::Bool(true))
}

fn std_set_is_superset_of(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    if (std_set_snapshot(&set).len() as f64) < record.size { return Ok(JsValue::Bool(false)); }
    for value in std_set_record_values(i, &record)? {
        let key = std_collection_key(&value);
        if !matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key))) {
            return Ok(JsValue::Bool(false));
        }
    }
    Ok(JsValue::Bool(true))
}

fn std_set_is_disjoint_from(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let set = std_collection_set(&this)?;
    let record = std_set_record(i, std_reflect_arg(&args, 0))?;
    let values = std_set_snapshot(&set);
    if values.len() as f64 <= record.size {
        for value in values { if std_set_record_has(i, &record, value)? { return Ok(JsValue::Bool(false)); } }
    } else {
        for value in std_set_record_values(i, &record)? {
            let key = std_collection_key(&value);
            if matches!(&set.borrow().exotic, Exotic::Set(state) if state.entries.iter().any(|candidate| candidate.same(&key))) {
                return Ok(JsValue::Bool(false));
            }
        }
    }
    Ok(JsValue::Bool(true))
}

type StdWeakObject = Weak<RefCell<JsObject>>;
struct StdWeakMapData { owner: StdWeakObject, entries: Vec<(StdWeakObject, JsValue)> }
struct StdWeakSetData { owner: StdWeakObject, entries: Vec<StdWeakObject> }
struct StdWeakRefData { owner: StdWeakObject, target: StdWeakObject }
struct StdFinalizationData {
    owner: StdWeakObject,
    _callback: JsValue,
    registrations: Vec<(StdWeakObject, JsValue, Option<JsValue>)>,
}
thread_local! {
    static STD_WEAK_MAPS: RefCell<HashMap<usize, StdWeakMapData>> = RefCell::new(HashMap::new());
    static STD_WEAK_SETS: RefCell<HashMap<usize, StdWeakSetData>> = RefCell::new(HashMap::new());
    static STD_WEAK_REFS: RefCell<HashMap<usize, StdWeakRefData>> = RefCell::new(HashMap::new());
    static STD_FINALIZATION: RefCell<HashMap<usize, StdFinalizationData>> = RefCell::new(HashMap::new());
}

fn std_weak_id(object: &Obj) -> usize { Rc::as_ptr(&object.0) as usize }
fn std_weak_key(value: &JsValue) -> Option<Obj> {
    if let JsValue::Object(object) = value { Some(object.clone()) } else { None }
}
fn std_weak_same(a: &StdWeakObject, b: &Obj) -> bool {
    a.upgrade().is_some_and(|object| Rc::ptr_eq(&object, &b.0))
}

impl Interpreter {
    fn install_std_weak_collections(&mut self) {
        for (name, constructor, methods) in [
            ("WeakMap", std_weak_map_constructor as crate::value::NativeFn,
                vec![("get", 1, std_weak_map_get as crate::value::NativeFn), ("set", 2, std_weak_map_set),
                     ("has", 1, std_weak_map_has), ("delete", 1, std_weak_map_delete)]),
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
            let constructor = Self::native(name, constructor, &self.function_proto);
            if let JsValue::Object(object) = &constructor {
                object.borrow_mut().define("name", symbol_data(JsValue::str(name), false, true)).unwrap();
                object.borrow_mut().define("length", symbol_data(JsValue::num(if name == "FinalizationRegistry" { 1.0 } else { 0.0 }), false, true)).unwrap();
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
        for entry in i.iterate(&source)? {
            if !entry.is_object() { return Err(Throw::type_error("WeakMap iterator value is not an object")); }
            let key = i.get_prop(&entry, "0")?;
            let value = i.get_prop(&entry, "1")?;
            i.call_value(&adder, this.clone(), vec![key, value])?;
        }
    }
    Ok(this)
}

fn std_weak_map_get(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(&std_reflect_arg(&args, 0)) else { return Ok(JsValue::Undefined); };
    Ok(STD_WEAK_MAPS.with(|registry| registry.borrow().get(&std_weak_id(&owner)).and_then(|data|
        data.entries.iter().find(|(candidate, _)| std_weak_same(candidate, &key)).map(|(_, value)| value.clone()))
        .unwrap_or(JsValue::Undefined)))
}
fn std_weak_map_set(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let key = std_weak_key(&std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakMap key must be an object"))?;
    let value = std_reflect_arg(&args, 1);
    STD_WEAK_MAPS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        data.entries.retain(|(candidate, _)| candidate.upgrade().is_some());
        if let Some((_, old)) = data.entries.iter_mut().find(|(candidate, _)| std_weak_same(candidate, &key)) {
            *old = value;
        } else { data.entries.push((Rc::downgrade(&key.0), value)); }
    });
    Ok(this)
}
fn std_weak_map_has(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(&std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    Ok(JsValue::Bool(STD_WEAK_MAPS.with(|registry| registry.borrow().get(&std_weak_id(&owner))
        .is_some_and(|data| data.entries.iter().any(|(candidate, _)| std_weak_same(candidate, &key))))))
}
fn std_weak_map_delete(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_map_owner(&this)?;
    let Some(key) = std_weak_key(&std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    let deleted = STD_WEAK_MAPS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.entries.len();
        data.entries.retain(|(candidate, _)| !std_weak_same(candidate, &key));
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
        for value in i.iterate(&source)? { i.call_value(&adder, this.clone(), vec![value])?; }
    }
    Ok(this)
}
fn std_weak_set_add(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let key = std_weak_key(&std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakSet value must be an object"))?;
    STD_WEAK_SETS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        data.entries.retain(|candidate| candidate.upgrade().is_some());
        if !data.entries.iter().any(|candidate| std_weak_same(candidate, &key)) {
            data.entries.push(Rc::downgrade(&key.0));
        }
    });
    Ok(this)
}
fn std_weak_set_has(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let Some(key) = std_weak_key(&std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    Ok(JsValue::Bool(STD_WEAK_SETS.with(|registry| registry.borrow().get(&std_weak_id(&owner))
        .is_some_and(|data| data.entries.iter().any(|candidate| std_weak_same(candidate, &key))))))
}
fn std_weak_set_delete(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_weak_set_owner(&this)?;
    let Some(key) = std_weak_key(&std_reflect_arg(&args, 0)) else { return Ok(JsValue::Bool(false)); };
    let deleted = STD_WEAK_SETS.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.entries.len();
        data.entries.retain(|candidate| !std_weak_same(candidate, &key));
        data.entries.len() != old
    });
    Ok(JsValue::Bool(deleted))
}

fn std_weak_ref_constructor(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("WeakRef requires new")); };
    let target = std_weak_key(&std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("WeakRef target must be an object"))?;
    STD_WEAK_REFS.with(|registry| registry.borrow_mut().insert(std_weak_id(owner),
        StdWeakRefData { owner: Rc::downgrade(&owner.0), target: Rc::downgrade(&target.0) }));
    Ok(this)
}
fn std_weak_ref_deref(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible WeakRef receiver")); };
    STD_WEAK_REFS.with(|registry| registry.borrow().get(&std_weak_id(owner))
        .filter(|data| std_weak_same(&data.owner, owner))
        .map(|data| data.target.upgrade().map(|target| JsValue::Object(Obj(target))).unwrap_or(JsValue::Undefined))
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
fn std_finalization_register(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_finalization_owner(&this)?;
    let target = std_weak_key(&std_reflect_arg(&args, 0)).ok_or_else(|| Throw::type_error("target must be an object"))?;
    let held = std_reflect_arg(&args, 1);
    if std_same_value(&JsValue::Object(target.clone()), &held) {
        return Err(Throw::type_error("target and holdings must differ"));
    }
    let token = args.get(2).filter(|value| !value.is_undefined()).cloned();
    if token.as_ref().is_some_and(|value| std_weak_key(value).is_none()) {
        return Err(Throw::type_error("unregister token must be an object"));
    }
    STD_FINALIZATION.with(|registry| registry.borrow_mut().get_mut(&std_weak_id(&owner)).unwrap()
        .registrations.push((Rc::downgrade(&target.0), held, token)));
    Ok(JsValue::Undefined)
}
fn std_finalization_unregister(_: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let owner = std_finalization_owner(&this)?;
    let token = std_reflect_arg(&args, 0);
    if std_weak_key(&token).is_none() { return Err(Throw::type_error("unregister token must be an object")); }
    let removed = STD_FINALIZATION.with(|registry| {
        let mut registry = registry.borrow_mut();
        let data = registry.get_mut(&std_weak_id(&owner)).unwrap();
        let old = data.registrations.len();
        data.registrations.retain(|(_, _, candidate)| !candidate.as_ref().is_some_and(|value| std_same_value(value, &token)));
        data.registrations.len() != old
    });
    Ok(JsValue::Bool(removed))
}
