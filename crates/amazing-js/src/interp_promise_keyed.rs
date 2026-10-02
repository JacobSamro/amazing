// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Keyed combinator entry, element closures and null-prototype result creation
// adapted to Amazing's private native callback records and Promise capabilities.

fn native_promise_all_keyed(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    keyed_combinator_entry(i, this, args, false)
}

fn native_promise_all_settled_keyed(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    keyed_combinator_entry(i, this, args, true)
}

fn keyed_combinator_entry(
    i: &mut Interpreter, constructor: JsValue, args: Vec<JsValue>, settled: bool,
) -> Result<JsValue, Throw> {
    let target = new_promise_capability(i, &constructor)?;
    let result = (|| {
        let resolve = i.get_prop(&constructor, "resolve")?;
        if callable_function(&resolve).is_none() { return Err(Throw::type_error("Promise resolve is not callable")); }
        let source = args.first().cloned().unwrap_or(JsValue::Undefined);
        let JsValue::Object(source) = source else { return Err(Throw::type_error("keyed Promise input is not an object")); };
        perform_keyed_combinator(i, &source, &constructor, &target, &resolve, settled)
    })();
    if let Err(error) = result {
        let error = i.normalize_throw(error).0;
        finish_promise_capability(&target, PromiseFate::Rejected(error), i)?;
    }
    Ok(capability_promise(&target))
}

fn perform_keyed_combinator(
    i: &mut Interpreter, source: &Obj, constructor: &JsValue, target: &Obj,
    resolve: &JsValue, settled: bool,
) -> Result<(), Throw> {
    let all_keys = i.own_property_keys(&JsValue::Object(source.clone()))?;
    let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    let keys = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    let values = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    state.borrow_mut().set_own("target", JsValue::Object(target.clone()));
    state.borrow_mut().set_own("keys", JsValue::Object(keys.clone()));
    state.borrow_mut().set_own("values", JsValue::Object(values.clone()));
    state.borrow_mut().set_own("count", JsValue::num(0.0));
    state.borrow_mut().set_own("remaining", JsValue::num(1.0));
    state.borrow_mut().set_own("settled", JsValue::Bool(settled));
    let mut index = 0;
    for key in all_keys {
        let descriptor = i.default_get_own_property(source, &key)?;
        if !descriptor.is_some_and(|descriptor| descriptor.enumerable) { continue; }
        let value = i.get_prop(&JsValue::Object(source.clone()), key.clone())?;
        let key = match key { PropertyKey::String(key) => JsValue::Str(key), PropertyKey::Symbol(key) => JsValue::Symbol(key) };
        keys.borrow_mut().set_own(index.to_string(), key);
        values.borrow_mut().set_own(index.to_string(), JsValue::Undefined);
        state.borrow_mut().set_own("count", JsValue::num((index + 1) as f64));
        let next = i.call_value(resolve, constructor.clone(), vec![value])?;
        let element = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        element.borrow_mut().set_own("state", JsValue::Object(state.clone()));
        element.borrow_mut().set_own("index", JsValue::num(index as f64));
        element.borrow_mut().set_own("called", JsValue::Bool(false));
        let fulfilled = promise_callback_with_data(i, native_keyed_fulfill, &element, 1);
        let rejected = if settled { promise_callback_with_data(i, native_keyed_reject, &element, 1) }
            else { target.borrow().get_own("reject").unwrap() };
        let remaining = state_remaining(&state)?;
        state.borrow_mut().set_own("remaining", JsValue::num((remaining + 1) as f64));
        let then = i.get_prop(&next, "then")?;
        i.call_value(&then, next, vec![fulfilled, rejected])?;
        index += 1;
    }
    if decrement_remaining(&state)? == 0 { keyed_finish_result(i, &state)?; }
    Ok(())
}

fn native_keyed_fulfill(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    keyed_element_completion(i, args, false)
}

fn native_keyed_reject(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    keyed_element_completion(i, args, true)
}

fn keyed_element_completion(i: &mut Interpreter, args: Vec<JsValue>, rejected: bool) -> Result<JsValue, Throw> {
    let element = native_data(i)?;
    if element.borrow().get_own("called") == Some(JsValue::Bool(true)) { return Ok(JsValue::Undefined); }
    element.borrow_mut().set_own("called", JsValue::Bool(true));
    let (state, _, index) = combinator_state(i)?;
    let settled = state.borrow().get_own("settled") == Some(JsValue::Bool(true));
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    let value = if settled {
        settled_record(i, if rejected { "rejected" } else { "fulfilled" },
            if rejected { "reason" } else { "value" }, value)
    } else { value };
    let JsValue::Object(values) = state_values(&state, "values")? else { unreachable!() };
    values.borrow_mut().set_own(index.to_string(), value);
    if decrement_remaining(&state)? == 0 { return keyed_finish_result(i, &state); }
    Ok(JsValue::Undefined)
}

fn keyed_finish_result(i: &mut Interpreter, state: &Obj) -> Result<JsValue, Throw> {
    let JsValue::Object(keys) = state_values(state, "keys")? else { unreachable!() };
    let JsValue::Object(values) = state_values(state, "values")? else { unreachable!() };
    let count = match state.borrow().get_own("count") { Some(JsValue::Num(count)) => count as usize, _ => unreachable!() };
    let result = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    for index in 0..count {
        let key = keys.borrow().get_own(index.to_string()).unwrap();
        let key = i.to_property_key(&key)?;
        let value = values.borrow().get_own(index.to_string()).unwrap();
        result.borrow_mut().set_own(key, value);
    }
    let Some(JsValue::Object(target)) = state.borrow().get_own("target") else { unreachable!() };
    let resolve = target.borrow().get_own("resolve").unwrap();
    i.call_value(&resolve, JsValue::Undefined, vec![JsValue::Object(result)])
}

#[cfg(test)]
mod keyed_promise_tests {
    use super::*;

    #[test]
    fn keyed_combinators_preserve_keys_and_settle_elements_once() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in [
                "var ok=false,s=Symbol();var input={a:Promise.resolve(1),b:2,[s]:3};Object.defineProperty(input,'__proto__',{value:4,enumerable:true});Promise.allKeyed(input).then(r=>{ok=Object.getPrototypeOf(r)===null&&r.a===1&&r.b===2&&r[s]===3&&r.__proto__===4});",
                "var ok=false;Promise.allSettledKeyed({a:Promise.resolve(1),b:Promise.reject(2)}).then(r=>{ok=Object.getPrototypeOf(r)===null&&r.a.status==='fulfilled'&&r.a.value===1&&r.b.status==='rejected'&&r.b.reason===2});",
                "var ok=false;class P extends Promise{};P.resolve=v=>({then(a,b){a(v);a(99);b(98)}});var q=P.allSettledKeyed({a:1,b:2});q.then(r=>{ok=q instanceof P&&r.a.value===1&&r.b.value===2});",
                "var ok=false,events=[];var input={get a(){events.push('a');delete input.b;return 1},b:2};Promise.allKeyed(input).then(r=>{ok=events.join(',')==='a'&&r.a===1&&!Object.hasOwn(r,'b')});",
                "var ok=false;Promise.allKeyed(7).catch(e=>{ok=e instanceof TypeError});",
                "var ok=false;Promise.allKeyed({}).then(r=>{ok=Object.getPrototypeOf(r)===null&&Reflect.ownKeys(r).length===0});",
            ] {
                let mut i = Interpreter::new();
                i.run_script_with_backend(source, backend).unwrap_or_else(|error|panic!("{backend:?}: {source}: {error:?}"));
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("ok").unwrap(), JsValue::Bool(true), "{backend:?}: {source}");
            }
        }
    }
}
