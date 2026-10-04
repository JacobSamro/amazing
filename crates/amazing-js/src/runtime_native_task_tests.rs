#[cfg(test)]
mod native_task_fifo_tests {
    use super::*;
    use std::cell::Cell;

    fn install_host(runtime: &mut Interpreter, owner: u64) {
        let next = Rc::new(Cell::new(1_u64));
        runtime.set_host_native_dispatcher(Rc::new(move |runtime, id, _, args, _| {
            let slot = next.get();
            next.set(slot.checked_add(1).unwrap());
            runtime.enqueue_native_task(NativeTaskToken::new(owner, slot))?;
            if id == HostFunctionId(41002) {
                runtime.enqueue_native_microtask(7, NativeMicrotaskKind::MutationObserver)?;
            }
            if let Some(value) = args.first() { return Err(Throw(value.clone())); }
            Ok(JsValue::Undefined)
        }));
        for (name, id) in [("nativeTask", 41001), ("nativeTaskAndMicrotask", 41002)] {
            let value = runtime.binding_create_native_function(
                HostFunctionId(id), name, 0, HostCallPolicy::CallOnly,
            );
            runtime.global_env.declare_var(name, value).unwrap();
        }
    }

    fn labels(requests: &[SchedulingRequest]) -> Vec<&'static str> {
        requests.iter().map(|request| match request {
            SchedulingRequest::NativeTask { .. } => "native",
            SchedulingRequest::Timer { .. } => "timer",
            SchedulingRequest::Microtask { .. } => "job",
            SchedulingRequest::NativeMicrotask { .. } => "mutation",
            _ => "other",
        }).collect()
    }

    #[test]
    fn actual_host_calls_keep_both_task_timer_orders_and_js_checkpoint_order() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for native_first in [false, true] {
                for gateway in ["nativeTask()", "nativeTask.call(null)",
                    "Reflect.apply(nativeTask,null,[])", "nativeTask.bind(null)()",
                    "new Proxy(nativeTask,{})()"] {
                    let mut runtime = Interpreter::new_with_external_scheduler(8).unwrap();
                    install_host(&mut runtime, 901);
                    let timer = "setTimeout(()=>console.log('timer'),0)";
                    let pair = if native_first { format!("{gateway};{timer}") }
                        else { format!("{timer};{gateway}") };
                    let source = format!("Promise.resolve().then(()=>console.log('first'));{pair};queueMicrotask(()=>console.log('last'))");
                    runtime.run_script_with_backend(&source, backend).unwrap();
                    assert!(runtime.output.borrow().is_empty());
                    let requests = runtime.take_scheduling_requests();
                    let expected = if native_first { vec!["job","native","timer","job"] }
                        else { vec!["job","timer","native","job"] };
                    assert_eq!(labels(&requests), expected, "{backend:?}/{gateway}");
                    for request in requests {
                        match request {
                            SchedulingRequest::Microtask { job_token } => runtime.invoke_one_job(job_token).unwrap(),
                            SchedulingRequest::Timer { callback_token, .. } => {
                                runtime.invoke_one_callback(callback_token, JsValue::Undefined, None).unwrap();
                                assert!(runtime.invoke_one_callback(callback_token, JsValue::Undefined, None).is_err());
                            }
                            SchedulingRequest::NativeTask { token } => {
                                assert_eq!(token, NativeTaskToken::new(901,1));
                            }
                            _ => unreachable!(),
                        }
                    }
                    assert_eq!(&*runtime.output.borrow(), &["first","timer","last"]);
                }
            }
        }
    }

    #[test]
    fn full_js_quota_still_admits_native_tasks_and_single_mutation_microtask() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut runtime = Interpreter::new_with_external_scheduler(1).unwrap();
            install_host(&mut runtime, 902);
            assert_eq!(runtime.run_script_with_backend(r#"
                var quotaError=false, mutationError=false;
                setTimeout(()=>console.log('timer'),0);
                nativeTask();nativeTaskAndMicrotask();
                try { queueMicrotask(()=>console.log('unexpected')); } catch(e) { quotaError=e instanceof TypeError; }
                try { nativeTaskAndMicrotask(); } catch(e) { mutationError=e instanceof TypeError; }
                nativeTask();quotaError && mutationError
            "#, backend).unwrap(), JsValue::Bool(true));
            let requests = runtime.take_scheduling_requests();
            assert_eq!(labels(&requests), ["timer","native","native","mutation","native","native"]);
            let tokens: Vec<_> = requests.iter().filter_map(|r| match r {
                SchedulingRequest::NativeTask { token } => Some(*token), _ => None,
            }).collect();
            assert_eq!(tokens, (1..=4).map(|slot| NativeTaskToken::new(902,slot)).collect::<Vec<_>>());
            let SchedulingRequest::Timer { callback_token, .. } = requests[0] else { unreachable!() };
            runtime.invoke_one_callback(callback_token, JsValue::Undefined, None).unwrap();
            assert_eq!(&*runtime.output.borrow(), &["timer"]);
            // A drain admits no host work by itself and resets only outbox quota.
            runtime.run_script_with_backend("queueMicrotask(()=>console.log('next'))",backend).unwrap();
            let SchedulingRequest::Microtask { job_token } = runtime.take_scheduling_requests()[0] else { unreachable!() };
            runtime.invoke_one_job(job_token).unwrap();
            assert_eq!(&*runtime.output.borrow(), &["timer","next"]);
        }
    }

    #[test]
    fn native_admission_preserves_author_abrupt_identity_and_host_owner_across_realms() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut runtime = Interpreter::new_with_external_scheduler(8).unwrap();
            install_host(&mut runtime, 903);
            let caller = runtime.binding_current_realm();
            let global = runtime.binding_create_object(None).unwrap();
            let child = runtime.binding_create_realm(global).unwrap();
            // Dispatchers are realm-owned: the genuine foreign native must use
            // this same host arena/counter from its actual creation realm.
            let dispatch = runtime.host_native_dispatch.clone().unwrap();
            runtime.binding_with_realm(child, |runtime| {
                runtime.set_host_native_dispatcher(dispatch); Ok(())
            }).unwrap();
            let foreign = runtime.binding_with_realm(child, |runtime| Ok(runtime.binding_create_native_function(
                HostFunctionId(41001), "foreignNativeTask", 0, HostCallPolicy::CallOnly,
            ))).unwrap();
            runtime.global_env.declare_var("foreignNativeTask", foreign).unwrap();
            assert_eq!(runtime.run_script_with_backend(r#"
                var sentinel={get name(){throw 'must not inspect';}};
                var same=false;
                try { foreignNativeTask(sentinel); } catch(e) { same=e===sentinel; }
                setTimeout(()=>{},0);nativeTask();same
            "#,backend).unwrap(),JsValue::Bool(true));
            assert_eq!(runtime.binding_current_realm(),caller);
            let requests=runtime.take_scheduling_requests();
            assert_eq!(labels(&requests),["native","timer","native"]);
            assert_eq!(requests[0],SchedulingRequest::NativeTask{token:NativeTaskToken::new(903,1)});
            assert_eq!(requests[2],SchedulingRequest::NativeTask{token:NativeTaskToken::new(903,2)});
        }
    }

    #[test]
    fn disabled_scheduler_native_error_uses_real_foreign_callee_intrinsic() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut runtime=Interpreter::new();
            install_host(&mut runtime,906);
            let caller=runtime.binding_current_realm();
            let global=runtime.binding_create_object(None).unwrap();
            let child=runtime.binding_create_realm(global).unwrap();
            let dispatch=runtime.host_native_dispatch.clone().unwrap();
            runtime.binding_with_realm(child,|runtime| {
                runtime.set_host_native_dispatcher(dispatch); Ok(())
            }).unwrap();
            let expected=runtime.binding_eval_in_realm(child,"TypeError.prototype").unwrap();
            let foreign=runtime.binding_with_realm(child,|runtime|Ok(runtime.binding_create_native_function(
                HostFunctionId(41001),"disabledForeignNativeTask",0,HostCallPolicy::CallOnly,
            ))).unwrap();
            runtime.binding_eval_in_realm(child,"TypeError=function(){throw 'poisoned constructor'}").unwrap();
            runtime.global_env.declare_var("disabledForeignNativeTask",foreign).unwrap();
            runtime.global_env.declare_var("expectedNativeErrorPrototype",expected).unwrap();
            assert_eq!(runtime.run_script_with_backend(r#"
                var caught;
                try { disabledForeignNativeTask(); } catch(e) { caught=e; }
                Object.getPrototypeOf(caught)===expectedNativeErrorPrototype &&
                    caught.message==='external scheduler is not enabled'
            "#,backend).unwrap(),JsValue::Bool(true));
            assert_eq!(runtime.binding_current_realm(),caller);
            assert!(!runtime.external_scheduler_enabled());
            assert!(runtime.take_scheduling_requests().is_empty());
        }
    }

    #[test]
    fn trusted_host_tokens_are_scalar_and_do_not_consume_js_quota_or_callback_roots() {
        let mut legacy=Interpreter::new();
        assert!(!legacy.external_scheduler_enabled());
        let error=legacy.enqueue_native_task(NativeTaskToken::new(904,1)).unwrap_err();
        assert!(error.describe().contains("external scheduler is not enabled"));
        assert!(legacy.take_scheduling_requests().is_empty());
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut runtime=Interpreter::new_with_external_scheduler(1).unwrap();
            assert!(runtime.external_scheduler_enabled());
            let roots=runtime.external_scheduling.as_ref().unwrap().tasks.len();
            for slot in 1..=2048 { runtime.enqueue_native_task(NativeTaskToken::new(904,slot)).unwrap(); }
            assert_eq!(runtime.external_scheduling.as_ref().unwrap().tasks.len(),roots);
            runtime.run_script_with_backend("queueMicrotask(()=>console.log('done'))",backend).unwrap();
            assert!(runtime.output.borrow().is_empty());
            let requests=runtime.take_scheduling_requests();
            assert_eq!(requests.len(),2049);
            for (index,request) in requests[..2048].iter().enumerate() {
                let SchedulingRequest::NativeTask{token}=request else { unreachable!() };
                assert_eq!((token.owner(),token.slot()),(904,index as u64+1));
            }
            let SchedulingRequest::Microtask{job_token}=requests[2048] else { unreachable!() };
            runtime.invoke_one_job(job_token).unwrap();
            assert_eq!(&*runtime.output.borrow(), &["done"]);
            assert!(runtime.take_scheduling_requests().is_empty());
        }
        assert_ne!(NativeTaskToken::new(904,1),NativeTaskToken::new(905,1));
    }
}
