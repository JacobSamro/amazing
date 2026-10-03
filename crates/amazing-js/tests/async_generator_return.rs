use amazing_js::{
    Interpreter,
    ast::{ExprKind, StmtKind},
    interp::ExecutionBackend,
    value::{FunctionBody, JsValue},
};

// Exercise the real native frame and the real pooled-worker consumer independently
// of top-level compilation policy. Only the body representation is replaced.
fn matrix(cases: &[(&str, &str, &str)]) {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for actor in [false, true] {
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
                if actor {
                    let parsed = amazing_js::parse_script(factory).unwrap();
                    let StmtKind::Expression(expression) = &parsed.body[0].kind else {
                        panic!("expression")
                    };
                    let ExprKind::Paren(inner) = &expression.kind else {
                        panic!("group")
                    };
                    let ExprKind::Function(item) = &inner.kind else {
                        panic!("function")
                    };
                    function.borrow_mut().func.as_mut().unwrap().body =
                        FunctionBody::Block(item.body.clone());
                }
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
    for actor in [false, true] {
        let mut i = Interpreter::new();
        let child = i
            .binding_create_realm(JsValue::object(amazing_js::value::JsObject::ordinary()))
            .unwrap();
        let function = i.binding_eval_in_realm(child, factory).unwrap();
        if actor {
            let parsed = amazing_js::parse_script(factory).unwrap();
            let StmtKind::Expression(expression) = &parsed.body[0].kind else {
                panic!("expression")
            };
            let ExprKind::Paren(inner) = &expression.kind else {
                panic!("group")
            };
            let ExprKind::Function(item) = &inner.kind else {
                panic!("function")
            };
            let JsValue::Object(function) = &function else {
                panic!("factory")
            };
            function.borrow_mut().func.as_mut().unwrap().body =
                FunctionBody::Block(item.body.clone());
        }
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
        i.run_script("var ok=false;var next=Object.getPrototypeOf(Object.getPrototypeOf((async function*(){})())).next;var request=next.call(it);var requestParent=request instanceof Promise;request.then(r=>{var first=Object.getPrototypeOf(r)===Object.prototype&&Object.getPrototypeOf(r.value)===childArray;return next.call(it).then(r=>ok=first&&requestParent&&Object.getPrototypeOf(r)===childObject&&Object.getPrototypeOf(r.value)===childArray&&r.done)})").unwrap();
        i.drain_microtasks().unwrap();
        let actual = i.run_script("ok").unwrap();
        if actual != JsValue::Bool(true) {
            failures.push(format!("actor={actor}: {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn async_disposal_awaits_after_return_and_preserves_suppressed_error_order() {
    matrix(&[
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){await using x={[Symbol.asyncDispose](){log.push('dispose');return p}};return 7})",
            "var request=g().next();request.then(r=>ok=r.done&&r.value===7&&log.join(',')==='before,dispose');log.push('before');finish()",
        ),
        (
            "var marker={},ok=false;",
            "(async function*(){await using x={[Symbol.asyncDispose](){return Promise.reject(marker)}};return 7})",
            "g().next().then(r=>ok=false,e=>ok=e===marker)",
        ),
        (
            "var original={},a={},b={},ok=false;",
            "(async function*(){await using x={[Symbol.asyncDispose](){return Promise.reject(b)}};await using y={[Symbol.asyncDispose](){return Promise.reject(a)}};throw original})",
            "g().next().then(r=>ok=false,e=>ok=e.error===b&&e.suppressed.error===a&&e.suppressed.suppressed===original)",
        ),
        (
            "var marker={},ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){await using x={[Symbol.asyncDispose](){log.push('dispose');return p}};yield 1})",
            "var it=g();it.next().then(()=>{var request=it.return(7);request.then(r=>ok=r.done&&r.value===7&&log.join(',')==='before,dispose');log.push('before');finish()})",
        ),
    ]);
}

#[test]
fn externally_scheduled_return_and_disposal_never_pump_at_script_entry() {
    use amazing_js::interp::SchedulingRequest;
    let factory = "(async function*(){try{await using x={[Symbol.asyncDispose](){log.push('dispose');return cleanup}};yield 1}finally{log.push('finally')}})";
    let mut failures = Vec::new();
    for actor in [false, true] {
        let mut i = Interpreter::new_with_external_scheduler(128).unwrap();
        i.run_script("var finish,dispose,log=[],ok=false,p=new Promise(r=>finish=r),cleanup=new Promise(r=>dispose=r)").unwrap();
        i.run_script(&format!("var g={factory}")).unwrap();
        if actor {
            let parsed = amazing_js::parse_script(factory).unwrap();
            let StmtKind::Expression(expression) = &parsed.body[0].kind else {
                panic!("expression")
            };
            let ExprKind::Paren(inner) = &expression.kind else {
                panic!("group")
            };
            let ExprKind::Function(item) = &inner.kind else {
                panic!("function")
            };
            let JsValue::Object(function) = i.run_script("g").unwrap() else {
                panic!("factory")
            };
            function.borrow_mut().func.as_mut().unwrap().body =
                FunctionBody::Block(item.body.clone());
        }
        i.run_script("var it=g();it.next().then(r=>log.push('first'));it.return(p).then(r=>log.push('return:'+r.value));it.next().then(r=>log.push('next:'+r.done))").unwrap();
        let before = i.run_script("log.length===0").unwrap();
        // Host checkpoints, never an engine-owned nested drain. Each token is
        // invoked exactly once; newly emitted reactions join the next batch.
        let checkpoint = |i: &mut Interpreter| {
            for _ in 0..100 {
                let jobs = i.take_scheduling_requests();
                if jobs.is_empty() {
                    return;
                }
                for job in jobs {
                    let SchedulingRequest::Microtask { job_token } = job else {
                        panic!("unexpected host work")
                    };
                    i.invoke_one_job(job_token).unwrap();
                }
            }
            panic!("checkpoint did not quiesce");
        };
        checkpoint(&mut i);
        i.run_script("log.push('before');finish(7)").unwrap();
        let no_entry_pump = i.run_script("log.join(',')==='first,before'").unwrap();
        checkpoint(&mut i);
        let cleanup_pending = i
            .run_script("log.join(',')==='first,before,dispose'")
            .unwrap();
        i.run_script("dispose()").unwrap();
        checkpoint(&mut i);
        let after = i
            .run_script("log.join(',')==='first,before,dispose,finally,return:7,next:true'")
            .unwrap();
        if [before, no_entry_pump, cleanup_pending, after]
            .iter()
            .any(|value| value != &JsValue::Bool(true))
        {
            failures.push(format!(
                "actor={actor}: {:?}",
                i.run_script("log.join(',')").unwrap()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn return_await_precedes_iterator_close_and_preserves_completion_precedence() {
    let iterable = "var original={},closing={},closed=0,ok=false;var iterable={[Symbol.iterator](){return {next(){return {value:1,done:false}},return(){closed++;throw closing}}}};";
    matrix(&[
        (
            iterable,
            "(async function*(){try{for(var x of iterable)return Promise.reject(original)}catch(e){return e===original&&closed===1}})",
            "g().next().then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            iterable,
            "(async function*(){try{for(var x of iterable)return Promise.resolve(7)}catch(e){return e===closing&&closed===1}})",
            "g().next().then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            iterable,
            "(async function*(){try{for(var x of iterable)yield x}catch(e){return e===original&&closed===1}})",
            "var it=g();it.next().then(()=>it.return(Promise.reject(original))).then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
        (
            iterable,
            "(async function*(){try{for(var x of iterable)yield x}catch(e){return e===closing&&closed===1}})",
            "var it=g();it.next().then(()=>it.return(Promise.resolve(7))).then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
    ]);
}

#[test]
fn return_await_preserves_pending_completion_across_finalizer_yield_and_queue() {
    matrix(&[
        (
            "var ok=false,finish,log=[],p=new Promise(r=>finish=r);",
            "(async function*(){try{yield 1}finally{yield 2}})",
            "var it=g();it.next().then(()=>{it.return(p).then(r=>log.push(r.value+':'+r.done));it.next().then(r=>ok=r.value===7&&r.done&&log.join(',')==='2:false');finish(7)})",
        ),
        (
            "var ok=false,original={},replacement={};",
            "(async function*(){try{yield 1}finally{try{yield 2}catch(e){return e===replacement}}})",
            "var it=g();it.next().then(()=>it.return(original)).then(r=>it.throw(replacement)).then(r=>ok=r.done&&r.value===true,e=>ok=false)",
        ),
    ]);
}
