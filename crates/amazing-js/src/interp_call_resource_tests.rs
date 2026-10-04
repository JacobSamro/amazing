#[cfg(test)]
mod call_resource_tests {
    use super::*;

    // Build the interpreter on this thread, rather than transferring Rc-backed
    // interpreter state. 2 MiB is the proposed minimum supported embedding.
    fn minimum_stack(body: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new().name("js-call-resource-2m".into())
            .stack_size(2 * 1024 * 1024).spawn(body).unwrap().join().unwrap();
    }

    fn interpreter(ast: bool) -> Interpreter {
        let mut i = Interpreter::new();
        let usable = crate::call_stack::usable_bytes().expect("supported OS bounds");
        let remaining = crate::call_stack::remaining().expect("actual current thread marker");
        assert!(remaining <= usable);
        assert!(remaining >= i.synchronous_call_policy().reserve_bytes());
        eprintln!("call-resource requested_stack=2097152 actual_usable={usable} entry_remaining={remaining} forced_ast={ast}");
        i.force_ast_callables = ast;
        let value = i.run_script("(function resourceProbe(){ return 1; })").unwrap();
        let JsValue::Object(function) = value else { panic!("callable probe") };
        let body = function.borrow().func.as_ref().unwrap().body.clone();
        assert!(if ast { matches!(body, FunctionBody::Block(_)) }
            else { matches!(body, FunctionBody::Bytecode(_)) }, "actual callable body");
        i
    }

    fn assert_restored(i: &mut Interpreter, source: &str) {
        let realm = i.binding_current_realm();
        let depth = i.depth;
        let frames = i.frames.len();
        let callers = i.realm_callers.len();
        let entry = i.entry_realm;
        let strict = i.strict_eval_context;
        let completion = i.completion.clone();
        let source_context = i.active_source_override.clone();
        let native_stack = i.native_stack.len();
        let in_eval = i.in_eval;
        for _ in 0..3 {
            assert_eq!(i.run_script(&format!("{{{source}}}")).unwrap(), JsValue::Bool(true), "{source}");
            assert_eq!(i.depth, depth);
            assert_eq!(i.frames.len(), frames);
            assert_eq!(i.realm_callers.len(), callers);
            assert_eq!(i.entry_realm, entry);
            assert_eq!(i.strict_eval_context, strict);
            assert_eq!(i.completion, completion);
            assert_eq!(i.active_source_override, source_context);
            assert_eq!(i.native_stack.len(), native_stack);
            assert_eq!(i.in_eval, in_eval);
            assert_eq!(i.binding_current_realm(), realm);
            assert_eq!(i.run_script("(function(){return 7})()").unwrap(), JsValue::num(7.0));
        }
    }

    #[test]
    fn minimum_embedding_stack_rejects_recursive_callable_families_and_restores_context() {
        minimum_stack(|| {
            let cases = [
                "function recurse(){return recurse()} try{recurse();false}catch(e){e instanceof RangeError}",
                "var o={get x(){return this.x}};try{o.x;false}catch(e){e instanceof RangeError}",
                "var o={set x(v){this.x=v}};try{o.x=1;false}catch(e){e instanceof RangeError}",
                "var o={valueOf(){return +this}};try{+o;false}catch(e){e instanceof RangeError}",
                "function C(){new C()}try{new C();false}catch(e){e instanceof RangeError}",
                "class C{x=new C()}try{new C();false}catch(e){e instanceof RangeError}",
                "function f(){return Reflect.apply(f,null,[])}try{f();false}catch(e){e instanceof RangeError}",
                // Historical semantic-setter recursion is not present in this
                // published base: its setter writes a raw own property. Retain
                // the exact call as a nonexhausting control, not a false oracle.
                "var set=Object.getOwnPropertyDescriptor(Error.prototype,'stack').set;set.call(new Proxy(Error.prototype,{}),'x')===undefined",
                "function f(){try{return f()}finally{var marker=1}}try{f();false}catch(e){e instanceof RangeError}",
            ];
            for ast in [true, false] {
                let mut i = interpreter(ast);
                for source in cases { assert_restored(&mut i, source); }
            }
        });
    }

    #[test]
    fn transparent_proxy_bound_and_mixed_forwarders_are_admitted_before_dispatch() {
        minimum_stack(|| {
            for ast in [true, false] {
                for construct in [false, true] {
                    for kind in ["proxy", "bound", "mixed"] {
                        let mut i = interpreter(ast);
                        let setup = format!("var hits=0;var wrapped=function(){{hits++;return 1}};for(var n=0;n<10000;n++){{wrapped={} }};",
                            match kind { "proxy" => "new Proxy(wrapped,{})", "bound" => "wrapped.bind(null)", _ => "n%2?new Proxy(wrapped,{}):wrapped.bind(null)" });
                        i.run_script(&setup).unwrap();
                        let invoke = if construct { "new wrapped()" } else { "wrapped()" };
                        assert_restored(&mut i, &format!("try{{{invoke};false}}catch(e){{e instanceof RangeError&&hits===0}}"));
                    }
                }
            }
        });
    }

    #[test]
    fn callable_proxy_trap_and_native_metadata_lookup_are_admitted_at_each_property_entry() {
        minimum_stack(|| {
            for ast in [true, false] {
                let mut i = interpreter(ast);
                i.run_script("var hits=0,handler={};for(var n=0;n<10000;n++){handler=new Proxy(handler,{})};var wrapped=new Proxy(function(){hits++},handler)").unwrap();
                for invoke in ["wrapped()", "new wrapped()"] {
                    assert_restored(&mut i, &format!("try{{{invoke};false}}catch(e){{e instanceof RangeError&&hits===0}}"));
                }
                // Ordinary prototype traversal occurs inside Function.bind's
                // metadata Get, before it ever invokes the bound target.
                i.run_script("var metadata={};for(var n=0;n<10000;n++){metadata=Object.create(metadata)};var target=function(){hits++};delete target.name;Object.setPrototypeOf(target,metadata)").unwrap();
                assert_restored(&mut i, "try{Function.prototype.bind.call(target,null);false}catch(e){e instanceof RangeError&&hits===0}");
            }
        });
    }

    #[test]
    fn host_native_reentry_and_callback_throw_identity_are_preserved() {
        minimum_stack(|| {
            for ast in [true, false] {
                let mut i = interpreter(ast);
                i.set_host_native_dispatcher(Rc::new(|runtime, _, _, _, _| {
                    let host = runtime.global_env.resolve("host").unwrap();
                    runtime.call_value(&host, JsValue::Undefined, vec![])
                }));
                let host = i.binding_create_native_function(HostFunctionId(812), "host", 0, HostCallPolicy::CallOnly);
                i.global_env.declare_var("host", host).unwrap();
                assert_restored(&mut i, "try{host();false}catch(e){e instanceof RangeError}");
                let original = i.binding_create_object(None).unwrap();
                let thrown = original.clone();
                i.set_host_native_dispatcher(Rc::new(move |_, _, _, _, _| Err(Throw(thrown.clone()))));
                let host = i.global_env.resolve("host").unwrap();
                assert_eq!(i.call_value(&host, JsValue::Undefined, vec![]).unwrap_err().0, original);
                assert_eq!(i.depth, 0);
                assert!(i.frames.is_empty());
            }
        });
    }

    #[test]
    fn suspended_workers_use_actual_thread_bounds_and_restore_driver_calls() {
        minimum_stack(|| {
            for ast in [true, false] {
                let mut i = interpreter(ast);
                i.run_script(r#"
                    var generatorOK=false, asyncOK=false, asyncGeneratorOK=false;
                    function recurse(){return recurse()}
                    function* g(){yield 1;try{recurse()}catch(e){generatorOK=e instanceof RangeError}return 2}
                    async function a(){await 1;try{recurse()}catch(e){asyncOK=e instanceof RangeError}return 3}
                    async function* ag(){yield 1;try{recurse()}catch(e){asyncGeneratorOK=e instanceof RangeError}return 4}
                    var iterator=g();iterator.next();iterator.next();a();
                    var ai=ag();ai.next().then(function(){return ai.next()});
                "#).unwrap();
                i.drain_microtasks().unwrap();
                assert_eq!(i.run_script("generatorOK&&asyncOK&&asyncGeneratorOK").unwrap(), JsValue::Bool(true));
                assert_eq!(i.depth, 0);
                assert!(i.frames.is_empty());
                assert_eq!(i.run_script("(function(){return 7})()").unwrap(), JsValue::num(7.0));
            }
        });
    }

    #[test]
    fn physical_policy_is_agent_owned_and_resource_errors_ignore_author_constructors() {
        minimum_stack(|| {
            let mut i = Interpreter::new();
            let policy = SynchronousCallPolicy::new(usize::MAX).unwrap();
            assert!(SynchronousCallPolicy::new(0).is_none());
            let parent = i.binding_current_realm();
            let global = i.binding_create_object(None).unwrap();
            let child = i.binding_create_realm(global).unwrap();
            let function = i.binding_eval_in_realm(child, "(function(){return 1})").unwrap();
            i.run_script("var poisoned=0;RangeError=function(){poisoned++;throw 9}").unwrap();
            let prototype = JsValue::Object(i.intrinsic_constructors.get("RangeError").unwrap().1.clone());
            i.set_synchronous_call_policy(policy);
            let error = i.call_value(&function, JsValue::Undefined, vec![]).unwrap_err();
            let JsValue::Object(error) = error.0 else { panic!("resource error object") };
            assert_eq!(error.borrow().proto.clone().map(JsValue::Object), Some(prototype));
            assert_eq!(i.binding_current_realm(), parent);
            assert_eq!(i.synchronous_call_policy(), policy);
            i.set_synchronous_call_policy(SynchronousCallPolicy::default());
            assert_eq!(i.run_script("poisoned").unwrap(), JsValue::num(0.0));
            assert_eq!(i.call_value(&function, JsValue::Undefined, vec![]).unwrap(), JsValue::num(1.0));
        });
    }
}
