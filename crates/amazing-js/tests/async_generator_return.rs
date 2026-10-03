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
