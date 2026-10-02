// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Promise.prototype.finally and its then/catch closures adapted to Amazing's
// native callback home objects, species construction and resolution helpers.

fn finally_callback(
    i: &Interpreter, callback: crate::value::NativeFn, data: &Obj, arity: usize,
) -> JsValue {
    let function = Interpreter::native_arity("", callback, &i.function_proto, arity);
    if let JsValue::Object(function) = &function {
        function.borrow_mut().func.as_mut().unwrap().home = Some(data.clone());
    }
    function
}

fn native_promise_finally(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Promise.finally receiver is not an object")); }
    let constructor = promise_species_constructor(i, &this)?;
    let on_finally = args.first().cloned().unwrap_or(JsValue::Undefined);
    let (fulfilled, rejected) = if callable_function(&on_finally).is_some() {
        let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        data.borrow_mut().set_own("constructor", constructor);
        data.borrow_mut().set_own("onFinally", on_finally);
        (finally_callback(i, native_then_finally, &data, 1),
            finally_callback(i, native_catch_finally, &data, 1))
    } else { (on_finally.clone(), on_finally) };
    let then = i.get_prop(&this, "then")?;
    i.call_value(&then, this, vec![fulfilled, rejected])
}

fn native_then_finally(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    run_finally_cleanup(i, args, false)
}

fn native_catch_finally(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    run_finally_cleanup(i, args, true)
}

fn run_finally_cleanup(i: &mut Interpreter, args: Vec<JsValue>, rejected: bool) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    let constructor = data.borrow().get_own("constructor").unwrap();
    let callback = data.borrow().get_own("onFinally").unwrap();
    let result = i.call_value(&callback, JsValue::Undefined, Vec::new())?;
    let promise = promise_resolve_constructor(i, &constructor, result)?;
    let captured = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    captured.borrow_mut().set_own("value", args.first().cloned().unwrap_or(JsValue::Undefined));
    let thunk = finally_callback(i, if rejected { native_finally_throw } else { native_finally_value }, &captured, 0);
    let then = i.get_prop(&promise, "then")?;
    i.call_value(&then, promise, vec![thunk])
}

fn native_finally_value(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(native_data(i)?.borrow().get_own("value").unwrap())
}

fn native_finally_throw(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Err(Throw(native_data(i)?.borrow().get_own("value").unwrap()))
}

#[cfg(test)]
mod promise_finally_tests {
    use super::*;

    #[test]
    fn finally_is_generic_deferred_and_awaits_cleanup() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for (source, expected) in [
                "var events=[];var p=Promise.resolve(7);p.finally(()=>{events.push('cleanup')}).then(v=>events.push(v));events.push('sync');",
                "var events=[],resolve;var p=new Promise(r=>resolve=r);var q=p.finally(()=>{events.push('cleanup')});if(q===p)throw 'identity';q.then(v=>events.push(v));events.push('sync');resolve(7);",
                "var events=[];Promise.reject(7).finally(()=>{events.push('cleanup')}).catch(v=>events.push(v));events.push('sync');",
                "var events=[];Promise.resolve(0).finally(()=>Promise.reject(7)).catch(v=>events.push(v));events.push('sync');",
                "var events=[];Promise.resolve(0).finally(()=>{throw 7}).catch(v=>events.push(v));events.push('sync');",
                "var events=[],resolve;var cleanup=new Promise(r=>resolve=r);Promise.resolve(7).finally(()=>cleanup).then(v=>events.push(v));events.push('sync');queueMicrotask(()=>resolve());",
                "var events=[];var receiver={then(a,b){events.push(a===3&&b===3)}};Promise.prototype.finally.call(receiver,3);",
                "var events=[];class P extends Promise{};var p=P.resolve(7);var q=p.finally(()=>{});events.push(q instanceof P);q.then(v=>events.push(v));",
                "var events=[],fulfill,reject;var p=Promise.resolve();p.then=function(a,b){fulfill=a;reject=b;return 7};var result=p.finally(()=>{});events.push(result===7&&fulfill.name===''&&reject.name===''&&fulfill.length===1&&reject.length===1);",
            ].into_iter().zip([
                "sync,cleanup,7", "sync,cleanup,7", "sync,cleanup,7", "sync,7",
                "sync,7", "sync,7", "true", "true,7", "true",
            ]) {
                let mut i = Interpreter::new();
                i.run_script_with_backend(source, backend).unwrap_or_else(|error|panic!("{backend:?}: {source}: {error:?}"));
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("events.join(',')").unwrap(), JsValue::str(expected), "{backend:?}: {source}");
            }
        }
    }
}
