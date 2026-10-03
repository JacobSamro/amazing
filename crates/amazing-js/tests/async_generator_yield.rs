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
fn explicit_await_suspends_and_rejection_stays_inside_body_try() {
    matrix(&[
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),log=[];",
            "(async function*(){log.push('start');var x=await p;log.push('resume');yield x;return 9})",
            "var it=g();it.next().then(r=>ok=r.value===7&&!r.done&&log.join(',')==='start,before,resume');log.push('before');finish(7)",
        ),
        (
            "var ok=false,marker={};",
            "(async function*(){try{await Promise.reject(marker)}catch(e){yield e===marker}})",
            "g().next().then(r=>ok=r.value===true&&!r.done,e=>ok=false)",
        ),
        (
            "var ok=false,marker={},p=Promise.resolve(1);Object.defineProperty(p,'constructor',{get(){throw marker}});",
            "(async function*(){try{await p}catch(e){yield e===marker}})",
            "g().next().then(r=>ok=r.value===true&&!r.done,e=>ok=false)",
        ),
    ]);
}
#[test]
fn ordinary_yield_awaits_operand_before_yield_and_catches_rejection() {
    matrix(&[
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),log=[];",
            "(async function*(){yield p;log.push('body after')})",
            "var it=g();it.next().then(r=>{log.push('yield');ok=r.value===7&&!r.done&&log.join(',')==='before,yield'});log.push('before');finish(7)",
        ),
        (
            "var ok=false,marker={};",
            "(async function*(){try{yield Promise.reject(marker)}catch(e){yield e===marker}})",
            "g().next().then(r=>ok=r.value===true&&!r.done,e=>ok=false)",
        ),
        (
            "var ok=false,reads=0,value={get then(){reads++;return r=>r(7)}};",
            "(async function*(){yield value})",
            "g().next().then(r=>ok=r.value===7&&!r.done&&reads===1)",
        ),
        (
            "var ok=false,marker={},value={get then(){throw marker}};",
            "(async function*(){try{yield value}catch(e){yield e===marker}})",
            "g().next().then(r=>ok=r.value===true&&!r.done,e=>ok=false)",
        ),
    ]);
}
#[test]
fn await_and_yield_requests_remain_fifo_while_pending() {
    matrix(&[
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),log=[];",
            "(async function*(){try{yield p;yield 2}finally{log.push('finally')}})",
            "var it=g();var a=it.next(),b=it.next(),c=it.return(3);log.push('before');finish(1);Promise.all([a,b,c]).then(rs=>ok=rs[0].value===1&&!rs[0].done&&rs[1].value===2&&!rs[1].done&&rs[2].value===3&&rs[2].done&&log.join(',')==='before,finally')",
        ),
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),marker={};",
            "(async function*(){await p;try{yield 1}catch(e){yield e===marker};return 9})",
            "var it=g();var a=it.next(),b=it.throw(marker),c=it.next();finish();Promise.all([a,b,c]).then(rs=>ok=rs[0].value===1&&!rs[0].done&&rs[1].value===true&&!rs[1].done&&rs[2].value===9&&rs[2].done)",
        ),
    ]);
}
#[test]
fn delegation_prefers_async_method_and_caches_next() {
    matrix(&[
        (
            "var ok=false,log=[],n=0,inner={next(){log.push('next');return {done:n++>0,value:7}}},iterable={[Symbol.asyncIterator](){log.push('async');return inner},[Symbol.iterator](){throw 1}};",
            "(async function*(){yield* iterable;return 9})",
            "var it=g();it.next().then(a=>it.next().then(b=>ok=a.value===7&&!a.done&&b.value===9&&b.done&&log.join(',')==='async,next,next'))",
        ),
        (
            "var ok=false,reads=0,n=0,inner={get next(){reads++;return function(){if(n++===0){Object.defineProperty(inner,'next',{value(){throw 1}});return {done:false,value:7}}return {done:true,value:8}}},iterable={[Symbol.asyncIterator](){return inner}};",
            "(async function*(){var value=yield* iterable;return value})",
            "var it=g();it.next().then(a=>it.next().then(b=>ok=a.value===7&&!a.done&&b.value===8&&b.done&&reads===1))",
        ),
    ]);
}
#[test]
fn genuine_async_delegation_yields_raw_values_and_completes_raw() {
    matrix(&[
        (
            "var ok=false,reads=0,value={get then(){reads++;throw 1}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value}}}}};",
            "(async function*(){yield* iterable})",
            "g().next().then(r=>ok=r.value===value&&!r.done&&reads===0,e=>ok=false)",
        ),
        (
            "var ok=false,reads=0,value={get then(){reads++;throw 1}},seen,iterable={[Symbol.asyncIterator](){return {next(){return {done:true,value}}}}};",
            "(async function*(){seen=yield* iterable;return 0})",
            "g().next().then(r=>ok=r.value===0&&r.done&&seen===value&&reads===0,e=>ok=false)",
        ),
        (
            "var ok=false,reads=0,value={get then(){reads++;throw 1}},seen,iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},throw(){return {done:true,value}}}}};",
            "(async function*(){seen=yield* iterable;return 0})",
            "var it=g();it.next().then(()=>it.throw(7)).then(r=>ok=r.done&&r.value===0&&seen===value&&reads===0,e=>ok=false)",
        ),
    ]);
}
#[test]
fn missing_delegate_return_performs_second_await_after_get_method() {
    matrix(&[
        (
            "var ok=false,log=[],value={get then(){log.push('then')}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},get return(){log.push('return')}}}};",
            "(async function*(){yield* iterable})",
            "var it=g();it.next().then(()=>it.return(value)).then(r=>ok=r.done&&r.value===value&&log.join(',')==='then,return,then')",
        ),
        (
            "var ok=false,log=[],value={get then(){log.push('then')}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},get return(){log.push('return');return null}}}};",
            "(async function*(){yield* iterable})",
            "var it=g();it.next().then(()=>it.return(value)).then(r=>ok=r.done&&r.value===value&&log.join(',')==='then,return,then')",
        ),
        (
            "var ok=false,marker={},reads=0,value={get then(){if(++reads===2)throw marker}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e===marker}})",
            "var it=g();it.next().then(()=>it.return(value)).then(r=>ok=r.done&&r.value===true&&reads===2,e=>ok=false)",
        ),
    ]);
}
#[test]
fn done_delegate_return_preserves_raw_value_without_another_await() {
    matrix(&[
        (
            "var ok=false,reads=0,value={get then(){reads++;throw 1}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){return {done:true,value}}}}};",
            "(async function*(){yield* iterable})",
            "var it=g();it.next().then(()=>it.return(7)).then(r=>ok=r.done&&r.value===value&&reads===0,e=>ok=false)",
        ),
        (
            "var ok=false,log=[],reads=0,value={get then(){reads++;throw 1}},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){return {done:true,value}}}}};",
            "(async function*(){try{yield* iterable}finally{log.push('finally')}})",
            "var it=g();it.next().then(()=>it.return(7)).then(r=>ok=r.done&&r.value===value&&reads===0&&log.join(',')==='finally',e=>ok=false)",
        ),
    ]);
}
#[test]
fn async_from_sync_delegation_awaits_values_and_live_iteration() {
    matrix(&[
        (
            "var ok=false,finish,p=new Promise(r=>finish=r);",
            "(async function*(){yield* [p,2];return 9})",
            "var it=g(),a=it.next();finish(7);a.then(x=>it.next().then(y=>it.next().then(z=>ok=x.value===7&&!x.done&&y.value===2&&!y.done&&z.value===9&&z.done)))",
        ),
        (
            "var ok=false,a=[1];",
            "(async function*(){yield* a;return 9})",
            "var it=g();it.next().then(x=>{a.push(2);return it.next().then(y=>it.next().then(z=>ok=x.value===1&&y.value===2&&!y.done&&z.value===9&&z.done))})",
        ),
        (
            "var ok=false;",
            "(async function*(){yield* '\\uD83D\\uDE00x';return 9})",
            "var it=g();it.next().then(x=>it.next().then(y=>it.next().then(z=>ok=x.value==='\\uD83D\\uDE00'&&y.value==='x'&&z.value===9&&z.done)))",
        ),
        (
            "var ok=false,marker={};",
            "(async function*(){try{yield* [Promise.reject(marker)]}catch(e){yield e===marker}})",
            "g().next().then(r=>ok=r.value===true&&!r.done,e=>ok=false)",
        ),
    ]);
}
#[test]
fn delegated_method_and_result_abrupts_keep_identity() {
    matrix(&[
        (
            "var ok=false,marker={},iterable={[Symbol.asyncIterator](){return {get next(){throw marker}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,iterable={[Symbol.asyncIterator](){return {next:1}}};",
            "(async function*(){try{yield* iterable}catch(e){return e instanceof TypeError}})",
            "g().next().then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,iterable={[Symbol.asyncIterator](){return {next(){return 1}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e instanceof TypeError}})",
            "g().next().then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,marker={},iterable={[Symbol.asyncIterator](){return {next(){return {get done(){throw marker}}}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,marker={},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,get value(){throw marker}}}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return:1}}};",
            "(async function*(){try{yield* iterable}catch(e){return e instanceof TypeError}})",
            "var it=g();it.next().then(()=>it.return(7)).then(r=>ok=r.done&&r.value===true)",
        ),
    ]);
}
#[test]
fn missing_throw_closes_async_iterator_before_protocol_type_error() {
    matrix(&[
        (
            "var ok=false,log=[],iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){log.push('close');return Promise.resolve({get done(){throw 1},get value(){throw 2}})}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e instanceof TypeError&&log.join(',')==='close'}})",
            "var it=g();it.next().then(()=>it.throw(7)).then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,marker={},iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){return Promise.reject(marker)}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e===marker}})",
            "var it=g();it.next().then(()=>it.throw(7)).then(r=>ok=r.done&&r.value===true)",
        ),
        (
            "var ok=false,iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){return 1}}}};",
            "(async function*(){try{yield* iterable}catch(e){return e instanceof TypeError}})",
            "var it=g();it.next().then(()=>it.throw(7)).then(r=>ok=r.done&&r.value===true)",
        ),
    ]);
}
#[test]
fn pending_delegate_return_preserves_queue_and_finalizer_order() {
    matrix(&[
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),log=[],iterable={[Symbol.asyncIterator](){return {next(){return {done:false,value:1}},return(){log.push('return');return p}}}};",
            "(async function*(){try{yield* iterable}finally{log.push('finally')}})",
            "var it=g();it.next().then(()=>{var a=it.return(7),b=it.next();log.push('before');finish({done:true,value:9});return Promise.all([a,b])}).then(rs=>ok=rs[0].value===9&&rs[0].done&&rs[1].done&&log.join(',')==='before,return,finally')",
        ),
        (
            "var ok=false,n=0,iterable={[Symbol.asyncIterator](){return {next(){return {done:++n>1,value:9}},return(){return {done:false,value:2}}}}};",
            "(async function*(){yield* iterable;return 7})",
            "var it=g();it.next().then(()=>it.return(3)).then(x=>it.next().then(y=>ok=x.value===2&&!x.done&&y.value===7&&y.done))",
        ),
    ]);
}
#[test]
fn async_disposal_after_await_and_yield_preserves_suppressed_error() {
    matrix(&[
        (
            "var ok=false,marker={},other={},log=[];",
            "(async function*(){try{await using r={[Symbol.asyncDispose](){log.push('dispose');return Promise.reject(other)}};yield Promise.reject(marker)}catch(e){return e instanceof SuppressedError&&e.error===other&&e.suppressed===marker}})",
            "g().next().then(r=>ok=r.done&&r.value===true&&log.join(',')==='dispose')",
        ),
        (
            "var ok=false,finish,p=new Promise(r=>finish=r),log=[];",
            "(async function*(){await using r={[Symbol.asyncDispose](){log.push('dispose');return p}};yield 1;return 7})",
            "var it=g();it.next().then(()=>{var a=it.next();log.push('before');finish();return a}).then(r=>ok=r.done&&r.value===7&&log.join(',')==='before,dispose')",
        ),
    ]);
}
