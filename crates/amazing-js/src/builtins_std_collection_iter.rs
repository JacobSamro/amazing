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
        for (prototype, tag) in [(&map, "Map Iterator"), (&set, "Set Iterator")] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, std_collection_iter_next), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::Iterator.symbol(), symbol_data(std_collection_method(self, "[Symbol.iterator]", 0, std_collection_iter_self), true, true)).unwrap();
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
        let object = Obj(Rc::new(RefCell::new(object)));
        self.std_collection_iterator_states.borrow_mut().insert(std_weak_id(&object),
            StdCollectionIterData { owner: Rc::downgrade(&object.0), target, cursor: 0, finished: false, kind });
        JsValue::Object(object)
    }
}

fn std_collection_iter_self(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let valid = i.std_collection_iterator_states.borrow().get(&std_weak_id(object))
        .is_some_and(|data| std_weak_same(&data.owner, object));
    if valid { Ok(this) } else { Err(Throw::type_error("incompatible iterator receiver")) }
}

fn std_collection_iter_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible iterator receiver")); };
    let (target, cursor, kind, finished) = {
        let mut registry = i.std_collection_iterator_states.borrow_mut();
        let data = registry.get_mut(&std_weak_id(owner)).filter(|data| std_weak_same(&data.owner, owner))
            .ok_or_else(|| Throw::type_error("incompatible iterator receiver"))?;
        let current = (data.target.clone(), data.cursor, data.kind, data.finished);
        if !data.finished { data.cursor = data.cursor.saturating_add(1); }
        current
    };
    if finished { return Ok(sequence_result(i, JsValue::Undefined, true)); }
    let value = match kind {
        StdCollectionIterKind::MapKeys | StdCollectionIterKind::MapValues | StdCollectionIterKind::MapEntries => {
            let entry = if let Exotic::Map(state) = &target.borrow().exotic { state.entries.get(cursor).cloned() } else { None };
            match entry {
                Some((key, value)) => Some(match kind {
                    StdCollectionIterKind::MapKeys => key_value(&key),
                    StdCollectionIterKind::MapValues => value,
                    StdCollectionIterKind::MapEntries => array_values(i, vec![key_value(&key), value])?,
                    _ => unreachable!(),
                }),
                None => None,
            }
        }
        StdCollectionIterKind::SetValues | StdCollectionIterKind::SetEntries => {
            let entry = if let Exotic::Set(state) = &target.borrow().exotic { state.entries.get(cursor).cloned() } else { None };
            match entry {
                Some(key) => { let value = key_value(&key);
                    Some(if matches!(kind, StdCollectionIterKind::SetEntries) { array_values(i, vec![value.clone(), value])? } else { value })
                }
                None => None,
            }
        }
    };
    if value.is_none() {
        if let Some(data) = i.std_collection_iterator_states.borrow_mut().get_mut(&std_weak_id(owner)) { data.finished = true; }
    }
    Ok(sequence_result(i, value.clone().unwrap_or(JsValue::Undefined), value.is_none()))
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
