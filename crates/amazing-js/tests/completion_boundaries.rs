use amazing_js::{interp::ExecutionBackend, value::JsValue, Interpreter};

fn matrix(cases: &[(&str, JsValue)]) {
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for (source, expected) in cases {
            let mut i = Interpreter::new();
            let actual = i.run_script_with_backend(source, backend);
            match actual {
                Ok(actual) if &actual == expected => {}
                other => failures.push(format!(
                    "{backend:?}: {source} -> {other:?}, expected {expected:?}"
                )),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
fn num(v: f64) -> JsValue {
    JsValue::num(v)
}

#[test]
fn empty_block_and_declaration_values_keep_statement_list_value() {
    matrix(&[
        ("4;{}", num(4.0)),
        ("4;{;}", num(4.0)),
        ("4;{var x;}", num(4.0)),
        ("4;{let x;}", num(4.0)),
        ("4;{class C{}}", num(4.0)),
        ("4;{function f(){}}", num(4.0)),
        ("4;{5;{};}", num(5.0)),
        ("4;{undefined;}", JsValue::Undefined),
        ("4; L:{}", num(4.0)),
        ("4;{if(false){}}", JsValue::Undefined),
    ]);
}

#[test]
fn normal_finally_discards_its_value_but_preserves_try_or_catch_value() {
    matrix(&[
        ("try{}finally{5}", JsValue::Undefined),
        ("try{7}finally{5}", num(7.0)),
        ("try{}catch(e){}finally{5}", JsValue::Undefined),
        ("try{throw 1}catch(e){}finally{5}", JsValue::Undefined),
        ("try{7;throw 1}catch(e){}finally{5}", JsValue::Undefined),
        ("try{throw 1}catch(e){8}finally{5}", num(8.0)),
        ("try{try{8}finally{9}}finally{10}", num(8.0)),
    ]);
}

#[test]
fn abrupt_finally_overrides_try_and_applies_empty_to_undefined() {
    matrix(&[("function f(){try{return 7}finally{return 8}};f()",num(8.0)),("var marker={};var same=false;try{try{throw 1}finally{throw marker}}catch(e){same=e===marker};same",JsValue::Bool(true)),("L:{try{5;throw 1}finally{break L}}",JsValue::Undefined),("L:{try{5}finally{6;break L}}",num(6.0)),("var n=0;while(n++<2){try{5}finally{continue}}",JsValue::Undefined)]);
}

#[test]
fn disposal_preserves_normal_value_and_throws_over_normal_or_return() {
    matrix(&[("{4;using x=null;}",num(4.0)),("{4;using x={[Symbol.dispose](){9;}};}",num(4.0)),("4;{using x={[Symbol.dispose](){9;}};}",num(4.0)),("function f(){using x={[Symbol.dispose](){9;}};return 4};f()",num(4.0)),("var marker={},same=false;function f(){using x={[Symbol.dispose](){throw marker}};return 4};try{f()}catch(e){same=e===marker};same",JsValue::Bool(true)),("var a={},b={},ok=false;try{using x={[Symbol.dispose](){throw b}};throw a}catch(e){ok=e.error===b&&e.suppressed===a};ok",JsValue::Bool(true))]);
}

#[test]
fn loops_and_labels_update_empty_abrupt_values() {
    matrix(&[
        ("L:{5;break L}", num(5.0)),
        ("L:{5;{break L}}", num(5.0)),
        ("while(true){5;break}", num(5.0)),
        ("for(;;){5;{break}}", num(5.0)),
        ("for(var x of [1]){5;break}", num(5.0)),
        ("var n=1;while(n--){var x=2;}", JsValue::Undefined),
        ("4;while(false){}", JsValue::Undefined),
        (
            "var n=0;while(n++<2){if(n===1){5}else{}}",
            JsValue::Undefined,
        ),
        (
            "var n=0;while(n++<2){if(n===1){5}else{;}}",
            JsValue::Undefined,
        ),
        ("eval('L:{5;with({}){break L}}')", JsValue::Undefined),
        ("eval('L:{with({}){6;{break L}}}')", num(6.0)),
        ("switch(0){case 0:5;case 1:;}", num(5.0)),
        (
            "eval('outer:while(true){5;try{with({}){break outer}}finally{9}}')",
            JsValue::Undefined,
        ),
        ("4;switch(1){case 1:break}", JsValue::Undefined),
    ]);
}

#[test]
fn call_initializers_and_eval_keep_caller_completion_value() {
    matrix(&[
        ("function f(){9;}4;var x=f();", num(4.0)),
        (
            "function f(){9;}4;var x=Reflect.apply(f,null,[]);",
            num(4.0),
        ),
        ("4;var x=eval('9');", num(4.0)),
        ("function C(){9;}4;var x=new C();", num(4.0)),
        ("4;class C{static{9;}}",num(4.0)),
        ("4;var x=class{static{9;}};",num(4.0)),
        ("var n=0;while((class{static{9;}},n++<2)){if(n===1)5;else{;}}",JsValue::Undefined),
        ("var n=0;while((class{static{9;}},n++<2)){var x=0;}",JsValue::Undefined),
        ("var n=0;while(n++<2){5;var x=class{static{9;}};}",num(5.0)),
        ("var marker={},same=false;try{class C{static{throw marker}}}catch(e){same=e===marker}same",JsValue::Bool(true)),
        ("function C(){9;}4;var x=Reflect.construct(C,[]);", num(4.0)),
        ("eval('4;{}')", num(4.0)),
        ("eval('try{7}finally{9}')", num(7.0)),
        (
            "var n=0;function f(){9;return n++<1};while(f()){5}",
            num(5.0),
        ),
    ]);
}

#[test]
fn generator_resumptions_keep_body_and_driver_completion_separate() {
    matrix(&[
        ("function* g(){L:{5;yield 1;try{break L}finally{yield 2;9}}return 7}var it=g();it.next();it.next();it.next().value",num(7.0)),
        ("var g=eval('(function*(){L:{5;yield 1;try{with({}){break L}}finally{yield 2;9}}return 7})');var it=g();it.next();it.next();it.next().value",num(7.0)),
        ("function* g(){var n=0;L:while(n++<2){yield n;try{continue L}finally{yield 8}}return 7}var it=g();it.next();it.next();it.next();it.next();it.next().value",num(7.0)),
        (
            "function* g(){try{yield 1;return 7}finally{9}}var it=g();it.next();it.next().value",
            num(7.0),
        ),
        (
            "function* g(){yield 1;9;}var it=g();it.next();4;var result=it.next();",
            num(4.0),
        ),
        (
            "function* g(){try{yield 1}finally{9}}var it=g();it.next();4;var result=it.return(7);",
            num(4.0),
        ),
    ]);
}

#[test]
fn async_finally_and_disposal_preserve_pending_return_and_throw_identity() {
    let cases = [
        ("var out=0;async function f(){try{return 7}finally{await 0;9}}f().then(v=>out=v)", num(7.0)),
        ("var out=0;async function f(){try{return 7}finally{await 0;return 8}}f().then(v=>out=v)", num(8.0)),
        ("var out=0;async function f(){await using x={[Symbol.asyncDispose](){return Promise.resolve(9)}};return 7}f().then(v=>out=v)", num(7.0)),
        ("var marker={},out=false;async function f(){await using x={[Symbol.asyncDispose](){return Promise.reject(marker)}};return 7}f().catch(e=>out=e===marker)", JsValue::Bool(true)),
        ("var out=0;async function f(){L:while(true){try{await 0;break L}finally{await 0;9}}return 7}f().then(v=>out=v)", num(7.0)),
    ];
    let mut failures = Vec::new();
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for (source, expected) in &cases {
            let mut i = Interpreter::new();
            let setup = i.run_script_with_backend(source, backend);
            let actual = setup.and_then(|_| i.run_script_with_backend("out", backend));
            if actual.as_ref().ok() != Some(expected) {
                failures.push(format!(
                    "{backend:?}: {source}: {actual:?}, expected {expected:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn injected_generator_returns_cannot_mask_finalizer_or_disposer_throws() {
    let bodies = [
        "try{yield 1}finally{throw marker}",
        "using x={[Symbol.dispose](){throw marker}};yield 1",
        "try{yield 1}finally{yield 2;throw marker}",
    ];
    let mut cases = Vec::new();
    // An unsupported With in the callable forces the existing AST coroutine
    // adapter on this fixed base. Without it the ordinary factory uses VM.
    for actor_prefix in ["", "with({}){};"] {
        for body in bodies {
            let drive = if body.contains("yield 2") {
                "it.return(original);it.next()"
            } else {
                "it.return(original)"
            };
            cases.push(format!("var original={{}},marker={{}},same=false;var g=eval('(function*(){{{actor_prefix}{body}}})');var it=g();it.next();try{{{drive}}}catch(e){{same=e===marker}}same"));
        }
        cases.push(format!("var original={{}},marker={{}},caught=false;var g=eval('(function*(){{{actor_prefix}try{{yield 1}}finally{{try{{throw marker}}catch(e){{caught=e===marker}}}}}})');var it=g();it.next();var r=it.return(original);r.done&&r.value===original&&caught"));
        cases.push(format!("var original={{}},same=false;var g=eval('(function*(){{{actor_prefix}try{{yield 1}}finally{{throw undefined}}}})');var it=g();it.next();try{{it.return(original)}}catch(e){{same=e===undefined}}same"));
        cases.push(format!("var original={{}};var g=eval('(function*(){{{actor_prefix}try{{yield 1}}finally{{9}}}})');var it=g();it.next();var r=it.return(original);r.done&&r.value===original"));
    }
    for actor_prefix in ["", "with({}){};"] {
        for original_throw in [false, true] {
            let signal = if original_throw {
                "it.throw(original)"
            } else {
                "it.return(original)"
            };
            let expected = if original_throw { "original" } else { "marker" };
            cases.push(format!("var original={{}},marker={{}},same=false;var iterable={{[Symbol.iterator](){{return {{next(){{return {{value:1,done:false}}}},return(){{throw marker}}}}}}}};var g=eval('(function*(){{{actor_prefix}for(var x of iterable) yield x;}})');var it=g();it.next();try{{{signal}}}catch(e){{same=e==={expected}}}same"));
        }
        for return_value in ["null", "undefined"] {
            cases.push(format!("var original={{}};var iterable={{[Symbol.iterator](){{return {{next(){{return {{value:1,done:false}}}},return:{return_value}}}}}}};var g=eval('(function*(){{{actor_prefix}for(var x of iterable) yield x;}})');var it=g();it.next();var r=it.return(original);r.done&&r.value===original"));
        }
        for return_property in ["return:1", "return(){return 1}"] {
            cases.push(format!("var original={{}},same=false;var iterable={{[Symbol.iterator](){{return {{next(){{return {{value:1,done:false}}}},{return_property}}}}}}};var g=eval('(function*(){{{actor_prefix}for(var x of iterable) yield x;}})');var it=g();it.next();try{{it.return(original)}}catch(e){{same=e instanceof TypeError}}same"));
        }
        cases.push(format!("var original={{}},marker={{}},same=false;var iterable={{[Symbol.iterator](){{return {{next(){{return {{value:1,done:false}}}},get return(){{throw marker}}}}}}}};var g=eval('(function*(){{{actor_prefix}for(var x of iterable) yield x;}})');var it=g();it.next();try{{it.return(original)}}catch(e){{same=e===marker}}same"));
        cases.push(format!("var original={{}},same=false;var iterable={{[Symbol.iterator](){{return {{next(){{return {{value:1,done:false}}}},return(){{throw undefined}}}}}}}};var g=eval('(function*(){{{actor_prefix}for(var x of iterable) yield x;}})');var it=g();it.next();try{{it.return(original)}}catch(e){{same=e===undefined}}same"));
    }
    let borrowed: Vec<_> = cases
        .iter()
        .map(|source| (source.as_str(), JsValue::Bool(true)))
        .collect();
    matrix(&borrowed);
}
