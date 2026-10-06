// %AsyncIteratorPrototype%[@@asyncDispose], ECMA-262 27.1.4.1.
// Uses the existing Boa-backed PromiseCapability/PromiseResolve/PerformPromiseThen
// gateways; never calls an observable Promise.resolve, then, or species method.

fn native_async_iterator_dispose(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let intrinsic = i.intrinsic_promise_constructor.clone();
    let capability = new_promise_capability(i, &intrinsic)?;
    let result = (|| {
        if this.is_null() || this.is_undefined() {
            return Err(Throw::type_error("async iterator receiver is nullish"));
        }
        let method = i.get_prop(&this, "return")?;
        if method.is_null() || method.is_undefined() {
            return finish_promise_capability(&capability, PromiseFate::Fulfilled(JsValue::Undefined), i);
        }
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("async iterator return is not callable"));
        }
        let result = i.call_value(&method, this, Vec::new())?;
        let wrapper = promise_resolve_constructor(i, &intrinsic, result)?;
        let promise = promise_object(&wrapper)?;
        let fulfilled = Interpreter::native_arity("", native_async_iterator_dispose_fulfilled, &i.function_proto, 1);
        perform_promise_then(i, &promise, Some(fulfilled), None, Some(capability.clone()))
    })();
    if let Err(error) = result {
        let reason = i.normalize_throw(error).0;
        finish_promise_capability(&capability, PromiseFate::Rejected(reason), i)?;
    }
    Ok(capability_promise(&capability))
}

fn native_async_iterator_dispose_fulfilled(_: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Undefined)
}

#[cfg(test)]
mod async_iterator_dispose_tests {
    use super::*;

    fn check(source: &str) {
        for native in [false, true] {
            let mut i = Interpreter::new();
            i.test_force_ast_callables(!native);
            i.run_script_with_backend("var valid=true; var proto=Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())));var dispose=proto[Symbol.asyncDispose];", if native { ExecutionBackend::Bytecode } else { ExecutionBackend::TreeWalk }).unwrap();
            i.run_script_with_backend(source, if native { ExecutionBackend::Bytecode } else { ExecutionBackend::TreeWalk })
                .unwrap_or_else(|error| panic!("native={native}: {}", error.describe()));
            if source.contains("async function* g()") {
                let JsValue::Object(function) = i.run_script("g").unwrap() else { panic!("generator callable") };
                let function = function.borrow();
                match &function.func.as_ref().unwrap().body {
                    FunctionBody::Bytecode(_) if native => {},
                    FunctionBody::Block(_) if !native => {},
                    other => panic!("native={native}: {other:?}"),
                }
            }
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("valid").unwrap(), JsValue::Bool(true), "native={native}");
        }
    }

    #[test]
    fn installed_metadata_and_nonconstructor_contract() {
        check(r#"var d=Object.getOwnPropertyDescriptor(proto,Symbol.asyncDispose);
        valid=typeof dispose==='function'&&dispose.name==='[Symbol.asyncDispose]'&&dispose.length===0&&d.writable&&!d.enumerable&&d.configurable&&!Object.hasOwn(dispose,'prototype');
        try {new dispose();valid=false}catch(e){valid=valid&&e instanceof TypeError}"#);
    }

    #[test]
    fn absent_nullish_return_fulfills_undefined_for_generic_receivers() {
        check(r#"for(var x of [{},{return:null},{return:undefined},1,true,'x',Symbol('x')]){
        dispose.call(x).then(v=>{valid=valid&&v===undefined},()=>{valid=false});}"#);
    }

    #[test]
    fn return_getter_once_receiver_and_zero_arguments() {
        check(r#"var gets=0,calls=0;var x={get return(){gets++;return function(){calls++;valid=valid&&this===x&&arguments.length===0;return 7}}};
        dispose.call(x).then(v=>{valid=valid&&v===undefined&&gets===1&&calls===1},()=>{valid=false});"#);
    }

    #[test]
    fn abrupt_getter_call_and_noncallable_reject_without_sync_throw() {
        check(r#"var marker={};var checks=0;for(var x of [{get return(){throw marker}},{return(){throw marker}},{return:1},null,undefined]){
        var p=dispose.call(x);p.then(()=>{valid=false},e=>{checks++;valid=valid&&(e===marker||e instanceof TypeError)});}Promise.resolve().then(()=>{valid=valid&&checks===5});"#);
    }

    #[test]
    fn return_thenable_getter_rejection_and_fulfillment_are_assimilated() {
        check(r#"var marker={},reads=0,calls=0;dispose.call({return(){return {get then(){reads++;return function(resolve){calls++;resolve(99)}}}}}).then(v=>{valid=valid&&v===undefined&&reads===1&&calls===1},()=>{valid=false});
        dispose.call({return(){return {get then(){throw marker}}}}).then(()=>{valid=false},e=>{valid=valid&&e===marker});"#);
    }

    #[test]
    fn pending_return_and_rejected_promise_preserve_reason_and_discard_result() {
        check(r#"var resolve,seen=false,marker={};var pending=new Promise(r=>resolve=r);
        dispose.call({return(){return pending}}).then(v=>{seen=true;valid=valid&&v===undefined},()=>{valid=false});valid=valid&&!seen;resolve(42);
        dispose.call({return(){return Promise.reject(marker)}}).then(()=>{valid=false},e=>{valid=valid&&e===marker});"#);
    }

    #[test]
    fn internal_reactions_ignore_author_then_species_and_mutable_global_promise() {
        check(r#"var p=Promise.resolve(7),reads=0;Object.defineProperty(p,'then',{get(){reads++;throw 98}});Object.defineProperty(Promise,Symbol.species,{get(){reads++;throw 97}});
        var OriginalPromise=Promise;Promise=function(){throw 96};var result=dispose.call({return(){return p}});valid=valid&&reads===0;Promise=OriginalPromise;delete OriginalPromise[Symbol.species];result.then(v=>{valid=valid&&v===undefined&&reads===0},()=>{valid=false});"#);
    }

    #[test]
    fn revoked_and_callable_proxies_preserve_get_call_and_rejection_order() {
        check(r#"var trace=[],target={};var x=new Proxy(target,{get(t,k){trace.push(k);return new Proxy(function(){valid=valid&&this===x&&arguments.length===0;return 8},{apply(fn,receiver,args){trace.push('call');return Reflect.apply(fn,receiver,args)}})}});
        dispose.call(x).then(v=>{valid=valid&&v===undefined&&trace.join(',')==='return,call'},()=>{valid=false});
        var pair=Proxy.revocable({},{});pair.revoke();dispose.call(pair.proxy).then(()=>{valid=false},e=>{valid=valid&&e instanceof TypeError});"#);
    }

    #[test]
    fn actual_async_generator_return_runs_finally_after_suspension() {
        check(r#"var trace=[];async function* g(){try{yield 1;yield 2}finally{trace.push('finally')}}var it=g();
        it.next().then(r=>{valid=valid&&r.value===1;return dispose.call(it)}).then(v=>{valid=valid&&v===undefined&&trace.join(',')==='finally';return it.next()}).then(r=>{valid=valid&&r.done},()=>{valid=false});"#);
    }
    #[test]
    fn foreign_builtin_uses_creation_realm_intrinsics_and_rejection_errors() {
        let mut i = Interpreter::new();
        let global = i.binding_create_object(None).unwrap();
        let child = i.binding_create_realm(global).unwrap();
        let method = i.binding_eval_in_realm(child, "Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())))[Symbol.asyncDispose]").unwrap();
        let promise_proto = i.binding_eval_in_realm(child, "Promise.prototype").unwrap();
        let error_proto = i.binding_eval_in_realm(child, "TypeError.prototype").unwrap();
        i.define_global("foreignDispose", method);
        i.define_global("foreignPromisePrototype", promise_proto);
        i.define_global("foreignTypeErrorPrototype", error_proto);
        i.run_script("var valid=true;var p=foreignDispose.call(null);valid=Object.getPrototypeOf(p)===foreignPromisePrototype;p.then(()=>{valid=false},e=>{valid=valid&&Object.getPrototypeOf(e)===foreignTypeErrorPrototype});").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("valid").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn inherited_disposer_composes_with_await_using_and_generator_completion() {
        check(r#"var trace=[];async function f(){await using x=Object.create(proto);x.return=function(){trace.push('return');return Promise.resolve(123)};trace.push('body')}
        f().then(()=>{valid=valid&&trace.join(',')==='body,return'},()=>{valid=false});"#);
    }

    #[test]
    fn every_observable_reaction_settles_and_generator_disposal_completes() {
        for native in [false, true] {
            let mut i=Interpreter::new();
            i.test_force_ast_callables(!native);
            i.run_script_with_backend(r#"
                var count=0,valid=true,marker={},finish;
                var proto=Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})()))),dispose=proto[Symbol.asyncDispose];
                dispose.call({}).then(v=>{count++;valid=valid&&v===undefined});
                dispose.call({return(){return {then(resolve){resolve(7)}}}}).then(v=>{count++;valid=valid&&v===undefined});
                dispose.call({get return(){throw marker}}).then(()=>{valid=false},e=>{count++;valid=valid&&e===marker});
                dispose.call({return(){return Promise.reject(marker)}}).then(()=>{valid=false},e=>{count++;valid=valid&&e===marker});
                var pending=new Promise(r=>finish=r);dispose.call({return(){return pending}}).then(v=>{count++;valid=valid&&v===undefined});finish(9);
                async function* g(){try{yield 1}finally{valid=valid&&true}}var it=g();
                it.next().then(()=>dispose.call(it)).then(v=>{count++;valid=valid&&v===undefined;return it.next()}).then(r=>{count++;valid=valid&&r.done},()=>{valid=false});
            "#,if native {ExecutionBackend::Bytecode}else{ExecutionBackend::TreeWalk}).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("valid&&count===7").unwrap(),JsValue::Bool(true),"native={native}");
        }
    }

}
