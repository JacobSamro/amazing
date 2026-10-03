// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// for-of async-from-sync continuation and yield_delegate_async)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//! AsyncFromSyncIteratorContinuation adapted to native intrinsic Promises.
//! Read the result synchronously, then unwrap its value through one internal
//! reaction. The caller's outer Await supplies the second suspension boundary.
use super::*;

pub(super) fn request(
    i: &mut Interpreter, iterator: JsValue, next: JsValue, kind: u8, input: Option<JsValue>,
) -> JsValue {
    let result = (|| {
        let method = if kind == 0 { next } else {
            i.get_prop(&iterator, if kind == 1 { "return" } else { "throw" })?
        };
        if method.is_undefined() || method.is_null() {
            if kind == 1 {
                return Ok(None);
            }
            if kind == 2 {
                // Missing throw closes with a normal completion; close errors
                // therefore precede the subsequent protocol TypeError.
                let close = i.get_prop(&iterator, "return")?;
                if !close.is_undefined() && !close.is_null() {
                    if callable_function(&close).is_none() {
                        return Err(Throw::type_error("iterator return is not callable"));
                    }
                    if !i.call_value(&close, iterator.clone(), Vec::new())?.is_object() {
                        return Err(Throw::type_error("iterator return result is not an object"));
                    }
                }
                return Err(Throw::type_error("delegated iterator does not have a throw method"));
            }
        }
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("iterator method is not callable"));
        }
        i.call_value(&method, iterator.clone(), input.clone().into_iter().collect()).map(Some)
    })();
    match result {
        Ok(None) => new_promise(i, PromiseFate::Fulfilled(generator_result(i, input.unwrap_or(JsValue::Undefined), true))),
        Ok(Some(result)) => continuation(i, Ok(result), iterator, kind != 1),
        Err(Throw(error)) => new_promise(i, rejection(i, error)),
    }
}

pub(super) fn continuation(
    i: &mut Interpreter, result: Result<JsValue, Throw>, iterator: JsValue,
    close_on_rejection: bool,
) -> JsValue {
    let wrapper = new_promise(i, PromiseFate::Pending);
    let JsValue::Object(target) = &wrapper else { unreachable!() };
    let result = (|| {
        let result = result?;
        if !result.is_object() {
            return Err(Throw::type_error("iterator result is not an object"));
        }
        let done = i.get_prop(&result, "done")?.to_boolean();
        let value = i.get_prop(&result, "value")?;
        match async_resolve_value_checked(i, value) {
            Ok(promise) => Ok((promise, done)),
            Err(error) => {
                if !done && close_on_rejection { close_preserving_throw(i, &iterator); }
                Err(error)
            }
        }
    })();
    match result {
        Ok((JsValue::Object(promise), done)) => {
            let fulfilled = callback(i, target.clone(), iterator.clone(), done, false);
            let rejected = callback(i, target.clone(), iterator, done || !close_on_rejection, true);
            // The callbacks settle the real wrapper capability explicitly;
            // there is no discarded public then result to assimilate.
            if let Err(Throw(error)) = perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None) {
                let _ = settle_promise(target, rejection(i, error), i);
            }
        }
        Ok(_) => unreachable!("PromiseResolve produced an intrinsic Promise"),
        Err(Throw(error)) => { let _ = settle_promise(target, rejection(i, error), i); }
    }
    wrapper
}

/// IteratorClose with an existing throw retains that throw even when obtaining
/// or calling return fails. No await occurs for a synchronous close result.
fn close_preserving_throw(i: &mut Interpreter, iterator: &JsValue) {
    if let Ok(method) = i.get_prop(iterator, "return") {
        if callable_function(&method).is_some() {
            let _ = i.call_value(&method, iterator.clone(), Vec::new());
        }
    }
}

fn callback(i: &Interpreter, target: Obj, iterator: JsValue, flag: bool, reject: bool) -> JsValue {
    let env = EnvHandle::new(EnvKind::Function, Some(i.global_env.clone()));
    env.declare_var("[[Wrapper]]", JsValue::Object(target)).unwrap();
    env.declare_var("[[Iterator]]", iterator).unwrap();
    env.declare_var("[[DoneOrSkipClose]]", JsValue::Bool(flag)).unwrap();
    let mut callback = JsObject::function(JsFunction {
        name: String::new(), params: Vec::new(),
        body: FunctionBody::Native(if reject { rejected } else { fulfilled }),
        is_arrow: false, is_async: false, is_generator: false, env,
        home: None, proto_for_new: None,
    });
    callback.proto = Some(i.function_proto.clone());
    JsValue::object(callback)
}

fn captured(i: &Interpreter) -> Result<(Obj, JsValue, bool), Throw> {
    let callback = i.native_stack.last().ok_or_else(|| Throw::type_error("missing iterator wrapper callback"))?;
    let env = callback.borrow().func.as_ref().unwrap().env.clone();
    let Some(JsValue::Object(target)) = env.resolve("[[Wrapper]]") else {
        return Err(Throw::type_error("missing iterator wrapper capability"));
    };
    Ok((target, env.resolve("[[Iterator]]").unwrap(), env.resolve("[[DoneOrSkipClose]]").unwrap().to_boolean()))
}

fn fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (target, _, done) = captured(i)?;
    let result = generator_result(i, args.into_iter().next().unwrap_or(JsValue::Undefined), done);
    // Resolve the actual capability, including observable inherited then.
    async_complete(i, &target, result)?;
    Ok(JsValue::Undefined)
}

fn rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (target, iterator, skip_close) = captured(i)?;
    let error = args.into_iter().next().unwrap_or(JsValue::Undefined);
    if !skip_close { close_preserving_throw(i, &iterator); }
    settle_promise(&target, PromiseFate::Rejected(error), i)?;
    Ok(JsValue::Undefined)
}
