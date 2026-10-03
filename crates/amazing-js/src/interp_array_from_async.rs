// Portions ported from boa-dev/boa (core/engine/src/builtins/array/from_async.rs,
// core/engine/src/builtins/iterable/async_from_sync_iterator.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Resumable Array.fromAsync and its private AsyncFromSyncIterator adapter.
// Upstream traced coroutine captures become inaccessible native home objects.

const FROM_ASYNC_ARRAY_VALUE: u8 = 0;
const FROM_ASYNC_ITERATOR_NEXT: u8 = 1;
const FROM_ASYNC_MAPPED_VALUE: u8 = 2;
const FROM_ASYNC_CLOSE: u8 = 3;

fn from_async_field(state: &Obj, name: &str) -> JsValue {
    state.borrow().get_own(name).expect("private Array.fromAsync state field")
}
fn from_async_object(state: &Obj, name: &str) -> Obj {
    let JsValue::Object(object) = from_async_field(state, name) else { unreachable!() };
    object
}
fn from_async_index(state: &Obj, name: &str) -> usize {
    let JsValue::Num(value) = from_async_field(state, name) else { unreachable!() };
    value as usize
}
fn from_async_callback(i: &Interpreter, state: &Obj, function: crate::value::NativeFn) -> JsValue {
    let callback = Interpreter::native("", function, &i.function_proto);
    let JsValue::Object(object) = &callback else { unreachable!() };
    object.borrow_mut().func.as_mut().unwrap().home = Some(state.clone());
    callback
}
fn from_async_get_method(i: &mut Interpreter, value: &JsValue, key: impl Into<PropertyKey>) -> Result<Option<JsValue>, Throw> {
    if value.is_null() || value.is_undefined() { return Err(Throw::type_error("iterator source is nullish")); }
    let method = i.get_prop(value, key)?;
    if method.is_null() || method.is_undefined() { return Ok(None); }
    if callable_function(&method).is_none() { return Err(Throw::type_error("iterator method is not callable")); }
    Ok(Some(method))
}
fn from_async_finish(i: &mut Interpreter, state: &Obj, result: Result<JsValue, Throw>) -> Result<(), Throw> {
    if from_async_field(state, "done").to_boolean() { return Ok(()); }
    state.borrow_mut().set_own("done", JsValue::Bool(true));
    let capability = from_async_object(state, "capability");
    let fate = match result {
        Ok(value) => PromiseFate::Fulfilled(value),
        Err(error) => PromiseFate::Rejected(i.normalize_throw(error).0),
    };
    finish_promise_capability(&capability, fate, i)
}
fn from_async_await(i: &mut Interpreter, state: &Obj, stage: u8, value: JsValue) -> Result<(), Throw> {
    let intrinsic = from_async_field(state, "intrinsic");
    let awaited = promise_resolve_constructor(i, &intrinsic, value)?;
    let promise = promise_object(&awaited)?;
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("state", JsValue::Object(state.clone()));
    data.borrow_mut().set_own("stage", JsValue::num(stage as f64));
    let fulfilled = from_async_callback(i, &data, native_from_async_fulfilled);
    let rejected = from_async_callback(i, &data, native_from_async_rejected);
    perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None)
}
fn native_from_async_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    from_async_resume(i, &from_async_object(&data, "state"), from_async_index(&data, "stage") as u8,
        Ok(args.first().cloned().unwrap_or(JsValue::Undefined)))?;
    Ok(JsValue::Undefined)
}
fn native_from_async_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    from_async_resume(i, &from_async_object(&data, "state"), from_async_index(&data, "stage") as u8,
        Err(Throw(args.first().cloned().unwrap_or(JsValue::Undefined))))?;
    Ok(JsValue::Undefined)
}

fn from_async_complete(i: &mut Interpreter, state: &Obj) -> Result<(), Throw> {
    let target = from_async_field(state, "target");
    array_set(i, &target, "length", JsValue::num(from_async_index(state, "index") as f64))?;
    from_async_finish(i, state, Ok(target))
}
fn from_async_close(i: &mut Interpreter, state: &Obj, error: Throw) -> Result<(), Throw> {
    if !from_async_field(state, "iterating").to_boolean() { return from_async_finish(i, state, Err(error)); }
    let error = i.normalize_throw(error);
    let iterator = from_async_field(state, "iterator");
    let result = (|| {
        let Some(method) = from_async_get_method(i, &iterator, "return")? else { return Ok(None); };
        i.call_value(&method, iterator.clone(), Vec::new()).map(Some)
    })();
    if let Ok(Some(value)) = result {
        state.borrow_mut().set_own("close_error", error.0.clone());
        if from_async_await(i, state, FROM_ASYNC_CLOSE, value).is_ok() { return Ok(()); }
    }
    from_async_finish(i, state, Err(error))
}
fn from_async_store(i: &mut Interpreter, state: &Obj, value: JsValue) -> Result<(), Throw> {
    let index = from_async_index(state, "index");
    let target = from_async_field(state, "target");
    if let Err(error) = array_create_data_property(i, &target, index, value) {
        return from_async_close(i, state, error);
    }
    state.borrow_mut().set_own("index", JsValue::num(index as f64 + 1.0));
    from_async_start_step(i, state)
}
fn from_async_map(i: &mut Interpreter, state: &Obj, value: JsValue) -> Result<(), Throw> {
    let mapper = from_async_field(state, "mapper");
    if mapper.is_undefined() { return from_async_store(i, state, value); }
    let index = from_async_index(state, "index");
    let mapped = i.call_value(&mapper, from_async_field(state, "this_arg"), vec![value, JsValue::num(index as f64)]);
    let result = mapped.and_then(|value| from_async_await(i, state, FROM_ASYNC_MAPPED_VALUE, value));
    match result { Ok(()) => Ok(()), Err(error) => from_async_close(i, state, error) }
}
fn from_async_start_step(i: &mut Interpreter, state: &Obj) -> Result<(), Throw> {
    let index = from_async_index(state, "index");
    if from_async_field(state, "iterating").to_boolean() {
        if index >= 9_007_199_254_740_991 {
            return from_async_close(i, state, Throw::type_error("Array.fromAsync exceeds maximum index"));
        }
        let value = i.call_value(&from_async_field(state, "next"), from_async_field(state, "iterator"), Vec::new())?;
        from_async_await(i, state, FROM_ASYNC_ITERATOR_NEXT, value)
    } else if index >= from_async_index(state, "length") {
        from_async_complete(i, state)
    } else {
        let value = i.get_prop(&from_async_field(state, "source"), index.to_string())?;
        from_async_await(i, state, FROM_ASYNC_ARRAY_VALUE, value)
    }
}
fn from_async_resume(i: &mut Interpreter, state: &Obj, stage: u8, result: Result<JsValue, Throw>) -> Result<(), Throw> {
    if from_async_field(state, "done").to_boolean() { return Ok(()); }
    let resumed = (|| match stage {
        FROM_ASYNC_ARRAY_VALUE => from_async_map(i, state, result?),
        FROM_ASYNC_ITERATOR_NEXT => {
            let next = result?;
            if !next.is_object() { return Err(Throw::type_error("async iterator result is not an object")); }
            if i.get_prop(&next, "done")?.to_boolean() { return from_async_complete(i, state); }
            let value = i.get_prop(&next, "value")?;
            from_async_map(i, state, value)
        }
        FROM_ASYNC_MAPPED_VALUE => match result {
            Ok(value) => from_async_store(i, state, value),
            Err(error) => from_async_close(i, state, error),
        },
        FROM_ASYNC_CLOSE => from_async_finish(i, state, Err(Throw(from_async_field(state, "close_error")))),
        _ => unreachable!(),
    })();
    match resumed { Ok(()) => Ok(()), Err(error) => from_async_finish(i, state, Err(error)) }
}

fn native_array_from_async(i: &mut Interpreter, constructor: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let intrinsic = i.intrinsic_promise_constructor.clone();
    let capability = new_promise_capability(i, &intrinsic)?;
    let promise = capability_promise(&capability);
    let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    {
        let mut data = state.borrow_mut();
        data.set_own("capability", JsValue::Object(capability));
        data.set_own("intrinsic", intrinsic.clone());
        data.set_own("done", JsValue::Bool(false));
        data.set_own("index", JsValue::num(0.0));
        data.set_own("mapper", args.get(1).cloned().unwrap_or(JsValue::Undefined));
        data.set_own("this_arg", args.get(2).cloned().unwrap_or(JsValue::Undefined));
    }
    let initialized = (|| {
        let mapper = from_async_field(&state, "mapper");
        if !mapper.is_undefined() && callable_function(&mapper).is_none() {
            return Err(Throw::type_error("Array.fromAsync mapFn is not callable"));
        }
        let source = args.first().cloned().unwrap_or(JsValue::Undefined);
        let async_method = from_async_get_method(i, &source, WellKnownSymbol::AsyncIterator.symbol())?;
        let method = if async_method.is_some() { async_method.clone() }
            else { from_async_get_method(i, &source, WellKnownSymbol::Iterator.symbol())? };
        if let Some(method) = method {
            let iterator = i.call_value(&method, source, Vec::new())?;
            if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
            let next = i.get_prop(&iterator, "next")?;
            let (iterator, next) = if async_method.is_some() { (iterator, next) }
                else { from_async_sync_adapter(i, iterator, next, intrinsic.clone()) };
            let target = if is_constructor(&constructor) { i.construct(&constructor, Vec::new())? }
                else { array_create(i, 0)? };
            let mut data = state.borrow_mut();
            data.set_own("iterating", JsValue::Bool(true));
            data.set_own("iterator", iterator);
            data.set_own("next", next);
            data.set_own("target", target);
        } else {
            let source = array_receiver(i, source)?;
            let length = array_search_length(i, &source)?;
            let target = if is_constructor(&constructor) { i.construct(&constructor, vec![JsValue::num(length as f64)])? }
                else { array_create(i, length)? };
            let mut data = state.borrow_mut();
            data.set_own("iterating", JsValue::Bool(false));
            data.set_own("source", source);
            data.set_own("length", JsValue::num(length as f64));
            data.set_own("target", target);
        }
        from_async_start_step(i, &state)
    })();
    if let Err(error) = initialized { from_async_finish(i, &state, Err(error))?; }
    Ok(promise)
}

// Private AsyncFromSyncIterator next/return adapter used by this operation.
// The original sync iterator and cached next method stay in native home state.
fn from_async_sync_adapter(i: &Interpreter, iterator: JsValue, next: JsValue, intrinsic: JsValue) -> (JsValue, JsValue) {
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("iterator", iterator);
    data.borrow_mut().set_own("next", next);
    data.borrow_mut().set_own("intrinsic", intrinsic);
    let next = from_async_callback(i, &data, native_from_async_sync_next);
    let return_method = from_async_callback(i, &data, native_from_async_sync_return);
    let mut wrapper = JsObject::ordinary();
    wrapper.set_own("next", next.clone());
    wrapper.set_own("return", return_method);
    (JsValue::object(wrapper), next)
}
fn from_async_sync_close(i: &mut Interpreter, iterator: &JsValue) {
    if let Ok(Some(method)) = from_async_get_method(i, iterator, "return") {
        // IteratorClose with an original throw completion preserves that
        // completion even if return throws, returns a primitive or a Promise.
        let _ = i.call_value(&method, iterator.clone(), Vec::new());
    }
}
fn from_async_iterator_result(i: &Interpreter, value: JsValue, done: bool) -> JsValue {
    let mut result = JsObject::ordinary();
    result.proto = Some(i.object_proto.clone());
    result.set_own("value", value);
    result.set_own("done", JsValue::Bool(done));
    JsValue::object(result)
}
fn from_async_sync_continue(i: &mut Interpreter, record: &Obj, capability: &Obj,
    result: JsValue, close_on_rejection: bool) -> Result<(), Throw> {
    if !result.is_object() { return Err(Throw::type_error("sync iterator result is not an object")); }
    let done = i.get_prop(&result, "done")?.to_boolean();
    let value = i.get_prop(&result, "value")?;
    let intrinsic = from_async_field(record, "intrinsic");
    let value = match promise_resolve_constructor(i, &intrinsic, value) {
        Ok(value) => value,
        Err(error) => {
            if !done && close_on_rejection { from_async_sync_close(i, &from_async_field(record, "iterator")); }
            return Err(error);
        }
    };
    let promise = promise_object(&value)?;
    let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    data.borrow_mut().set_own("capability", JsValue::Object(capability.clone()));
    data.borrow_mut().set_own("iterator", from_async_field(record, "iterator"));
    data.borrow_mut().set_own("done", JsValue::Bool(done));
    data.borrow_mut().set_own("close_on_rejection", JsValue::Bool(close_on_rejection));
    let fulfilled = from_async_callback(i, &data, native_from_async_sync_fulfilled);
    let rejected = from_async_callback(i, &data, native_from_async_sync_rejected);
    perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None)
}
fn native_from_async_sync_fulfilled(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    let value = from_async_iterator_result(i, args.first().cloned().unwrap_or(JsValue::Undefined),
        from_async_field(&data, "done").to_boolean());
    finish_promise_capability(&from_async_object(&data, "capability"), PromiseFate::Fulfilled(value), i)?;
    Ok(JsValue::Undefined)
}
fn native_from_async_sync_rejected(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    if !from_async_field(&data, "done").to_boolean() && from_async_field(&data, "close_on_rejection").to_boolean() {
        from_async_sync_close(i, &from_async_field(&data, "iterator"));
    }
    finish_promise_capability(&from_async_object(&data, "capability"),
        PromiseFate::Rejected(args.first().cloned().unwrap_or(JsValue::Undefined)), i)?;
    Ok(JsValue::Undefined)
}
fn from_async_sync_call(i: &mut Interpreter, record: &Obj, args: Vec<JsValue>, returning: bool) -> Result<JsValue, Throw> {
    let capability = new_promise_capability(i, &from_async_field(record, "intrinsic"))?;
    let promise = capability_promise(&capability);
    let result = (|| {
        let iterator = from_async_field(record, "iterator");
        let method = if returning { from_async_get_method(i, &iterator, "return")? }
            else { Some(from_async_field(record, "next")) };
        let Some(method) = method else {
            let result = from_async_iterator_result(i, args.first().cloned().unwrap_or(JsValue::Undefined), true);
            return finish_promise_capability(&capability, PromiseFate::Fulfilled(result), i);
        };
        let value = i.call_value(&method, iterator, args.into_iter().take(1).collect())?;
        from_async_sync_continue(i, record, &capability, value, !returning)
    })();
    if let Err(error) = result {
        finish_promise_capability(&capability, PromiseFate::Rejected(i.normalize_throw(error).0), i)?;
    }
    Ok(promise)
}
fn native_from_async_sync_next(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let record = native_data(i)?;
    from_async_sync_call(i, &record, args, false)
}
fn native_from_async_sync_return(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let record = native_data(i)?;
    from_async_sync_call(i, &record, args, true)
}

#[cfg(test)]
mod array_from_async_tests {
    use super::*;

    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend(source, backend).unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("result").unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn from_async_sequential_arraylike_and_sync_values_await_mapping() {
        check(r#"
            var result=false, events=[], receiver={};
            var source={length:2,get 0(){events.push('get0');return {then(r){events.push('value0');r(3);}};},
                get 1(){events.push('get1');return Promise.resolve(4);}};
            Array.fromAsync(source,function(v,k){events.push('map'+k);if(this!==receiver)throw 1;
                return {then(r){events.push('mapped'+k);r(v+k);}};},receiver).then(a=>{
                result=a.join(',')==='3,5' && events.join(',')==='get0,value0,map0,mapped0,get1,map1,mapped1';
                return Array.fromAsync([Promise.resolve(6),{then(r){r(7);}}]);
            }).then(a=>{result=result && a.join(',')==='6,7';});
        "#);
    }

    #[test]
    fn from_async_async_iterator_keeps_values_and_awaits_mapped_values() {
        check(r#"
            var result=false, gets=0, calls=0, payload=Promise.resolve(8);
            var source={[Symbol.asyncIterator](){return {get next(){gets++;return function(){calls++;return Promise.resolve(
                calls===1 ? {value:payload,done:false} : {get value(){throw 99;},done:true});};}};},
                [Symbol.iterator](){throw 98;}};
            Array.fromAsync(source).then(a=>{result=gets===1 && calls===2 && a.length===1 && a[0]===payload;
                calls=0;return Array.fromAsync(source,(v,k)=>v);}).then(a=>{
                    result=result && gets===2 && calls===2 && a[0]===8;});
        "#);
    }

    #[test]
    fn from_async_close_preserves_original_throw_and_awaits_return() {
        check(r#"
            var result=false, original={}, other={}, close=0, events=[];
            var source={[Symbol.asyncIterator](){return {next(){return {value:1,done:false};},return(){close++;
                return {then(r,j){events.push('close');j(other);}};}};}};
            Array.fromAsync(source,()=>{throw original;}).then(()=>{result=false;},e=>{
                result=e===original && close===1 && events.join(',')==='close';
                return Array.fromAsync({[Symbol.iterator](){return {next(){return {value:Promise.reject(original),done:false};},
                    return(){close++;throw other;}};}}).then(()=>{result=false;},e=>{result=result && e===original && close===2;});
            });
        "#);
    }

    #[test]
    fn from_async_next_failures_do_not_close_and_initial_errors_reject() {
        check(r#"
            var result=false, original={}, closes=0, calls=0;
            var source={[Symbol.asyncIterator](){return {next(){throw original;},return(){closes++;return {};}};}};
            var p=Array.fromAsync(source);result=p instanceof Promise;
            p.then(()=>{result=false;},e=>{result=result && e===original && closes===0;
                return Array.fromAsync({get [Symbol.asyncIterator](){calls++;throw original;}},42)
                    .then(()=>{result=false;},e=>{result=result && e instanceof TypeError && calls===0;});
            }).then(()=>Array.fromAsync(null).then(()=>{result=false;},e=>{result=result && e instanceof TypeError;}));
        "#);
    }

    #[test]
    fn from_async_constructor_arguments_semantic_properties_and_internal_await() {
        check(r#"
            var result=false, sizes=[], defines=[], completed=0;
            function C(n){sizes.push(arguments.length+':'+n);return new Proxy({},{defineProperty(t,k,d){defines.push(k);
                return Reflect.defineProperty(t,k,d);}});}
            var value=Promise.resolve(9);Object.defineProperty(value,'then',{get(){throw 98;}});
            Array.fromAsync.call(C,{0:value,length:1}).then(a=>{
                result=a[0]===9 && a.length===1 && sizes.join(',')==='1:1' && defines.join(',')==='0,length';
                return Array.fromAsync.call(C,[2]);
            }).then(a=>{result=result && a[0]===2 && sizes[1]==='0:undefined';
                var closed=0;function Frozen(){return Object.preventExtensions({});}
                return Array.fromAsync.call(Frozen,{[Symbol.asyncIterator](){return {next(){return {value:1,done:false};},
                    return(){closed++;return {done:true};}};}}).then(()=>{result=false;},e=>{result=result && e instanceof TypeError && closed===1;});
            });
        "#);
    }
}
