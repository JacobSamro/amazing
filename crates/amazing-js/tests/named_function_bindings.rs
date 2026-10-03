use amazing_js::{interp::ExecutionBackend, value::JsValue, Interpreter};

fn check_both(source: &str) {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.run_script_with_backend(source, backend)
            .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
        i.drain_microtasks().unwrap();
        assert_eq!(
            i.run_script("result").unwrap(),
            JsValue::Bool(true),
            "{backend:?}: {source}"
        );
    }
}

#[test]
fn sloppy_named_expression_writes_keep_the_recursive_identity() {
    for form in ["function", "function*", "async function", "async function*"] {
        for write in ["selfName=17", "(()=>selfName=17)()", "eval('selfName=17')"] {
            let call = if form.contains('*') {
                if form.contains("async") {
                    "f().next().then(r=>result=r.value)"
                } else {
                    "result=f().next().value"
                }
            } else if form.contains("async") {
                "f().then(r=>result=r)"
            } else {
                "result=f()"
            };
            check_both(&format!("var result=false;var f={form} selfName(){{var original=selfName;var assigned={write};return assigned===17&&selfName===original}};{call};"));
        }
    }
}

#[test]
fn strict_named_expression_writes_throw_and_keep_identity() {
    for form in ["function", "function*", "async function", "async function*"] {
        for write in ["selfName=17", "(()=>selfName=17)()", "eval('selfName=17')"] {
            let call = if form.contains('*') {
                if form.contains("async") {
                    "f().next().then(r=>result=r.value)"
                } else {
                    "result=f().next().value"
                }
            } else if form.contains("async") {
                "f().then(r=>result=r)"
            } else {
                "result=f()"
            };
            check_both(&format!("var result=false;var f={form} selfName(){{'use strict';var original=selfName;try{{{write};return false}}catch(e){{return e instanceof TypeError&&selfName===original}}}};{call};"));
        }
    }
}

#[test]
fn immutable_binding_policy_preserves_const_class_and_shadowing() {
    check_both("var result=false;var f=function selfName(selfName){selfName=3;return selfName};var a=false,b=false;try{const c=1;c=2}catch(e){a=e instanceof TypeError};var C=class Inner{static test(){try{Inner=0;return false}catch(e){return e instanceof TypeError&&Inner===C}}};b=C.test();result=a&&b&&f(1)===3&&typeof selfName==='undefined';");
}

#[test]
fn strict_nested_closures_override_a_sloppy_function_name_cell() {
    check_both("var result=false;var f=function selfName(){var original=selfName,a=false,b=false;try{(function(){'use strict';selfName=2})()}catch(e){a=e instanceof TypeError};try{eval('\"use strict\";selfName=3')}catch(e){b=e instanceof TypeError};selfName=4;return a&&b&&selfName===original};result=f();");
}

#[test]
fn existing_immutable_cells_tdz_and_live_import_aliases_keep_their_policy() {
    use amazing_js::env::{EnvHandle, EnvKind};
    let exporter = EnvHandle::new(EnvKind::Module, None);
    exporter.declare_var("live", JsValue::num(1.0)).unwrap();
    let retained = exporter.local_cell("live").unwrap();
    let importer = EnvHandle::new(EnvKind::Module, None);
    importer.declare_import("alias", retained.clone()).unwrap();
    assert!(importer
        .assign_with_strict("alias", JsValue::num(3.0), false)
        .is_err());
    exporter
        .assign_with_strict("live", JsValue::num(2.0), false)
        .unwrap();
    assert_eq!(retained.read().unwrap(), JsValue::num(2.0));
    assert_eq!(importer.resolve("alias"), Some(JsValue::num(2.0)));
    exporter
        .declare_lexical("constant", JsValue::num(4.0), false)
        .unwrap();
    assert!(exporter
        .clone()
        .assign_with_strict("constant", JsValue::num(5.0), false)
        .is_err());
    exporter.declare_uninitialized("tdz", false).unwrap();
    assert!(exporter
        .assign_with_strict("tdz", JsValue::num(5.0), false)
        .unwrap_err()
        .describe()
        .contains("uninitialized"));
}
