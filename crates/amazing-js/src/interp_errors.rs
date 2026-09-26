// Error-family construction, ECMA-262 §20.5.
// https://tc39.es/ecma262/multipage/fundamental-objects.html#sec-error-objects
fn runtime_error_object(i: &mut Interpreter, message: &JsValue) -> Result<Obj, Throw> {
    let mut object = JsObject::ordinary();
    object.exotic = Exotic::Error;
    if let Some(function) = i.native_stack.last() {
        if let Some(JsValue::Object(prototype)) = function.borrow().get_own("prototype") {
            object.proto = Some(prototype);
        }
    }
    if !message.is_undefined() {
        object.define("message", symbol_data(JsValue::str(i.to_string(message)?), true, true)).unwrap();
    }
    Ok(Obj(Rc::new(RefCell::new(object))))
}

fn runtime_install_error_cause(i: &mut Interpreter, error: &Obj, options: &JsValue) -> Result<(), Throw> {
    if options.is_object() && std_reflect_has(i, JsValue::Undefined,
        vec![options.clone(), JsValue::str("cause")])?.to_boolean() {
        let cause = i.get_prop(options, "cause")?;
        error.borrow_mut().define("cause", symbol_data(cause, true, true)).unwrap();
    }
    Ok(())
}

fn native_aggregate_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let error = runtime_error_object(i, args.get(1).unwrap_or(&JsValue::Undefined))?;
    runtime_install_error_cause(i, &error, args.get(2).unwrap_or(&JsValue::Undefined))?;
    let iterator = object_iterator(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    let mut errors = Vec::new();
    while let Some(value) = std_iterator_step(i, &iterator)? { errors.push(value); }
    let errors = array_values(i, errors)?;
    error.borrow_mut().define("errors", symbol_data(errors, true, true)).unwrap();
    Ok(JsValue::Object(error))
}

fn native_suppressed_error(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let error = runtime_error_object(i, args.get(2).unwrap_or(&JsValue::Undefined))?;
    error.borrow_mut().define("error", symbol_data(args.first().cloned().unwrap_or(JsValue::Undefined), true, true)).unwrap();
    error.borrow_mut().define("suppressed", symbol_data(args.get(1).cloned().unwrap_or(JsValue::Undefined), true, true)).unwrap();
    Ok(JsValue::Object(error))
}

// Promise.any uses the intrinsic directly, independent of global replacement.
fn runtime_aggregate_rejection(i: &Interpreter, errors: JsValue) -> JsValue {
    let mut object = JsObject::ordinary();
    object.exotic = Exotic::Error;
    object.proto = i.intrinsic_constructors.get("AggregateError").map(|(_, prototype)| prototype.clone());
    object.define("errors", symbol_data(errors, true, true)).unwrap();
    JsValue::Object(Obj(Rc::new(RefCell::new(object))))
}

fn native_error_is_error(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(matches!(args.first(), Some(JsValue::Object(object))
        if matches!(object.borrow().exotic, Exotic::Error))))
}

#[cfg(test)]
mod error_construction_tests {
    use super::*;

    #[test]
    fn promise_any_rejects_with_the_intrinsic_aggregate_error() {
        let mut i = Interpreter::new();
        i.run_script(r#"
            var saved = AggregateError;
            AggregateError = function() { throw 'replaced'; };
            Promise.any([]).catch(e => console.log(e instanceof saved, e.errors.length));
            Promise.any([Promise.reject(1), Promise.reject(2)]).catch(e =>
                console.log(e instanceof saved, e.errors.join(','), Object.keys(e).length));
        "#).unwrap();
        assert_eq!(i.output.borrow().as_slice(), ["true 0", "true 1,2 0"]);
    }

    #[test]
    fn errors_keep_cause_and_message_non_enumerable_and_observe_options_in_order() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var order = [];
            var message = {toString() { order.push('message'); return 'failure'; }};
            var options = Object.create({get cause() { order.push('cause'); return 42; }});
            var errors = {[Symbol.iterator]() { order.push('iterate'); return [1,2][Symbol.iterator](); }};
            var error = new AggregateError(errors, message, options);
            order.join(',') === 'message,cause,iterate' && error.cause === 42 &&
            error.errors.join(',') === '1,2' && Object.keys(error).length === 0 &&
            error instanceof Error && error instanceof AggregateError &&
            Error.isError(error) && !Error.isError(Error.prototype) &&
            !Error.isError(Object.create(Error.prototype))
        "#).unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
            [Error, EvalError, RangeError, ReferenceError, SyntaxError, TypeError, URIError].every(function(C) {
                var e = C('message', {cause: undefined});
                var d = Object.getOwnPropertyDescriptor(e, 'cause');
                return e.message === 'message' && Object.keys(e).length === 0 &&
                    d.writable && d.configurable && !d.enumerable && Error.isError(e);
            })
        "#).unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script(r#"
            class Sub extends SuppressedError {}
            var e = new Sub(1, 2, 'failure', {get cause() { throw 1; }});
            e instanceof Sub && e instanceof SuppressedError && e instanceof Error &&
            e.error === 1 && e.suppressed === 2 && e.message === 'failure' &&
            !Object.hasOwn(e, 'cause') && Object.keys(e).length === 0
        "#).unwrap(), JsValue::Bool(true));
    }
}
