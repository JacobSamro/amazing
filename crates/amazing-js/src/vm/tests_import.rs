use super::*;

#[test]
fn suspended_import_preserves_arguments_rejection_phases_and_source_context() {
    let mut failures = Vec::new();
    for force_ast in [false, true] {
        for kind in ["async", "generator", "async-generator", "async-method", "async-arrow"] {
            for phase in ["import", "import.source", "import.defer"] {
                for stage in ["success", "specifier", "options", "string", "attributes", "loader"] {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut i = Interpreter::new();
                        i.force_ast_callables = force_ast;
                        let calls = Rc::new(std::cell::RefCell::new(Vec::new()));
                        let recorded = calls.clone();
                        i.binding_set_phase_module_loader_hook(Rc::new(move |i, request, referrer| {
                            recorded.borrow_mut().push((request.clone(), referrer.to_string(), i.binding_current_realm()));
                            i.run_script("log.push('load')")?;
                            if stage == "loader" { return Err(Throw(i.global_env.resolve("marker").unwrap())); }
                            i.run_script("({value:17})")
                        }));
                        let pause = if kind == "generator" || kind == "async-generator" { "yield 0" } else { "await 0" };
                        let specifier = if stage == "specifier" { "thrower()" } else { "specifier" };
                        let options = if stage == "options" { "thrower()" } else { "options" };
                        let body = format!("try{{return {phase}(({pause},{specifier}),({pause},{options}))}}catch(e){{caught=true;return e}}");
                        let expression = match kind {
                            "async" => format!("async function(){{{body}}}"),
                            "generator" => format!("function*(){{{body}}}"),
                            "async-generator" => format!("async function*(){{{body}}}"),
                            "async-method" => format!("({{async f(){{{body}}}}}).f"),
                            _ => format!("async ()=>{{{body}}}"),
                        };
                        i.run_script_with_source_context(&format!(r#"
                            var log=[],marker={{}},caught=false,failed=false,done=false,out;
                            function thrower(){{log.push('eval');throw marker}}
                            var specifier={{toString(){{log.push('string');if('{stage}'==='string')throw marker;return 'dep'}}}};
                            var options={{get with(){{log.push('with');if('{stage}'==='attributes')throw marker;
                                return {{get type(){{log.push('type');return 'json'}}}}}}}};
                            var f={expression};
                        "#), "https://example.test/a/source.js").unwrap();
                        let Some(JsValue::Object(function)) = i.global_env.resolve("f") else { unreachable!() };
                        assert_eq!(matches!(function.borrow().func.as_ref().unwrap().body, FunctionBody::Bytecode(_)), !force_ast);
                        let driver = match kind {
                            "generator" => "var g=f(),r;do{r=g.next()}while(!r.done);Promise.resolve(r.value)",
                            "async-generator" => "var g=f();async function drive(){var r;do{r=await g.next()}while(!r.done);return r.value}drive()",
                            _ => "f()",
                        };
                        i.run_script_with_source_context(&format!("{driver}.then(v=>{{out=v;done=true}},e=>{{out=e;failed=true;done=true}})"), "https://example.test/b/caller.js").unwrap();
                        i.drain_microtasks().unwrap();
                        let (expected, log, count) = match stage {
                            "success" => ("out.value===17&&!failed&&!caught", "string,with,type,load", 1),
                            "specifier" | "options" => ("out===marker&&!failed&&caught", "eval", 0),
                            "string" => ("out===marker&&failed&&!caught", "string", 0),
                            "attributes" => ("out===marker&&failed&&!caught", "string,with", 0),
                            _ => ("out===marker&&failed&&!caught", "string,with,type,load", 1),
                        };
                        assert_eq!(i.run_script(&format!("done&&({expected})&&log.join(',')==='{log}'")).unwrap(), JsValue::Bool(true));
                        let calls = calls.borrow();
                        assert_eq!(calls.len(), count);
                        if let Some((request, referrer, realm)) = calls.first() {
                            assert_eq!(request.specifier.to_utf8_lossy(), "dep");
                            assert_eq!(request.attributes, vec![("type".into(), "json".into())]);
                            assert_eq!(request.phase, match phase { "import" => crate::module::ImportPhase::Evaluation, "import.source" => crate::module::ImportPhase::Source, _ => crate::module::ImportPhase::Defer });
                            assert_eq!(referrer, "https://example.test/a/source.js");
                            assert_eq!(*realm, i.binding_current_realm());
                        }
                        assert!(i.frames.is_empty());
                    }));
                    if outcome.is_err() { failures.push(format!("AST={force_ast} {kind} {phase} {stage}")); }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn resumed_import_captures_module_referrer_and_callee_realm() {
    for force_ast in [false, true] {
        let mut i = Interpreter::new();
        i.force_ast_callables = force_ast;
        let parent = i.binding_current_realm();
        let child = i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
        let calls = Rc::new(std::cell::RefCell::new(Vec::new()));
        let recorded = calls.clone();
        i.binding_set_phase_module_loader_hook(Rc::new(move |i, request, referrer| {
            recorded.borrow_mut().push((request.phase, referrer.to_string(), i.binding_current_realm()));
            Ok(JsValue::num(7.0))
        }));
        let function = i.binding_with_realm(child, |i| {
            i.global_env.declare_var("\0module.id", JsValue::str("module:A"))?;
            i.run_script_with_source_context("async function f(){return await import((await 0,'dep'),(await 0,undefined))}f", "https://example.test/a/module.js")
        }).unwrap();
        i.global_env.declare_var("foreignImport", function).unwrap();
        i.run_script_with_source_context("var out;foreignImport().then(v=>out=v)", "https://example.test/b/caller.js").unwrap();
        i.drain_microtasks().unwrap();
        assert_eq!(i.run_script("out===7").unwrap(), JsValue::Bool(true));
        assert_eq!(&*calls.borrow(), &[(crate::module::ImportPhase::Evaluation, "module:A".into(), child)]);
        assert_eq!(i.binding_current_realm(), parent);
        assert!(i.frames.is_empty());
    }
}
