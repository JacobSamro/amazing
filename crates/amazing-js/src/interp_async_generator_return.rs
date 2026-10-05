// Async-generator Return/Await completion portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, return statement/yield_one)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//
// Request/continuation adapter uses Amazing's existing pooled-worker, prepared
// intrinsic Promise and internal PerformPromiseThen contracts. State/capability
// objects are strongly owned by the generator and pending callback environment.
// No RefCell borrow survives JavaScript execution or a worker handoff. A pending
// reaction roots its state until completion; completed coroutines are released.

struct ThreadAsyncGeneratorRequest {
    kind: u8,
    value: JsValue,
    target: Obj,
}

fn thread_async_generator_enqueue(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    kind: u8,
    value: JsValue,
) -> Result<JsValue, Throw> {
    // Capability belongs to the invoked builtin's realm; body execution below
    // belongs to the generator's captured creation realm, including borrowed calls.
    let promise = new_promise(i, PromiseFate::Pending);
    let target = promise_object(&promise)?;
    state
        .borrow_mut()
        .async_queue
        .push_back(ThreadAsyncGeneratorRequest {
            kind,
            value,
            target,
        });
    thread_async_generator_drain(i, state)?;
    Ok(promise)
}

fn thread_async_generator_drain(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
) -> Result<(), Throw> {
    loop {
        let request = {
            let mut guard = state.borrow_mut();
            if guard.async_executing || guard.async_awaiting {
                return Ok(());
            }
            guard.async_queue.pop_front()
        };
        let Some(request) = request else {
            return Ok(());
        };
        let (started, completed, unavailable) = {
            let guard = state.borrow();
            (
                guard.started,
                guard.started && guard.coro.is_none(),
                !guard.started && guard.coro.is_none(),
            )
        };
        state.borrow_mut().async_request = Some(request.target);
        if unavailable {
            // Preserve the existing platform policy when no worker could be
            // spawned; it must not become a completed generator after one call.
            thread_async_generator_finish(i, &state, Err(Throw::type_error(UNSUPPORTED_MSG)))?;
            continue;
        }
        if completed || (!started && request.kind != 0) {
            // GeneratorResumeAbrupt at suspendedStart never executes the body.
            let coro = {
                let mut guard = state.borrow_mut();
                guard.started = true;
                guard.coro.take()
            };
            if let Some(mut coro) = coro {
                let signal = if request.kind == 1 {
                    Resume::Return(request.value.clone())
                } else {
                    Resume::Throw(request.value.clone())
                };
                coro.resume(signal);
            }
            if request.kind == 1 {
                return thread_async_generator_await_return(i, state, request.value);
            }
            let result = if request.kind == 2 {
                Err(Throw(request.value))
            } else {
                Ok(create_iter_result(i, JsValue::Undefined, true))
            };
            thread_async_generator_finish(i, &state, result)?;
            continue;
        }
        state.borrow_mut().started = true;
        let signal = match request.kind {
            0 => Resume::Next(request.value),
            1 => Resume::Return(request.value),
            _ => Resume::Throw(request.value),
        };
        return thread_async_generator_step(i, state, signal);
    }
}

fn thread_async_generator_finish(
    i: &mut Interpreter,
    state: &Rc<RefCell<ThreadGeneratorState>>,
    result: Result<JsValue, Throw>,
) -> Result<(), Throw> {
    let target = {
        let mut guard = state.borrow_mut();
        guard.async_executing = false;
        guard.async_awaiting = false;
        guard.async_await_return = false;
        guard
            .async_request
            .take()
            .ok_or_else(|| Throw::type_error("missing async generator request"))?
    };
    let fate = match result {
        Ok(result) => PromiseFate::Fulfilled(result),
        Err(error) => PromiseFate::Rejected(i.normalize_throw(error).0),
    };
    settle_promise(&target, fate, i)
}

fn thread_async_generator_step(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    signal: Resume,
) -> Result<(), Throw> {
    let caller_realm = i.binding_current_realm();
    let realm = state.borrow().body_realm;
    i.binding_with_realm(realm, |i| {
        thread_async_generator_step_in_realm(i, state, signal, caller_realm)
    })
}

fn thread_async_generator_step_in_realm(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    signal: Resume,
    caller_realm: RealmHandle,
) -> Result<(), Throw> {
    let coro = {
        let mut guard = state.borrow_mut();
        guard.async_executing = true;
        guard.async_awaiting = false;
        guard.coro.take()
    };
    let Some(mut coro) = coro else {
        thread_async_generator_finish(i, &state, Err(Throw::type_error(UNSUPPORTED_MSG)))?;
        return thread_async_generator_drain(i, state);
    };
    let suspend = coro.resume(signal);
    {
        let mut guard = state.borrow_mut();
        guard.async_executing = false;
        if !coro.done() {
            guard.coro = Some(coro);
        }
    }
    match suspend {
        Suspend::Await(promise) => {
            state.borrow_mut().async_awaiting = true;
            thread_async_generator_attach(i, state, promise)
        }
        Suspend::Yield(value) => {
            // Retain the existing yield policy. This ticket adds Return Await,
            // rather than silently broadening legacy async delegation/yield.
            let result =
                i.binding_with_realm(caller_realm, |i| Ok(create_iter_result(i, value, false)))?;
            thread_async_generator_finish(i, &state, Ok(result))?;
            thread_async_generator_drain(i, state)
        }
        Suspend::YieldIteratorResult(result) => {
            thread_async_generator_finish(i, &state, Ok(result))?;
            thread_async_generator_drain(i, state)
        }
        Suspend::Done(value) => {
            // ReturnStatement already performed its Await inside the body.
            thread_async_generator_finish(i, &state, Ok(create_iter_result(i, value, true)))?;
            thread_async_generator_drain(i, state)
        }
        Suspend::Throw(error) => {
            thread_async_generator_finish(i, &state, Err(Throw(error)))?;
            thread_async_generator_drain(i, state)
        }
    }
}

fn thread_async_generator_await_return(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    value: JsValue,
) -> Result<(), Throw> {
    state.borrow_mut().async_await_return = true;
    match vm::async_resolve_value_checked(i, value) {
        Ok(promise) => {
            state.borrow_mut().async_awaiting = true;
            thread_async_generator_attach(i, state, promise)
        }
        Err(error) => {
            thread_async_generator_finish(i, &state, Err(error))?;
            thread_async_generator_drain(i, state)
        }
    }
}

fn thread_async_generator_attach(
    i: &mut Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    promise: JsValue,
) -> Result<(), Throw> {
    let fulfilled = thread_async_generator_callback(i, state.clone(), false);
    let rejected = thread_async_generator_callback(i, state.clone(), true);
    let result = promise_object(&promise).and_then(|promise| {
        perform_promise_then(i, &promise, Some(fulfilled), Some(rejected), None)
    });
    if let Err(error) = result {
        thread_async_generator_finish(i, &state, Err(error))?;
        thread_async_generator_drain(i, state)?;
    }
    Ok(())
}

fn thread_async_generator_callback(
    i: &Interpreter,
    state: Rc<RefCell<ThreadGeneratorState>>,
    reject: bool,
) -> JsValue {
    let env = EnvHandle::new(EnvKind::Function, Some(i.global_env.clone()));
    let mut holder = JsObject::ordinary();
    holder.exotic = Exotic::ThreadGenerator(state);
    let _ = env.declare_var("[[ThreadAsyncState]]", JsValue::object(holder));
    let mut function = JsObject::function(JsFunction {
        source: Default::default(),
        name: if reject {
            "asyncGeneratorReturnRejected"
        } else {
            "asyncGeneratorReturnFulfilled"
        }
        .into(),
        params: vec![],
        body: FunctionBody::Native(if reject {
            thread_async_generator_rejected
        } else {
            thread_async_generator_fulfilled
        }),
        is_arrow: false,
        is_async: false,
        is_generator: false,
        env,
        home: None,
        proto_for_new: None,
    });
    function.realm = Some(i.host_realm);
    function.proto = Some(i.function_proto.clone());
    JsValue::object(function)
}

fn thread_async_generator_continue(
    i: &mut Interpreter,
    value: JsValue,
    reject: bool,
) -> Result<JsValue, Throw> {
    let state = thread_async_callback_state(i)?;
    let outside_body = state.borrow().async_await_return;
    if outside_body {
        let result = if reject {
            Err(Throw(value))
        } else {
            Ok(create_iter_result(i, value, true))
        };
        thread_async_generator_finish(i, &state, result)?;
        thread_async_generator_drain(i, state)?;
    } else {
        let signal = if reject {
            Resume::Throw(value)
        } else {
            Resume::Next(value)
        };
        thread_async_generator_step(i, state, signal)?;
    }
    Ok(JsValue::Undefined)
}

fn thread_async_generator_fulfilled(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    thread_async_generator_continue(
        i,
        args.into_iter().next().unwrap_or(JsValue::Undefined),
        false,
    )
}
fn thread_async_generator_rejected(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    thread_async_generator_continue(
        i,
        args.into_iter().next().unwrap_or(JsValue::Undefined),
        true,
    )
}
