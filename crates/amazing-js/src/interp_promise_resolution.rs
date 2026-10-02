// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// CreateResolvingFunctions, Promise Resolve Functions and
// NewPromiseResolveThenableJob adapted to interpreter-owned microtasks.

fn promise_resolving_functions(i: &Interpreter, target: &Obj) -> (JsValue, JsValue) {
    let gate = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    gate.borrow_mut().set_own("[[AlreadyResolved]]", JsValue::Bool(false));
    let resolve = promise_callback(i, "", target, native_promise_resolve);
    let reject = promise_callback(i, "", target, native_promise_reject);
    for callback in [&resolve, &reject] {
        if let JsValue::Object(function) = callback {
            function.borrow_mut().func.as_mut().unwrap().home = Some(gate.clone());
        }
    }
    (resolve, reject)
}

fn claim_current_promise_resolution(i: &Interpreter, target: &Obj) -> bool {
    if let Some(gate) = i.native_stack.last().and_then(|function|
        function.borrow().func.as_ref()?.home.clone()) {
        let mut gate = gate.borrow_mut();
        if let Some(JsValue::Bool(resolved)) = gate.get_own("[[AlreadyResolved]]") {
            if resolved { return false; }
            gate.set_own("[[AlreadyResolved]]", JsValue::Bool(true));
            return true;
        }
    }
    // VM-created resolving callbacks from earlier compiled async jobs retain
    // their existing target-state gate until those call sites use a pair.
    claim_promise_resolution(target)
}

fn resolve_promise_value(i: &mut Interpreter, target: &Obj, value: JsValue) -> Result<(), Throw> {
    if matches!(&value, JsValue::Object(source) if Rc::ptr_eq(&source.0, &target.0)) {
        let error = i.normalize_throw(Throw::type_error("promise resolution cycle")).0;
        return settle_promise(target, PromiseFate::Rejected(error), i);
    }
    if !value.is_object() {
        return settle_promise(target, PromiseFate::Fulfilled(value), i);
    }
    let then = match i.get_prop(&value, "then") {
        Ok(then) => then,
        Err(Throw(error)) => return settle_promise(target, PromiseFate::Rejected(error), i),
    };
    if callable_function(&then).is_none() {
        return settle_promise(target, PromiseFate::Fulfilled(value), i);
    }
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("then", then);
    data.borrow_mut().set_own("thenable", value);
    data.borrow_mut().set_own("target", JsValue::Object(target.clone()));
    let job = Interpreter::native("", native_promise_thenable_job, &i.function_proto);
    if let JsValue::Object(function) = &job {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data);
    }
    i.queue_microtask(job)
}

fn native_promise_thenable_job(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    let then = data.borrow().get_own("then").unwrap();
    let thenable = data.borrow().get_own("thenable").unwrap();
    let JsValue::Object(target) = data.borrow().get_own("target").unwrap() else { unreachable!() };
    let (resolve, reject) = promise_resolving_functions(i, &target);
    match i.call_value(&then, thenable, vec![resolve, reject.clone()]) {
        Ok(value) => Ok(value),
        Err(Throw(error)) => i.call_value(&reject, JsValue::Undefined, vec![error]),
    }
}
