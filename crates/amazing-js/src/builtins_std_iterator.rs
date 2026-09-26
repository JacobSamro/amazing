// Iterator constructor, direct-iterator wrappers, and lazy helpers.

#[derive(Clone)]
struct StdIteratorRecord { iterator: JsValue, next: JsValue }

struct StdIteratorConcatSource { iterable: JsValue, method: JsValue }

enum StdIteratorOperation {
    Wrap,
    Map(JsValue), Filter(JsValue), Take(f64), Drop(f64), FlatMap(JsValue),
    Concat { sources: Vec<StdIteratorConcatSource>, position: usize },
    Chunks { size: usize, buffer: Vec<JsValue> },
    Windows { size: usize, partial: bool, buffer: Vec<JsValue> },
    Zip { records: Vec<StdIteratorRecord>, keys: Option<Vec<PropertyKey>>,
        mode: String, padding: Vec<JsValue>, exhausted: Vec<bool> },
}

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
    owner: Weak<RefCell<JsObject>>,
    base: Obj,
    helper: Obj,
    wrap: Obj,
    array: Obj,
    string: Obj,
}

thread_local! {
    static STD_ITERATOR_STATES: RefCell<HashMap<usize, StdIteratorState>> = RefCell::new(HashMap::new());
    static STD_ITERATOR_PROTOS: RefCell<HashMap<usize, StdIteratorPrototypes>> = RefCell::new(HashMap::new());
}

impl Interpreter {
    fn install_std_iterator_helpers(&mut self) {
        let mut base = JsObject::ordinary();
        base.proto = Some(self.object_proto.clone());
        let base = Obj(Rc::new(RefCell::new(base)));
        self.std_collection_iter_set_base(&base);
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
        let constructor = Self::native("Iterator", std_iterator_constructor, &self.function_proto);
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
        for prototype in [&helper, &wrap] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, std_iterator_next), true, true)).unwrap();
            prototype.borrow_mut().define("return", symbol_data(std_collection_method(self, "return", 0, std_iterator_return), true, true)).unwrap();
        }
        helper.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str("Iterator Helper"), false, true)).unwrap();
        for (prototype, tag) in [(&array, "Array Iterator"), (&string, "String Iterator")] {
            prototype.borrow_mut().define("next", symbol_data(std_collection_method(self, "next", 0, native_sequence_iterator_next), true, true)).unwrap();
            prototype.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(tag), false, true)).unwrap();
        }
        STD_ITERATOR_PROTOS.with(|registry| registry.borrow_mut().insert(std_weak_id(&self.global_obj),
            StdIteratorPrototypes { owner: Rc::downgrade(&self.global_obj.0), base, helper, wrap, array, string }));
        self.define_global("Iterator", constructor);
        if let Some(value) = self.global_env.resolve("Iterator") {
            self.global_obj.borrow_mut().define("Iterator", symbol_data(value, true, true)).unwrap();
        }
    }

    fn std_iterator_prototypes(&self) -> Option<(Obj, Obj, Obj)> {
        STD_ITERATOR_PROTOS.with(|registry| registry.borrow().get(&std_weak_id(&self.global_obj))
            .filter(|data| std_weak_same(&data.owner, &self.global_obj))
            .map(|data| (data.base.clone(), data.helper.clone(), data.wrap.clone())))
    }

    fn std_sequence_iterator_proto(&self, kind: SequenceIteratorKind) -> Option<Obj> {
        STD_ITERATOR_PROTOS.with(|registry| registry.borrow().get(&std_weak_id(&self.global_obj))
            .filter(|data| std_weak_same(&data.owner, &self.global_obj))
            .map(|data| if kind == SequenceIteratorKind::String { data.string.clone() } else { data.array.clone() }))
    }

    fn std_iterator_object(&self, source: Option<StdIteratorRecord>, operation: StdIteratorOperation, wrapped: bool) -> JsValue {
        let prototype = self.std_iterator_prototypes().map(|(_, helper, wrap)| if wrapped { wrap } else { helper });
        let mut object = JsObject::ordinary();
        object.proto = prototype;
        let object = Obj(Rc::new(RefCell::new(object)));
        STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().insert(std_weak_id(&object),
            StdIteratorState { owner: Rc::downgrade(&object.0), source, operation, index: 0,
                inner: None, done: false, started: false, executing: false }));
        JsValue::Object(object)
    }

    fn std_iterator_direct(&mut self, iterator: JsValue) -> Result<StdIteratorRecord, Throw> {
        if !iterator.is_object() { return Err(Throw::type_error("iterator must be an object")); }
        let next = self.get_prop(&iterator, "next")?;
        Ok(StdIteratorRecord { iterator, next })
    }

    fn std_iterator_from_value(&mut self, value: JsValue) -> Result<StdIteratorRecord, Throw> {
        if !value.is_object() && !matches!(value, JsValue::Str(_)) {
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
            let mut prototype = Some(object.clone());
            while let Some(current) = prototype {
                if Rc::ptr_eq(&current.0, &base.0) { return Ok(record.iterator); }
                prototype = current.borrow().proto.clone();
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

fn std_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    if STD_ITERATOR_STATES.with(|registry| registry.borrow().get(&std_weak_id(owner)).is_some_and(|state| state.executing)) {
        return Err(Throw::type_error("Iterator helper is executing"));
    }
    let mut state = STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().remove(&std_weak_id(owner)))
        .filter(|state| std_weak_same(&state.owner, owner))
        .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
    state.started = true;
    let result = if state.done { Ok(None) } else { std_iterator_state_step(i, &mut state) };
    if result.as_ref().is_err() { state.done = true; }
    STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().insert(std_weak_id(owner), state));
    result.map(|value| sequence_result(i, value.clone().unwrap_or(JsValue::Undefined), value.is_none()))
}

fn std_iterator_return(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    if STD_ITERATOR_STATES.with(|registry| registry.borrow().get(&std_weak_id(owner)).is_some_and(|state| state.executing)) {
        return Err(Throw::type_error("Iterator helper is executing"));
    }
    let mut state = STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().remove(&std_weak_id(owner)))
        .filter(|state| std_weak_same(&state.owner, owner))
        .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
    let records = if state.done { Vec::new() }
        else if let StdIteratorOperation::Zip { records, exhausted, .. } = &state.operation {
            records.iter().zip(exhausted).rev().filter(|(_, done)| !**done)
                .map(|(record, _)| record.clone()).collect()
        } else { state.source.iter().cloned().collect() };
    state.executing = state.started && !state.done;
    state.done = true;
    STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().insert(std_weak_id(owner), state));
    let mut completion = Ok(());
    for record in records {
        if let Err(error) = std_iterator_close(i, &record) {
            if completion.is_ok() { completion = Err(error); }
        }
    }
    STD_ITERATOR_STATES.with(|registry| {
        if let Some(state) = registry.borrow_mut().get_mut(&std_weak_id(owner)) { state.executing = false; }
    });
    completion?;
    Ok(sequence_result(i, JsValue::Undefined, true))
}

fn std_iterator_state_step(i: &mut Interpreter, state: &mut StdIteratorState) -> Result<Option<JsValue>, Throw> {
    loop {
        match &mut state.operation {
            StdIteratorOperation::Wrap => {
                let value = std_iterator_step(i, state.source.as_ref().unwrap())?;
                if value.is_none() { state.done = true; }
                return Ok(value);
            }
            StdIteratorOperation::Map(callback) => {
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = state.index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
                return std_iterator_callback(i, callback, value, index)
                    .map(Some).map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error));
            }
            StdIteratorOperation::Filter(callback) => {
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = state.index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
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
                    if std_iterator_step(i, state.source.as_ref().unwrap())?.is_none() {
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
                    if let Some(value) = std_iterator_step(i, inner)? { return Ok(Some(value)); }
                    state.inner = None;
                }
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = state.index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
                let mapped = std_iterator_callback(i, callback, value, index)
                    .map_err(|error| std_iterator_abrupt_close(i, state.source.as_ref().unwrap(), error))?;
                state.inner = Some(i.std_iterator_from_value(mapped)
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
                            else { array_values(i, std::mem::take(buffer)).map(Some) };
                    };
                    buffer.push(value);
                }
                return array_values(i, std::mem::take(buffer)).map(Some);
            }
            StdIteratorOperation::Windows { size, partial, buffer } => {
                if buffer.len() == *size { buffer.remove(0); }
                loop {
                    let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                        state.done = true;
                        return if *partial && !buffer.is_empty() && buffer.len() < *size {
                            array_values(i, std::mem::take(buffer)).map(Some)
                        } else { Ok(None) };
                    };
                    buffer.push(value);
                    if buffer.len() == *size {
                        return array_values(i, buffer.clone()).map(Some);
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
                return array_values(i, values).map(Some);
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
    if number.is_nan() || amount < 0.0 || amount > 9_007_199_254_740_991.0 {
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
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
    }
    Ok(accumulator)
}
fn std_iterator_to_array(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let mut values = Vec::new();
    while let Some(value) = std_iterator_step(i, &source)? { values.push(value); }
    array_values(i, values)
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
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
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
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
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
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
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
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
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
            if object.borrow().own_prop(&key).is_some_and(|property| property.enumerable) {
                let value = i.get_prop(&iterables, key.clone()).map_err(|error|
                    std_iterator_close_all(i, &records, &vec![false; records.len()], error))?;
                if value.is_undefined() { continue; }
                let record = i.std_iterator_from_value(value).map_err(|error|
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
            let record = (if value.is_object() { i.std_iterator_from_value(value) }
                else { Err(Throw::type_error("Iterator.zip input must be an object")) }).map_err(|error| {
                let error = std_iterator_abrupt_close(i, &outer, error);
                std_iterator_close_all(i, &records, &vec![false; records.len()], error)
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
