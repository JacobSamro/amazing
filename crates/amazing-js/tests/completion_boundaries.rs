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
