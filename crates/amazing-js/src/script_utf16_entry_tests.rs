use super::*;
fn source(prefix: &str, payload: &[u16], suffix: &str) -> Vec<u16> {
    prefix.encode_utf16().chain(payload.iter().copied()).chain(suffix.encode_utf16()).collect()
}
#[test]
fn genuine_script_entry_preserves_literal_and_function_source_in_all_modes() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for ast in [false, true] {
            for unit in [0xd800, 0xdc00, 0xfffd] {
                let mut i = Interpreter::new();
                i.test_force_ast_callables(ast);
                let script = source("function f(){return '", &[unit], "'}; f().charCodeAt(0)");
                assert_eq!(i.run_script_utf16_with_backend(&script, backend).unwrap(), JsValue::num(unit as f64));
                assert_eq!(i.run_script(r#"f.toString().charCodeAt(f.toString().indexOf("return '")+8)"#).unwrap(), JsValue::num(unit as f64));
            }
        }
    }
}
#[test]
fn utf16_context_is_retained_by_delayed_closures_and_restored_after_failure() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let capture = seen.clone();
        i.binding_set_phase_module_loader_hook(Rc::new(move |_, request, referrer| {
            capture.borrow_mut().push((request.specifier.clone(), referrer.to_owned()));
            Ok(JsValue::Undefined)
        }));
        let a = source("var later=()=>import('", &[0xd800], "'); var shared=1;");
        i.run_script_utf16_with_source_context_and_backend(&a, "https://example.test/a.js", backend).unwrap();
        i.run_script_utf16_with_source_context_and_backend(&"shared++;".encode_utf16().collect::<Vec<_>>(), "https://example.test/b.js", backend).unwrap();
        i.run_script("later();import('plain');").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(seen.borrow()[0].0.units(), &[0xd800]);
        assert_eq!(seen.borrow()[0].1, "https://example.test/a.js");
        assert_eq!(seen.borrow()[1].1, "");
        assert_eq!(i.run_script("shared").unwrap(), JsValue::num(2.0));
        assert!(i.run_script_utf16_with_source_context_and_backend(&[0xd800], "parse-error.js", backend).is_err());
        assert!(i.global_env.source_context().is_none());
        assert!(i.run_script_utf16_with_source_context(&"throw 7".encode_utf16().collect::<Vec<_>>(), "runtime-error.js").is_err());
        assert!(i.global_env.source_context().is_none());
    }
}
#[test]
fn plain_and_context_utf16_entries_share_the_live_global_environment() {
    let mut i = Interpreter::new();
    let value = source("var exact='", &[0xd800, 0xdc00, 0], "'; exact.length");
    assert_eq!(i.run_script_utf16(&value).unwrap(), JsValue::num(3.0));
    assert_eq!(i.run_script_utf16_with_source_context(&"exact.charCodeAt(2)".encode_utf16().collect::<Vec<_>>(), "https://example.test/source.js").unwrap(), JsValue::num(0.0));
}
#[test]
fn utf16_parse_errors_normalize_to_current_realm_and_restore_context() {
    let mut i = Interpreter::new();
    let global = i.binding_create_object(None).unwrap();
    let realm = i.binding_create_realm(global).unwrap();
    i.binding_with_realm(realm, |i| {
        let error = i.run_script_utf16_with_source_context(&[0xd800], "https://example.test/bad.js").unwrap_err();
        assert!(matches!(error.0, JsValue::Object(_)));
        assert!(i.global_env.source_context().is_none());
        Ok(JsValue::Undefined)
    }).unwrap();
}
#[test]
fn typed_phase_loader_preserves_units_and_never_falls_back_to_legacy_hook() {
    let mut i = Interpreter::new();
    i.binding_set_typed_module_loader_hook(Rc::new(|_, _, _| panic!("typed import fell back")));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let capture = seen.clone();
    i.binding_set_phase_module_loader_hook(Rc::new(move |_, request, _| {
        capture.borrow_mut().push(request.specifier.clone()); Ok(JsValue::Undefined)
    }));
    let script = source("import('", &[0xd800], "');import('\u{fffd}');");
    i.run_script_utf16(&script).unwrap();
    i.drain_microtasks().unwrap();
    assert_eq!(seen.borrow()[0].units(), &[0xd800]);
    assert_eq!(seen.borrow()[1].units(), &[0xfffd]);
}
