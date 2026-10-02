// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// NewPromiseCapability, PromiseResolve and reaction capability dispatch adapted
// to Amazing's native callback home objects and interpreter construction API.

fn is_constructor(value: &JsValue) -> bool {
    matches!(value, JsValue::Object(object) if object.borrow().func.is_some() && object.borrow().constructible)
}

fn claim_promise_resolution(promise: &Obj) -> bool {
    let mut promise = promise.borrow_mut();
    let Exotic::Promise(state) = &mut promise.exotic else { return false; };
    if state.already_resolved { return false; }
    state.already_resolved = true;
    true
}

fn new_promise_capability(i: &mut Interpreter, constructor: &JsValue) -> Result<Obj, Throw> {
    if !is_constructor(constructor) {
        return Err(Throw::type_error("Promise capability requires a constructor"));
    }
    let record = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    record.borrow_mut().set_own("resolve", JsValue::Undefined);
    record.borrow_mut().set_own("reject", JsValue::Undefined);
    let executor = Interpreter::native_arity("", native_capability_executor, &i.function_proto, 2);
    if let JsValue::Object(function) = &executor {
        function.borrow_mut().func.as_mut().unwrap().home = Some(record.clone());
    }
    let promise = i.construct(constructor, vec![executor])?;
    let resolve = record.borrow().get_own("resolve").unwrap();
    let reject = record.borrow().get_own("reject").unwrap();
    if callable_function(&resolve).is_none() || callable_function(&reject).is_none() {
        return Err(Throw::type_error("Promise capability callbacks must be callable"));
    }
    record.borrow_mut().set_own("promise", promise);
    Ok(record)
}

fn native_capability_executor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let record = native_data(i)?;
    let mut record = record.borrow_mut();
    if !record.get_own("resolve").unwrap().is_undefined()
        || !record.get_own("reject").unwrap().is_undefined() {
        return Err(Throw::type_error("Promise capability executor called twice"));
    }
    record.set_own("resolve", args.first().cloned().unwrap_or(JsValue::Undefined));
    record.set_own("reject", args.get(1).cloned().unwrap_or(JsValue::Undefined));
    Ok(JsValue::Undefined)
}

fn capability_promise(record: &Obj) -> JsValue {
    record.borrow().get_own("promise").unwrap()
}

fn finish_promise_capability(record: &Obj, fate: PromiseFate, i: &mut Interpreter) -> Result<(), Throw> {
    let (name, value) = match fate {
        PromiseFate::Fulfilled(value) => ("resolve", value),
        PromiseFate::Rejected(value) => ("reject", value),
        PromiseFate::Pending => return Ok(()),
    };
    let callback = record.borrow().get_own(name).unwrap();
    i.call_value(&callback, JsValue::Undefined, vec![value])?;
    Ok(())
}

fn promise_resolve_constructor(i: &mut Interpreter, constructor: &JsValue, value: JsValue) -> Result<JsValue, Throw> {
    if !constructor.is_object() {
        return Err(Throw::type_error("Promise.resolve requires an object receiver"));
    }
    if promise_object(&value).is_ok() && i.get_prop(&value, "constructor")? == *constructor {
        return Ok(value);
    }
    let capability = new_promise_capability(i, constructor)?;
    finish_promise_capability(&capability, PromiseFate::Fulfilled(value), i)?;
    Ok(capability_promise(&capability))
}

fn promise_species_constructor(i: &mut Interpreter, promise: &JsValue) -> Result<JsValue, Throw> {
    let constructor = i.get_prop(promise, "constructor")?;
    if constructor.is_undefined() { return Ok(i.intrinsic_promise_constructor.clone()); }
    if !constructor.is_object() { return Err(Throw::type_error("Promise constructor is not an object")); }
    let species = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
    if species.is_null() || species.is_undefined() {
        return Ok(i.intrinsic_promise_constructor.clone());
    }
    if !is_constructor(&species) { return Err(Throw::type_error("Promise species is not a constructor")); }
    Ok(species)
}
