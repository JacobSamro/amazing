// Await/Yield/delegation portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, yield_one/yield_delegate_async/async_gen_yield_resume)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// The actual IteratorRecord/protocol algorithm is shared with vm/async_delegate.rs;
// the adapter retains it on the strongly owned coroutine stack across suspension.
// Driver continuations already retain the generator, capability and body realm.

fn coroutine_async_generator_await(i: &mut Interpreter, value: JsValue) -> Result<JsValue, Throw> {
    match coroutine_await(i, value.clone()) {
        Some(result) => result,
        None => i.await_value(value),
    }
}

fn coroutine_async_yield_delegate(
    i: &mut Interpreter,
    iterable: JsValue,
) -> Result<JsValue, Throw> {
    let mut state = vm::start_async_yield_delegate(i, &iterable)?;
    let (mut kind, mut input) = (0, JsValue::Undefined);
    loop {
        match vm::step_async_yield_delegate(i, &mut state, kind, input)? {
            vm::AsyncYieldDelegateStep::Await(value) => {
                match coroutine_async_generator_await(i, value) {
                    Ok(value) => {
                        kind = 0;
                        input = value;
                    }
                    Err(Throw(error)) => {
                        kind = 2;
                        input = error;
                    }
                }
            }
            vm::AsyncYieldDelegateStep::Yield(value) => {
                // Yield* calls AsyncGeneratorYield directly: genuine async
                // iterator values remain raw, unlike ordinary Yield operands.
                match park_segment(i, Suspend::Yield(value)) {
                    Resume::Next(value) => {
                        kind = 0;
                        input = value;
                    }
                    Resume::Throw(error) => {
                        kind = 2;
                        input = error;
                    }
                    Resume::Return(value) => {
                        // UnwrapYieldResumption precedes delegate GetMethod.
                        // A rejected checked normalization becomes Throw here.
                        match coroutine_return_value(i, value) {
                            Ok(value) => {
                                kind = 1;
                                input = value;
                            }
                            Err(Throw(error)) => {
                                kind = 2;
                                input = error;
                            }
                        }
                    }
                }
            }
            vm::AsyncYieldDelegateStep::Complete(value) => return Ok(value),
            vm::AsyncYieldDelegateStep::Return(value) => {
                // Delegate ReturnCompletion is already fully formed. Do not
                // apply ReturnStatement Await or a blanket Completed Await.
                set_pending_return(value);
                return Err(Throw(JsValue::Undefined));
            }
        }
    }
}
