// Portions ported from boa-dev/boa (core/engine/src/builtins/error/{mod,aggregate,type}.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Error-family allocation, metadata and iterable algorithms adapted to Amazing.
// SuppressedError reuses the ported allocation/message algorithm with the
// current ECMA-26220.5.8 sequence (message, error, suppressed; no cause).
// Error-family construction, ECMA-262 §20.5.
// https://tc39.es/ecma262/multipage/fundamental-objects.html#sec-error-objects
fn runtime_is_error_constructor(native: crate::value::NativeFn) -> bool {
    [native_error as crate::value::NativeFn, native_eval_error, native_type_error,
        native_range_error, native_reference_error, native_syntax_error, native_uri_error,
        native_aggregate_error, native_suppressed_error]
        .iter().any(|candidate| std::ptr::fn_addr_eq(native, *candidate))
}

fn runtime_error_object(i: &mut Interpreter, message: &JsValue) -> Result<Obj, Throw> {
    let constructor = JsValue::Object(i.native_stack.last().cloned()
        .ok_or_else(|| Throw::type_error("missing Error constructor context"))?);
    let target = i.current_new_target().unwrap_or_else(|| constructor.clone());
    // OrdinaryCreateFromConstructor reads the actual newTarget once. The named
    // intrinsic fallback is resolved in its semantic GetFunctionRealm only if
    // the prototype is not an object; global replacement cannot alter it.
    let prototype = match i.get_prop(&target, "prototype")? {
        JsValue::Object(prototype) => prototype,
        _ => i.constructor_fallback_prototype(&constructor, &target)?,
    };
    let mut object = JsObject::ordinary();
    object.exotic = Exotic::Error;
    object.proto = Some(prototype);
    if !message.is_undefined() {
        object.define("message", symbol_data(JsValue::str(i.to_string(message)?), true, true)).unwrap();
    }
    Ok(Obj(Rc::new(RefCell::new(object))))
}

fn runtime_error_iterator(i: &mut Interpreter, source: &JsValue) -> Result<StdIteratorRecord, Throw> {
    let method = i.get_prop(source, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("AggregateError errors are not iterable"));
    }
    let iterator = i.call_value(&method, source.clone(), Vec::new())?;
    if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
    let next = i.get_prop(&iterator, "next")?;
    Ok(StdIteratorRecord { iterator, next })
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
    let iterator = runtime_error_iterator(i, args.first().unwrap_or(&JsValue::Undefined))?;
    let mut errors = Vec::new();
    while let Some(value) = std_iterator_step(i, &iterator)? { errors.push(value); }
    let list = array_values(i, Vec::new())?;
    for (index, value) in errors.into_iter().enumerate() {
        array_create_data_property(i, &list, index, value)?;
    }
    let errors = list;
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

#[cfg(test)]
mod error_family_protocol_tests {
    use super::*;
    fn check(source:&str) {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}\n{source}",e.describe())),JsValue::Bool(true),"{backend:?}\n{source}");
        }
    }
    #[test]
    fn installed_error_family_descriptors_chains_and_brands_are_precise() {
        check(r#"
          var valid=true,names=['Error','EvalError','TypeError','RangeError','ReferenceError','SyntaxError','URIError','AggregateError','SuppressedError'];
          for(var name of names){var C=globalThis[name],p=C.prototype,d=Object.getOwnPropertyDescriptor(C,'prototype');
            valid=valid && !d.writable && !d.enumerable && !d.configurable && d.value===p && C.length===(name==='AggregateError'?2:name==='SuppressedError'?3:1);
            for(var key of ['name','length']){var d=Object.getOwnPropertyDescriptor(C,key);valid=valid && !d.writable && !d.enumerable && d.configurable;}
            valid=valid && C.name===name && Object.keys(C).length===0 && Object.keys(p).length===0 && p.name===name && p.message==='' && p.constructor===C;
            for(var key of ['name','message','constructor']){var d=Object.getOwnPropertyDescriptor(p,key);valid=valid && d.writable && !d.enumerable && d.configurable;}
            valid=valid && Object.getPrototypeOf(C)===(name==='Error'?Function.prototype:Error) && Object.getPrototypeOf(p)===(name==='Error'?Object.prototype:Error.prototype) && !Error.isError(p);
            var args=name==='AggregateError'?[[],'message']:name==='SuppressedError'?[1,2,'message']:['message'];
            var e=Reflect.apply(C,undefined,args),n=Reflect.construct(C,args);valid=valid && Error.isError(e) && Error.isError(n) && Object.getPrototypeOf(e)===p && e.message==='message';
          }
          var m=Error.prototype.toString,d=Object.getOwnPropertyDescriptor(Error.prototype,'toString');valid=valid && m.name==='toString' && m.length===0 && !m.hasOwnProperty('prototype') && d.writable && !d.enumerable && d.configurable;
          try{new m();valid=false;}catch(e){valid=valid && e instanceof TypeError;}valid;
        "#);
    }
    #[test]
    fn prototype_lookup_is_once_before_message_cause_and_aggregate_iteration() {
        check(r#"
          var valid=true,names=['Error','EvalError','TypeError','RangeError','ReferenceError','SyntaxError','URIError','AggregateError','SuppressedError'];
          for(var name of names){var C=globalThis[name],log=[],chosen={},Target=function(){},target=new Proxy(Target,{get(t,k,r){if(k==='prototype'){log.push('prototype');return chosen;}return Reflect.get(t,k,r);}});
            var message={toString(){log.push('message');return'm';}},options=new Proxy({cause:7},{has(t,k){log.push('has:'+k);return Reflect.has(t,k);},get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);}});
            var errors={get [Symbol.iterator](){log.push('iterator');return function(){return{next(){return{done:true};}}};}};
            var args=name==='AggregateError'?[errors,message,options]:name==='SuppressedError'?[1,2,message,options]:[message,options];
            var e=Reflect.construct(C,args,target),expected=name==='AggregateError'?'prototype,message,has:cause,get:cause,iterator':name==='SuppressedError'?'prototype,message':'prototype,message,has:cause,get:cause';
            valid=valid && Object.getPrototypeOf(e)===chosen && Error.isError(e) && log.join(',')===expected;
            valid=valid && Object.keys(e).length===0;
          }valid;
        "#);
    }
    #[test]
    fn prototype_abrupt_completion_precedes_all_argument_observation() {
        check(r#"
          var valid=true,error={},reads=0,names=['Error','EvalError','TypeError','RangeError','ReferenceError','SyntaxError','URIError','AggregateError','SuppressedError'];
          for(var name of names){var target=new Proxy(function(){},{get(t,k,r){if(k==='prototype')throw error;return Reflect.get(t,k,r);}}),message={toString(){reads++;throw'unexpected';}},options=new Proxy({},{has(){reads++;throw'unexpected';}}),errors={get [Symbol.iterator](){reads++;throw'unexpected';}};
            var args=name==='AggregateError'?[errors,message,options]:name==='SuppressedError'?[1,2,message,options]:[message,options];try{Reflect.construct(globalThis[name],args,target);valid=false;}catch(e){valid=valid && e===error && reads===0;}
          }valid;
        "#);
    }
    #[test]
    fn error_cause_observes_has_then_get_and_preserves_message_abrupt_identity() {
        check(r#"
          var valid=true,log=[],cause={},options=new Proxy({cause},{has(t,k){log.push('has:'+k);return true;},get(t,k,r){log.push('get:'+k);return cause;}}),message={[Symbol.toPrimitive](hint){log.push('message:'+hint);return'm';}};
          var e=Error(message,options);valid=valid && log.join(',')==='message:string,has:cause,get:cause' && e.cause===cause;
          var d=Object.getOwnPropertyDescriptor(e,'cause');valid=valid && d.writable && !d.enumerable && d.configurable;
          var e=Error(undefined,{cause:undefined});valid=valid && !e.hasOwnProperty('message') && e.hasOwnProperty('cause') && e.cause===undefined;
          var gets=0,e=Error(1n,new Proxy({},{has(){return false;},get(){gets++;throw'unused';}}));valid=valid && e.message==='1' && !e.hasOwnProperty('cause') && gets===0;
          var error={},message={toString(){throw error;}},options=new Proxy({},{has(){gets++;throw'unused';}});try{Error(message,options);valid=false;}catch(e){valid=valid && e===error && gets===0;}
          for(var stage of ['has','get']){var options=new Proxy({cause:1},{has(){if(stage==='has')throw error;return true;},get(){throw error;}});try{TypeError('m',options);valid=false;}catch(e){valid=valid && e===error;}}
          try{Error(Symbol(),options);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          valid=valid && Error(null,3).message==='null' && !Error('m','cause').hasOwnProperty('cause');valid;
        "#);
    }
    #[test]
    fn aggregate_strict_cached_iterator_streams_and_skips_terminal_value() {
        check(r#"
          var valid=true,nextGets=0,calls=0,source,iterator;
          source={[Symbol.iterator](){valid=valid && this===source && arguments.length===0;return iterator;}};
          iterator={get next(){nextGets++;return function(){valid=valid && this===iterator && arguments.length===0;calls++;Object.defineProperty(iterator,'next',{value:function(){throw'not cached';},configurable:true});return calls===1?{done:false,value:7}:{done:true,get value(){throw'unused terminal value';}};};}};
          var e=AggregateError(source,'m',{cause:8});valid=valid && nextGets===1 && calls===2 && e.errors.length===1 && e.errors[0]===7 && e.message==='m' && e.cause===8;
          var d=Object.getOwnPropertyDescriptor(e,'errors');valid=valid && d.writable && !d.enumerable && d.configurable;
          for(var source of [{next(){return{done:true};}}, {[Symbol.iterator]:3}, {[Symbol.iterator](){return 1;}}, {[Symbol.iterator](){return{next:3};}}]){try{AggregateError(source);valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          valid;
        "#);
    }
    #[test]
    fn aggregate_step_failures_do_not_close_and_keep_original_abrupt_values() {
        check(r#"
          var valid=true,error={},closes=0;
          for(var stage of ['nextGet','nextCall','done','value']){
            var iterator={get next(){if(stage==='nextGet')throw error;return function(){if(stage==='nextCall')throw error;return{get done(){if(stage==='done')throw error;return false;},get value(){throw error;}};};},get return(){closes++;throw'unused return';}};
            var source={[Symbol.iterator](){return iterator;}};try{AggregateError(source);valid=false;}catch(e){valid=valid && e===error && closes===0;}
          }
          var source={[Symbol.iterator](){return{next(){return 3;},get return(){closes++;throw'unused';}};}};try{AggregateError(source);valid=false;}catch(e){valid=valid && e instanceof TypeError && closes===0;}valid;
        "#);
    }
    #[test]
    fn aggregate_error_list_uses_own_data_properties_after_iteration() {
        check(r#"
          var valid=true,sets=0,iterationDone=false,one={},two={},source={[Symbol.iterator](){var n=0;return{next(){n++;if(n===3){iterationDone=true;return{done:true};}return{done:false,value:n===1?one:two};}};}};
          Object.defineProperty(Array.prototype,'0',{set(){sets++;},configurable:true});
          var e;try{e=new AggregateError(source);}finally{delete Array.prototype[0];}
          valid=valid && iterationDone && sets===0 && e.errors.length===2 && e.errors[0]===one && e.errors[1]===two && Object.getPrototypeOf(e.errors)===Array.prototype;
          var d=Object.getOwnPropertyDescriptor(e.errors,'0');valid=valid && d.writable && d.enumerable && d.configurable;valid;
        "#);
    }
    #[test]
    fn suppressed_error_preserves_distinct_fields_and_ignores_cause_options() {
        check(r#"
          var valid=true,error={},suppressed={},reads=0,options=new Proxy({cause:1},{has(){reads++;throw'unused';},get(){reads++;throw'unused';}});
          var e=SuppressedError(error,suppressed,'m',options);valid=valid && e.error===error && e.suppressed===suppressed && e.message==='m' && !e.hasOwnProperty('cause') && reads===0 && Reflect.ownKeys(e).join(',')==='message,error,suppressed';
          for(var key of ['message','error','suppressed']){var d=Object.getOwnPropertyDescriptor(e,key);valid=valid && d.writable && !d.enumerable && d.configurable;}
          var e=SuppressedError();valid=valid && !e.hasOwnProperty('message') && e.hasOwnProperty('error') && e.hasOwnProperty('suppressed') && e.error===undefined && e.suppressed===undefined && Error.isError(e);
          var message={toString(){throw error;}};try{SuppressedError(1,2,message,options);valid=false;}catch(e){valid=valid && e===error && reads===0;}valid;
        "#);
    }
    #[test]
    fn ordinary_error_string_conversion_order_and_brand_are_preserved() {
        check(r#"
          var valid=true,log=[],object={get name(){log.push('name');return{toString(){log.push('name string');return'N';}};},get message(){log.push('message');return{toString(){log.push('message string');return'M';}};}};
          valid=valid && Error.prototype.toString.call(object)==='N: M' && log.join(',')==='name,name string,message,message string';
          valid=valid && Error.prototype.toString.call({name:'',message:'m'})==='m' && Error.prototype.toString.call({name:'N',message:''})==='N' && Error.prototype.toString.call({})==='Error';
          var error={},gets=0,object={get name(){return{toString(){throw error;}};},get message(){gets++;throw'unused';}};try{Error.prototype.toString.call(object);valid=false;}catch(e){valid=valid && e===error && gets===0;}
          valid=valid && !Error.isError({name:'Error',message:'fake'}) && !Error.isError(new Proxy(new Error(),{})) && Object.prototype.toString.call(new Error())==='[object Error]';valid;
        "#);
    }
    #[test]
    fn foreign_new_target_fallback_uses_each_intrinsic_error_prototype() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for name in ["Error","EvalError","TypeError","RangeError","ReferenceError","SyntaxError","URIError","AggregateError","SuppressedError"] {
                for (alias,source) in [(format!("foreign{name}"),name.to_string()),(format!("foreign{name}Prototype"),format!("{name}.prototype"))] {
                    let value=i.binding_eval_in_realm(child,&source).unwrap();i.global_env.declare_var(&alias,value).unwrap();
                }
            }
            for (name,source) in [("foreignTarget","(function ForeignTarget(){})"),("foreignArray","Array.prototype"),("foreignType","TypeError.prototype")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            let source=r#"
              var valid=true,names=['Error','EvalError','TypeError','RangeError','ReferenceError','SyntaxError','URIError','AggregateError','SuppressedError'];foreignTarget.prototype=3;
              var targets=[foreignTarget,foreignTarget.bind(null),new Proxy(foreignTarget,{}),new Proxy(foreignTarget.bind(null),{})];
              for(var name of names){var C=globalThis[name],foreign=globalThis['foreign'+name],prototype=globalThis['foreign'+name+'Prototype'],args=name==='AggregateError'?[[],'m']:name==='SuppressedError'?[1,2,'m']:['m'];
                for(var target of targets){var e=Reflect.construct(C,args,target);valid=valid && Object.getPrototypeOf(e)===prototype && Error.isError(e);}
                var chosen={},target=new Proxy(function(){},{get(t,k,r){if(k==='prototype')return chosen;return Reflect.get(t,k,r);}}),e=Reflect.construct(foreign,args,target);valid=valid && Object.getPrototypeOf(e)===chosen;
                if(name==='AggregateError')valid=valid && Object.getPrototypeOf(e.errors)===foreignArray;
              }
              var chosen={},r;r=Proxy.revocable(foreignTarget,{get(t,k,receiver){if(k==='prototype'){r.revoke();return chosen;}return Reflect.get(t,k,receiver);}});var e=Reflect.construct(foreignError,[],r.proxy);valid=valid && Object.getPrototypeOf(e)===chosen;
              var r;r=Proxy.revocable(foreignTarget,{get(t,k,receiver){if(k==='prototype'){r.revoke();return 3;}return Reflect.get(t,k,receiver);}});try{Reflect.construct(foreignTypeError,[],r.proxy);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
              var error={},target=new Proxy(foreignTarget,{get(t,k,r){if(k==='prototype')throw error;return Reflect.get(t,k,r);}});try{Reflect.construct(foreignError,[],target);valid=false;}catch(e){valid=valid && e===error;}valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
}
