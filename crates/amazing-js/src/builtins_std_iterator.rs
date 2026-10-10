// Portions ported from boa-dev/boa (core/engine/src/builtins/iterable/
// wrap_for_valid_iterator.rs, iterator_constructor.rs, iterator_prototype.rs,
// iterator_helper/{mod,map,filter,take,drop,flat_map,zip}.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Whole synchronous wrapper/helper/zip protocols adapted to Amazing's existing
// interpreter-owned weak-identity states; no coroutine or GC dependencies.
// Current ECMA-262 adaptation: preserve the normative finite take/drop limit
// cap and saturate Number callback counters at 2^53.
// Iterator constructor, direct-iterator wrappers, and lazy helpers.

#[derive(Clone)]
struct StdIteratorRecord { iterator: JsValue, next: JsValue }

#[derive(Clone)]
struct StdIteratorConcatSource { iterable: JsValue, method: JsValue }

#[derive(Clone)]
enum StdIteratorOperation {
    Wrap,
    Map(JsValue), Filter(JsValue), Take(f64), Drop(f64), FlatMap(JsValue),
    Concat { sources: Vec<StdIteratorConcatSource>, position: usize },
    Chunks { size: usize, buffer: Vec<JsValue> },
    Windows { size: usize, partial: bool, buffer: Vec<JsValue> },
    Zip { records: Vec<StdIteratorRecord>, keys: Option<Vec<PropertyKey>>,
        mode: String, padding: Vec<JsValue>, exhausted: Vec<bool> },
}

#[derive(Clone)]
struct StdIteratorState {
    owner: Weak<RefCell<JsObject>>,
    source: Option<StdIteratorRecord>,
    operation: StdIteratorOperation,
    index: u64,
    inner: Option<StdIteratorRecord>,
    done: bool,
    started: bool,
    executing: bool,
}

struct StdIteratorPrototypes {
    base: Obj,
    helper: Obj,
    wrap: Obj,
    array: Obj,
    string: Obj,
}

impl Interpreter {
    fn install_std_iterator_helpers(&mut self) {
        let mut base = JsObject::ordinary();
        base.proto = Some(self.object_proto.clone());
        let base = Obj(Rc::new(RefCell::new(base)));
        self.std_collection_iter_set_base(&base);
        self.regexp_string_iterator_proto.borrow_mut().proto = Some(base.clone());
        let mut helper = JsObject::ordinary();
        helper.proto = Some(base.clone());
        let helper = Obj(Rc::new(RefCell::new(helper)));
        let mut wrap = JsObject::ordinary();
        wrap.proto = Some(base.clone());
        let wrap = Obj(Rc::new(RefCell::new(wrap)));
        let mut array = JsObject::ordinary();
        array.proto = Some(base.clone());
        let array = Obj(Rc::new(RefCell::new(array)));
        let mut string = JsObject::ordinary();
        string.proto = Some(base.clone());
        let string = Obj(Rc::new(RefCell::new(string)));
        let constructor = Self::native_constructor("Iterator", std_iterator_constructor, &self.function_proto);
        if let JsValue::Object(object) = &constructor {
            object.borrow_mut().define("name", symbol_data(JsValue::str("Iterator"), false, true)).unwrap();
            object.borrow_mut().define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
            object.borrow_mut().define("prototype", symbol_data(JsValue::Object(base.clone()), false, false)).unwrap();
            for (name, length, native) in [
                ("from", 1, std_iterator_from as crate::value::NativeFn),
                ("concat", 0, std_iterator_concat as crate::value::NativeFn),
                ("zip", 1, std_iterator_zip as crate::value::NativeFn),
                ("zipKeyed", 1, std_iterator_zip_keyed as crate::value::NativeFn),
            ] {
                object.borrow_mut().define(name,
                    symbol_data(std_collection_method(self, name, length, native), true, true)).unwrap();
            }
        }
        for (key, getter_name, getter, setter_name, setter) in [
            (PropertyKey::from("constructor"), "get constructor", std_iterator_constructor_get as crate::value::NativeFn,
                "set constructor", std_iterator_constructor_set as crate::value::NativeFn),
            (WellKnownSymbol::ToStringTag.symbol().into(), "get [Symbol.toStringTag]", std_iterator_tag_get as crate::value::NativeFn,
                "set [Symbol.toStringTag]", std_iterator_tag_set as crate::value::NativeFn),
        ] {
            let getter = Self::native(getter_name, getter, &self.function_proto);
            let setter = Self::native(setter_name, setter, &self.function_proto);
            // Accessors store the callable object itself (JSRT callable identity).
            base.borrow_mut().define(key, Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: Some(getter),
                setter: Some(setter),
            }).unwrap();
        }
        base.borrow_mut().define(WellKnownSymbol::Iterator.symbol(),
            symbol_data(std_collection_method(self, "[Symbol.iterator]", 0, std_iterator_self), true, true)).unwrap();
        base.borrow_mut().define(WellKnownSymbol::Dispose.symbol(),
            symbol_data(std_collection_method(self, "[Symbol.dispose]", 0, std_iterator_dispose), true, true)).unwrap();
        for (name, length, native) in [
            ("map", 1, std_iterator_map as crate::value::NativeFn),
            ("filter", 1, std_iterator_filter as crate::value::NativeFn),
            ("take", 1, std_iterator_take as crate::value::NativeFn),
            ("drop", 1, std_iterator_drop as crate::value::NativeFn),
            ("flatMap", 1, std_iterator_flat_map as crate::value::NativeFn),
            ("reduce", 1, std_iterator_reduce as crate::value::NativeFn),
            ("toArray", 0, std_iterator_to_array as crate::value::NativeFn),
            ("join", 1, std_iterator_join as crate::value::NativeFn),
            ("forEach", 1, std_iterator_for_each as crate::value::NativeFn),
            ("some", 1, std_iterator_some as crate::value::NativeFn),
            ("every", 1, std_iterator_every as crate::value::NativeFn),
            ("find", 1, std_iterator_find as crate::value::NativeFn),
            ("includes", 1, std_iterator_includes as crate::value::NativeFn),
            ("chunks", 1, std_iterator_chunks as crate::value::NativeFn),
            ("windows", 1, std_iterator_windows as crate::value::NativeFn),
        ] {
            base.borrow_mut().define(name, symbol_data(std_collection_method(self, name, length, native), true, true)).unwrap();
        }
        for (prototype, next, ret) in [
            (&helper, std_iterator_next as crate::value::NativeFn, std_iterator_return as crate::value::NativeFn),
            (&wrap, std_iterator_wrap_next as crate::value::NativeFn, std_iterator_wrap_return as crate::value::NativeFn),
        ] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, next), true, true)).unwrap();
            prototype.borrow_mut().define("return", symbol_data(std_collection_method(self, "return", 0, ret), true, true)).unwrap();
        }
        helper.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str("Iterator Helper"), false, true)).unwrap();
        for (prototype, tag, next) in [(&array, "Array Iterator", native_array_sequence_iterator_next as crate::value::NativeFn), (&string, "String Iterator", native_string_sequence_iterator_next as crate::value::NativeFn)] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, next), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(tag), false, true)).unwrap();
        }
        self.std_iterator_prototypes = Some(Rc::new(
            StdIteratorPrototypes { base, helper, wrap, array, string }));
        self.define_global("Iterator", constructor);
        if let Some(value) = self.global_env.resolve("Iterator") {
            self.global_obj.borrow_mut().define("Iterator", symbol_data(value, true, true)).unwrap();
        }
    }

    fn std_iterator_prototypes(&self) -> Option<(Obj, Obj, Obj)> {
        self.std_iterator_prototypes.as_ref()
            .map(|data| (data.base.clone(), data.helper.clone(), data.wrap.clone()))
    }

    fn std_sequence_iterator_proto(&self, kind: SequenceIteratorKind) -> Option<Obj> {
        self.std_iterator_prototypes.as_ref()
            .map(|data| if kind == SequenceIteratorKind::String { data.string.clone() } else { data.array.clone() })
    }

    fn std_iterator_object(&self, source: Option<StdIteratorRecord>, operation: StdIteratorOperation, wrapped: bool) -> JsValue {
        let prototype = self.std_iterator_prototypes().map(|(_, helper, wrap)| if wrapped { wrap } else { helper });
        let mut object = JsObject::ordinary();
        object.proto = prototype;
        if !wrapped { object.realm = Some(self.host_realm); }
        let object = Obj(Rc::new(RefCell::new(object)));
        self.std_iterator_states.borrow_mut().insert(std_weak_id(&object),
            StdIteratorState { owner: Rc::downgrade(&object.0), source, operation, index: 0,
                inner: None, done: false, started: false, executing: false });
        JsValue::Object(object)
    }

    fn std_iterator_direct(&mut self, iterator: JsValue) -> Result<StdIteratorRecord, Throw> {
        if !iterator.is_object() { return Err(Throw::type_error("iterator must be an object")); }
        let next = self.get_prop(&iterator, "next")?;
        Ok(StdIteratorRecord { iterator, next })
    }

    fn std_iterator_from_value(&mut self, value: JsValue) -> Result<StdIteratorRecord, Throw> {
        self.std_iterator_flattenable(value, true)
    }

    fn std_iterator_flattenable(&mut self, value: JsValue, iterate_strings: bool) -> Result<StdIteratorRecord, Throw> {
        if !value.is_object() && !(iterate_strings && matches!(value, JsValue::Str(_))) {
            return Err(Throw::type_error("value is not iterable"));
        }
        let method = self.get_prop(&value, WellKnownSymbol::Iterator.symbol())?;
        if method.is_undefined() || method.is_null() { return self.std_iterator_direct(value); }
        if !std_callable(&method) { return Err(Throw::type_error("iterator method is not callable")); }
        let iterator = self.call_value(&method, value, Vec::new())?;
        self.std_iterator_direct(iterator)
    }
}

fn std_iterator_constructor(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = &this {
        if let Some((base, _, _)) = i.std_iterator_prototypes() {
            if object.borrow().proto.as_ref().is_some_and(|prototype| !Rc::ptr_eq(&prototype.0, &base.0)) {
                return Ok(this);
            }
        }
    }
    Err(Throw::type_error("Iterator is an abstract constructor"))
}

fn std_iterator_constructor_get(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(i.global_env.resolve("Iterator").unwrap_or(JsValue::Undefined))
}

fn std_iterator_tag_get(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::str("Iterator"))
}

fn std_iterator_accessor_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, key: PropertyKey) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = &this else { return Err(Throw::type_error("Iterator setter requires an object")); };
    if let Some((base, _, _)) = i.std_iterator_prototypes() {
        if Rc::ptr_eq(&object.0, &base.0) { return Err(Throw::type_error("cannot assign Iterator prototype property")); }
    }
    let value = std_reflect_arg(&args, 0);
    if object.borrow().own_prop(&key).is_some() { i.set_prop(&this, key, value)?; }
    else { object.borrow_mut().define(key, Property::data(value)).map_err(|_| Throw::type_error("cannot define Iterator property"))?; }
    Ok(JsValue::Undefined)
}

fn std_iterator_constructor_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_iterator_accessor_set(i, this, args, PropertyKey::from("constructor"))
}

fn std_iterator_tag_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_iterator_accessor_set(i, this, args, WellKnownSymbol::ToStringTag.symbol().into())
}

fn std_iterator_self(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
}

fn std_iterator_dispose(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Iterator.prototype[Symbol.dispose] requires an object")); }
    let method = i.get_prop(&this, "return")?;
    if !method.is_undefined() && !method.is_null() {
        if !std_callable(&method) { return Err(Throw::type_error("iterator return is not callable")); }
        i.call_value(&method, this, Vec::new())?;
    }
    Ok(JsValue::Undefined)
}

fn std_iterator_from(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let value = std_reflect_arg(&args, 0);
    let record = i.std_iterator_from_value(value)?;
    if let JsValue::Object(object) = &record.iterator {
        if let Some((base, _, _)) = i.std_iterator_prototypes() {
            let mut prototype = i.default_get_prototype(object)?;
            while let JsValue::Object(current) = prototype {
                if Rc::ptr_eq(&current.0, &base.0) { return Ok(record.iterator); }
                prototype = i.default_get_prototype(&current)?;
            }
        }
    }
    Ok(i.std_iterator_object(Some(record), StdIteratorOperation::Wrap, true))
}

fn std_iterator_concat(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let mut sources = Vec::with_capacity(args.len());
    for source in args {
        if !source.is_object() {
            return Err(Throw::type_error("Iterator.concat argument is not an object"));
        }
        let method = i.get_prop(&source, WellKnownSymbol::Iterator.symbol())?;
        if !std_callable(&method) {
            return Err(Throw::type_error("Iterator.concat argument is not iterable"));
        }
        sources.push(StdIteratorConcatSource { iterable: source, method });
    }
    Ok(i.std_iterator_object(None, StdIteratorOperation::Concat { sources, position: 0 }, false))
}

fn std_iterator_step(i: &mut Interpreter, record: &StdIteratorRecord) -> Result<Option<JsValue>, Throw> {
    let result = i.call_value(&record.next, record.iterator.clone(), Vec::new())?;
    if !result.is_object() { return Err(Throw::type_error("iterator result is not an object")); }
    if i.get_prop(&result, "done")?.to_boolean() { return Ok(None); }
    Ok(Some(i.get_prop(&result, "value")?))
}

fn std_iterator_step_done(i: &mut Interpreter, record: &StdIteratorRecord) -> Result<bool, Throw> {
    let result = i.call_value(&record.next, record.iterator.clone(), Vec::new())?;
    if !result.is_object() { return Err(Throw::type_error("iterator result is not an object")); }
    Ok(i.get_prop(&result, "done")?.to_boolean())
}

fn std_iterator_close(i: &mut Interpreter, record: &StdIteratorRecord) -> Result<(), Throw> {
    let return_method = i.get_prop(&record.iterator, "return")?;
    if return_method.is_null() || return_method.is_undefined() { return Ok(()); }
    if !std_callable(&return_method) { return Err(Throw::type_error("iterator return is not callable")); }
    let result = i.call_value(&return_method, record.iterator.clone(), Vec::new())?;
    if !result.is_object() { return Err(Throw::type_error("iterator return result is not an object")); }
    Ok(())
}

fn std_iterator_abrupt_close(i: &mut Interpreter, record: &StdIteratorRecord, error: Throw) -> Throw {
    let _ = std_iterator_close(i, record);
    error
}

fn std_iterator_close_all(i: &mut Interpreter, records: &[StdIteratorRecord], exhausted: &[bool], error: Throw) -> Throw {
    for (record, done) in records.iter().zip(exhausted).rev() {
        if !done { let _ = std_iterator_close(i, record); }
    }
    error
}

fn std_iterator_callback(i: &mut Interpreter, callback: &JsValue, value: JsValue, index: u64) -> Result<JsValue, Throw> {
    i.call_value(callback, JsValue::Undefined, vec![value, JsValue::num(index as f64)])
}

fn std_iterator_wrap_record(i: &Interpreter, this: &JsValue) -> Result<StdIteratorRecord, Throw> {
    let JsValue::Object(owner) = this else { return Err(Throw::type_error("incompatible Iterator wrapper receiver")); };
    let states = i.std_iterator_states.borrow();
    let state = states.get(&std_weak_id(owner))
        .filter(|state| std_weak_same(&state.owner, owner) && matches!(state.operation, StdIteratorOperation::Wrap))
        .ok_or_else(|| Throw::type_error("incompatible Iterator wrapper receiver"))?;
    Ok(state.source.as_ref().unwrap().clone())
}

fn std_iterator_wrap_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let record = std_iterator_wrap_record(i, &this)?;
    // Wrappers neither inspect nor recreate iterator results, nor track done.
    i.call_value(&record.next, record.iterator, Vec::new())
}

fn std_iterator_wrap_return(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let record = std_iterator_wrap_record(i, &this)?;
    let method = i.get_prop(&record.iterator, "return")?;
    if method.is_undefined() || method.is_null() {
        return Ok(sequence_result(i, JsValue::Undefined, true));
    }
    if !std_callable(&method) { return Err(Throw::type_error("iterator return is not callable")); }
    i.call_value(&method, record.iterator, Vec::new())
}

fn std_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    {
        let states = i.std_iterator_states.borrow();
        let state = states.get(&std_weak_id(owner)).filter(|state| std_weak_same(&state.owner, owner))
            .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
        if matches!(state.operation, StdIteratorOperation::Wrap) { return Err(Throw::type_error("incompatible Iterator helper receiver")); }
        if state.executing { return Err(Throw::type_error("Iterator helper is executing")); }
    }
    let mut state = i.std_iterator_states.borrow_mut().remove(&std_weak_id(owner)).unwrap();
    state.started = true;
    state.executing = true;
    // Publish the existing executing state without keeping a RefCell borrow
    // across user callbacks. All state stays owned by this Interpreter.
    i.std_iterator_states.borrow_mut().insert(std_weak_id(owner), state.clone());
    let result = if state.done {
        // GeneratorResume's completed result belongs to the method callee.
        Ok(sequence_result(i, JsValue::Undefined, true))
    } else {
        let realm = RealmHandle { agent: i.agent_id, realm: owner.borrow().realm.unwrap_or(i.host_realm) };
        i.binding_with_realm(realm, |i| {
            let result = std_iterator_state_step(i, &mut state);
            result.map(|value| sequence_result(i, value.clone().unwrap_or(JsValue::Undefined), value.is_none()))
                .map_err(|error| i.normalize_throw(error))
        })
    };
    if result.as_ref().is_err() { state.done = true; }
    state.executing = false;
    i.std_iterator_states.borrow_mut().insert(std_weak_id(owner), state);
    result
}

fn std_iterator_return(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    let (records, resume) = {
        let mut states = i.std_iterator_states.borrow_mut();
        let state = states.get_mut(&std_weak_id(owner)).filter(|state| std_weak_same(&state.owner, owner))
            .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
        if matches!(state.operation, StdIteratorOperation::Wrap) { return Err(Throw::type_error("incompatible Iterator helper receiver")); }
        if state.executing { return Err(Throw::type_error("Iterator helper is executing")); }
        let mut records = Vec::new();
        if !state.done {
            if let StdIteratorOperation::Zip { records: zipped, exhausted, .. } = &state.operation {
                records.extend(zipped.iter().zip(exhausted).rev().filter(|(_,done)| !**done)
                    .map(|(record,_)|record.clone()));
            } else {
                if state.started { records.extend(state.inner.iter().cloned()); }
                records.extend(state.source.iter().cloned());
            }
        }
        // suspended-start becomes completed before closing; suspended-yield
        // remains executing through closure resumption/closing.
        state.executing = state.started && !state.done;
        state.done = true;
        (records, state.executing)
    };
    let close = |i: &mut Interpreter| {
        let mut completion = Ok(());
        for record in records {
            if let Err(error) = std_iterator_close(i, &record) {
                if completion.is_ok() { completion = Err(error); }
            }
        }
        completion?;
        Ok(sequence_result(i, JsValue::Undefined, true))
    };
    let result = if resume {
        let realm = RealmHandle { agent: i.agent_id, realm: owner.borrow().realm.unwrap_or(i.host_realm) };
        i.binding_with_realm(realm, |i| close(i).map_err(|error| i.normalize_throw(error)))
    } else { close(i) };
    if let Some(state) = i.std_iterator_states.borrow_mut().get_mut(&std_weak_id(owner)) { state.executing = false; }
    result
}

fn std_iterator_counter_next(index: u64) -> u64 { index.saturating_add(1).min(1u64 << 53) }

fn std_iterator_array_from_list(i: &mut Interpreter, values: Vec<JsValue>) -> Result<JsValue, Throw> {
    let array = array_values(i, Vec::new())?;
    for (index, value) in values.into_iter().enumerate() {
        array_create_data_property(i, &array, index, value)?;
    }
    Ok(array)
}


fn std_iterator_state_step(i: &mut Interpreter, state: &mut StdIteratorState) -> Result<Option<JsValue>, Throw> {
    loop {
        match &mut state.operation {
            StdIteratorOperation::Wrap => return Err(Throw::type_error("incompatible Iterator helper receiver")),
            StdIteratorOperation::Map(callback) => {
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = std_iterator_counter_next(state.index);
                return std_iterator_callback(i, callback, value, index)
                    .map(Some).map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error));
            }
            StdIteratorOperation::Filter(callback) => {
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = std_iterator_counter_next(state.index);
                if std_iterator_callback(i, callback, value.clone(), index)
                    .map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error))?
                    .to_boolean() { return Ok(Some(value)); }
            }
            StdIteratorOperation::Take(remaining) => {
                if *remaining <= 0.0 {
                    std_iterator_close(i, state.source.as_ref().unwrap())?;
                    state.done = true; return Ok(None);
                }
                let value = std_iterator_step(i, state.source.as_ref().unwrap())?;
                if value.is_none() { state.done = true; }
                if remaining.is_finite() { *remaining -= 1.0; }
                return Ok(value);
            }
            StdIteratorOperation::Drop(remaining) => {
                while *remaining > 0.0 {
                    if std_iterator_step_done(i, state.source.as_ref().unwrap())? {
                        state.done = true; return Ok(None);
                    }
                    if remaining.is_finite() { *remaining -= 1.0; }
                }
                let value = std_iterator_step(i, state.source.as_ref().unwrap())?;
                if value.is_none() { state.done = true; }
                return Ok(value);
            }
            StdIteratorOperation::FlatMap(callback) => {
                if let Some(inner) = &state.inner {
                    if let Some(value) = std_iterator_step(i, inner)
                        .map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error))? {
                        return Ok(Some(value));
                    }
                    state.inner = None;
                }
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = std_iterator_counter_next(state.index);
                let mapped = std_iterator_callback(i, callback, value, index)
                    .map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error))?;
                state.inner = Some(i.std_iterator_flattenable(mapped, false)
                    .map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error))?);
            }
            StdIteratorOperation::Concat { sources, position } => {
                if state.source.is_none() {
                    let Some(source) = sources.get(*position) else { state.done = true; return Ok(None); };
                    let iterator = i.call_value(&source.method, source.iterable.clone(), Vec::new())?;
                    state.source = Some(i.std_iterator_direct(iterator)?);
                }
                if let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? {
                    return Ok(Some(value));
                }
                state.source = None;
                *position += 1;
            }
            StdIteratorOperation::Chunks { size, buffer } => {
                while buffer.len() < *size {
                    let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                        state.done = true;
                        return if buffer.is_empty() { Ok(None) }
                            else { std_iterator_array_from_list(i, std::mem::take(buffer)).map(Some) };
                    };
                    buffer.push(value);
                }
                return std_iterator_array_from_list(i, std::mem::take(buffer)).map(Some);
            }
            StdIteratorOperation::Windows { size, partial, buffer } => {
                if buffer.len() == *size { buffer.remove(0); }
                loop {
                    let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                        state.done = true;
                        return if *partial && !buffer.is_empty() && buffer.len() < *size {
                            std_iterator_array_from_list(i, std::mem::take(buffer)).map(Some)
                        } else { Ok(None) };
                    };
                    buffer.push(value);
                    if buffer.len() == *size {
                        return std_iterator_array_from_list(i, buffer.clone()).map(Some);
                    }
                }
            }
            StdIteratorOperation::Zip { records, keys, mode, padding, exhausted } => {
                if records.is_empty() { state.done = true; return Ok(None); }
                let mut values = Vec::with_capacity(records.len());
                let mut any_done = false;
                let mut all_done = true;
                for (index, record) in records.iter().enumerate() {
                    if exhausted[index] {
                        any_done = true;
                        values.push(padding.get(index).cloned().unwrap_or(JsValue::Undefined));
                        continue;
                    }
                    let step = std_iterator_step(i, record).map_err(|error| {
                        exhausted[index] = true;
                        std_iterator_close_all(i, records, exhausted, error)
                    })?;
                    match step {
                        Some(value) => { all_done = false; values.push(value); }
                        None => {
                            exhausted[index] = true;
                            any_done = true;
                            if mode == "strict" && index == 0 {
                                for (next_index, next_record) in records.iter().enumerate().skip(1) {
                                    let done = std_iterator_step_done(i, next_record).map_err(|error| {
                                        exhausted[next_index] = true;
                                        std_iterator_close_all(i, records, exhausted, error)
                                    })?;
                                    if !done {
                                        state.done = true;
                                        return Err(std_iterator_close_all(i, records, exhausted,
                                            Throw::type_error("zipped iterators have unequal lengths")));
                                    }
                                    exhausted[next_index] = true;
                                }
                                state.done = true;
                                return Ok(None);
                            }
                            if mode == "shortest" {
                                state.done = true;
                                let mut error = None;
                                for (record, done) in records.iter().zip(exhausted.iter()).rev() {
                                    if !done { if let Err(throw) = std_iterator_close(i, record) {
                                        if error.is_none() { error = Some(throw); }
                                    } }
                                }
                                if let Some(throw) = error { return Err(throw); }
                                return Ok(None);
                            }
                            if mode == "strict" && index != 0 {
                                state.done = true;
                                return Err(std_iterator_close_all(i, records, exhausted,
                                    Throw::type_error("zipped iterators have unequal lengths")));
                            }
                            values.push(padding.get(index).cloned().unwrap_or(JsValue::Undefined));
                        }
                    }
                }
                if all_done || (any_done && mode == "strict") {
                    state.done = true;
                    if !all_done {
                        return Err(std_iterator_close_all(i, records, exhausted,
                            Throw::type_error("zipped iterators have unequal lengths")));
                    }
                    return Ok(None);
                }
                if let Some(keys) = keys {
                    let mut object = JsObject::ordinary();
                    object.proto = None;
                    for (key, value) in keys.iter().cloned().zip(values) {
                        object.define(key, Property::data(value)).unwrap();
                    }
                    return Ok(Some(JsValue::object(object)));
                }
                return std_iterator_array_from_list(i, values).map(Some);
            }
        }
    }
}

fn std_iterator_helper(i: &mut Interpreter, this: JsValue, operation: StdIteratorOperation) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    Ok(i.std_iterator_object(Some(source), operation, false))
}

fn std_iterator_validate_callback(i: &mut Interpreter, this: &JsValue,
    callback: &JsValue, message: &str) -> Result<(), Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    if !std_callable(callback) {
        let error = Throw::type_error(message);
        return Err(std_iterator_abrupt_close(i, &StdIteratorRecord {
            iterator: this.clone(), next: JsValue::Undefined,
        }, error));
    }
    Ok(())
}

fn std_iterator_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "mapper must be callable")?;
    std_iterator_helper(i, this, StdIteratorOperation::Map(callback))
}
fn std_iterator_filter(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "predicate must be callable")?;
    std_iterator_helper(i, this, StdIteratorOperation::Filter(callback))
}
fn std_iterator_take(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let amount = std_iterator_limit(i, &this, std_reflect_arg(&args, 0))?;
    let source = i.std_iterator_direct(this)?;
    Ok(i.std_iterator_object(Some(source), StdIteratorOperation::Take(amount), false))
}
fn std_iterator_drop(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let amount = std_iterator_limit(i, &this, std_reflect_arg(&args, 0))?;
    let source = i.std_iterator_direct(this)?;
    Ok(i.std_iterator_object(Some(source), StdIteratorOperation::Drop(amount), false))
}

fn std_iterator_limit(i: &mut Interpreter, this: &JsValue, value: JsValue) -> Result<f64, Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    let number = i.to_number(&value).map_err(|error| std_iterator_abrupt_close(i,
        &StdIteratorRecord { iterator: this.clone(), next: JsValue::Undefined }, error))?;
    let amount = number.trunc();
    if number.is_nan() || amount < 0.0 || (number.is_finite() && number > 9_007_199_254_740_991.0) {
        return Err(std_iterator_abrupt_close(i,
            &StdIteratorRecord { iterator: this.clone(), next: JsValue::Undefined },
            Throw::range_error("iterator limit is out of range")));
    }
    Ok(amount)
}
fn std_iterator_flat_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "mapper must be callable")?;
    std_iterator_helper(i, this, StdIteratorOperation::FlatMap(callback))
}

fn std_iterator_reduce(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "reducer must be callable")?;
    let source = i.std_iterator_direct(this)?;
    let mut index = 0u64;
    let mut accumulator = if args.len() > 1 { args[1].clone() } else {
        let Some(value) = std_iterator_step(i, &source)? else { return Err(Throw::type_error("reduce of empty iterator")); };
        index = 1; value
    };
    while let Some(value) = std_iterator_step(i, &source)? {
        accumulator = i.call_value(&callback, JsValue::Undefined,
            vec![accumulator, value, JsValue::num(index as f64)])
            .map_err(|error| std_iterator_abrupt_close(i, &source, error))?;
        index = std_iterator_counter_next(index);
    }
    Ok(accumulator)
}
fn std_iterator_to_array(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let mut values = Vec::new();
    while let Some(value) = std_iterator_step(i, &source)? { values.push(value); }
    std_iterator_array_from_list(i, values)
}
fn std_iterator_join(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    let separator = std_reflect_arg(&args, 0);
    let separator = if separator.is_undefined() { JsValue::str(",") }
        else { separator };
    let separator = i.to_js_string(&separator)
        .map_err(|error| std_iterator_abrupt_close(i,
            &StdIteratorRecord { iterator: this.clone(), next: JsValue::Undefined }, error))?;
    let source = i.std_iterator_direct(this)?;
    let mut result = Vec::new();
    let mut first = true;
    while let Some(value) = std_iterator_step(i, &source)? {
        if !first { result.extend_from_slice(separator.units()); }
        first = false;
        if !value.is_null() && !value.is_undefined() {
            let text = i.to_js_string(&value)
                .map_err(|error| std_iterator_abrupt_close(i, &source, error))?;
            result.extend_from_slice(text.units());
        }
    }
    Ok(JsValue::str_units(result))
}
fn std_iterator_for_each(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "callback must be callable")?;
    let source = i.std_iterator_direct(this)?;
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        std_iterator_callback(i, &callback, value, index)
            .map_err(|error| std_iterator_abrupt_close(i, &source, error))?;
        index = std_iterator_counter_next(index);
    }
    Ok(JsValue::Undefined)
}
fn std_iterator_some(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "predicate must be callable")?;
    let source = i.std_iterator_direct(this)?;
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if std_iterator_callback(i, &callback, value, index)
            .map_err(|error| std_iterator_abrupt_close(i, &source, error))?
            .to_boolean() {
            std_iterator_close(i, &source)?; return Ok(JsValue::Bool(true));
        }
        index = std_iterator_counter_next(index);
    }
    Ok(JsValue::Bool(false))
}
fn std_iterator_every(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "predicate must be callable")?;
    let source = i.std_iterator_direct(this)?;
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if !std_iterator_callback(i, &callback, value, index)
            .map_err(|error| std_iterator_abrupt_close(i, &source, error))?
            .to_boolean() {
            std_iterator_close(i, &source)?; return Ok(JsValue::Bool(false));
        }
        index = std_iterator_counter_next(index);
    }
    Ok(JsValue::Bool(true))
}
fn std_iterator_find(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    std_iterator_validate_callback(i, &this, &callback, "predicate must be callable")?;
    let source = i.std_iterator_direct(this)?;
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if std_iterator_callback(i, &callback, value.clone(), index)
            .map_err(|error| std_iterator_abrupt_close(i, &source, error))?
            .to_boolean() {
            std_iterator_close(i, &source)?; return Ok(value);
        }
        index = std_iterator_counter_next(index);
    }
    Ok(JsValue::Undefined)
}

fn std_iterator_includes(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    let searched = std_reflect_arg(&args, 0);
    let skipped = std_reflect_arg(&args, 1);
    let count = if skipped.is_undefined() { 0.0 }
        else if let JsValue::Num(number) = skipped { number }
        else {
            return Err(std_iterator_abrupt_close(i,
                &StdIteratorRecord { iterator: this, next: JsValue::Undefined },
                Throw::type_error("skippedElements must be a Number")));
        };
    if count.is_nan() || (!count.is_infinite() && count.fract() != 0.0) {
        return Err(std_iterator_abrupt_close(i,
            &StdIteratorRecord { iterator: this, next: JsValue::Undefined },
            Throw::type_error("skippedElements must be integral")));
    }
    if count < 0.0 || (count.is_finite() && count > 9_007_199_254_740_991.0) {
        return Err(std_iterator_abrupt_close(i,
            &StdIteratorRecord { iterator: this, next: JsValue::Undefined },
            Throw::range_error("skippedElements is out of range")));
    }
    let source = i.std_iterator_direct(this)?;
    let mut skipped = 0.0;
    while let Some(value) = std_iterator_step(i, &source)? {
        if skipped < count { skipped += 1.0; continue; }
        let equal = match (&value, &searched) {
            (JsValue::Num(a), JsValue::Num(b)) if a.is_nan() && b.is_nan() => true,
            _ => value == searched,
        };
        if equal {
            std_iterator_close(i, &source)?;
            return Ok(JsValue::Bool(true));
        }
    }
    Ok(JsValue::Bool(false))
}

fn std_iterator_chunk_size(i: &mut Interpreter, this: &JsValue, value: JsValue) -> Result<usize, Throw> {
    let number = match value {
        JsValue::Num(number) if number.is_finite() && number.fract() == 0.0 => number,
        _ => {
            return Err(std_iterator_abrupt_close(i,
                &StdIteratorRecord { iterator: this.clone(), next: JsValue::Undefined },
                Throw::type_error("iterator chunk size must be an integral Number")));
        }
    };
    if !(1.0..=4_294_967_295.0).contains(&number) {
        return Err(std_iterator_abrupt_close(i,
            &StdIteratorRecord { iterator: this.clone(), next: JsValue::Undefined },
            Throw::range_error("iterator chunk size is out of range")));
    }
    Ok(number as usize)
}

fn std_iterator_chunks(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    let size = std_iterator_chunk_size(i, &this, std_reflect_arg(&args, 0))?;
    std_iterator_helper(i, this, StdIteratorOperation::Chunks { size, buffer: Vec::new() })
}

fn std_iterator_windows(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("iterator must be an object")); }
    let size = std_iterator_chunk_size(i, &this, std_reflect_arg(&args, 0))?;
    let undersized = std_reflect_arg(&args, 1);
    let partial = if undersized.is_undefined() || undersized == JsValue::str("only-full") { false }
        else if undersized == JsValue::str("allow-partial") { true }
        else {
            return Err(std_iterator_abrupt_close(i,
                &StdIteratorRecord { iterator: this, next: JsValue::Undefined },
                Throw::type_error("invalid undersized option")));
        };
    std_iterator_helper(i, this, StdIteratorOperation::Windows { size, partial, buffer: Vec::new() })
}

fn std_iterator_zip_options(i: &mut Interpreter, option: JsValue) -> Result<(String, JsValue), Throw> {
    if !option.is_undefined() && !option.is_object() {
        return Err(Throw::type_error("Iterator.zip options must be an object"));
    }
    let mode = if option.is_undefined() { JsValue::Undefined } else { i.get_prop(&option, "mode")? };
    let mode = if mode.is_undefined() { "shortest".to_string() }
        else if let JsValue::Str(value) = mode { value.to_utf8_lossy() }
        else { return Err(Throw::type_error("invalid Iterator.zip mode")); };
    if !matches!(mode.as_str(), "shortest" | "longest" | "strict") {
        return Err(Throw::type_error("invalid Iterator.zip mode"));
    }
    let padding = if option.is_undefined() || mode != "longest" { JsValue::Undefined }
        else { i.get_prop(&option, "padding")? };
    if !padding.is_undefined() && !padding.is_object() {
        return Err(Throw::type_error("Iterator.zip padding must be an object"));
    }
    Ok((mode, padding))
}

fn std_iterator_zip_impl(i: &mut Interpreter, args: Vec<JsValue>, keyed: bool) -> Result<JsValue, Throw> {
    let iterables = std_reflect_arg(&args, 0);
    if !iterables.is_object() {
        return Err(Throw::type_error("Iterator.zip input must be an object"));
    }
    let (mode, padding_arg) = std_iterator_zip_options(i, std_reflect_arg(&args, 1))?;
    let mut keys = None;
    let mut records = Vec::new();
    if keyed {
        let JsValue::Object(object) = &iterables else {
            return Err(Throw::type_error("Iterator.zipKeyed input must be an object"));
        };
        let own_keys = i.own_property_keys(&iterables)?;
        let mut selected = Vec::new();
        for key in own_keys {
            let descriptor = i.default_get_own_property(object, &key).map_err(|error|
                std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            if descriptor.is_some_and(|property| property.enumerable) {
                let value = i.get_prop(&iterables, key.clone()).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
                if value.is_undefined() { continue; }
                let record = i.std_iterator_flattenable(value, false).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
                records.push(record);
                selected.push(key);
            }
        }
        keys = Some(selected);
    } else {
        let method = i.get_prop(&iterables, WellKnownSymbol::Iterator.symbol())?;
        if !std_callable(&method) { return Err(Throw::type_error("Iterator.zip input is not iterable")); }
        let outer_value = i.call_value(&method, iterables, Vec::new())?;
        let outer = i.std_iterator_direct(outer_value)?;
        loop {
            let next = std_iterator_step(i, &outer).map_err(|error|
                std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            let Some(value) = next else { break; };
            let record = i.std_iterator_flattenable(value, false).map_err(|error| {
                let mut closing = vec![outer.clone()];
                closing.extend(records.iter().cloned());
                std_iterator_close_all(i, &closing, &vec![false; closing.len()], error)
            })?;
            records.push(record);
        }
    }
    let mut padding = Vec::new();
    if !padding_arg.is_undefined() {
        if keyed {
            for key in keys.as_ref().unwrap() {
                padding.push(i.get_prop(&padding_arg, key.clone()).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))?);
            }
        } else {
            let method = i.get_prop(&padding_arg, WellKnownSymbol::Iterator.symbol()).map_err(|error|
                std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            if !std_callable(&method) {
                return Err(std_iterator_close_all(i, &records, &vec![false; records.len()],
                    Throw::type_error("Iterator.zip padding is not iterable")));
            }
            let padding_iterator = i.call_value(&method, padding_arg, Vec::new()).map_err(|error|
                std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            let source = i.std_iterator_direct(padding_iterator).map_err(|error|
                std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            let mut exhausted = false;
            for _ in 0..records.len() {
                if let Some(value) = std_iterator_step(i, &source).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))? { padding.push(value); }
                else { exhausted = true; break; }
            }
            if !exhausted {
                std_iterator_close(i, &source).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
            }
        }
    }
    let exhausted = vec![false; records.len()];
    Ok(i.std_iterator_object(None, StdIteratorOperation::Zip { records, keys, mode, padding, exhausted }, false))
}

fn std_iterator_zip(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_iterator_zip_impl(i, args, false)
}

fn std_iterator_zip_keyed(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    std_iterator_zip_impl(i, args, true)
}

#[cfg(test)]
mod synchronous_iterator_protocol_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}\n{source}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn wrappers_forward_raw_cached_calls_and_recheck_return_without_arguments() {
        check(r#"
          var valid=true,gets=0,calls=0,returns=0,retGets=0,result={get done(){throw 'not read';},get value(){throw 'not read';}},base={
            get next(){gets++;return function(){valid=valid && this===base && arguments.length===0;calls++;return calls===1?result:7;};},
            get return(){retGets++;return function(){valid=valid && this===base && arguments.length===0;returns++;return returns===1?result:3;};}};
          var wrap=Iterator.from(base);
          Object.defineProperty(base,'next',{value:function(){throw 'cached next';}});
          valid=valid && wrap.next('ignored')===result && wrap.next()===7 && wrap.next()===7 && gets===1 && calls===3;
          valid=valid && wrap.return('ignored')===result && wrap.return()===3 && returns===2 && retGets===2;
          Object.defineProperty(base,'return',{value:null,configurable:true});
          var finished=wrap.return();valid=valid && finished.done && finished.value===undefined;
          Object.defineProperty(base,'return',{value:1,configurable:true});
          try{wrap.return();valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid;
        "#);
    }
    #[test]
    fn wrapper_and_helper_methods_enforce_separate_brands_and_native_metadata() {
        check(r#"
          var valid=true,calls=0,base={next(){calls++;return{value:1};}},wrap=Iterator.from(base),helper=wrap.take(1),wp=Object.getPrototypeOf(wrap),hp=Object.getPrototypeOf(helper);
          for(var pair of [[wp.next,helper],[wp.return,helper],[hp.next,wrap],[hp.return,wrap],[wp.next,new Proxy(wrap,{})],[hp.next,new Proxy(helper,{})]]){
            try{pair[0].call(pair[1]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          }
          valid=valid && calls===0 && wp.next!==hp.next && wp.return!==hp.return;
          for(var proto of [wp,hp])for(var name of ['next','return']){
            var d=Object.getOwnPropertyDescriptor(proto,name),f=d.value;
            valid=valid && f.name===name && f.length===0 && d.writable && !d.enumerable && d.configurable && !Object.hasOwn(f,'prototype');
          }
          valid;
        "#);
    }
    #[test]
    fn from_uses_semantic_prototype_walk_and_the_saved_intrinsic_base() {
        check(r#"
          var valid=true,log=[],base=Iterator.prototype,value=Object.create(base);value.next=function(){return{done:true};};
          var proxy=new Proxy(value,{get(t,k,r){log.push(k===Symbol.iterator?'iterator':k);return Reflect.get(t,k,r);},getPrototypeOf(){log.push('proto');return base;}});
          valid=valid && Iterator.from(proxy)===proxy && log.join(',')==='iterator,next,proto';
          var wrapped=Iterator.from(base);valid=valid && wrapped!==base;
          var error={},bad=new Proxy({next(){}},{getPrototypeOf(){throw error;}});
          try{Iterator.from(bad);valid=false;}catch(e){valid=valid && e===error;}
          var from=Iterator.from;Iterator={};valid=valid && from(value)===value;valid;
        "#);
    }
    #[test]
    fn take_drop_allow_infinity_preserve_finite_cap_and_skip_value_reads() {
        check(r#"
          var valid=true,take=Iterator.prototype.take,drop=Iterator.prototype.drop;
          valid=valid && Iterator.from([1,2]).take(Infinity).toArray().join(',')==='1,2' && Iterator.from([1,2]).drop(Infinity).next().done;
          for(var method of [take,drop])for(var limit of [NaN,-1,-Infinity,2**53]){
            var log=[],source={get next(){log.push('next');return function(){return{done:true};};},return(){log.push('return');throw 'ignored';}};
            try{method.call(source,limit);valid=false;}catch(e){valid=valid && e instanceof RangeError;}
            valid=valid && log.join(',')==='return';
          }
          for(var method of [take,drop]){
            var gets=0,source={get next(){gets++;return function(){return{done:true};};}};
            var h=method.call(source,2**53-1);valid=valid && gets===1 && h.next().done;
          }
          var n=0,reads=0,gets=0,source={get next(){gets++;return function(){var index=n++;return{done:index===3,get value(){reads++;if(index<2)throw 'skipped value';return index;}};};}};
          var h=drop.call(source,2);valid=valid && h.next().value===2 && h.next().done && reads===1 && gets===1;
          var source={next(){return{done:true,get value(){throw 'done value';}};}};valid=valid && drop.call(source,Infinity).next().done;valid;
        "#);
    }
    #[test]
    fn flat_map_rejects_primitives_and_closes_inner_before_outer() {
        check(r#"
          var valid=true,flat=Iterator.prototype.flatMap,desc=Object.getOwnPropertyDescriptor(String.prototype,Symbol.iterator),gets=0;
          Object.defineProperty(String.prototype,Symbol.iterator,{get(){gets++;return desc.value;},configurable:true});
          for(var mapped of ['x',1,true,null,undefined,2n,Symbol()]){
            var closed=0,source={next(){return{value:1};},return(){closed++;return{};}};
            var h=flat.call(source,function(){return mapped;});
            try{h.next();valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            valid=valid && closed===1 && gets===0;
          }
          valid=valid && Iterator.from('x').next().value==='x' && gets===1;
          Object.defineProperty(String.prototype,Symbol.iterator,desc);
          for(var mode of [0,1,2]){
            var log=[],first={},second={},inner={next(){return{value:7};},return(){log.push('inner');if(mode===1)throw first;if(mode===2)return 1;return{};}},outer={next(){return{value:1};},return(){log.push('outer');throw second;}};
            var h=flat.call(outer,function(){return inner;});valid=valid && h.next().value===7;
            try{h.return();valid=false;}catch(e){valid=valid && (mode===0?e===second:mode===1?e===first:e instanceof TypeError);}
            valid=valid && log.join(',')==='inner,outer' && h.next().done;
          }
          var called=0,closed=0,outer={next(){throw 'unused';},return(){closed++;return{};}},h=flat.call(outer,function(){called++;});
          valid=valid && h.return().done && closed===1 && called===0;valid;
        "#);
    }
    #[test]
    fn flat_map_inner_abrupt_steps_close_only_outer_with_original_error() {
        check(r#"
          var valid=true,error={},flat=Iterator.prototype.flatMap;
          for(var kind of ['next','done','value','iterator','mapper']){
            var log=[],inner={next(){if(kind==='next')throw error;return{get done(){if(kind==='done')throw error;return false;},get value(){throw error;}};},return(){log.push('inner');return{};}};
            if(kind==='iterator')Object.defineProperty(inner,Symbol.iterator,{get(){throw error;}});
            var outer={next(){return{value:1};},return(){log.push('outer');throw 'ignored';}},h=flat.call(outer,function(){if(kind==='mapper')throw error;return inner;});
            try{h.next();valid=false;}catch(e){valid=valid && e===error;}
            valid=valid && log.join(',')==='outer' && h.return().done;
          }
          var closed=0,outer={next(){throw error;},return(){closed++;return{};}},h=flat.call(outer,function(){});
          try{h.next();valid=false;}catch(e){valid=valid && e===error;}
          valid=valid && closed===0 && h.next().done;valid;
        "#);
    }
    #[test]
    fn helper_execution_reentrancy_and_completed_state_match_generator_protocol() {
        check(r#"
          var valid=true,h,caught=0,source={next(){return{value:1};},return(){return{};}};
          h=Iterator.prototype.map.call(source,function(v){for(var method of [h.next,h.return]){try{method.call(h);valid=false;}catch(e){valid=valid && e instanceof TypeError;caught++;}}return v;});
          valid=valid && h.next().value===1 && caught===2 && h.return().done && h.next().done;
          var h,source={next(){throw 'unused';},return(){valid=valid && h.next().done;return{};}};
          h=Iterator.prototype.take.call(source,1);valid=valid && h.return().done;
          var h,closed=0,source={next(){return{value:1};},return(){closed++;try{h.next();valid=false;}catch(e){valid=valid && e instanceof TypeError;}return{};}};
          h=Iterator.prototype.take.call(source,1);valid=valid && h.next().value===1 && h.return().done && h.next().done && closed===1;valid;
        "#);
    }
    #[test]
    fn zip_initialization_closes_in_reverse_list_order_and_keyed_descriptors_are_semantic() {
        check(r#"
          var valid=true,error={},log=[],a={next(){return{done:true};},return(){log.push('a');throw 'ignored';}},b={next(){return{done:true};},return(){log.push('b');return{};}},bad={get [Symbol.iterator](){throw error;}},n=0,input={next(){return{value:[a,b,bad][n++],done:false};},return(){log.push('input');return{};},[Symbol.iterator](){return this;}};
          try{Iterator.zip(input);valid=false;}catch(e){valid=valid && e===error;}
          valid=valid && log.join(',')==='b,a,input';
          log=[];var data={a,b},proxy=new Proxy(data,{ownKeys(){log.push('keys');return['a','b'];},getOwnPropertyDescriptor(t,k){log.push('desc:'+k);if(k==='b')throw error;return Reflect.getOwnPropertyDescriptor(t,k);},get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);}});
          try{Iterator.zipKeyed(proxy);valid=false;}catch(e){valid=valid && e===error;}
          valid=valid && log.join(',')==='keys,desc:a,get:a,desc:b,a';
          var symbol=Symbol(),values={a:[1],b:undefined,[symbol]:[2]},proxy=new Proxy(values,{}),h=Iterator.zipKeyed(proxy),r=h.next().value;
          valid=valid && r.a===1 && r[symbol]===2 && Object.getPrototypeOf(r)===null && Reflect.ownKeys(r).length===2 && h.next().done;
          for(var value of ['x',1,true,null,2n,Symbol()]){
            var closed=0,iter={next(){return{done:true};},return(){closed++;return{};}};
            try{Iterator.zipKeyed({a:iter,b:value});valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            valid=valid && closed===1;
          }
          valid;
        "#);
    }
    #[test]
    fn zip_runtime_modes_and_padding_use_step_without_strict_value_get() {
        check(r#"
          var valid=true,error={},log=[],empty={next(){return{done:true};}},a={next(){return{value:1};},return(){log.push('a');throw error;}},b={next(){return{value:2};},return(){log.push('b');throw 'ignored';}};
          var h=Iterator.zip([empty,a,b]);try{h.next();valid=false;}catch(e){valid=valid && e==='ignored';}
          valid=valid && log.join(',')==='b,a' && h.next().done;
          var alive={next(){return{done:true,get value(){throw 'strict must not read value';}};}};
          valid=valid && Iterator.zip([empty,alive],{mode:'strict'}).next().done;
          var reads=0,padding={get a(){reads++;return 9;},get b(){reads++;return 8;}};
          var h=Iterator.zipKeyed({a:[1],b:[2,3]},{mode:'longest',padding}),one=h.next().value,two=h.next().value;
          valid=valid && reads===2 && one.a===1 && one.b===2 && two.a===9 && two.b===3 && h.next().done;
          valid;
        "#);
    }
    #[test]
    fn result_lists_create_own_data_without_invoking_inherited_numeric_setters() {
        check(r#"
          var valid=true,desc=Object.getOwnPropertyDescriptor(Array.prototype,'0'),calls=0,a=[1,2],z=[[3],[4]],c=[5,6],w=[7,8];
          Object.defineProperty(Array.prototype,'0',{set(){calls++;throw 'inherited setter';},configurable:true});
          try{
            var list=Iterator.from(a).toArray(),zip=Iterator.zip(z).next().value,chunks=Iterator.from(c).chunks(2).next().value,windows=Iterator.from(w).windows(2).next().value;
            valid=valid && list[0]===1 && list[1]===2 && zip[0]===3 && zip[1]===4 && chunks[0]===5 && windows[0]===7 && calls===0;
          }finally{if(desc)Object.defineProperty(Array.prototype,'0',desc);else delete Array.prototype[0];}
          for(var array of [list,zip,chunks,windows]){var d=Object.getOwnPropertyDescriptor(array,'0');valid=valid && d.writable && d.enumerable && d.configurable && !('get' in d);}
          valid;
        "#);
    }
    #[test]
    fn borrowed_foreign_helper_methods_preserve_resume_completed_and_brand_realms() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
            for(name,source) in [
                ("foreignNext","Object.getPrototypeOf(Iterator.from([1]).take(1)).next"),
                ("foreignReturn","Object.getPrototypeOf(Iterator.from([1]).take(1)).return"),
                ("foreignWrapNext","Object.getPrototypeOf(Iterator.from({next(){}})).next"),
                ("foreignType","TypeError.prototype"),("foreignObject","Object.prototype"),
            ]{
                let v=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,v).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,source={next(){return{value:1};},return(){return 1;}},h=Iterator.prototype.take.call(source,1);
              var result=foreignNext.call(h);valid=valid && result.value===1 && Object.getPrototypeOf(result)===Object.prototype;
              try{foreignReturn.call(h);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===TypeError.prototype;}
              valid=valid && Object.getPrototypeOf(foreignNext.call(h))===foreignObject;
              var h=Iterator.prototype.take.call(source,1);
              try{foreignReturn.call(h);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
              for(var method of [foreignNext,foreignReturn,foreignWrapNext]){
                try{method.call({});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
              }
              var h=Iterator.prototype.flatMap.call({next(){return{value:1};}},function(){return 'bad';});
              try{foreignNext.call(h);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===TypeError.prototype;}
              var wrapper=Iterator.from({next(){return 3;}});valid=valid && foreignWrapNext.call(wrapper)===3;
              valid;
            "#,backend).unwrap_or_else(|e|panic!("{}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn number_counters_saturate_in_the_existing_actor_owned_state() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            let value=i.run_script_with_backend("var indices=[];Iterator.prototype.map.call({next(){return{value:1};}},function(value,index){indices.push(index);return value;})",backend).unwrap();
            let JsValue::Object(owner)=&value else {panic!("helper")};
            i.std_iterator_states.borrow_mut().get_mut(&std_weak_id(owner)).unwrap().index=(1u64<<53)-1;
            i.global_env.declare_var("helper",value).unwrap();
            assert_eq!(i.run_script_with_backend("helper.next();helper.next();helper.next();indices[0]===2**53-1 && indices[1]===2**53 && indices[2]===2**53",backend).unwrap(),JsValue::Bool(true));
        }
    }

}
