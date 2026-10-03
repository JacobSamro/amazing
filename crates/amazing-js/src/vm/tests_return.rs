use crate::{
    Interpreter,
    interp::ExecutionBackend,
    value::{FunctionBody, JsValue},
};

// Native half of the shared language/VM return boundary matrix. The coroutine
// continuation counterpart is gated independently by its owner.
fn matrix(cases: &[(&str, &str, &str)]) {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false] {
            for (setup, factory, drive) in cases {
                let mut i = Interpreter::new();
                i.run_script_with_backend(setup, backend).unwrap();
                i.run_script_with_backend(&format!("var g={factory}"), backend)
                    .unwrap();
                let JsValue::Object(function) = i.run_script("g").unwrap() else {
                    panic!("factory")
                };
                assert!(
                    matches!(
                        function.borrow().func.as_ref().unwrap().body,
                        FunctionBody::Bytecode(_)
                    ),
                    "native factory: {factory}"
                );

                let result = i
                    .run_script_with_backend(drive, backend)
                    .and_then(|_| i.drain_microtasks())
                    .and_then(|_| i.run_script("ok"));
                if !matches!(result, Ok(JsValue::Bool(true))) {
                    failures.push(format!(
                        "{backend:?} actor={actor}: {factory}; {drive} -> {result:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn explicit_return_awaits_inside_try_and_preserves_rejection_identity() {
    matrix(&[
        (
            "var marker={},ok=false,caught=false;",
            "(async function*(){try{return Promise.reject(marker)}catch(e){caught=e===marker;return caught}})",
            "g().next().then(r=>ok=r.done&&r.value===true&&caught,e=>ok=false)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){try{return {then(r,j){j(marker)}}}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){try{return import({toString(){throw marker}})}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            "var marker={},ok=false,p=Promise.resolve(1);Object.defineProperty(p,'constructor',{get(){throw marker}});",
            "(async function*(){try{return p}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
    ]);
}

#[test]
fn pending_return_waits_before_finally_and_does_not_await_done_twice() {
    matrix(&[
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){try{return p}finally{log.push('finally')}})",
            "var it=g();it.next().then(r=>{log.push('done');ok=r.value===7&&log.join(',')==='before,finally,done'});log.push('before');finish(7)",
        ),
        (
            "var ok=false,reads=0,p=Promise.resolve(7);Object.defineProperty(p,'constructor',{get(){reads++;return Promise}});",
            "(async function*(){return p})",
            "g().next().then(r=>ok=r.done&&r.value===7&&reads===1)",
        ),
        (
            "var marker={},other={},ok=false;",
            "(async function*(){try{return Promise.reject(marker)}finally{throw other}})",
            "g().next().then(r=>ok=false,e=>ok=e===other)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){try{return Promise.reject(marker)}finally{return 9}})",
            "g().next().then(r=>ok=r.done&&r.value===9,e=>ok=false)",
        ),
    ]);
}

#[test]
fn injected_return_awaits_at_yield_before_catch_finally_and_disposal() {
    matrix(&[
        (
            "var marker={},ok=false;",
            "(async function*(){try{yield 1}catch(e){return e===marker}})",
            "var it=g();it.next().then(()=>it.return(Promise.reject(marker))).then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){try{yield 1}finally{log.push('finally')}})",
            "var it=g();it.next().then(()=>{var request=it.return(p);log.push('before');finish(7);return request}).then(r=>ok=r.done&&r.value===7&&log.join(',')==='before,finally')",
        ),
        (
            "var marker={},other={},ok=false;",
            "(async function*(){try{yield 1}finally{throw other}})",
            "var it=g();it.next().then(()=>it.return(Promise.reject(marker))).then(r=>ok=false,e=>ok=e===other)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){try{using resource={[Symbol.dispose](){throw marker}};yield 1}finally{}})",
            "var it=g();it.next().then(()=>it.return(7)).then(r=>ok=false,e=>ok=e===marker)",
        ),
    ]);
}

#[test]
fn suspended_start_completed_return_and_queued_requests_are_ordered() {
    matrix(&[
        (
            "var ok=false,ran=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){ran=true;yield 1})",
            "var it=g();it.return(p).then(r=>{log.push('return');ok=r.value===7&&!ran});it.next().then(r=>{log.push('next');ok=ok&&r.done&&r.value===undefined&&log.join(',')==='return,next'});finish(7)",
        ),
        (
            "var marker={},ok=false,ran=false;",
            "(async function*(){ran=true;yield 1})",
            "g().return(Promise.reject(marker)).then(r=>ok=false,e=>ok=e===marker&&!ran)",
        ),
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){return 1})",
            "var it=g();it.next().then(()=>{it.return(p).then(r=>log.push(r.value));it.next().then(r=>ok=r.done&&log[0]===7);finish(7)})",
        ),
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){return p})",
            "var it=g();it.next().then(r=>log.push(r.value));it.next().then(r=>ok=r.done&&log[0]===7);finish(7)",
        ),
    ]);
}

#[test]
fn ordinary_async_returns_do_not_implicitly_await_inside_try() {
    matrix(&[
        (
            "var marker={},ok=false,caught=false,finalized=false;",
            "(async function(){try{return Promise.reject(marker)}catch(e){caught=true;return 9}finally{finalized=true}})",
            "g().then(v=>ok=false,e=>ok=e===marker&&!caught&&finalized)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){async function f(){try{return Promise.reject(marker)}catch(e){return 9}}try{return f()}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.value===true&&r.done,e=>ok=false)",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){function f(){return Promise.reject(marker)}try{return f()}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.value===true&&r.done,e=>ok=false)",
        ),
    ]);
}

#[test]
fn bare_and_fallthrough_returns_have_no_operand_await() {
    matrix(&[
        (
            "var ok=false,log=[];",
            "(async function*(){return})",
            "g().next().then(r=>log.push('done'));Promise.resolve().then(()=>{log.push('tick');ok=log.join(',')==='done,tick'})",
        ),
        (
            "var ok=false,log=[];",
            "(async function*(){})",
            "g().next().then(r=>log.push('done'));Promise.resolve().then(()=>{log.push('tick');ok=log.join(',')==='done,tick'})",
        ),
        (
            "var ok=false,log=[];",
            "(async function*(){return undefined})",
            "g().next().then(r=>{log.push('done');ok=log.join(',')==='tick,done'});Promise.resolve().then(()=>log.push('tick'))",
        ),
    ]);
}

#[test]
fn return_constructor_get_abrupt_is_injected_before_a_reaction() {
    matrix(&[
        (
            "var marker={},ok=false,log=[],p=Promise.resolve(7);Object.defineProperty(p,'constructor',{get(){log.push('get');throw marker}});",
            "(async function*(){try{yield 1}catch(e){log.push('catch');return e===marker}})",
            "var it=g();it.next().then(()=>{it.return(p).then(r=>ok=r.done&&r.value===true&&log.join(',')==='get,catch,after');log.push('after')})",
        ),
        (
            "var marker={},ok=false,log=[],p=Promise.resolve(7);Object.defineProperty(p,'constructor',{get(){log.push('get');throw marker}});",
            "(async function*(){try{return p}catch(e){log.push('catch');return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true&&log.join(',')==='get,catch,after');log.push('after')",
        ),
    ]);
}

#[test]
fn borrowed_generator_methods_preserve_body_and_request_realms() {
    let mut failures = Vec::new();
    let factory = "(async function*(){yield [];return []})";
    for actor in [false] {
        let mut i = Interpreter::new();
        let child = i
            .binding_create_realm(JsValue::object(crate::value::JsObject::ordinary()))
            .unwrap();
        let function = i.binding_eval_in_realm(child, factory).unwrap();

        let iterator = i
            .binding_call_in_realm(child, &function, JsValue::Undefined, vec![])
            .unwrap();
        let child_array_proto = i.binding_eval_in_realm(child, "Array.prototype").unwrap();
        let child_object_proto = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
        i.global_env.declare_var("it", iterator).unwrap();
        i.global_env
            .declare_var("childArray", child_array_proto)
            .unwrap();
        i.global_env
            .declare_var("childObject", child_object_proto)
            .unwrap();
        i.run_script("var ok=false;var next=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())).next;var request=next.call(it);var requestParent=request instanceof Promise;request.then(r=>{var first=Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray;return next.call(it).then(r=>ok=first&&requestParent&&Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray&&r.done)})").unwrap();
        i.drain_microtasks().unwrap();
        let actual = i.run_script("ok").unwrap();
        if actual != JsValue::Bool(true) {
            failures.push(format!("actor={actor}: {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn initial_and_completed_constructor_errors_reject_and_drain_without_a_reaction() {
    for completed in [false, true] {
        let mut i = Interpreter::new();
        i.run_script("var marker={},log=[],ok=false,p=Promise.resolve(7);Object.defineProperty(p,'constructor',{get(){log.push('get');throw marker}});async function* f(){}var g=f()").unwrap();
        if completed { i.run_script("g.next()").unwrap(); }
        i.run_script("var r=g.return(p);r.catch(e=>{log.push('catch');ok=e===marker});g.next().then(r=>{log.push('next');ok=ok&&r.done&&r.value===undefined});Promise.resolve().then(()=>log.push('tick'));log.push('after')").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok&&log.join(',')==='get,after,catch,next,tick'").unwrap(), JsValue::Bool(true),
            "completed={completed}: {:?}", i.run_script("JSON.stringify(log)").unwrap());
    }
}

#[test]
fn borrowed_return_uses_body_intrinsic_only_for_suspended_yield() {
    for stage in ["initial", "yield", "completed"] {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(JsValue::object(crate::value::JsObject::ordinary())).unwrap();
        i.run_script("var marker={},gets=0,p=Promise.resolve(7);Object.defineProperty(p,'then',{get(){gets++;throw marker}})").unwrap();
        let marker = i.run_script("marker").unwrap();
        i.binding_with_realm(child, |i| {
            i.global_env.declare_var("marker", marker.clone())?;
            Ok(())
        }).unwrap();
        let factory = if stage == "completed" { "(async function*(){})" }
            else { "(async function*(){try{yield 0}catch(e){return e===marker}})" };
        let function = i.binding_eval_in_realm(child, factory).unwrap();
        let iterator = i.binding_call_in_realm(child, &function, JsValue::Undefined, vec![]).unwrap();
        i.global_env.declare_var("it", iterator).unwrap();
        i.run_script("var methods=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())),ok=false,requestParent=false").unwrap();
        if stage != "initial" {
            i.run_script("methods.next.call(it)").unwrap();
            i.drain_microtasks().unwrap();
        }
        i.run_script("var request=methods.return.call(it,p);requestParent=request instanceof Promise").unwrap();
        let expected = if stage == "yield" { "true&&gets===1" } else { "7&&gets===0" };
        i.run_script(&format!("request.then(r=>ok=r.done&&r.value==={expected})")).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok&&requestParent").unwrap(), JsValue::Bool(true), "stage={stage}");
        assert_eq!(i.binding_current_realm(), parent);
        assert!(i.frames.is_empty());
    }
}

#[test]
fn injected_normalization_runs_under_saved_invocation_and_strictness() {
    use super::*;
    fn constructor(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
        let Some(JsValue::Object(expected)) = i.global_env.resolve("f") else { unreachable!() };
        // The constructor getter has its own native invocation. The nearest
        // interpreted invocation beneath it must be the saved generator,
        // not the unrelated function that called the borrowed return method.
        let observed = i.frames.iter().rev().filter_map(|frame| frame.own_function.as_ref())
            .find(|function| function.borrow().func.as_ref().is_some_and(|function|
                matches!(function.body, FunctionBody::Block(_) | FunctionBody::Expr(_) | FunctionBody::Bytecode(_))))
            .is_some_and(|active| Rc::ptr_eq(&active.0, &expected.0));
        i.global_env.assign("observed", JsValue::Bool(observed && i.strict_eval_context))?;
        Err(Throw(i.global_env.resolve("marker").unwrap()))
    }
    let mut i = Interpreter::new();
    i.run_script("var marker={},observed=false,out=false,p=Promise.resolve(7);async function* f(){'use strict';try{yield 1}catch(e){return e===marker}}var g=f()").unwrap();
    let getter = JsValue::object(JsObject::function(JsFunction {
        name: "constructorProbe".into(), params: vec![], body: FunctionBody::Native(constructor),
        is_arrow: false, is_async: false, is_generator: false, env: i.global_env.clone(),
        home: None, proto_for_new: None,
    }));
    let Some(JsValue::Object(promise)) = i.global_env.resolve("p") else { unreachable!() };
    promise.borrow_mut().define("constructor", Property {
        value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
        getter: Some(getter), setter: None, accessor: true,
    }).unwrap();
    i.run_script("g.next().then(()=>g.return(p).then(r=>out=r.done&&r.value===true))").unwrap();
    i.drain_microtasks().unwrap();
    assert_eq!(i.run_script("observed&&out").unwrap(), JsValue::Bool(true));
    assert!(i.frames.is_empty());
    assert!(!i.strict_eval_context);
}

#[test]
fn queued_borrowed_next_requests_use_the_actual_reaction_realm() {
    let mut i = Interpreter::new();
    let parent = i.binding_current_realm();
    let child = i.binding_create_realm(JsValue::object(crate::value::JsObject::ordinary())).unwrap();
    let function = i.binding_eval_in_realm(child,
        "(async function*(){await 0;yield [];yield [];return []})").unwrap();
    let iterator = i.binding_call_in_realm(child, &function, JsValue::Undefined, vec![]).unwrap();
    let child_array = i.binding_eval_in_realm(child, "Array.prototype").unwrap();
    let child_object = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
    i.global_env.declare_var("it", iterator).unwrap();
    i.global_env.declare_var("childArray", child_array).unwrap();
    i.global_env.declare_var("childObject", child_object).unwrap();
    i.run_script("var next=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())).next,checks=[],order=[],ok=false;var a=next.call(it),b=next.call(it),c=next.call(it);var parentPromises=a instanceof Promise&&b instanceof Promise&&c instanceof Promise;a.then(r=>{checks.push(!r.done&&Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray);order.push('a')});b.then(r=>{checks.push(!r.done&&Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray);order.push('b')});c.then(r=>{checks.push(r.done&&Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray);order.push('c');ok=parentPromises&&checks.every(x=>x)&&order.join(',')==='a,b,c'})").unwrap();
    i.drain_microtasks().unwrap();
    assert_eq!(i.run_script("ok").unwrap(), JsValue::Bool(true),
        "{:?}", i.run_script("JSON.stringify([checks,order])").unwrap());
    assert_eq!(i.binding_current_realm(), parent);
    assert!(i.frames.is_empty());
}

#[test]
fn borrowed_yield_results_follow_the_actual_await_reaction_realm() {
    for (body, finalizer) in [
        ("yield [];return []", false),
        ("yield;return []", false),
        ("await 0;yield [];return []", false),
        ("await 0;yield;return []", false),
        ("try{yield []}finally{yield []}", true),
    ] {
        let mut i = Interpreter::new();
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(JsValue::object(crate::value::JsObject::ordinary())).unwrap();
        let function = i.binding_eval_in_realm(child, &format!("(async function*(){{{body}}})")).unwrap();
        let iterator = i.binding_call_in_realm(child, &function, JsValue::Undefined, vec![]).unwrap();
        let child_array = i.binding_eval_in_realm(child, "Array.prototype").unwrap();
        let child_object = i.binding_eval_in_realm(child, "Object.prototype").unwrap();
        i.global_env.declare_var("it", iterator).unwrap();
        i.global_env.declare_var("childArray", child_array).unwrap();
        i.global_env.declare_var("childObject", child_object).unwrap();
        i.run_script("var methods=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())),checks=[],ok=false;function record(r){checks.push(Object.getPrototypeOf(r)===childObject&&(r.value===undefined||typeof r.value==='number'||Object.getPrototypeOf(r.value)===childArray));return r}").unwrap();
        let second = if finalizer { "methods.return.call(it,7)" } else { "methods.next.call(it)" };
        let tail = if finalizer { "return methods.next.call(it).then(r=>{record(r);ok=r.done&&r.value===7&&checks.every(x=>x)})" }
            else { "ok=r.done&&checks.every(x=>x)" };
        i.run_script(&format!("var request=methods.next.call(it);var parentRequest=request instanceof Promise;request.then(r=>{{record(r);return {second}.then(r=>{{record(r);{tail}}})}})")).unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("ok&&parentRequest").unwrap(), JsValue::Bool(true),
            "{body}: {:?}", i.run_script("JSON.stringify(checks)").unwrap());
        assert_eq!(i.binding_current_realm(), parent);
        assert!(i.frames.is_empty());
    }
}
