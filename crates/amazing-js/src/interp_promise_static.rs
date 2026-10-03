// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Promise.try and Promise.withResolvers adapted to existing capability records.

// Promise.try's 2026 normative revision calls before capability creation and
// uses PromiseResolve on normal completion to preserve constructor identity.
// https://tc39.es/ecma262/#sec-promise.try
fn native_promise_try(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    if !this.is_object() { return Err(Throw::type_error("Promise.try requires an object receiver")); }
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    match i.call_value(&callback, JsValue::Undefined, args.into_iter().skip(1).collect()) {
        Ok(value) => promise_resolve_constructor(i, &this, value),
        Err(error) => {
            let capability = new_promise_capability(i, &this)?;
            let reason = i.normalize_throw(error).0;
            finish_promise_capability(&capability, PromiseFate::Rejected(reason), i)?;
            Ok(capability_promise(&capability))
        }
    }
}

fn native_promise_with_resolvers(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let capability = new_promise_capability(i, &this)?;
    let result = i.object_value();
    let JsValue::Object(object) = &result else { unreachable!() };
    for name in ["promise", "resolve", "reject"] {
        let value = capability.borrow().get_own(name).unwrap();
        object.borrow_mut().define(name, Property::data(value)).unwrap();
    }
    Ok(result)
}

#[cfg(test)]
mod promise_static_tests {
    use super::*;

    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {}", error.describe()));
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("valid").unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn with_resolvers_returns_data_record_and_original_one_shot_resolvers() {
        check(r#"
            var valid=true, seen, rejected=false, record=Promise.withResolvers();
            valid=valid && Object.getPrototypeOf(record)===Object.prototype &&
                Object.keys(record).join(',')==='promise,resolve,reject' && record.promise instanceof Promise;
            for(var name of ['promise','resolve','reject']) {
                var d=Object.getOwnPropertyDescriptor(record,name);
                valid=valid && d.value===record[name] && d.writable && d.enumerable && d.configurable && !('get' in d);
            }
            for(var fn of [record.resolve,record.reject]) {
                valid=valid && typeof fn==='function' && fn.length===1 && fn.name==='' && !Object.hasOwn(fn,'prototype');
                try { Reflect.construct(fn,[]);valid=false; } catch(e) {valid=valid && e instanceof TypeError;}
            }
            record.promise.then(v=>seen=v,()=>rejected=true);
            record.resolve(7);record.reject(8);record.resolve(9);
            var denied=Promise.withResolvers(),sentinel={};
            denied.promise.then(()=>valid=false,e=>valid=valid && e===sentinel);
            denied.reject(sentinel);denied.resolve(3);
            queueMicrotask(()=>valid=valid && seen===7 && !rejected);
        "#);
    }

    #[test]
    fn with_resolvers_is_generic_and_ignores_species_and_resolve_properties() {
        check(r#"
            var valid=true, calls=0, resolve=function(){},reject=function(){},marker={};
            function C(executor) {calls++;valid=valid && arguments.length===1 && executor.length===2;
                executor(resolve,reject);return marker;}
            Object.defineProperty(C,'resolve',{get(){throw 1}});
            Object.defineProperty(C,Symbol.species,{get(){throw 2}});
            for(var constructor of [C,C.bind(null),new Proxy(C,{construct(t,a,n){
                valid=valid && n===constructor;return Reflect.construct(t,a,n);}})]) {
                var r=Promise.withResolvers.call(constructor);
                valid=valid && r.promise===marker && r.resolve===resolve && r.reject===reject &&
                    Object.getPrototypeOf(r)===Object.prototype;
            }
            valid=valid && calls===3;
            class Sub extends Promise {}
            var sub=Promise.withResolvers.call(Sub);
            valid=valid && sub.promise instanceof Sub;
        "#);
    }

    #[test]
    fn promise_try_calls_synchronously_with_undefined_this_and_forwarded_arguments() {
        check(r#"
            var valid=true, log=[], args=[{},2,3], value;
            var result=Promise.try(function(a,b,c){'use strict';log.push('callback');
                valid=valid && this===undefined && a===args[0] && b===2 && c===3;return 7;},...args);
            log.push('after');valid=valid && result instanceof Promise && log.join(',')==='callback,after';
            result.then(v=>value=v);
            queueMicrotask(()=>valid=valid && value===7);
            var order=[],thenable={get then(){order.push('get');return resolve=>{order.push('call');resolve(11)}}};
            Promise.try(()=>{order.push('callback');return thenable}).then(v=>valid=valid && v===11);
            order.push('after');valid=valid && order.join(',')==='callback,get,after';
        "#);
    }

    #[test]
    fn promise_try_rejects_original_callback_throw_and_actual_native_error() {
        check(r#"
            var valid=true, sentinel={}, ran=false;
            var result=Promise.try(()=>{throw sentinel});ran=true;
            valid=valid && result instanceof Promise && ran;
            result.then(()=>valid=false,e=>valid=valid && e===sentinel);
            Promise.try(null).then(()=>valid=false,e=>valid=valid && e instanceof TypeError);
            Promise.try(()=>{return null.missing}).then(()=>valid=false,e=>valid=valid && e instanceof TypeError);
            Promise.try(async()=>{throw sentinel}).then(()=>valid=false,e=>valid=valid && e===sentinel);
        "#);
    }

    #[test]
    fn promise_try_generic_capability_order_and_resolver_throws_are_synchronous() {
        check(r#"
            var valid=true, order=[], marker={}, rejection={}, seenThis;
            function C(executor) {order.push('construct');executor(
                function(v){'use strict';seenThis=this;order.push('resolve:'+v)},
                function(e){'use strict';seenThis=this;order.push(e===rejection?'reject':'wrong')});return marker;}
            var result=Promise.try.call(C,function(a){order.push('callback');return a},9);
            valid=valid && result===marker && seenThis===undefined && order.join(',')==='callback,construct,resolve:9';
            order=[];result=Promise.try.call(C,()=>{order.push('callback');throw rejection});
            valid=valid && result===marker && seenThis===undefined && order.join(',')==='callback,construct,reject';
            var sentinel={};
            for(var throwing of [false,true]) {
                function D(executor){executor(()=>{throw sentinel},()=>{throw sentinel});}
                try{Promise.try.call(D,()=>{if(throwing)throw rejection;return 3});valid=false;}
                catch(e){valid=valid && e===sentinel;}
            }
            var callbackRan=false;
            function Broken(executor){throw sentinel;}
            try{Promise.try.call(Broken,()=>callbackRan=true);valid=false;}catch(e){valid=valid && e===sentinel && callbackRan;}
        "#);
    }

    #[test]
    fn invalid_promise_static_receivers_and_capabilities_preserve_callback_order() {
        check(r#"
            var valid=true, ran;
            for(var fn of [Promise.try,Promise.withResolvers]) {
                for(var receiver of [undefined,null,1,'x',true,Symbol(),2n,{},()=>{}]) {
                    ran=false;
                    try{fn.call(receiver,()=>ran=true);valid=false;}
                    catch(e){valid=valid && e instanceof TypeError && ran===(fn===Promise.try && (typeof receiver==='object' && receiver!==null || typeof receiver==='function'));}
                }
                function Missing(executor) {}
                function Noncallable(executor) {executor(1,2);}
                for(var bad of [Missing,Noncallable]) {
                    ran=false;
                    try{fn.call(bad,()=>ran=true);valid=false;}
                    catch(e){valid=valid && e instanceof TypeError && ran===(fn===Promise.try);}
                }
                var revoked=Proxy.revocable(Promise,{});revoked.revoke();ran=false;
                try{fn.call(revoked.proxy,()=>ran=true);valid=false;}
                catch(e){valid=valid && e instanceof TypeError && ran===(fn===Promise.try);}
            }
        "#);
    }

    #[test]
    fn promise_try_reuses_matching_promises_and_propagates_constructor_get_abrupt() {
        check(r#"
            var valid=true, p=Promise.resolve(7), reads=0;
            Object.defineProperty(p,'constructor',{get(){reads++;return Promise},configurable:true});
            valid=valid && Promise.try(()=>p)===p && reads===1;
            class Sub extends Promise {}
            var sub=Sub.resolve(9);valid=valid && Sub.try(()=>sub)===sub;
            var wrapped=Promise.try(()=>sub);valid=valid && wrapped!==sub && wrapped instanceof Promise;
            var sentinel={},bad=Promise.resolve();
            Object.defineProperty(bad,'constructor',{get(){throw sentinel}});
            try{Promise.try(()=>bad);valid=false;}catch(e){valid=valid && e===sentinel;}
            var nonconstructor={},spoof=Promise.resolve();
            Object.defineProperty(spoof,'constructor',{value:nonconstructor});
            valid=valid && Promise.try.call(nonconstructor,()=>spoof)===spoof;
            var ignored=0;Object.defineProperty(Promise,'resolve',{get(){ignored++;throw sentinel}});
            valid=valid && Promise.try(()=>p)===p && ignored===0;
        "#);
    }

    #[test]
    fn promise_constructor_and_prototype_metadata_and_tag_are_complete() {
        check(r#"
            var valid=true, statics={resolve:1,reject:1,all:1,allSettled:1,any:1,race:1,try:1,withResolvers:0,allKeyed:1,allSettledKeyed:1};
            for(var name of Object.keys(statics)) {
                var f=Promise[name],d=Object.getOwnPropertyDescriptor(Promise,name);
                valid=valid && f.name===name && f.length===statics[name] && d.value===f && d.writable && !d.enumerable && d.configurable;
                try{Reflect.construct(f,[]);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
            }
            for(var name of ['then','catch','finally']) {
                var d=Object.getOwnPropertyDescriptor(Promise.prototype,name);
                valid=valid && d.value.name===name && d.value.length===(name==='then'?2:1) && d.writable && !d.enumerable && d.configurable;
            }
            var tag=Object.getOwnPropertyDescriptor(Promise.prototype,Symbol.toStringTag);
            valid=valid && tag.value==='Promise' && !tag.writable && !tag.enumerable && tag.configurable;
            valid=valid && Object.prototype.toString.call(Promise.prototype)==='[object Promise]' &&
                Object.prototype.toString.call(Promise.resolve())==='[object Promise]';
            var p=Promise.resolve();Object.defineProperty(p,Symbol.toStringTag,{value:undefined});
            valid=valid && Object.prototype.toString.call(p)==='[object Object]';
            delete Promise.prototype[Symbol.toStringTag];
            valid=valid && Object.prototype.toString.call(Promise.resolve())==='[object Object]';
        "#);
    }

    #[test]
    fn foreign_promise_statics_use_callee_record_and_error_realms() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for (name, source) in [
                ("foreignTry", "Promise.try"), ("foreignWith", "Promise.withResolvers"),
                ("foreignObjectProto", "Object.prototype"), ("foreignTypeErrorProto", "TypeError.prototype"),
                ("foreignPromise", "Promise"),
            ] {
                let value = i.binding_eval_in_realm(child, source).unwrap();
                i.global_env.declare_var(name, value).unwrap();
            }
            i.run_script_with_backend(r#"
                var valid=true, record=foreignWith.call(Promise),reason;
                valid=valid && Object.getPrototypeOf(record)===foreignObjectProto && record.promise instanceof Promise;
                foreignTry.call(Promise,null).catch(e=>reason=e);
                try{foreignWith.call({});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignTypeErrorProto;}
                var other=foreignTry.call(foreignPromise,()=>7);
                valid=valid && other instanceof foreignPromise;
            "#, backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(i.run_script("valid && Object.getPrototypeOf(reason)===foreignTypeErrorProto").unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }
}
