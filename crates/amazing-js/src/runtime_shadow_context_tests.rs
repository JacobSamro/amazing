use super::*;

#[test]
fn shadow_script_suspends_actual_outer_function_and_restores_all_contexts() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
        let observations = Rc::new(RefCell::new(Vec::new()));
        let observed = observations.clone();
        i.binding_set_typed_module_loader_hook(Rc::new(move |i, _, referrer| {
            observed.borrow_mut().push((i.binding_current_realm(), referrer.to_owned()));
            Ok(JsValue::Undefined)
        }));
        i.run_script_with_source_context_and_backend("var realm = new ShadowRealm();",
            "https://example.test/creation.js", backend).unwrap();
        i.run_script_with_source_context_and_backend(r#"
            function invoke(x) {
                let before = x;
                try { realm.evaluate("var factory=Function(\"return import('./created.js')\"); factory()") }
                catch(e) { if(!(e instanceof TypeError)) throw e }
                try { realm.evaluate('throw 1') } catch(e) { if(!(e instanceof TypeError)) throw e }
                return before===x;
            }
            if(!invoke(7)) throw 1;
        "#, "https://example.test/later.js", backend).unwrap();
        // The Promise returned by the final expression cannot cross a Shadow
        // boundary. Loading still occurs as its actual already-enqueued job.
        i.drain_microtasks().unwrap();
        let rows = observations.borrow();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "https://example.test/creation.js");
        assert!(i.frames.is_empty());
        assert!(i.eval_grammar_override.is_none());
        assert_eq!(i.run_script("realm.evaluate('factory=undefined; 3')").unwrap(), JsValue::num(3.0));
    }
}

#[test]
fn fresh_shadow_eval_does_not_inherit_constructor_or_private_grammar() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
        assert_eq!(i.run_script_with_backend(r#"(()=>{
            let realm=new ShadowRealm();
            class Base {};
            class Child extends Base {
                #x=1;
                constructor(){super();let failures=0;
                    for(let text of ['new.target','super.x','super()','this.#x'])
                        try{realm.evaluate(text)}catch(e){if(e instanceof SyntaxError)failures++}
                    this.ok=failures===4&&eval('new.target')===Child;
                }
            }
            let f=realm.evaluate('()=>eval("typeof arguments")');
            return new Child().ok&&f()==='undefined'&&realm.evaluate('this===globalThis');
        })()"#,backend).unwrap(), JsValue::Bool(true));
    }
}

#[test]
fn child_global_prototype_is_its_own_intrinsic_not_the_creator_prototype() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
        assert_eq!(i.run_script_with_backend(r#"(()=>{
            Object.prototype.creatorMarker=17;
            let realm=new ShadowRealm();
            let isolated=realm.evaluate('Object.getPrototypeOf(globalThis)===Object.prototype && typeof globalThis.creatorMarker===\"undefined\"');
            realm.evaluate('Object.prototype.childMarker=23; undefined');
            let retained=realm.evaluate('globalThis.childMarker')===23;
            delete Object.prototype.creatorMarker;
            return isolated&&retained&&typeof ({}).childMarker==='undefined';
        })()"#,backend).unwrap(), JsValue::Bool(true));
    }
}
