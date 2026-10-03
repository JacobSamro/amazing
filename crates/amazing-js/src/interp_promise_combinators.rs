// Portions ported from boa-dev/boa (core/engine/src/builtins/promise/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Classic combinator entries, streaming iteration and element closures adapted
// to Amazing's private native-home records. Verified against current ECMA-262
// 27.5.4 on 2026-10-03: exhausted Promise.any calls Reject directly (the pinned
// upstream instead throws its aggregate error through the outer entry).

#[derive(Clone, Copy, PartialEq)]
enum ClassicCombinator { All, AllSettled, Any, Race }

fn classic_iterator(i: &mut Interpreter, source: &JsValue) -> Result<StdIteratorRecord, Throw> {
    let method = i.get_prop(source, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("Promise combinator input is not iterable"));
    }
    let iterator = i.call_value(&method, source.clone(), Vec::new())?;
    if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
    let next = i.get_prop(&iterator, "next")?;
    Ok(StdIteratorRecord { iterator, next })
}

fn classic_private_object(value: JsValue) -> Obj {
    match value { JsValue::Object(object) => object, _ => unreachable!("private combinator object") }
}

fn classic_cap_callback(capability: &Obj, name: &str) -> JsValue {
    capability.borrow().get_own(name).unwrap()
}

fn classic_finish(i: &mut Interpreter, state: &Obj) -> Result<JsValue, Throw> {
    let count = state.borrow().get_own("count").unwrap().to_number()? as usize;
    let list = classic_private_object(state_values(state, "values")?);
    let values = (0..count).map(|index| list.borrow().get_own(&index.to_string()).unwrap()).collect();
    let values: Vec<JsValue> = values;
    let values_array = array_values(i, Vec::new())?;
    for (index, value) in values.into_iter().enumerate() {
        array_create_data_property(i, &values_array, index, value)?;
    }
    let values = values_array;
    let capability = classic_private_object(state_values(state, "target")?);
    let any = state.borrow().get_own("any").unwrap().to_boolean();
    let (name, value) = if any { ("reject", runtime_aggregate_rejection(i, values)) }
        else { ("resolve", values) };
    i.call_value(&classic_cap_callback(&capability, name), JsValue::Undefined, vec![value])
}

fn classic_element(i: &mut Interpreter, args: Vec<JsValue>, rejected: bool) -> Result<JsValue, Throw> {
    let data = native_data(i)?;
    if data.borrow().get_own("called").unwrap().to_boolean() { return Ok(JsValue::Undefined); }
    data.borrow_mut().set_own("called", JsValue::Bool(true));
    let (state, _, index) = combinator_state(i)?;
    let mut value = args.first().cloned().unwrap_or(JsValue::Undefined);
    if state.borrow().get_own("settled").unwrap().to_boolean() {
        value = if rejected { settled_record(i, "rejected", "reason", value) }
            else { settled_record(i, "fulfilled", "value", value) };
    }
    let list = classic_private_object(state_values(&state, "values")?);
    list.borrow_mut().set_own(&index.to_string(), value);
    if decrement_remaining(&state)? == 0 { return classic_finish(i, &state); }
    Ok(JsValue::Undefined)
}

fn native_classic_fulfill(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_element(i, args, false)
}
fn native_classic_reject(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_element(i, args, true)
}

fn perform_classic_combinator(
    i: &mut Interpreter, ctor: &JsValue, capability: &Obj, resolve: &JsValue,
    iterator: &StdIteratorRecord, done: &mut bool, kind: ClassicCombinator,
) -> Result<(), Throw> {
    // The private list is never exposed. A fresh array is created only when
    // completing, in the entry/element builtin's realm, just like CreateArrayFromList.
    let list = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
    let state = make_combinator_state(capability, 1, JsValue::Object(list.clone()));
    state.borrow_mut().set_own("count", JsValue::num(0.0));
    state.borrow_mut().set_own("any", JsValue::Bool(kind == ClassicCombinator::Any));
    state.borrow_mut().set_own("settled", JsValue::Bool(kind == ClassicCombinator::AllSettled));
    let mut index = 0usize;
    loop {
        // IteratorStepValue marks Done on *all* abrupt completions, including
        // next, done and value access. Such failures must not call return.
        let value = match std_iterator_step(i, iterator) {
            Ok(Some(value)) => value,
            Ok(None) => { *done = true; break; }
            Err(error) => { *done = true; return Err(error); }
        };
        if kind != ClassicCombinator::Race {
            list.borrow_mut().set_own(&index.to_string(), JsValue::Undefined);
            state.borrow_mut().set_own("count", JsValue::num((index + 1) as f64));
        }
        let next_promise = i.call_value(resolve, ctor.clone(), vec![value])?;
        let original_resolve = classic_cap_callback(capability, "resolve");
        let original_reject = classic_cap_callback(capability, "reject");
        let (fulfilled, rejected) = if kind == ClassicCombinator::Race {
            (original_resolve, original_reject)
        } else {
            let element = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
            element.borrow_mut().set_own("state", JsValue::Object(state.clone()));
            element.borrow_mut().set_own("index", JsValue::num(index as f64));
            element.borrow_mut().set_own("called", JsValue::Bool(false));
            let pair = match kind {
                ClassicCombinator::All => (promise_callback_with_data(i, native_classic_fulfill, &element, 1), original_reject),
                ClassicCombinator::Any => (original_resolve, promise_callback_with_data(i, native_classic_reject, &element, 1)),
                ClassicCombinator::AllSettled => (promise_callback_with_data(i, native_classic_fulfill, &element, 1),
                    promise_callback_with_data(i, native_classic_reject, &element, 1)),
                ClassicCombinator::Race => unreachable!(),
            };
            let remaining = state_remaining(&state)? + 1;
            state.borrow_mut().set_own("remaining", JsValue::num(remaining as f64));
            pair
        };
        index += 1;
        let then = i.get_prop(&next_promise, "then")?;
        i.call_value(&then, next_promise, vec![fulfilled, rejected])?;
    }
    if kind != ClassicCombinator::Race && decrement_remaining(&state)? == 0 {
        classic_finish(i, &state)?;
    }
    Ok(())
}

fn classic_combinator_entry(i: &mut Interpreter, ctor: JsValue, args: Vec<JsValue>, kind: ClassicCombinator) -> Result<JsValue, Throw> {
    let capability = new_promise_capability(i, &ctor)?;
    let result = (|| {
        let resolve = i.get_prop(&ctor, "resolve")?;
        if callable_function(&resolve).is_none() { return Err(Throw::type_error("resolve is not callable")); }
        let source = args.first().cloned().unwrap_or(JsValue::Undefined);
        let iterator = classic_iterator(i, &source)?;
        let mut done = false;
        match perform_classic_combinator(i, &ctor, &capability, &resolve, &iterator, &mut done, kind) {
            Err(error) if !done => Err(std_iterator_abrupt_close(i, &iterator, error)),
            result => result,
        }
    })();
    if let Err(error) = result {
        let error = i.normalize_throw(error).0;
        i.call_value(&classic_cap_callback(&capability, "reject"), JsValue::Undefined, vec![error])?;
    }
    Ok(capability_promise(&capability))
}
fn native_promise_all(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_combinator_entry(i, this, args, ClassicCombinator::All)
}
fn native_promise_all_settled(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_combinator_entry(i, this, args, ClassicCombinator::AllSettled)
}
fn native_promise_any(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_combinator_entry(i, this, args, ClassicCombinator::Any)
}
fn native_promise_race(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    classic_combinator_entry(i, this, args, ClassicCombinator::Race)
}

#[cfg(test)]
mod classic_combinator_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(source, backend);
            let error = result.as_ref().err().map(|error| (i.get_prop(&error.0, "name"), i.get_prop(&error.0, "message")));
            assert_eq!(result.unwrap_or_else(|e| panic!("{backend:?}: {e:?} {error:?}; output {:?}", i.output.borrow())), JsValue::Bool(true), "{backend:?}; output {:?}", i.output.borrow());
        }
    }

    #[test]
    fn streaming_iteration_caches_next_and_forwards_original_capability_functions() {
        check(r#"
          var valid=true;
          for(var kind of ['all','allSettled','any','race']){
            var log=[], resolveCap=function(){},rejectCap=function(){},captured=[],marker={};
            var C=function(executor){executor(resolveCap,rejectCap);return marker;};
            Object.defineProperty(C,'resolve',{get(){log.push('resolve-get');return function(v){log.push('resolve-'+v);return {then(a,b){captured.push([a,b]);log.push('then-'+v);}};};}});
            var index=0,it={get next(){log.push('next-get');return function(){log.push('next-'+index);return index<2?{done:false,value:index++}:{done:true};};}};
            var source={[Symbol.iterator](){log.push('iterator');return it;}};
            valid=valid && Promise[kind].call(C,source)===marker;
            valid=valid && log.join(',')==='resolve-get,iterator,next-get,next-0,resolve-0,then-0,next-1,resolve-1,then-1,next-2';
            if(kind==='all')valid=valid && captured[0][1]===rejectCap;
            if(kind==='any')valid=valid && captured[0][0]===resolveCap;
            if(kind==='race')valid=valid && captured[0][0]===resolveCap && captured[0][1]===rejectCap;
            else {var cb=kind==='any'?captured[0][1]:captured[0][0];valid=valid && cb.length===1 && cb.name==='' && !Object.hasOwn(cb,'prototype');try{new cb;valid=false;}catch(e){valid=valid && e instanceof TypeError;}}
          }
          valid;
        "#);
    }

    #[test]
    fn per_element_guards_and_sentinel_preserve_escaped_callbacks_and_returns() {
        check(r#"
          var valid=true;
          for(var kind of ['all','allSettled','any']){
            var pairs=[],count=0,result,returned={};
            function C(executor){executor(v=>{count++;result=v;return returned;},e=>{count++;result=e;return returned;});}
            C.resolve=v=>({then(a,b){pairs.push([a,b]);}});
            Promise[kind].call(C,[1,2]);
            var branch=kind==='any'?1:0, a=pairs[0][branch],b=pairs[1][branch];
            valid=valid && a(7)===undefined && a(8)===undefined && count===0;
            if(kind==='allSettled')valid=valid && pairs[0][1](9)===undefined && count===0;
            valid=valid && b(11)===returned && b(12)===undefined && count===1;
            var values=kind==='any'?result.errors:result;
            valid=valid && values.length===2 && (kind==='allSettled'?values[0].value===7 && values[1].value===11:values[0]===7 && values[1]===11);
            count=0;result=undefined;C.resolve=v=>({then(a,b){(kind==='any'?b:a)(v);valid=valid && count===0;}});
            Promise[kind].call(C,[3,4]);valid=valid && count===1;
          }
          var calls=[];function Race(e){e(v=>calls.push('r'+v),v=>calls.push('j'+v));}Race.resolve=v=>({then(r,j){r(v);j(v);}});
          Promise.race.call(Race,[1,2]);valid=valid && calls.join(',')==='r1,j1,r2,j2';valid;
        "#);
    }

    #[test]
    fn abrupt_iterator_step_does_not_close_but_resolution_and_then_do() {
        check(r#"
          var valid=true,error={},closeError={};
          for(var kind of ['all','allSettled','any','race'])for(var stage of ['next','result','done','value','resolve','then-get','then-call']){
            var closed=0,rejection;
            var C=function(e){e(()=>{},e=>{rejection=e;});};
            C.resolve=function(v){if(stage==='resolve')throw error;return {get then(){if(stage==='then-get')throw error;return function(){if(stage==='then-call')throw error;};}};};
            var it={next(){if(stage==='next')throw error;if(stage==='result')return 1;return {get done(){if(stage==='done')throw error;return false;},get value(){if(stage==='value')throw error;return 1;}};},get return(){closed++;throw closeError;}};
            Promise[kind].call(C,{[Symbol.iterator](){return it;}});
            var expected=['resolve','then-get','then-call'].includes(stage)?1:0; if(!(stage==='result'?rejection instanceof TypeError:rejection===error)||closed!==expected)console.log('failed',kind,stage,rejection===error,closed,expected); valid=valid && (stage==='result'?rejection instanceof TypeError:rejection===error) && closed===expected;
          }
          for(var kind of ['all','allSettled','any','race']){
            var rejected,access=0;var C=function(e){e(()=>{},e=>rejected=e);};C.resolve=v=>v;
            Promise[kind].call(C,{next(){access++;return {done:true};}});valid=valid && rejected instanceof TypeError && access===0;
            var error={},count=0;var D=function(e){e(()=>{},e=>{count++;throw error;});};D.resolve=undefined;
            try{Promise[kind].call(D,[]);valid=false;}catch(e){valid=valid && e===error && count===1;}
          }
          valid;
        "#);
    }

    #[test]
    fn empty_and_native_results_have_correct_arrays_records_and_aggregate_errors() {
        check(r#"
          var valid=true,results=[];
          Promise.all([Promise.resolve(1),2]).then(v=>{valid=valid && v.join(',')==='1,2';results.push('all');});
          Promise.allSettled([Promise.reject(3),4]).then(v=>{valid=valid && v[0].reason===3 && v[1].value===4 && Object.keys(v[0]).join(',')==='status,reason';results.push('settled');});
          Promise.any([Promise.reject(5),6]).then(v=>{valid=valid && v===6;results.push('any');});
          Promise.race([7,8]).then(v=>{valid=valid && v===7;results.push('race');});
          function C(e){e(v=>{valid=valid && Array.isArray(v) && v.length===0;},e=>{var d=Object.getOwnPropertyDescriptor(e,'errors');valid=valid && e instanceof AggregateError && d.writable && !d.enumerable && d.configurable && d.value.length===0;});}
          C.resolve=v=>v;Promise.all.call(C,[]);Promise.allSettled.call(C,[]);Promise.any.call(C,[]);
          var count=0;function R(e){e(()=>count++,()=>count++);}R.resolve=v=>v;Promise.race.call(R,[]);valid=valid && count===0;
          valid;
        "#);
    }
    #[test]
    fn native_jobs_deliver_ordered_results_and_original_rejections() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            i.run_script_with_backend(r#"
              Promise.all([Promise.resolve(1),2]).then(v=>console.log('all',v.join(',')));
              Promise.allSettled([Promise.reject(3),4]).then(v=>console.log('settled',v[0].reason,v[1].value,Object.keys(v[0]).join(',')));
              Promise.any([Promise.reject(5),6]).then(v=>console.log('any',v));
              Promise.race([7,8]).then(v=>console.log('race',v));
              var error={};Promise.all([Promise.reject(error)]).catch(e=>console.log('identity',e===error));
              Promise.any([Promise.reject(9),Promise.reject(10)]).catch(e=>console.log('aggregate',e instanceof AggregateError,e.errors.join(',')));
            "#,backend).unwrap();
            let mut output=i.output.borrow().clone();output.sort();
            assert_eq!(output,["aggregate true 9,10","all 1,2","any 6","identity true","race 7","settled 3 4 status,reason"],"{backend:?}");
        }
    }

    #[test]
    fn escaped_element_builtins_use_original_callee_realm() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
            for(name,source) in [("foreignAll","Promise.all"),("foreignSettled","Promise.allSettled"),("foreignAny","Promise.any"),
                ("foreignArray","Array.prototype"),("foreignObject","Object.prototype"),("foreignAggregate","AggregateError.prototype"),("foreignFunction","Function.prototype")] {
                let value=i.binding_eval_in_realm(child,source).unwrap();i.global_env.declare_var(name,value).unwrap();
            }
            assert_eq!(i.run_script_with_backend(r#"
              var valid=true,pair,result;
              function C(e){e(v=>result=v,e=>result=e);}C.resolve=v=>({then(a,b){pair=[a,b];}});
              foreignAll.call(C,[1]);valid=valid && Object.getPrototypeOf(pair[0])===foreignFunction;pair[0](2);
              valid=valid && Object.getPrototypeOf(result)===foreignArray && result[0]===2;
              foreignSettled.call(C,[1]);pair[1](3);valid=valid && Object.getPrototypeOf(result)===foreignArray && Object.getPrototypeOf(result[0])===foreignObject;
              foreignAny.call(C,[1]);pair[1](4);valid=valid && Object.getPrototypeOf(result)===foreignAggregate && Object.getPrototypeOf(result.errors)===foreignArray && result.errors[0]===4;
              valid;
            "#,backend).unwrap(),JsValue::Bool(true),"{backend:?}");
        }
    }

}
