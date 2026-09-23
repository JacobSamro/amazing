// Iterator constructor, direct-iterator wrappers, and lazy helpers.

#[derive(Clone)]
struct StdIteratorRecord { iterator: JsValue, next: JsValue }

enum StdIteratorOperation {
    Wrap,
    Map(JsValue), Filter(JsValue), Take(f64), Drop(f64), FlatMap(JsValue),
    Concat { sources: Vec<JsValue>, position: usize },
}

struct StdIteratorState {
    owner: Weak<RefCell<JsObject>>,
    source: Option<StdIteratorRecord>,
    operation: StdIteratorOperation,
    index: u64,
    inner: Option<StdIteratorRecord>,
    done: bool,
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
            ] {
                object.borrow_mut().define(name,
                    symbol_data(std_collection_method(self, name, length, native), true, true)).unwrap();
            }
        }
        base.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
        base.borrow_mut().define(WellKnownSymbol::Iterator.symbol(),
            symbol_data(std_collection_method(self, "[Symbol.iterator]", 0, std_iterator_self), true, true)).unwrap();
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
            StdIteratorState { owner: Rc::downgrade(&object.0), source, operation, index: 0, inner: None, done: false }));
        JsValue::Object(object)
    }

    fn std_iterator_direct(&mut self, iterator: JsValue) -> Result<StdIteratorRecord, Throw> {
        if !iterator.is_object() { return Err(Throw::type_error("iterator must be an object")); }
        let next = self.get_prop(&iterator, "next")?;
        if !std_callable(&next) { return Err(Throw::type_error("iterator next is not callable")); }
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

fn std_iterator_self(_: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(this)
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
    for source in &args {
        if !source.is_object() && !matches!(source, JsValue::Str(_)) {
            return Err(Throw::type_error("Iterator.concat argument is not an object"));
        }
    }
    Ok(i.std_iterator_object(None, StdIteratorOperation::Concat { sources: args, position: 0 }, false))
}

fn std_iterator_step(i: &mut Interpreter, record: &StdIteratorRecord) -> Result<Option<JsValue>, Throw> {
    let result = i.call_value(&record.next, record.iterator.clone(), Vec::new())?;
    if !result.is_object() { return Err(Throw::type_error("iterator result is not an object")); }
    if i.get_prop(&result, "done")?.to_boolean() { return Ok(None); }
    Ok(Some(i.get_prop(&result, "value")?))
}

fn std_iterator_close(i: &mut Interpreter, record: &StdIteratorRecord) -> Result<(), Throw> {
    let return_method = i.get_prop(&record.iterator, "return")?;
    if return_method.is_null() || return_method.is_undefined() { return Ok(()); }
    if !std_callable(&return_method) { return Err(Throw::type_error("iterator return is not callable")); }
    let result = i.call_value(&return_method, record.iterator.clone(), Vec::new())?;
    if !result.is_object() { return Err(Throw::type_error("iterator return result is not an object")); }
    Ok(())
}

fn std_iterator_callback(i: &mut Interpreter, callback: &JsValue, value: JsValue, index: u64) -> Result<JsValue, Throw> {
    i.call_value(callback, JsValue::Undefined, vec![value, JsValue::num(index as f64)])
}

fn std_iterator_next(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    let mut state = STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().remove(&std_weak_id(owner)))
        .filter(|state| std_weak_same(&state.owner, owner))
        .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
    let result = if state.done { Ok(None) } else { std_iterator_state_step(i, &mut state) };
    if result.as_ref().is_err() { state.done = true; }
    STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().insert(std_weak_id(owner), state));
    result.map(|value| sequence_result(i, value.clone().unwrap_or(JsValue::Undefined), value.is_none()))
}

fn std_iterator_return(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let JsValue::Object(owner) = &this else { return Err(Throw::type_error("incompatible Iterator helper receiver")); };
    let mut state = STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().remove(&std_weak_id(owner)))
        .filter(|state| std_weak_same(&state.owner, owner))
        .ok_or_else(|| Throw::type_error("incompatible Iterator helper receiver"))?;
    let close = if !state.done { state.source.as_ref().map(|source| std_iterator_close(i, source)).transpose() } else { Ok(None) };
    state.done = true;
    STD_ITERATOR_STATES.with(|registry| registry.borrow_mut().insert(std_weak_id(owner), state));
    close?;
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
                return std_iterator_callback(i, callback, value, index).map(Some);
            }
            StdIteratorOperation::Filter(callback) => {
                let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? else {
                    state.done = true; return Ok(None);
                };
                let index = state.index;
                state.index = state.index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
                if std_iterator_callback(i, callback, value.clone(), index)?.to_boolean() { return Ok(Some(value)); }
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
                let mapped = std_iterator_callback(i, callback, value, index)?;
                state.inner = Some(i.std_iterator_from_value(mapped)?);
            }
            StdIteratorOperation::Concat { sources, position } => {
                if state.source.is_none() {
                    let Some(value) = sources.get(*position) else { state.done = true; return Ok(None); };
                    state.source = Some(i.std_iterator_from_value(value.clone())?);
                }
                if let Some(value) = std_iterator_step(i, state.source.as_ref().unwrap())? {
                    return Ok(Some(value));
                }
                state.source = None;
                *position += 1;
            }
        }
    }
}

fn std_iterator_helper(i: &mut Interpreter, this: JsValue, operation: StdIteratorOperation) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    Ok(i.std_iterator_object(Some(source), operation, false))
}

fn std_iterator_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("mapper must be callable")); }
    std_iterator_helper(i, this, StdIteratorOperation::Map(callback))
}
fn std_iterator_filter(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("predicate must be callable")); }
    std_iterator_helper(i, this, StdIteratorOperation::Filter(callback))
}
fn std_iterator_take(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let amount = i.to_number(&std_reflect_arg(&args, 0))?;
    let amount = if amount.is_nan() { 0.0 } else { amount.trunc() };
    if amount < 0.0 { return Err(Throw::range_error("take limit must be nonnegative")); }
    Ok(i.std_iterator_object(Some(source), StdIteratorOperation::Take(amount), false))
}
fn std_iterator_drop(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let amount = i.to_number(&std_reflect_arg(&args, 0))?;
    let amount = if amount.is_nan() { 0.0 } else { amount.trunc() };
    if amount < 0.0 { return Err(Throw::range_error("drop limit must be nonnegative")); }
    Ok(i.std_iterator_object(Some(source), StdIteratorOperation::Drop(amount), false))
}
fn std_iterator_flat_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("mapper must be callable")); }
    std_iterator_helper(i, this, StdIteratorOperation::FlatMap(callback))
}

fn std_iterator_reduce(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("reducer must be callable")); }
    let mut index = 0u64;
    let mut accumulator = if args.len() > 1 { args[1].clone() } else {
        let Some(value) = std_iterator_step(i, &source)? else { return Err(Throw::type_error("reduce of empty iterator")); };
        index = 1; value
    };
    while let Some(value) = std_iterator_step(i, &source)? {
        accumulator = i.call_value(&callback, JsValue::Undefined,
            vec![accumulator, value, JsValue::num(index as f64)])?;
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
    let array = std_iterator_to_array(i, this, Vec::new())?;
    let join = i.get_prop(&array, "join")?;
    i.call_value(&join, array, args)
}
fn std_iterator_for_each(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("callback must be callable")); }
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        std_iterator_callback(i, &callback, value, index)?;
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
    }
    Ok(JsValue::Undefined)
}
fn std_iterator_some(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("predicate must be callable")); }
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if std_iterator_callback(i, &callback, value, index)?.to_boolean() {
            std_iterator_close(i, &source)?; return Ok(JsValue::Bool(true));
        }
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
    }
    Ok(JsValue::Bool(false))
}
fn std_iterator_every(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("predicate must be callable")); }
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if !std_iterator_callback(i, &callback, value, index)?.to_boolean() {
            std_iterator_close(i, &source)?; return Ok(JsValue::Bool(false));
        }
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
    }
    Ok(JsValue::Bool(true))
}
fn std_iterator_find(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = i.std_iterator_direct(this)?;
    let callback = std_reflect_arg(&args, 0);
    if !std_callable(&callback) { return Err(Throw::type_error("predicate must be callable")); }
    let mut index = 0u64;
    while let Some(value) = std_iterator_step(i, &source)? {
        if std_iterator_callback(i, &callback, value.clone(), index)?.to_boolean() {
            std_iterator_close(i, &source)?; return Ok(value);
        }
        index = index.checked_add(1).ok_or_else(|| Throw::range_error("iterator index overflow"))?;
    }
    Ok(JsValue::Undefined)
}
