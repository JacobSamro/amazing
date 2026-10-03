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
fn intrinsic_arrays_use_live_actual_iterator_records() {
    matrix(&[
        (
            "var ok=false,a=[1];",
            "(function(){var out=[];for(var x of a){out.push(x);if(x===1)a.push(2)}return out.join(',')==='1,2'})",
            "ok=g()",
        ),
        (
            "var ok=false,a=[1,2,3];",
            "(function(){var out=[];for(var x of a){out.push(x);a.length=1}return out.join(',')==='1'})",
            "ok=g()",
        ),
        (
            "var ok=false,a=[1,2];",
            "(function(){var out=[];for(var x of a){out.push(x===undefined?'hole':x);delete a[1]}return out.join(',')==='1,hole'})",
            "ok=g()",
        ),
        (
            "var ok=false,a=[1,2],reads=0;Object.defineProperty(a,Symbol.iterator,{get(){reads++;return function(){var n=0;return {next(){return {done:n++>0,value:9}}}}}});",
            "(function(){var out=[];for(var x of a)out.push(x);return reads===1&&out.join(',')==='9'})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn intrinsic_iterator_return_hooks_run_on_abrupt_body_only() {
    matrix(&[
        (
            "var ok=false,closes=0,p=Object.getPrototypeOf([][Symbol.iterator]());p.return=function(){closes++;return {}};",
            "(function(){for(var x of [1,2])break;return closes===1})",
            "ok=g()",
        ),
        (
            "var ok=false,closes=0,p=Object.getPrototypeOf([][Symbol.iterator]());p.return=function(){closes++;return {}};",
            "(function(){for(var x of [1,2])return 7})",
            "ok=g()===7&&closes===1",
        ),
        (
            "var ok=false,closes=0,p=Object.getPrototypeOf([][Symbol.iterator]());p.return=function(){closes++;return {}};",
            "(function(){for(var x of [1,2])continue;return closes===0})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn primitive_strings_use_actual_unicode_iterators_and_overrides() {
    matrix(&[
        (
            "var ok=false,reads=0;String.prototype[Symbol.iterator]=function(){reads++;var n=0;return {next(){return {done:n++>0,value:'override'}}}};",
            "(function(){var out=[];for(var x of 'abc')out.push(x);return reads===1&&out.join(',')==='override'})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){var out=[];for(var x of '\\uD83D\\uDE00x\\uD800')out.push(x);return out.length===3&&out[0]==='\\uD83D\\uDE00'&&out[1]==='x'&&out[2]==='\\uD800'})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn iterator_method_and_cached_next_are_each_observed_once() {
    matrix(&[
        (
            "var ok=false,methodReads=0,nextReads=0,n=0,inner={get next(){nextReads++;return function(){if(n++===0){Object.defineProperty(inner,'next',{value(){throw 1}});return {done:false,value:7}}return {done:true}}}},iterable={get [Symbol.iterator](){methodReads++;return function(){return inner}}};",
            "(function(){var out=[];for(var x of iterable)out.push(x);return methodReads===1&&nextReads===1&&out.join(',')==='7'})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={[Symbol.iterator](){log.push('iterator');return {get next(){log.push('next');return 1},get return(){log.push('return');throw 7}}}};",
            "(function(){try{for(var x of iterable){log.push('body')}}catch(e){return e instanceof TypeError&&log.join(',')==='iterator,next'}return false})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn malformed_iterator_records_never_enter_body_or_close() {
    matrix(&[
        (
            "var ok=false,log=[],iterable={[Symbol.iterator](){return 1}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e instanceof TypeError&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={next(){log.push('bare next');return {done:true}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e instanceof TypeError&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={[Symbol.iterator]:null,next(){log.push('bare next');return {done:true}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e instanceof TypeError&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={[Symbol.iterator]:1,next(){log.push('bare next');return {done:true}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e instanceof TypeError&&log.length===0}return false})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn next_done_and_value_abrupts_do_not_observe_return() {
    matrix(&[
        (
            "var ok=false,marker={},log=[],iterable={[Symbol.iterator](){return {next(){throw marker},get return(){log.push('return');throw 1}}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e===marker&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={[Symbol.iterator](){return {next(){return 1},get return(){log.push('return');throw 1}}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e instanceof TypeError&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},log=[],iterable={[Symbol.iterator](){return {next(){return {get done(){throw marker}}},get return(){log.push('return');throw 1}}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e===marker&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},log=[],iterable={[Symbol.iterator](){return {next(){return {done:false,get value(){throw marker}}},get return(){log.push('return');throw 1}}}};",
            "(function(){try{for(var x of iterable)log.push('body')}catch(e){return e===marker&&log.length===0}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],iterable={[Symbol.iterator](){return {next(){return {done:true,get value(){throw 1}}},get return(){log.push('return');throw 1}}}};",
            "(function(){for(var x of iterable)log.push('body');return log.length===0})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn original_throw_wins_over_all_close_failures() {
    matrix(&[
        (
            "var ok=false,marker={},other={},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},get return(){throw other}}}};",
            "(function(){try{for(var x of iterable)throw marker}catch(e){return e===marker}})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},other={},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){throw other}}}};",
            "(function(){try{for(var x of iterable)throw marker}catch(e){return e===marker}})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){return 1}}}};",
            "(function(){try{for(var x of iterable)throw marker}catch(e){return e===marker}})",
            "ok=g()",
        ),
        (
            "var ok=false,iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){throw 1}}}};",
            "(function(){try{for(var x of iterable)throw undefined}catch(e){return e===undefined}})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn return_and_break_are_replaced_by_close_failures() {
    matrix(&[
        (
            "var ok=false,marker={},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){throw marker}}}};",
            "(function(){try{for(var x of iterable)return 7}catch(e){return e===marker}})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},get return(){throw marker}}}};",
            "(function(){try{for(var x of iterable)break}catch(e){return e===marker}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return:1}}};",
            "(function(){try{for(var x of iterable)break}catch(e){return e instanceof TypeError}return false})",
            "ok=g()",
        ),
        (
            "var ok=false,iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){return 1}}}};",
            "(function(){try{for(var x of iterable)break}catch(e){return e instanceof TypeError}return false})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn close_get_method_absence_and_receiver_and_arguments_are_exact() {
    matrix(&[
        (
            "var ok=false,calls=0,inner={next(){return {done:false,value:1}},return(){calls++;return {}}},iterable={[Symbol.iterator](){return inner}};Object.defineProperty(inner,'return',{value:function(){if(this!==inner||arguments.length!==0)throw 1;calls++;return {}}});",
            "(function(){for(var x of iterable)break;return calls===1})",
            "ok=g()",
        ),
        (
            "var ok=false,iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return:null}}};",
            "(function(){for(var x of iterable)break;return true})",
            "ok=g()",
        ),
        (
            "var ok=false,iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return:undefined}}};",
            "(function(){for(var x of iterable)return 7})",
            "ok=g()===7",
        ),
    ]);
}
#[test]
fn labeled_control_and_nested_close_order_preserve_loop_targets() {
    matrix(&[
        (
            "var ok=false,log=[],iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){log.push('close');return {}}}};",
            "(function(){outer:for(var n=0;n<1;n++){for(var x of iterable)continue outer}return log.join(',')==='close'})",
            "ok=g()",
        ),
        (
            "var ok=false,log=[],outer={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){log.push('outer');return {}}}},inner={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){log.push('inner');return {}}}};",
            "(function(){done:for(var x of outer){for(var y of inner)break done}return log.join(',')==='inner,outer'})",
            "ok=g()",
        ),
        (
            "var ok=false,closes=0,n=0,iterable={[Symbol.iterator](){return {next(){return {done:n++>1,value:n}},return(){closes++;return {}}}};",
            "(function(){for(var x of iterable)continue;return closes===0})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn binding_and_put_abrupts_close_after_successful_value_step() {
    matrix(&[
        (
            "var ok=false,marker={},log=[],o={set x(v){throw marker}},iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){log.push('close');throw 1}}}};",
            "(function(){try{for(o.x of iterable){}}catch(e){return e===marker&&log.join(',')==='close'}})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},log=[],iterable={[Symbol.iterator](){return {next(){return {done:false,value:[]}},return(){log.push('close');return {}}}};",
            "(function(){try{for(var [x=(()=>{throw marker})()] of iterable){}}catch(e){return e===marker&&log.join(',')==='close'}})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},log=[],iterable={[Symbol.iterator](){return {next(){return {done:false,value:1}},return(){log.push('close');return {}}}};",
            "(function(){try{for(var {x=(()=>{throw marker})()} of iterable){}}catch(e){return e===marker&&log.join(',')==='close'}})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn iteration_cells_and_repeated_block_functions_stay_coherent() {
    matrix(&[
        (
            "var ok=false;",
            "(function(){var f=[];for(let x of [1,2])f.push(()=>x);return f[0]()===1&&f[1]()===2})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){var f=[];for(const x of [1,2])f.push(()=>x);return f[0]()===1&&f[1]()===2})",
            "ok=g()",
        ),
        (
            "var ok=false;",
            "(function(){for(var x of [1]){var C=function(){return 'first'}}for(var x of [1]){function C(){return 'second'}if(C()!=='second')return false}return true})",
            "ok=g()",
        ),
        (
            "var ok=false,x=[1];",
            "(function(){try{for(let x of x){}}catch(e){return e instanceof ReferenceError}return false})",
            "ok=g()",
        ),
    ]);
}
#[test]
fn per_iteration_disposal_precedes_close_and_keeps_throw_precedence() {
    matrix(&[
        (
            "var ok=false,log=[],resource={[Symbol.dispose](){log.push('dispose')}},iterable={[Symbol.iterator](){return {next(){return {done:false,value:resource}},return(){log.push('close');return {}}}};",
            "(function(){for(using x of iterable){log.push('body');break}return log.join(',')==='body,dispose,close'})",
            "ok=g()",
        ),
        (
            "var ok=false,marker={},log=[],resource={[Symbol.dispose](){log.push('dispose');throw marker}},iterable={[Symbol.iterator](){return {next(){return {done:false,value:resource}},return(){log.push('close');throw 1}}}};",
            "(function(){try{for(using x of iterable)break}catch(e){return e===marker&&log.join(',')==='dispose,close'}return false})",
            "ok=g()",
        ),
    ]);
}
