use super::*;

fn setup(backend: ExecutionBackend) -> (Interpreter, Rc<RefCell<Vec<bool>>>) {
    let mut i = Interpreter::new_with_external_scheduler(256).unwrap();
    i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
    let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
    i.binding_eval_in_realm(child, "var value=7;").unwrap();
    let window = i.binding_create_window_proxy(child).unwrap();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let capture = seen.clone();
    i.binding_set_window_proxy_access_hook(Rc::new(move |i, _| {
        capture.borrow_mut().push(i.binding_execution_context_stack_is_empty());
        Ok(())
    }));
    i.global_env.declare_var("windowProbe", window).unwrap();
    fn probe(i: &mut Interpreter, _: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
        Ok(JsValue::Bool(i.binding_execution_context_stack_is_empty()))
    }
    let function = Interpreter::native("emptyStack", probe, &i.function_proto);
    i.global_env.declare_var("emptyStack", function).unwrap();
    assert!(i.binding_execution_context_stack_is_empty());
    (i, seen)
}
fn execute(i: &mut Interpreter, backend: ExecutionBackend, source: &str) -> JsValue {
    let result = i.run_script_with_backend(source, backend).unwrap();
    assert!(i.binding_execution_context_stack_is_empty(), "{source}");
    result
}
fn assert_observed(seen: &Rc<RefCell<Vec<bool>>>) {
    let seen = seen.borrow();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|empty| !empty), "{seen:?}");
}
fn jobs(i: &mut Interpreter) {
    let mut requests: VecDeque<_> = i.take_scheduling_requests().into();
    let mut count = 0;
    while let Some(request) = requests.pop_front() {
        count += 1; assert!(count < 128);
        if let SchedulingRequest::Microtask { job_token } = request {
            i.invoke_one_job(job_token).unwrap();
            assert!(i.binding_execution_context_stack_is_empty());
        }
        requests.extend(i.take_scheduling_requests());
    }
}

#[test]
fn script_entries_and_direct_vm_run_install_real_context_before_host_property_access() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        assert_eq!(execute(&mut i, backend, "windowProbe.value"), JsValue::num(7.0));
        assert_observed(&seen); seen.borrow_mut().clear();
        let p = crate::parse_script("windowProbe.value").unwrap();
        let code = vm::compile(&p, false).unwrap();
        assert_eq!(code.run(&mut i, vm::DEFAULT_INSTRUCTION_LIMIT).unwrap(), JsValue::num(7.0));
        assert!(i.binding_execution_context_stack_is_empty()); assert_observed(&seen);
        seen.borrow_mut().clear();
        let window = i.binding_get_property(&i.binding_global_object(), "windowProbe").unwrap();
        i.binding_get_property(&window, "value").unwrap();
        assert!(!seen.borrow().is_empty());
        assert!(seen.borrow().iter().all(|empty| *empty));
    }
}

#[test]
fn ordinary_native_bound_proxy_and_tail_entries_restore_after_completion() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        for source in ["(function(){return windowProbe.value+Number(emptyStack());})()",
            "(()=>windowProbe.value)()", "({m(){return windowProbe.value;}}).m()",
            "(function(){return windowProbe.value;}).bind(null)()",
            "new Proxy(function(){return windowProbe.value;},{})()",
            "function tail(n){'use strict';return n?tail(n-1):windowProbe.value;}tail(1000)"] {
            assert_eq!(execute(&mut i, backend, source), JsValue::num(7.0));
        }
        assert_eq!(execute(&mut i, backend, "emptyStack()"), JsValue::Bool(false));
        assert_observed(&seen);
    }
}

#[test]
fn direct_indirect_eval_and_abrupt_boundaries_restore_the_actual_context() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        for source in ["eval('windowProbe.value')", "(0,eval)('windowProbe.value')",
            "(function(){return eval('windowProbe.value');})()"] {
            assert_eq!(execute(&mut i, backend, source), JsValue::num(7.0));
        }
        for source in ["windowProbe.value;throw 1;", "eval('windowProbe.value;throw 2')",
            "class C{};C()", "new Proxy(function(){},{apply(){windowProbe.value;throw 3;}})()"] {
            assert!(i.run_script_with_backend(source, backend).is_err());
            assert!(i.binding_execution_context_stack_is_empty());
        }
        assert_observed(&seen);
    }
}

#[test]
fn reentrant_host_callback_preserves_its_outer_context_even_after_nested_throw() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        i.set_host_native_dispatcher(Rc::new(move |i, _, _, _, _| {
            assert!(!i.binding_execution_context_stack_is_empty());
            assert!(i.run_script_with_backend("windowProbe.value;throw 1;", backend).is_err());
            assert!(!i.binding_execution_context_stack_is_empty());
            i.run_script_with_backend("windowProbe.value", backend)
        }));
        let host = i.binding_create_native_function(HostFunctionId(965), "reenter", 0, HostCallPolicy::CallOnly);
        assert_eq!(i.call_value(&host, JsValue::Undefined, vec![]).unwrap(), JsValue::num(7.0));
        assert!(i.binding_execution_context_stack_is_empty()); assert_observed(&seen);
    }
}

#[test]
fn parked_generators_install_context_only_during_next_throw_return_segments() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        execute(&mut i, backend, "var generator=(function*(){try{windowProbe.value;yield 1;windowProbe.value;yield 2;}finally{windowProbe.value;}})();");
        assert!(seen.borrow().is_empty());
        execute(&mut i, backend, "generator.next();"); assert_observed(&seen);
        execute(&mut i, backend, "generator.return(9);"); assert_observed(&seen);
        execute(&mut i, backend, "var second=(function*(){try{yield windowProbe.value;}catch(e){windowProbe.value;throw e;}})();second.next();");
        assert!(i.run_script_with_backend("second.throw(3)", backend).is_err());
        assert!(i.binding_execution_context_stack_is_empty()); assert_observed(&seen);
    }
}

#[test]
fn parked_async_and_async_generator_jobs_restore_the_driver_context() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        execute(&mut i, backend, "var resolvePending;var pending=new Promise(r=>resolvePending=r);var done=0;async function work(){windowProbe.value;await pending;windowProbe.value;done++;}work();");
        assert_observed(&seen);
        execute(&mut i, backend, "resolvePending();"); jobs(&mut i);
        assert_eq!(execute(&mut i, backend, "done"), JsValue::num(1.0));
        execute(&mut i, backend, "var ag=(async function*(){windowProbe.value;yield 1;await Promise.resolve();windowProbe.value;yield 2;})();ag.next();");jobs(&mut i);
        execute(&mut i, backend, "ag.next();");jobs(&mut i);
        execute(&mut i, backend, "ag.return(3);");jobs(&mut i);
        assert_observed(&seen);
    }
}

#[test]
fn promise_and_microtask_entries_install_context_while_host_queue_is_idle_between_jobs() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        execute(&mut i, backend, "Promise.resolve().then(()=>windowProbe.value);queueMicrotask(()=>windowProbe.value);");
        assert!(seen.borrow().is_empty()); jobs(&mut i); assert_observed(&seen);
        seen.borrow_mut().clear();
        execute(&mut i, backend, "setTimeout(()=>windowProbe.value,0);requestAnimationFrame(()=>windowProbe.value);");
        assert!(seen.borrow().is_empty());
        let requests=i.take_scheduling_requests();
        let mut invoked=0;
        for request in requests {
            match request {
                SchedulingRequest::Timer {callback_token, ..} => {
                    i.invoke_one_callback(callback_token, JsValue::Undefined, None).unwrap();invoked+=1;
                }
                SchedulingRequest::AnimationFrame {callback_token, ..} => {
                    i.invoke_one_callback(callback_token, JsValue::Undefined, Some(Duration::from_millis(16))).unwrap();invoked+=1;
                }
                _=>{}
            }
            assert!(i.binding_execution_context_stack_is_empty());
        }
        assert_eq!(invoked,2);assert_observed(&seen);
    }
}

#[test]
fn foreign_realm_and_constructor_entries_restore_the_owning_agent_stack() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen) = setup(backend);
        let parent=i.binding_current_realm();
        let child=i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let window=i.binding_get_property(&i.binding_global_object(), "windowProbe").unwrap();
        let capture=seen.clone();
        i.binding_with_realm(child, |i| {
            i.binding_set_window_proxy_access_hook(Rc::new(move |i, _| {
                capture.borrow_mut().push(i.binding_execution_context_stack_is_empty()); Ok(())
            }));
            i.global_env.declare_var("windowProbe", window)?; Ok(())
        }).unwrap();
        let constructor=i.binding_with_realm(child, |i| i.run_script_with_backend("(class C{constructor(){this.x=windowProbe.value;}})", backend)).unwrap();
        assert!(i.binding_execution_context_stack_is_empty());
        i.construct_with_new_target(&constructor, vec![], constructor.clone()).unwrap();
        assert!(i.binding_execution_context_stack_is_empty());
        assert!(i.call_value(&constructor, JsValue::Undefined, vec![]).is_err());
        assert!(i.binding_execution_context_stack_is_empty());
        assert_eq!(i.binding_current_realm(), parent); assert_observed(&seen);
    }
}

#[test]
fn synchronous_and_top_level_await_module_contexts_restore_after_evaluation() {
    use crate::module::ModuleLinker;
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for source in ["export const x=windowProbe.value;", "await Promise.resolve();export const x=windowProbe.value;"] {
            let (mut i, seen)=setup(backend);
            let mut linker=ModuleLinker::new(|_: &str, _: &str| Err("no dependency".to_owned()));
            linker.link("root", source).unwrap();
            // External scheduler is only needed for the parked-function controls;
            // this genuine module entry uses the existing internal queue contract.
            i.external_scheduling=None;
            assert_eq!(linker.evaluate("root", &mut i).unwrap()["x"], JsValue::num(7.0));
            assert!(i.binding_execution_context_stack_is_empty()); assert_observed(&seen);
            let (mut i, seen)=setup(backend);
            let mut linker=ModuleLinker::new(|_: &str, _: &str| Err("no dependency".to_owned()));
            linker.link("root", source).unwrap();
            let shared=Rc::new(RefCell::new(linker));
            let capability=ModuleLinker::evaluate_async_shared(&shared, "root", &mut i).unwrap();
            assert!(i.binding_execution_context_stack_is_empty());
            jobs(&mut i);
            assert!(matches!(i.binding_module_evaluation_state(&capability).unwrap(), PromiseFate::Fulfilled(_)));
            assert_observed(&seen);
        }
    }
}

#[test]
fn callable_controls_use_genuine_ast_and_native_bytecode_bodies() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let (mut i, seen)=setup(backend);
        let function=execute(&mut i, backend, "(function body(){return windowProbe.value;})");
        let JsValue::Object(object)=&function else {panic!("callable");};
        let body=object.borrow().func.as_ref().unwrap().body.clone();
        match backend {
            ExecutionBackend::TreeWalk=>assert!(matches!(body, FunctionBody::Block(_))),
            ExecutionBackend::Bytecode=>assert!(matches!(body, FunctionBody::Bytecode(_))),
        }
        assert_eq!(i.call_value(&function, JsValue::Undefined, vec![]).unwrap(), JsValue::num(7.0));
        assert!(i.binding_execution_context_stack_is_empty());assert_observed(&seen);
    }
}
