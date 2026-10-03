// Portions ported from boa-dev/boa (core/engine/src/builtins/{map,set}/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Existing actor-owned deletion-adjusted cursors replace upstream ordered-list locks.
// Live Map and Set iterators, with prototype objects per realm.

#[derive(Clone, Copy)]
enum StdCollectionIterKind { MapKeys, MapValues, MapEntries, SetValues, SetEntries }
struct StdCollectionIterData {
    owner: Weak<RefCell<JsObject>>,
    target: Obj,
    cursor: usize,
    finished: bool,
    kind: StdCollectionIterKind,
}
struct StdCollectionIterPrototypes {
    map: Obj,
    set: Obj,
}
impl Interpreter {
    fn std_collection_iter_set_base(&self, base: &Obj) {
        if let Some(prototypes) = self.std_collection_iterator_prototypes.as_ref() {
            prototypes.map.borrow_mut().proto = Some(base.clone());
            prototypes.set.borrow_mut().proto = Some(base.clone());
        }
    }

    fn install_std_collection_iterators(&mut self) {
        let mut map = JsObject::ordinary();
        map.proto = Some(self.object_proto.clone());
        let map = Obj(Rc::new(RefCell::new(map)));
        let mut set = JsObject::ordinary();
        set.proto = Some(self.object_proto.clone());
        let set = Obj(Rc::new(RefCell::new(set)));
        for (prototype, tag, native) in [(&map, "Map Iterator", std_map_iter_next as crate::value::NativeFn), (&set, "Set Iterator", std_set_iter_next as crate::value::NativeFn)] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, native), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(tag), false, true)).unwrap();
        }
        self.std_collection_iterator_prototypes = Some(Rc::new(
            StdCollectionIterPrototypes { map, set }));
    }

    fn std_collection_iterator(&self, target: Obj, kind: StdCollectionIterKind) -> JsValue {
        let prototype = self.std_collection_iterator_prototypes.as_ref()
            .map(|data| if matches!(kind, StdCollectionIterKind::MapKeys | StdCollectionIterKind::MapValues | StdCollectionIterKind::MapEntries) { data.map.clone() } else { data.set.clone() });
        let mut object = JsObject::ordinary();
        object.proto = prototype;
        object.realm = Some(self.host_realm);
        let object = Obj(Rc::new(RefCell::new(object)));
        self.std_collection_iterator_states.borrow_mut().insert(std_weak_id(&object),
            StdCollectionIterData { owner: Rc::downgrade(&object.0), target, cursor: 0, finished: false, kind });
        JsValue::Object(object)
    }
}

// Internal consumers never call a mutable public next method or expose this cursor.
fn std_collection_cursor_entry(i: &Interpreter, this: &JsValue) -> Result<Option<(JsValue, JsValue)>, Throw> {
    let JsValue::Object(owner) = this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let (target, cursor, finished) = {
        let mut registry = i.std_collection_iterator_states.borrow_mut();
        let data = registry.get_mut(&std_weak_id(owner)).filter(|data| std_weak_same(&data.owner, owner))
            .ok_or_else(|| Throw::type_error("incompatible iterator receiver"))?;
        let current = (data.target.clone(), data.cursor, data.finished);
        if !data.finished { data.cursor = data.cursor.saturating_add(1); }
        current
    };
    if finished { return Ok(None); }
    let value = match &target.borrow().exotic {
        Exotic::Map(state) => state.entries.get(cursor).map(|(key, value)| (key_value(key), value.clone())),
        Exotic::Set(state) => state.entries.get(cursor).map(|key| { let value = key_value(key); (value.clone(), value) }),
        _ => None,
    };
    if value.is_none() {
        if let Some(data) = i.std_collection_iterator_states.borrow_mut().get_mut(&std_weak_id(owner)) { data.finished = true; }
    }
    Ok(value)
}
fn std_collection_cursor_release(i: &Interpreter, this: &JsValue) {
    if let JsValue::Object(owner) = this { i.std_collection_iterator_states.borrow_mut().remove(&std_weak_id(owner)); }
}
fn std_collection_entry_array(i: &mut Interpreter, key: JsValue, value: JsValue) -> Result<JsValue, Throw> {
    let array = array_values(i, Vec::new())?;
    array_create_data_property(i, &array, 0, key)?;
    array_create_data_property(i, &array, 1, value)?;
    Ok(array)
}
fn std_map_iter_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_collection_next(i, this, true)
}
fn std_set_iter_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_collection_next(i, this, false)
}
fn std_collection_next(i: &mut Interpreter, this: JsValue, map: bool) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let (kind, finished) = i.std_collection_iterator_states.borrow().get(&std_weak_id(owner))
        .filter(|data| std_weak_same(&data.owner, owner)).map(|data| (data.kind, data.finished))
        .ok_or_else(|| Throw::type_error("incompatible iterator receiver"))?;
    let is_map = matches!(kind, StdCollectionIterKind::MapKeys | StdCollectionIterKind::MapValues | StdCollectionIterKind::MapEntries);
    if map != is_map { return Err(Throw::type_error("incompatible collection iterator receiver")); }
    if finished { return Ok(sequence_result(i, JsValue::Undefined, true)); }
    let realm = RealmHandle { agent: i.agent_id, realm: owner.borrow().realm.unwrap_or(i.host_realm) };
    // The active generator body, including its first terminal result, resumes
    // in the creation realm. Already-completed results above use the callee.
    i.binding_with_realm(realm, |i| {
        let result = (|| {
            let Some((key, value)) = std_collection_cursor_entry(i, &this)? else {
                return Ok(sequence_result(i, JsValue::Undefined, true));
            };
            let value = match kind {
                StdCollectionIterKind::MapKeys => key,
                StdCollectionIterKind::MapValues | StdCollectionIterKind::SetValues => value,
                StdCollectionIterKind::MapEntries | StdCollectionIterKind::SetEntries => std_collection_entry_array(i, key, value)?,
            };
            Ok(sequence_result(i, value, false))
        })();
        result.map_err(|error| i.normalize_throw(error))
    })
}

fn std_collection_iter_removed(i: &Interpreter, target: &Obj, index: Option<usize>) {
    for data in i.std_collection_iterator_states.borrow_mut().values_mut() {
        if !Rc::ptr_eq(&data.target.0, &target.0) { continue; }
        match index {
            Some(index) if index < data.cursor => data.cursor -= 1,
            None => data.cursor = 0,
            _ => (),
        }
    }
}
