#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_equality_handles_html_dda_internal_slot() {
        let mut interpreter = Interpreter::new();
        let mut html_dda = JsObject::ordinary();
        html_dda.proto = Some(interpreter.object_proto.clone());
        html_dda.is_html_dda = true;
        let html_dda = JsValue::object(html_dda);
        let mut ordinary = JsObject::ordinary();
        ordinary.proto = Some(interpreter.object_proto.clone());
        let ordinary = JsValue::object(ordinary);
        for nullish in [JsValue::Null, JsValue::Undefined] {
            assert!(interpreter.loose_eq(&html_dda, &nullish).unwrap());
            assert!(interpreter.loose_eq(&nullish, &html_dda).unwrap());
            assert!(!interpreter.loose_eq(&ordinary, &nullish).unwrap());
        }
        assert!(!interpreter.loose_eq(&html_dda, &JsValue::Bool(false)).unwrap());
    }

    fn external() -> Interpreter {
        Interpreter::new_with_external_scheduler(64).unwrap()
    }

    fn take(i: &mut Interpreter, all: &mut Vec<SchedulingRequest>) {
        all.extend(i.take_scheduling_requests());
    }

    fn job(request: SchedulingRequest) -> SchedulingToken {
        match request {
            SchedulingRequest::Microtask { job_token } => job_token,
            _ => panic!("expected microtask request"),
        }
    }

    fn callback(request: SchedulingRequest) -> SchedulingToken {
        match request {
            SchedulingRequest::Timer { callback_token, .. }
            | SchedulingRequest::AnimationFrame { callback_token, .. } => callback_token,
            _ => panic!("expected callback request"),
        }
    }

    #[test]
    fn external_registration_is_ordered_and_runs_nothing_until_invoked() {
        let mut i = external();
        i.run_script("setTimeout(()=>console.log('timer'),0); setInterval(()=>console.log('interval'),5); requestAnimationFrame(()=>console.log('frame')); Promise.resolve(1).then(()=>console.log('promise')); queueMicrotask(()=>console.log('microtask'));").unwrap();
        assert!(i.output.borrow().is_empty());
        let requests = i.take_scheduling_requests();
        assert!(matches!(&requests[..], [
            SchedulingRequest::Timer { repeat: false, .. },
            SchedulingRequest::Timer { repeat: true, .. },
            SchedulingRequest::AnimationFrame { .. },
            SchedulingRequest::Microtask { .. },
            SchedulingRequest::Microtask { .. },
        ]));
        assert!(i.pump_timers().is_err());
        assert!(i.drain_microtasks().is_err());
    }

    #[test]
    fn external_driver_controls_every_checkpoint_and_nested_eval() {
        let mut i = external();
        i.run_script("setTimeout(()=>{console.log('A'); Promise.resolve().then(()=>{console.log('P'); queueMicrotask(()=>console.log('R'))}); queueMicrotask(()=>console.log('Q')); eval('1+1');},0); setTimeout(()=>console.log('B'),0)").unwrap();
        let mut requests = i.take_scheduling_requests();
        let a = callback(requests[0]);
        let b = callback(requests[1]);
        i.invoke_one_callback(a, JsValue::Object(i.global_obj.clone()), None).unwrap();
        assert_eq!(&*i.output.borrow(), &["A"]);
        take(&mut i, &mut requests);
        let p = job(requests[2]);
        let q = job(requests[3]);
        i.invoke_one_job(p).unwrap();
        assert_eq!(&*i.output.borrow(), &["A", "P"]);
        take(&mut i, &mut requests);
        let r = job(requests[4]);
        i.invoke_one_job(q).unwrap();
        i.invoke_one_job(r).unwrap();
        i.invoke_one_callback(b, JsValue::Object(i.global_obj.clone()), None).unwrap();
        assert_eq!(&*i.output.borrow(), &["A", "P", "Q", "R", "B"]);
        assert!(i.invoke_one_job(p).is_err());
        assert!(i.invoke_one_callback(a, JsValue::Undefined, None).is_err());
    }

    #[test]
    fn promise_chains_enqueue_new_jobs_without_draining_existing_ones() {
        let mut i = external();
        i.run_script("Promise.resolve(1).then(x=>{console.log('P1'); return x+1}).then(()=>console.log('P2')); Promise.resolve(3).then(()=>console.log('Q'));").unwrap();
        let mut requests = i.take_scheduling_requests();
        assert_eq!(requests.len(), 2);
        let p = job(requests[0]);
        let q = job(requests[1]);
        i.invoke_one_job(p).unwrap();
        assert_eq!(&*i.output.borrow(), &["P1"]);
        take(&mut i, &mut requests);
        assert_eq!(requests.len(), 3);
        i.invoke_one_job(q).unwrap();
        i.invoke_one_job(job(requests[2])).unwrap();
        assert_eq!(&*i.output.borrow(), &["P1", "Q", "P2"]);
    }

    #[test]
    fn native_between_js_entries_can_be_merged_in_order() {
        let mut i = external();
        i.run_script("Promise.resolve().then(()=>console.log('P'))").unwrap();
        let first = i.take_scheduling_requests();
        assert_eq!(first.len(), 1);
        let mut order = vec!["P"];
        // A DOM native notification between entries uses the driver's FIFO.
        order.push("M");
        i.run_script("queueMicrotask(()=>console.log('Q'))").unwrap();
        let second = i.take_scheduling_requests();
        assert_eq!(second.len(), 1);
        order.push("Q");
        assert_eq!(order, ["P", "M", "Q"]);
        i.invoke_one_job(job(first[0])).unwrap();
        i.invoke_one_job(job(second[0])).unwrap();
        assert_eq!(&*i.output.borrow(), &["P", "Q"]);
    }

    #[test]
    fn throwing_callback_leaves_new_jobs_and_later_callbacks_owned() {
        let mut i = external();
        i.run_script("setTimeout(()=>{console.log('A'); queueMicrotask(()=>console.log('Q')); throw 9},0); setTimeout(()=>console.log('B'),0)").unwrap();
        let requests = i.take_scheduling_requests();
        let a = callback(requests[0]);
        let b = callback(requests[1]);
        assert!(i.invoke_one_callback(a, JsValue::Undefined, None).is_err());
        assert_eq!(&*i.output.borrow(), &["A"]);
        let queued = i.take_scheduling_requests();
        i.invoke_one_job(job(queued[0])).unwrap();
        i.invoke_one_callback(b, JsValue::Undefined, None).unwrap();
        assert_eq!(&*i.output.borrow(), &["A", "Q", "B"]);
    }

    #[test]
    fn cancellation_suppresses_due_tokens_and_intervals_can_clear_themselves() {
        let mut i = external();
        i.run_script("let b=setTimeout(()=>console.log('B'),0); setTimeout(()=>{clearTimeout(b); console.log('A')},0); let interval=setInterval(()=>{console.log('I'); clearInterval(interval)},0)").unwrap();
        let requests = i.take_scheduling_requests();
        let b = callback(requests[0]);
        let a = callback(requests[1]);
        let interval = callback(requests[2]);
        i.invoke_one_callback(a, JsValue::Undefined, None).unwrap();
        assert!(matches!(i.take_scheduling_requests()[..], [SchedulingRequest::CancelTimer { .. }]));
        assert!(i.invoke_one_callback(b, JsValue::Undefined, None).is_err());
        i.invoke_one_callback(interval, JsValue::Undefined, None).unwrap();
        assert!(matches!(i.take_scheduling_requests()[..], [SchedulingRequest::CancelTimer { .. }]));
        assert!(i.invoke_one_callback(interval, JsValue::Undefined, None).is_err());
        assert_eq!(&*i.output.borrow(), &["A", "I"]);
    }

    #[test]
    fn frame_timestamp_receiver_and_timer_arguments_are_supplied_by_driver() {
        let mut i = external();
        i.run_script("setTimeout(function(a,b){console.log(this===window,a,b)},0,2,3); requestAnimationFrame(function(ts){console.log(this===window,ts)})").unwrap();
        let requests = i.take_scheduling_requests();
        let timer = callback(requests[0]);
        let frame = callback(requests[1]);
        assert!(matches!(requests[1], SchedulingRequest::AnimationFrame { .. }));
        let window = JsValue::Object(i.global_obj.clone());
        assert!(i.invoke_one_callback(frame, window.clone(), None).is_err());
        i.invoke_one_callback(timer, window.clone(), None).unwrap();
        i.invoke_one_callback(frame, window, Some(Duration::from_millis(32))).unwrap();
        assert_eq!(&*i.output.borrow(), &["true 2 3", "true 32"]);
    }

    #[test]
    fn frame_cancellation_can_happen_from_its_prior_frames_microtask() {
        let mut i = external();
        i.run_script("let b=requestAnimationFrame(()=>console.log('B')); requestAnimationFrame(()=>{console.log('A'); Promise.resolve().then(()=>cancelAnimationFrame(b))})").unwrap();
        let requests = i.take_scheduling_requests();
        let b = callback(requests[0]);
        let a = callback(requests[1]);
        i.invoke_one_callback(a, JsValue::Undefined, Some(Duration::from_millis(4))).unwrap();
        assert_eq!(&*i.output.borrow(), &["A"]);
        let queued = i.take_scheduling_requests();
        i.invoke_one_job(job(queued[0])).unwrap();
        assert!(matches!(i.take_scheduling_requests()[..], [SchedulingRequest::CancelAnimationFrame { .. }]));
        assert!(i.invoke_one_callback(b, JsValue::Undefined, Some(Duration::from_millis(4))).is_err());
    }

    #[test]
    fn full_outbox_rejects_registration_without_losing_prior_work() {
        let mut i = Interpreter::new_with_external_scheduler(1).unwrap();
        i.run_script("queueMicrotask(()=>console.log('kept'))").unwrap();
        assert!(i.run_script("queueMicrotask(()=>console.log('lost'))").is_err());
        let requests = i.take_scheduling_requests();
        assert_eq!(requests.len(), 1);
        let token = job(requests[0]);
        let mut other = external();
        assert!(other.invoke_one_job(token).is_err());
        i.invoke_one_job(token).unwrap();
        assert_eq!(&*i.output.borrow(), &["kept"]);
    }

    #[test]
    fn full_outbox_rejects_cancellation_but_retains_the_timer() {
        let mut i = Interpreter::new_with_external_scheduler(1).unwrap();
        i.run_script("let timer=setTimeout(()=>console.log('kept'),0)").unwrap();
        assert!(i.run_script("clearTimeout(timer)").is_err());
        let requests = i.take_scheduling_requests();
        let token = callback(requests[0]);
        i.invoke_one_callback(token, JsValue::Undefined, None).unwrap();
        assert_eq!(&*i.output.borrow(), &["kept"]);
    }

    #[test]
    fn timer_delay_uses_signed_webidl_long_before_dom_clamping() {
        let mut i = external();
        i.run_script("setTimeout(()=>{},-1); setTimeout(()=>{},4294967295); setInterval(()=>{},4294967297.9); setTimeout(()=>{},NaN); setTimeout(()=>{},Infinity)").unwrap();
        let delays: Vec<_> = i.take_scheduling_requests().into_iter().map(|request| {
            match request {
                SchedulingRequest::Timer { delay_ms, .. } => delay_ms,
                _ => panic!("expected timer"),
            }
        }).collect();
        assert_eq!(delays, [-1, -1, 1, 0, 0]);
    }

    #[derive(Default)]
    struct NativeMutationState { pending: bool, forwarded: bool, fail: bool }

    struct NativeMutationHost(Rc<RefCell<NativeMutationState>>);

    impl DomHost for NativeMutationHost {
        fn query_selector(&mut self, _: &str) -> Option<u32> { Some(1) }
        fn query_selector_all(&mut self, _: &str) -> Vec<u32> { vec![1] }
        fn matches_selector(&self, _: u32, _: &str) -> bool { true }
        fn parent_node(&self, _: u32) -> Option<u32> { None }
        fn get_text_content(&self, _: u32) -> Option<String> { None }
        fn set_text_content(&mut self, _: u32, _: &str) -> Result<(), String> { Ok(()) }
        fn get_document_title(&self) -> String { String::new() }
        fn set_document_title(&mut self, _: &str) -> Result<(), String> { Ok(()) }
        fn fetch(&mut self, _: &str, _: &str) -> Result<HostResponse, String> { Err("not used".into()) }
        fn get_inner_html(&self, _: u32) -> Option<String> { None }
        fn set_inner_html(&mut self, _: u32, _: &str) -> Result<(), String> { Ok(()) }
        fn get_attribute(&self, _: u32, _: &str) -> Option<Option<String>> { None }
        fn set_attribute(&mut self, _: u32, _: &str, _: &str) -> Result<(), String> {
            let mut state = self.0.borrow_mut();
            state.pending = true;
            if state.fail { Err("mutated then failed".into()) } else { Ok(()) }
        }
        fn remove_attribute(&mut self, _: u32, _: &str) -> Result<(), String> { Ok(()) }
        fn create_element(&mut self, _: &str) -> Result<u32, String> { Ok(1) }
        fn create_text_node(&mut self, _: &str) -> Result<u32, String> { Ok(1) }
        fn append_child(&mut self, _: u32, _: u32) -> Result<(), String> { Ok(()) }
        fn append_text(&mut self, _: u32, _: &str) -> Result<(), String> { Ok(()) }
        fn remove_node(&mut self, _: u32) -> Result<(), String> { Ok(()) }
        fn get_style(&self, _: u32, _: &str) -> Option<String> { None }
        fn set_style(&mut self, _: u32, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn pending_native_microtasks(&self) -> Vec<NativeMicrotaskRequest> {
            let state = self.0.borrow();
            if state.pending && !state.forwarded {
                vec![NativeMicrotaskRequest { global: 7, kind: NativeMicrotaskKind::MutationObserver }]
            } else { Vec::new() }
        }
        fn acknowledge_native_microtask(&mut self, _: NativeMicrotaskRequest) {
            self.0.borrow_mut().forwarded = true;
        }
    }

    fn with_native_host(capacity: usize, fail: bool) -> (Interpreter, Rc<RefCell<NativeMutationState>>) {
        let state = Rc::new(RefCell::new(NativeMutationState { fail, ..Default::default() }));
        let host: Rc<RefCell<dyn DomHost>> = Rc::new(RefCell::new(NativeMutationHost(state.clone())));
        let mut i = Interpreter::new_with_external_scheduler(capacity).unwrap();
        i.set_dom_host(host);
        (i, state)
    }

    #[test]
    fn mutation_microtask_keeps_same_script_p_m_q_order() {
        let (mut i, state) = with_native_host(4, false);
        i.run_script("Promise.resolve().then(()=>{}); document.querySelector('x').setAttribute('x','y'); queueMicrotask(()=>{})").unwrap();
        let requests = i.take_scheduling_requests();
        assert!(matches!(&requests[..], [
            SchedulingRequest::Microtask { .. },
            SchedulingRequest::NativeMicrotask { global: 7, kind: NativeMicrotaskKind::MutationObserver },
            SchedulingRequest::Microtask { .. },
        ]));
        assert!(state.borrow().pending); // DOM queued flag remains until checkpoint.
        assert!(state.borrow().forwarded); // Only the forwarding bit is acknowledged.
    }

    #[test]
    fn host_mutation_microtask_precedes_await_continuation() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let (mut i, state) = with_native_host(4, false);
            i.set_host_native_dispatcher(Rc::new(|runtime, _, _, _, _| {
                runtime.dom_host.as_ref().unwrap().borrow_mut()
                    .set_attribute(1, "x", "y").map_err(Throw::type_error)?;
                Ok(JsValue::Undefined)
            }));
            let mutate = i.binding_create_native_function(
                HostFunctionId(1), "mutate", 0, HostCallPolicy::CallOnly,
            );
            i.global_obj.borrow_mut().set_own("mutate", mutate);
            let source = if backend == ExecutionBackend::Bytecode {
                "async function f(){ mutate(); await Promise.resolve(); console.log('continued') } f()"
            } else {
                "mutate(); Promise.resolve().then(() => console.log('continued'))"
            };
            i.run_script_with_backend(source, backend).unwrap();
            let requests = i.take_scheduling_requests();
            assert!(matches!(&requests[..], [
                SchedulingRequest::NativeMicrotask { global: 7, kind: NativeMicrotaskKind::MutationObserver },
                SchedulingRequest::Microtask { .. },
            ]), "{backend:?}: {requests:?}");
            assert!(state.borrow().forwarded);
            assert!(i.output.borrow().is_empty());
            i.invoke_one_job(job(requests[1])).unwrap();
            assert_eq!(&*i.output.borrow(), &["continued"]);
        }
    }

    #[test]
    fn native_mutation_is_forwarded_even_when_operation_throws() {
        let (mut i, state) = with_native_host(3, true);
        assert!(i.run_script("Promise.resolve().then(()=>{}); document.querySelector('x').setAttribute('x','y'); queueMicrotask(()=>{})").is_err());
        assert!(matches!(&i.take_scheduling_requests()[..], [
            SchedulingRequest::Microtask { .. },
            SchedulingRequest::NativeMicrotask { .. },
        ]));
        assert!(state.borrow().forwarded);
    }

    #[test]
    fn reserved_native_slot_keeps_m_before_caught_q_capacity_failure() {
        let (mut i, state) = with_native_host(1, false);
        i.run_script("Promise.resolve().then(()=>{}); document.querySelector('x').setAttribute('x','y'); try { queueMicrotask(()=>{}) } catch(e) { console.log('caught') }").unwrap();
        let requests = i.take_scheduling_requests();
        assert!(matches!(&requests[..], [
            SchedulingRequest::Microtask { .. },
            SchedulingRequest::NativeMicrotask { .. },
        ]));
        assert_eq!(&*i.output.borrow(), &["caught"]);
        assert!(state.borrow().pending);
        assert!(state.borrow().forwarded);
    }

    fn num(v: JsValue) -> f64 {
        match v {
            JsValue::Num(n) => n,
            _ => panic!("expected number: {v:?}"),
        }
    }

    #[test]
    fn evaluates_arithmetic_and_coercion() {
        let mut i = Interpreter::new();
        assert_eq!(num(i.run_script("1 + 2 * 3").unwrap()), 7.0);
        let v = i.run_script("'a' + 2").unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a2");
    }

    #[test]
    fn closures_capture_and_mutate_lexical_bindings() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("function make(){ let n=0; return ()=>++n; } let f=make(); f()+f();")
            .unwrap();
        assert_eq!(num(v), 3.0);
    }

    #[test]
    fn arrays_objects_and_builtin_methods_work() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let a=[1,2]; a.push(3); a.join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "1-2-3");
        let v = i.run_script("Object.keys({a:1,b:2}).length").unwrap();
        assert_eq!(num(v), 2.0);
    }

    #[test]
    fn error_object_converts_to_name_and_message_in_scaffolding() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.run_script("String(new Error('sentinel'))").unwrap();
        assert_eq!(interpreter.to_string(&value).unwrap(), "Error: sentinel");
    }

    #[test]
    fn array_stringification_uses_join_and_empty_nullish_elements() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.run_script("String([1,null,undefined,[2,3]])").unwrap();
        assert_eq!(interpreter.to_string(&value).unwrap(), "1,,,2,3");
    }

    #[test]
    fn math_random_is_a_callable_builtin_with_unit_interval_result() {
        let mut interpreter = Interpreter::new();
        let value = interpreter.run_script("Math.random()").unwrap();
        let JsValue::Num(number) = value else { panic!("Math.random must return Number") };
        assert!((0.0..1.0).contains(&number));
    }

    #[test]
    fn math_random_seed_repeats_per_interpreter() {
        let mut first = Interpreter::new();
        let mut second = Interpreter::new();
        first.set_random_seed(42);
        second.set_random_seed(42);
        for _ in 0..4 {
            let left = first.run_script("Math.random()").unwrap();
            let right = second.run_script("Math.random()").unwrap();
            assert!(matches!((&left, &right), (JsValue::Num(a), JsValue::Num(b)) if a == b));
        }
    }

    #[test]
    fn object_keys_result_inherits_array_methods() {
        let mut interpreter = Interpreter::new();
        let value = interpreter
            .run_script("Object.keys({a:1,b:2}).join(',');")
            .expect("Object.keys result has Array.prototype");
        assert_eq!(interpreter.to_string(&value).unwrap(), "a,b");
    }

    #[test]
    fn common_array_string_and_object_utilities_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("[1,2,3].map(x=>x*2).filter(x=>x>2).reduce((a,b)=>a+b,0);")
            .unwrap();
        assert_eq!(num(value), 10.0);
        let value = i
            .run_script("'  hello  '.trim().includes('ell') && 'abc'.charAt(1)==='b' && 'abc'.indexOf('c')===2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("Object.values({a:1,b:2}).join(',');").unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "1,2");
        let value = i
            .run_script("[1,2,3,4].slice(1,3).concat([5]).includes(5) && [1,2,3].indexOf(2)===1 && 'abcdef'.replace('cd','X')==='abXef' && 'abc'.startsWith('a') && 'abc'.endsWith('c');")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut k = Interpreter::new();
        let value = k
            .run_script("Array.from('ab').concat(Array.of(3)).join(',') === 'a,b,3' && Number.isNaN(NaN) && Number.isFinite(3);")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = k
            .run_script("Math.floor(1.9)===1 && Math.ceil(1.1)===2 && Math.round(1.5)===2 && Math.sqrt(9)===3 && Math.pow(2,3)===8 && Math.trunc(-1.9)===-1 && Math.sign(-2)===-1;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let mut j = Interpreter::new();
        let value = j
            .run_script("JSON.stringify({a:1,b:[2]}) === '{\"a\":1,\"b\":[2]}' && JSON.parse('{\"x\":4}').x === 4 && parseInt('0x10') === 16 && parseFloat('1.5px') === 1.5;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn function_call_apply_and_bind_preserve_receiver_and_arguments() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("function f(a,b){return this.x+a+b;} let o={x:1}; f.call(o,2,3)+f.apply(o,[2,3])+f.bind(o,2)(3);")
            .unwrap();
        assert_eq!(num(value), 18.0);
    }

    #[test]
    fn object_prototype_helpers_and_array_is_array_work() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let p={x:4}; let o=Object.create(p); o.y=2; Object.getPrototypeOf(o)===p && Object.hasOwn(o,'y') && !Object.hasOwn(o,'x') && Array.isArray([]) && !Array.isArray({});")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn class_inheritance_dispatches_super_methods_with_the_real_receiver() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { value(){ return 4; } } class B extends A { value(){ return super.value()+1; } } new B().value();")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn derived_constructor_can_call_super() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class A { constructor(value){ this.value=value; } } class B extends A { constructor(value){ super(value+1); } } new B(4).value;")
            .unwrap();
        assert_eq!(num(value), 5.0);
    }

    #[test]
    fn derived_this_is_initialized_only_by_super_and_return_rules_apply() {
        let cases = [
            "class A {} class B extends A { constructor(){ this.x=1; super(); } } try { new B(); false } catch (e) { e instanceof ReferenceError }",
            "class A {} class B extends A { constructor(){ super(); super(); } } try { new B(); false } catch (e) { e instanceof ReferenceError }",
            "class A {} class B extends A { constructor(){ return 1; } } try { new B(); false } catch (e) { e instanceof TypeError }",
            "class A {} class B extends A { constructor(){ super(); return undefined; } } new B() instanceof B",
            "class A { constructor(x){ this.x=x; } } class B extends A {} new B(7).x === 7",
            "class A {} class B extends A { constructor(){ return {}; } } new B() instanceof B === false",
            "class A {} class B extends A { constructor(){ super(); } } function Target() {} Reflect.construct(B, [], Target) instanceof Target",
            "class Child extends Array {} var child = new Child(); child instanceof Child && child instanceof Array",
            "class Child extends Object { valueOf(){ return 42; } } new Child().valueOf() === 42",
            "class Base {} class Child extends Base { constructor(){ var callSuper = () => super(); callSuper(); } } new Child() instanceof Child",
        ];
        for source in cases {
            let mut i = Interpreter::new();
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn class_getters_and_setters_use_property_descriptors() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("class C { constructor(){this._x=1;} get x(){return this._x;} set x(v){this._x=v;} } let c=new C(); c.x=7; c.x;")
            .unwrap();
        assert_eq!(num(value), 7.0);
    }

    #[test]
    fn map_uses_same_value_zero_and_preserves_insertion_order() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let m=new Map([['a',1],['b',2]]); m.set('a',9).set('c',3); m.get('a') + m.size;",
            )
            .unwrap();
        assert_eq!(num(v), 12.0);
        let v = i
            .run_script("new Map([['a',1],['b',2]]).keys().join(',');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "a,b");
        let v = i
            .run_script("let m2=new Map(); m2.set(NaN,1); m2.has(NaN);")
            .unwrap();
        assert!(matches!(v, JsValue::Bool(true)));
    }

    #[test]
    fn set_deduplicates_same_value_zero_and_supports_mutation() {
        let mut i = Interpreter::new();
        let v = i
            .run_script("let s=new Set([1,1,2,NaN,NaN]); s.add(3); s.delete(2); s.size;")
            .unwrap();
        assert_eq!(num(v), 3.0);
        let v = i
            .run_script("new Set(['x','y']).values().join('-');")
            .unwrap();
        assert_eq!(i.to_string(&v).unwrap(), "x-y");
    }

    #[test]
    fn settled_promises_chain_and_catch_synchronously() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.resolve(2).then(x=>x+1).then(x=>console.log(x)); Promise.reject('bad').catch(e=>console.log(e));",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["bad".to_string(), "3".to_string()]);
    }

    #[test]
    fn pending_promise_reactions_run_when_resolved() {
        let mut i = Interpreter::new();
        i.run_script(
            "let resolve; let p=new Promise(r=>{resolve=r;}); p.then(x=>console.log(x)); resolve(5);",
        )
        .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn class_expressions_are_strict_and_restore_the_outer_context() {
        let mut i = Interpreter::new();
        i.run_script(r#"
            var frozen = Object.preventExtensions({});
            try { class C { [frozen.key = 1]() {} } }
            catch (error) { console.log(error.name); }
            try { class C extends (frozen.base = 1) {} }
            catch (error) { console.log(error.name); }
            frozen.sloppy = 1;
            console.log(frozen.sloppy);
        "#).unwrap();
        assert_eq!(i.output.borrow().as_slice(), ["TypeError", "TypeError", "undefined"]);
    }

    #[test]
    fn prepared_module_uses_strict_declarations_and_execution() {
        let mut i = Interpreter::new();
        let program = crate::parse_script(r#"
            function hoisted() { Object.preventExtensions({}).key = 1; }
            try { hoisted(); } catch (error) { console.log(error.name); }
            try { Object.preventExtensions({}).key = 1; }
            catch (error) { console.log(error.name); }
            (0, eval)('Object.preventExtensions({}).key = 1');
        "#).unwrap();
        i.prepare_module(&program).unwrap();
        i.run_program(&program).unwrap();
        assert_eq!(i.output.borrow().as_slice(), ["TypeError", "TypeError"]);
    }

    #[test]
    fn parameter_bindings_allow_var_redeclaration_without_escaping() {
        let mut i = Interpreter::new();
        i.run_script(r#"
            var dbname = 'outer';
            function createdb_for_multiple_tests(dbname, version) {
                var dbname = dbname + '-test';
                var version;
                return dbname + ':' + version;
            }
            console.log(createdb_for_multiple_tests('database', 2));
            console.log(dbname);
            function keep(value) { var value; return value; }
            console.log(keep(7));
            function duplicate(value, value) { var value; return value; }
            console.log(duplicate(1, 2));
            function patterned({value}, ...rest) {
                var value; var rest;
                return value + rest[0];
            }
            console.log(patterned({value: 3}, 4));
            console.log(((value) => { var value; return value; })(8));
            function defaults(a = 5, b = a) { var a; return a + b; }
            console.log(defaults(undefined));
            try { (function(arguments, p = eval('var arguments')) {})(); }
            catch (error) { console.log(error.name); }
            console.log((function(arguments) { var arguments; return arguments; })(9));
            var later = 'outer';
            try { (function(a = later, later = 1) {} )(); }
            catch (error) { console.log(error.name); }
            try { (function(a = a) {} )(); }
            catch (error) { console.log(error.name); }
        "#).unwrap();
        assert_eq!(i.output.borrow().as_slice(),
            ["database-test:2", "outer", "7", "2", "7", "8", "10", "SyntaxError", "9",
             "ReferenceError", "ReferenceError"]);
    }

    #[test]
    fn async_function_returns_a_promise() {
        let mut i = Interpreter::new();
        i.run_script("async function f(){ return 4; } f().then(x=>console.log(x+1));")
            .unwrap();
        assert_eq!(&*i.output.borrow(), &["5".to_string()]);
    }

    #[test]
    fn promise_reactions_run_after_current_script_turn() {
        let mut i = Interpreter::new();
        i.run_script(
            "console.log('before'); Promise.resolve(1).then(x=>console.log(x)); console.log('after');",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &["before".to_string(), "after".to_string(), "1".to_string()]
        );
    }

    #[test]
    fn eval_wrappers_and_number_constants_follow_primitive_rules() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("eval('1 + 1') + new Boolean(true) + Number.MIN_VALUE > 2;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i
            .run_script("Number.POSITIVE_INFINITY + 1 === Infinity;")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn date_default_value_uses_string_conversion_and_in_uses_left_key() {
        let mut i = Interpreter::new();
        let value = i
            .run_script("let d = new Date(0); d + d === d.toString() + d.toString();")
            .unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
        let value = i.run_script("'x' in {x: 1};").unwrap();
        assert!(matches!(value, JsValue::Bool(true)));
    }

    #[test]
    fn compound_assignment_evaluates_member_reference_once() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let calls=0; function base(){calls++; return {x:0};} base().x ||= 2; calls;",
            )
            .unwrap();
        assert_eq!(num(value), 1.0);
    }

    #[test]
    fn descriptors_and_completion_values_are_observable() {
        let mut i = Interpreter::new();
        let value = i
            .run_script(
                "let o={}; Object.defineProperty(o,'x',{get:function(){return 0;},set:undefined,enumerable:true,configurable:true}); o.x &&= 1; o.x;",
            )
            .unwrap();
        assert_eq!(num(value), 0.0);
        let value = i
            .run_script("eval('1; for (var x=true; x; x=false) { 3; }');")
            .unwrap();
        assert_eq!(num(value), 3.0);
    }

    #[test]
    fn promise_combinators_preserve_order_and_settlement_kind() {
        let mut i = Interpreter::new();
        i.run_script(
            "Promise.all([Promise.resolve(1), 2]).then(v=>console.log(v.join(','))); Promise.allSettled([Promise.resolve(3), Promise.reject(4)]).then(v=>console.log(v[0].status, v[1].status)); Promise.any([Promise.reject(1), Promise.resolve(5)]).then(v=>console.log(v));",
        )
        .unwrap();
        assert_eq!(
            &*i.output.borrow(),
            &[
                "1,2".to_string(),
                "fulfilled rejected".to_string(),
                "5".to_string()
            ]
        );
    }

    #[test]
    fn array_object_and_for_of_patterns_bind_values_defaults_and_rest() {
        let mut i = Interpreter::new();
        let value = match i.run_script(
            "let [a,b=3,...tail]=[1,undefined,4,5]; let {x,y:z=4,...rest}={x:1,y:undefined,q:2}; a+b+tail.length+x+z+Object.keys(rest).length;",
        ) {
            Ok(value) => value,
            Err(Throw(JsValue::Object(object))) => panic!("pattern error: {:?}", object.borrow().get_own("message")),
            Err(error) => panic!("pattern error: {:?}", error.0),
        };
        assert_eq!(num(value), 12.0);
        let mut j = Interpreter::new();
        let value = j
            .run_script("let total=0; for (const [a,b] of [[2,3]]) { total=a+b; } total;")
            .unwrap();
        assert_eq!(num(value), 5.0);
        let mut k = Interpreter::new();
        let value = k
            .run_script("let a=0,b=0; [a,b]=[3,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut m = Interpreter::new();
        let value = m
            .run_script("let key='x'; let {[key]: value}={x:9}; value;")
            .unwrap();
        assert_eq!(num(value), 9.0);
        let mut n = Interpreter::new();
        let value = n
            .run_script("let a=0,b=0; [a=3,b]=[undefined,4]; ({a,b}={a:5,b:6}); a+b;")
            .unwrap();
        assert_eq!(num(value), 11.0);
        let mut p = Interpreter::new();
        let value = p
            .run_script(
                "let out=[]; for (const [a,b] of [[1,2],[3,4]]) { out.push(a+b); } out.join(',');",
            )
            .unwrap();
        assert_eq!(p.to_string(&value).unwrap(), "3,7");
        let mut q = Interpreter::new();
        let value = q
            .run_script("let out=[]; let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; for (const x of it) out.push(x); out.join(',');")
            .unwrap();
        assert_eq!(q.to_string(&value).unwrap(), "1,2");
        let mut r = Interpreter::new();
        let value = r
            .run_script("let closed=0; let it={next:function(){throw 'stop';},return:function(){closed++;}}; try {[...it];} catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut s = Interpreter::new();
        let value = s
            .run_script("let closed=0; let n=0; let it={next:function(){n++;return {value:n,done:n>3};},return:function(){closed++;}}; try { for (const x of it) { if (x===2) throw 'stop'; } } catch(e) {} closed;")
            .unwrap();
        assert_eq!(num(value), 1.0);
        let mut t = Interpreter::new();
        let value = t
            .run_script("let it={n:0,next:function(){this.n++;return {value:this.n,done:this.n>2};}}; let iterable={ [Symbol.iterator]:function(){return it;} }; [...iterable].join(',');")
            .unwrap();
        assert_eq!(t.to_string(&value).unwrap(), "1,2");
    }

    #[test]
    fn loops_and_try_catch_are_executable() {
        let mut i = Interpreter::new();
        let v = i
            .run_script(
                "let s=0; for(let n=0;n<5;n++){s+=n;} try { throw 4; } catch(e) { s+=e; } s;",
            )
            .unwrap();
        assert_eq!(num(v), 14.0);
    }

    #[test]
    fn console_output_is_captured_without_io() {
        let mut i = Interpreter::new();
        i.run_script("console.log('hello', 3);").unwrap();
        assert_eq!(&*i.output.borrow(), &["hello 3".to_string()]);
    }

    #[test]
    fn timers_run_only_when_the_host_pumps_them() {
        let mut i = Interpreter::new();
        i.run_script("setTimeout(()=>console.log('later'),0);").unwrap();
        assert!(i.output.borrow().is_empty());
        assert!(i.pump_timers().unwrap());
        assert_eq!(&*i.output.borrow(), &["later".to_string()]);
        assert!(!i.pump_timers().unwrap());
        let mut interval = Interpreter::new();
        interval.run_script("setInterval(()=>console.log('tick'),0);").unwrap();
        assert!(interval.pump_timers().unwrap());
        assert!(interval.pump_timers().unwrap());
        interval.run_script("clearInterval(1);").unwrap();
        assert!(!interval.pump_timers().unwrap());
    }

    #[test]
    fn location_requests_navigation_and_updates_href() {
        let mut i = Interpreter::new();
        i.set_location_url("page.html");
        let value = i
            .run_script("location.assign('next.html'); location.href;")
            .unwrap();
        assert_eq!(i.to_string(&value).unwrap(), "next.html");
        assert_eq!(i.take_navigation().as_deref(), Some("next.html"));
        i.run_script("window.location='other.html';").unwrap();
        assert_eq!(i.take_navigation().as_deref(), Some("other.html"));
        i.run_script("history.back();").unwrap();
        assert_eq!(i.take_history_delta(), Some(-1));
        i.run_script("location.reload();").unwrap();
        assert_eq!(i.take_navigation().as_deref(), Some("other.html"));
        i.set_history_length(3);
        let length = i.run_script("history.length;").unwrap();
        assert_eq!(length, JsValue::num(3.0));
        let state_value = i.run_script("history.pushState({id:7}, '', 'spa.html'); history.state.id;").unwrap();
        assert_eq!(state_value, JsValue::num(7.0));
        let update = i.take_history_update().unwrap();
        assert_eq!((&update.0, update.1), (&"spa.html".to_string(), false));
        i.run_script("history.replaceState({id:8}, '', 'spa2.html');").unwrap();
        let update = i.take_history_update().unwrap();
        assert_eq!((&update.0, update.1), (&"spa2.html".to_string(), true));
    }

    #[test]
    fn array_sort_is_stable_and_keeps_undefined_before_holes() {
        let mut i = Interpreter::new();
        let source = "var a=[,undefined,3,1]; var same=a.sort(); [same===a,a[0]===1,a[1]===3,a[2]===undefined,!(3 in a),Object.keys(a).join(',')==='0,1,2'].join(',')";
        assert_eq!(i.run_script(source).unwrap(), JsValue::str("true,true,true,true,true,true"));
        let source = "var a=[{key:1,id:'a'},{key:0,id:'b'},{key:1,id:'c'}]; var b=a.toSorted((x,y)=>x.key-y.key); b.map(x=>x.id).join('')==='bac' && a.map(x=>x.id).join('')==='abc'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
        let source = "var a=[3,2,1]; try { a.sort((x,y)=>{throw new Error('stop')}) } catch(e) {} a.join(',')==='3,2,1'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn for_in_tracks_descriptors_prototypes_and_deletion() {
        let mut i = Interpreter::new();
        let source = "var a=[1,2], keys=''; for (var key in a) keys+=key; keys==='01'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
        let source = "class MyUint8Array extends Uint8Array {} var rab=new ArrayBuffer(100,{maxByteLength:200}),ta=new MyUint8Array(rab,0,3),keys=''; for(var key in ta) keys+=key; keys==='012' && Object.getOwnPropertyDescriptor(MyUint8Array.prototype,'constructor').enumerable===false";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
        let source = "var p={x:1,y:2}, o=Object.create(p); Object.defineProperty(o,'x',{value:3,enumerable:false}); o.z=4; var keys=''; for(var key in o) keys+=key; keys==='zy'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
        let source = "var o={a:1,b:2,c:3}, keys=''; for(var key in o){ keys+=key; if(key==='a') delete o.b; } keys==='ac'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
        let source = "var keys=''; for(var key in 'ab') keys+=key; keys==='01'";
        assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn ordinary_calls_bind_sloppy_and_strict_this() {
        let mut i = Interpreter::new();
        for source in [
            "this===globalThis",
            "(function(){return this===globalThis})()",
            "(function(){'use strict';return this===undefined})()",
            "(function(){return Object.getPrototypeOf(this)===Number.prototype}).call(5)",
        ] {
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn bound_native_then_forwards_receiver_and_executor_arguments() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend("var p=Promise.reject(new TypeError('x')); var direct=new Promise((resolve,reject)=>p.then(resolve,reject)); var bound=new Promise(p.then.bind(p)); var a='pending', b='pending'; direct.then(()=>a='fulfilled',e=>a=e instanceof TypeError?'rejected':'wrong'); bound.then(()=>b='fulfilled',e=>b=e instanceof TypeError?'rejected':'wrong');", backend).unwrap();
            assert_eq!(i.run_script_with_backend("a+','+b", backend).unwrap(), JsValue::str("rejected,rejected"));
            assert_eq!(i.run_script_with_backend(
                "var target; new Promise((resolve) => { target=resolve; }); Object.keys(target).length===0 && Object.getOwnPropertyNames(target).every(k => k!=='[[PromiseTarget]]')",
                backend,
            ).unwrap(), JsValue::Bool(true));
        }
    }

    #[test]
    fn resolving_with_pending_promise_keeps_outer_pending() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let source = "var settled=false; var inner=new Promise(()=>{}); new Promise(resolve=>resolve(inner)).then(()=>settled=true); settled";
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(false));
        }
    }

    #[test]
    fn accessors_keep_callable_identity_and_strict_receiver() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let source = "var get=function(){'use strict'; return this}; var set=function(v){'use strict'; this.saved=v}; Object.defineProperty(Number.prototype,'slot',{get:get,set:set,configurable:true}); var d=Object.getOwnPropertyDescriptor(Number.prototype,'slot'); var r=Reflect.getOwnPropertyDescriptor(Number.prototype,'slot'); d.get===get && d.set===set && r.get===get && r.set===set && (5).slot===5";
            assert_eq!(i.run_script_with_backend(source, backend).unwrap(), JsValue::Bool(true), "{backend:?}");
        }
    }

    #[test]
    fn arrows_capture_enclosing_arguments_across_native_calls() {
        let mut i = Interpreter::new();
        for source in [
            "function outer(a,b){ return [0].map(() => arguments.length)[0] }; outer(1,2)===2",
            "var inner; function outer(x){ inner=()=>arguments[0] }; outer(42); inner()===42",
        ] {
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn direct_eval_uses_callers_environment_for_this_and_var_conflicts() {
        let mut i = Interpreter::new();
        for source in [
            "function f(){'use strict'; return eval('this')}; f()===undefined",
            "var f=(p=eval('var arguments=1'),arguments)=>{}; try { f(); false } catch(e) { e instanceof SyntaxError }",
            "function outer(){var x=0; function inner(){eval('var x=1')} inner(); return x===0} outer()",
            "function optional(){var local=1; return eval?.('typeof local')} optional()==='undefined'",
            "function boundEval(){var local=1; var eval=globalThis.eval.bind(null,'typeof local'); return eval()} boundEval()==='undefined'",
            "try { throw null } catch(err) { eval('var err;') } true",
        ] {
            assert_eq!(i.run_script(source).unwrap(), JsValue::Bool(true), "{source}");
        }
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("Object.preventExtensions(globalThis); try { eval('var newGlobalName'); false } catch(e) { e instanceof TypeError }").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn array_push_rejects_null_receiver_even_without_arguments() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script("try { Array.prototype.push.call(null); false } catch(e) { e instanceof TypeError }").unwrap(), JsValue::Bool(true));
        assert_eq!(i.run_script("try { Array.prototype.slice.call(null); false } catch(e) { e instanceof TypeError }").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn intl_options_reject_embedded_nul() {
        let mut i = Interpreter::new();
        for source in [
            "Intl.Collator.supportedLocalesOf([], {localeMatcher: 'lookup\\0cookie'})",
            "new Intl.Collator('en', {localeMatcher: 'lookup\\0cookie'})",
            "new Intl.Collator('en', {usage: 'sort\\0cookie'})",
            "new Intl.Collator('en', {caseFirst: 'upper\\0cookie'})",
            "new Intl.Collator('en', {sensitivity: 'base\\0cookie'})",
            "Intl.DateTimeFormat.supportedLocalesOf([], {localeMatcher: 'lookup\\0cookie'})",
            "new Intl.DateTimeFormat('en', {localeMatcher: 'lookup\\0cookie'})",
            "new Intl.DateTimeFormat('en', {hourCycle: 'h24\\0cookie'})",
            "new Intl.DateTimeFormat('en', {weekday: 'narrow\\0cookie'})",
            "new Intl.DateTimeFormat('en', {era: 'narrow\\0cookie'})",
            "new Intl.DateTimeFormat('en', {year: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {month: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {day: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {hour: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {minute: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {second: '2-digit\\0cookie'})",
            "new Intl.DateTimeFormat('en', {formatMatcher: 'basic\\0cookie'})",
            "new Intl.DateTimeFormat('en', {timeZone: 'UTC\\0cookie'})",
        ] {
            let source = format!("try {{ {source}; false }} catch(e) {{ e instanceof RangeError }}");
            assert_eq!(i.run_script(&source).unwrap(), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn native_error_constructors_inherit_error_constructor() {
        let mut i = Interpreter::new();
        let error_constructor = i.global_env.resolve("Error").unwrap();
        let eval_error_constructor = i.global_env.resolve("EvalError").unwrap();
        let JsValue::Object(eval_error_object) = &eval_error_constructor else { unreachable!() };
        assert_eq!(eval_error_object.borrow().proto.clone().map(JsValue::Object), Some(error_constructor));
        for name in ["EvalError", "RangeError", "ReferenceError", "SyntaxError", "TypeError", "URIError"] {
            for source in [
                format!("Object.getPrototypeOf({name})===Error"),
                format!("Object.getPrototypeOf({name}.prototype)===Error.prototype"),
            ] {
                assert_eq!(i.run_script(&source).unwrap(), JsValue::Bool(true), "{source}");
            }
        }
        let sort = i.array_proto.borrow().get_own("sort").unwrap();
        let thrown = i.call_value(&sort, JsValue::Null, vec![]).unwrap_err();
        let JsValue::Object(error) = thrown.0 else { panic!("native TypeError is not an object") };
        let type_error = i.global_env.resolve("TypeError").unwrap();
        let prototype = i.get_prop(&type_error, "prototype").unwrap();
        assert_eq!(error.borrow().proto.clone().map(JsValue::Object), Some(prototype));
        assert_eq!(i.run_script("try { Array.prototype.sort.call(null) } catch(e) { e.constructor===TypeError && e instanceof TypeError && e instanceof Error }").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn sparse_array_search_bounds_follow_from_index() {
        let mut i = Interpreter::new();
        for source in [
            "var a=[]; a[4294967294]=true; a.indexOf(true,Infinity)===-1 && a.indexOf(true,'Infinity')===-1",
            "var needle={}; var a={length:Number.MAX_SAFE_INTEGER}; a[Number.MAX_SAFE_INTEGER-2]=needle; Array.prototype.indexOf.call(a,needle,Number.MAX_SAFE_INTEGER-4)===Number.MAX_SAFE_INTEGER-2",
            "var a={length:Number.MAX_SAFE_INTEGER}; a[Number.MAX_SAFE_INTEGER-2]='x'; Array.prototype.lastIndexOf.call(a,'x',Number.MAX_SAFE_INTEGER-2)===Number.MAX_SAFE_INTEGER-2",
            "var a={length:Number.MAX_SAFE_INTEGER}; a[Number.MAX_SAFE_INTEGER-2]='x'; Array.prototype.includes.call(a,'x',Number.MAX_SAFE_INTEGER-3)",
            "[,].includes(undefined) && [NaN].includes(NaN) && [NaN].indexOf(NaN)===-1",
        ] {
            assert_eq!(i.run_script(source).unwrap_or_else(|error| panic!("{source}: {}", error.describe())), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn object_function_and_boolean_audit_members_work() {
        let mut i = Interpreter::new();
        for source in [
            "Object.is(NaN,NaN) && !Object.is(0,-0) && Object.entries({a:1})[0].join(',')==='a,1'",
            "var x=Object.fromEntries([['a',1],['b',2]]); x.a===1 && x.b===2",
            "var p={}; Object.prototype.entrySetter=0; Object.defineProperty(Object.prototype,'entrySetter',{set:function(v){p.called=v},configurable:true}); var x=Object.fromEntries([['entrySetter',7]]); x.entrySetter===7 && p.called===undefined && Object.keys(x).join(',')==='entrySetter'",
            "var x={a:1}; Object.freeze(x); Object.isFrozen(x) && Object.isSealed(x) && Object.getOwnPropertyDescriptor(x,'a').writable===false",
            "var x=Object.defineProperties({}, {a:{value:1,enumerable:true},b:{value:2}}); Object.keys(x).join(',')==='a' && Object.getOwnPropertyDescriptors(x).b.value===2",
            "var p={}, x=Object.create(p); p.isPrototypeOf(x) && x.toLocaleString()==='[object Object]'",
            "Boolean.prototype.valueOf()===false && Boolean.prototype.toString()==='false' && new Boolean(true).valueOf()===true && true.toString()==='true'",
            "Object.keys(new Boolean(true)).length===0",
            "typeof Function.prototype.toString.call(Array.prototype.sort)==='string'",
            "Function.prototype.call.toString()==='function call() { [native code] }' && Function.prototype.apply.toString()==='function apply() { [native code] }' && Function.prototype.bind.toString()==='function bind() { [native code] }'",
            "Function.prototype.hasOwnProperty('call') && Function.prototype.hasOwnProperty('apply') && Function.prototype.hasOwnProperty('bind') && Function.prototype.call.length===1 && Function.prototype.apply.length===2 && Function.prototype.bind.length===1",
            "typeof Function.prototype==='function' && Function.prototype()===undefined && Function.prototype.length===0 && !('prototype' in Function.prototype)",
            "function testCase(){}; testCase.call.toString()==='function call() { [native code] }' && testCase.bind(null).toString()==='function bound() { [native code] }'",
            "function F(a,b){this.sum=a+b}; var B=F.bind(null,2); var x=new B(3); x.sum===5 && B.name==='bound F' && B.length===1 && !('prototype' in B)",
            "function f(a,b){return a+b}; f.apply(null,{0:2,1:3,length:2})===5",
            "function f(){}; var p={}; Object.setPrototypeOf(f,p); Object.getPrototypeOf(Function.prototype.bind.call(f,null))===p",
            "var x=Array(3); x.length===3 && !(0 in x) && Array.of(3).length===1 && Array[Symbol.species]===Array",
            "typeof Array.fromAsync==='function'",
            "Array.fromAsync.length===1",
            "!('prototype' in Array.fromAsync)",
            "Array.fromAsync([1,2]) instanceof Promise",
            "var x=Object.groupBy([1,2,3], n=>n%2); x[0].join(',')==='2' && x[1].join(',')==='1,3'",
        ] {
            let actual = i.run_script(source).unwrap_or_else(|error| panic!("{source}: {}", error.describe()));
            assert_eq!(actual, JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn array_and_object_regressions_from_test262_are_covered() {
        let mut i = Interpreter::new();
        for source in [
            "var a=[]; Object.defineProperty(a,'2',{get(){throw new TypeError()},configurable:true}); a.length===3",
            "var a=new Array(10), n=0; a[0]=a[1]=undefined; a.reduce((x,y)=>{n++;return y}); n===1",
            "var a=[0,1]; Object.defineProperty(a,'1',{get(){return 1},configurable:false}); try { Object.defineProperties(a,{length:{value:1}}); false } catch(e) { e instanceof TypeError && a.length===2 }",
            "var A=function(){Object.preventExtensions(this)}; var a=[1]; a.constructor={}; a.constructor[Symbol.species]=A; try {a.flat(); false} catch(e) {e instanceof TypeError}",
            "try {Object.freeze([]).pop(); false} catch(e) {e instanceof TypeError}",
            "var rab=new ArrayBuffer(4,{maxByteLength:8}); try {Object.freeze(new Uint8Array(rab,0,4)); false} catch(e) {e instanceof TypeError}",
            "function f(){return delete arguments.length}; f()===true",
            "function f(){}; Object.setPrototypeOf(f,{length:42}); delete f.length; Function.prototype.bind.call(f,null,1).length===0",
            "var x={length:2**53-1}; try {Array.prototype.unshift.call(x,null); false} catch(e) {e instanceof TypeError}",
            "function f(){return arguments}; var a=f(1,2,3); Array.prototype.indexOf.call(a,2)===1 && Array.from(a).join(',')==='1,2,3'",
            "function f(){return arguments}; var a=f(1,2,3); a[Symbol.isConcatSpreadable]=true; [].concat(a,a).join(',')==='1,2,3,1,2,3'",
            "Array.prototype.pop.call(true)===undefined",
            "Array.prototype.indexOf.call(new String('null'),'l')===2",
            "var empty=[]; var token={valueOf(){throw new Error('wrong')}}; empty.indexOf(1,token)===-1",
            "var x={0:'x',4294967295:'y',length:4294967296}; Array.prototype.pop.call(x)==='y' && x.length===4294967295",
        ] {
            assert_eq!(i.run_script(source).unwrap_or_else(|error| panic!("{source}: {error:?}")), JsValue::Bool(true), "{source}");
        }
    }

    #[test]
    fn object_model_operations_preserve_descriptors_and_receivers() {
        let mut i = Interpreter::new();
        for source in [
            "var x={}; Object.defineProperty(x,'a',{value:1,writable:true,configurable:true}); Object.defineProperty(x,'a',{value:2}); var d=Object.getOwnPropertyDescriptor(x,'a'); d.value===2 && d.writable===true && d.enumerable===false && d.configurable===true",
            "var p={x:{value:3,enumerable:false},y:{value:4,enumerable:true}}; var x=Object.create(null,p); Object.keys(x).join(',')==='y' && x.x===3 && Object.getPrototypeOf(x)===null",
            "var x=Object.create({}, {prop:{configurable:this}}); delete x.prop; !Object.hasOwn(x,'prop')",
            "var x=Object.create({}, {prop:{enumerable:this}}); Object.keys(x).join(',')==='prop'",
            "var x=Object.preventExtensions({}); Reflect.defineProperty(x,'a',{value:1})===false && !(('a') in x)",
            "var x=Object.preventExtensions({}); try {Object.defineProperty(x,'a',{value:1}); false} catch(e) {e instanceof TypeError}",
            "var p={x:1}, r={}; Reflect.set(p,'x',2,r)===true && p.x===1 && r.x===2",
            "var p={x:1}, q={}; Object.defineProperty(q,'x',{value:9,writable:false}); var r=Object.create(q); Reflect.set(p,'x',3,r)===true && r.x===3 && q.x===9",
            "var p={set x(v){this.y=v}}, r={}; Reflect.set(p,'x',7,r)===true && r.y===7 && !('y' in p)",
            "var p={}; Object.defineProperty(p,'x',{value:1,writable:false}); var x=Object.create(p); x.x=2; x.x===1 && !Object.hasOwn(x,'x')",
            "var p={set x(v){this.y=v}}, r=Object.create(p); r.x=5; r.y===5 && !Object.hasOwn(r,'x')",
            "var p={set x(v){this.y=v}}, r=Object.create(p); Object.assign(r,{x:6}); r.y===6 && !Object.hasOwn(r,'x')",
            "Object.assign(5,{x:1}).x===1",
            "try {Object.create(null,null); false} catch(e) {e instanceof TypeError}",
            "function F(a,b){this.sum=a+b}; Reflect.construct(F,{0:2,1:3,length:2}).sum===5",
            "var ta=new Uint8Array(2); Reflect.defineProperty(ta,'0',{value:3})===true && ta[0]===3 && Reflect.deleteProperty(ta,'0')===false",
            "new Map([[1,2]]).get(1)===2 && new Set([1,2]).has(2)",
        ] {
            assert_eq!(i.run_script(source).unwrap_or_else(|error| panic!("{source}: {error:?}")), JsValue::Bool(true), "{source}");
        }
    }
}
