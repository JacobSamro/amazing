//! `yield*` inside an async generator (ECMA-262 15.5.5 YieldExpression evaluation with
//! generatorKind async, 7.4.3 GetIterator(obj, async), 27.1.6 CreateAsyncFromSyncIterator).
//!
//! The sync `YieldDelegate` instruction drives the inner iterator synchronously. The async form
//! must `Await` every inner result (and, for a sync-iterator fallback, every inner *value*), so
//! it is a small state machine: each step either yields an inner value to the consumer, suspends
//! the frame on an `Await`, or finishes the `yield*` (normally, or with a return/throw). The
//! `YieldDelegateAsync` instruction re-executes after every suspension and feeds this module the
//! resumption it received: the consumer's `next`/`throw`/`return` after a yield, or the awaited
//! promise's settlement (0 = fulfilled, 2 = rejected) after an await.
use super::{callable_function, generator_result, Interpreter};
use crate::value::{JsValue, Throw, WellKnownSymbol};

/// Where an async delegation is suspended.
pub(super) enum AsyncDelegatePhase {
    /// Not started, or suspended at a yield: the next input is the consumer's resumption.
    Received,
    /// Awaiting the inner iterator's `next`/`throw`/`return` result (`kind` is which one).
    AwaitingResult { kind: u8 },
    /// Sync-iterator fallback: awaiting the inner result's `value` (CreateAsyncFromSyncIterator's
    /// continuation), with the already-read `done` flag.
    AwaitingSyncValue { kind: u8, done: bool },
    /// A `return` resumption reached a done inner result: awaiting its value before returning.
    AwaitingReturnValue,
    /// A `throw` resumption found no inner `throw` method: awaiting the inner `return()` result
    /// (AsyncIteratorClose) before throwing the protocol TypeError.
    AwaitingClose,
}

pub(super) struct AsyncDelegate {
    iterator: JsValue,
    next: JsValue,
    /// The inner iterator came from `@@iterator` (wrapped per CreateAsyncFromSyncIterator).
    sync: bool,
    pub(super) phase: AsyncDelegatePhase,
}

/// What the `YieldDelegateAsync` instruction does next.
pub(super) enum AsyncDelegateStep {
    /// Yield this inner value to the consumer (AsyncGeneratorYield; no further await).
    Yield(JsValue),
    /// Suspend the frame until this value settles.
    Await(JsValue),
    /// The `yield*` expression completes normally with this value.
    Complete(JsValue),
    /// The enclosing generator returns with this value (a `return` resumption).
    Return(JsValue),
}

/// GetMethod (7.3.11): `undefined`/`null` is absent, anything else must be callable.
fn get_method(i: &mut Interpreter, value: &JsValue, key: impl Into<crate::value::PropertyKey>)
    -> Result<Option<JsValue>, Throw>
{
    let method = i.get_prop(value, key)?;
    if method.is_undefined() || method.is_null() {
        return Ok(None);
    }
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("iterator method is not callable"));
    }
    Ok(Some(method))
}

/// GetIterator(value, async): `@@asyncIterator`, else a sync iterator from `@@iterator`.
pub(super) fn start(i: &mut Interpreter, iterable: &JsValue) -> Result<AsyncDelegate, Throw> {
    let (method, sync) = match get_method(i, iterable, WellKnownSymbol::AsyncIterator.symbol())? {
        Some(method) => (method, false),
        None => match get_method(i, iterable, WellKnownSymbol::Iterator.symbol())? {
            Some(method) => (method, true),
            None => return Err(Throw::type_error("yield* value is not async iterable")),
        },
    };
    let iterator = i.call_value(&method, iterable.clone(), Vec::new())?;
    if !iterator.is_object() {
        return Err(Throw::type_error("iterator is not an object"));
    }
    let next = i.get_prop(&iterator, "next")?;
    Ok(AsyncDelegate { iterator, next, sync, phase: AsyncDelegatePhase::Received })
}

/// Best-effort IteratorClose of a sync inner iterator (its own completion is discarded, since
/// the caller is already completing abruptly or with the protocol TypeError).
fn close_sync(i: &mut Interpreter, iterator: &JsValue) {
    if let Ok(Some(method)) = get_method(i, iterator, "return") {
        let _ = i.call_value(&method, iterator.clone(), Vec::new());
    }
}

/// Advance the delegation with `(kind, input)`: the consumer's resumption (0 next, 1 return,
/// 2 throw) in `Received`, or an awaited settlement (0 fulfilled, 2 rejected) otherwise. An
/// `Err` completes the `yield*` with a throw; the caller then drops the delegation state.
pub(super) fn step(i: &mut Interpreter, state: &mut AsyncDelegate, kind: u8, input: JsValue)
    -> Result<AsyncDelegateStep, Throw>
{
    match std::mem::replace(&mut state.phase, AsyncDelegatePhase::Received) {
        AsyncDelegatePhase::Received => call_inner(i, state, kind, input),
        AsyncDelegatePhase::AwaitingResult { kind: request } => {
            if kind == 2 {
                return Err(Throw(input));
            }
            if !input.is_object() {
                return Err(Throw::type_error("iterator result is not an object"));
            }
            let done = i.get_prop(&input, "done")?.to_boolean();
            inner_result(i, state, request, done, &input)
        }
        AsyncDelegatePhase::AwaitingSyncValue { kind: request, done } => {
            if kind == 2 {
                // CreateAsyncFromSyncIteratorContinuation's closeOnRejection: only `next`/`throw`
                // results close the sync iterator when their value rejects; `return` does not.
                if !done && request != 1 {
                    close_sync(i, &state.iterator);
                }
                return Err(Throw(input));
            }
            let result = generator_result(i, input, done);
            inner_result(i, state, request, done, &result)
        }
        AsyncDelegatePhase::AwaitingReturnValue => {
            if kind == 2 { Err(Throw(input)) } else { Ok(AsyncDelegateStep::Return(input)) }
        }
        AsyncDelegatePhase::AwaitingClose => {
            if kind == 2 {
                return Err(Throw(input));
            }
            if !input.is_object() {
                return Err(Throw::type_error("iterator return result is not an object"));
            }
            Err(Throw::type_error("delegated iterator does not have a throw method"))
        }
    }
}

/// Call the inner `next`/`throw`/`return` for a consumer resumption and await its result.
fn call_inner(i: &mut Interpreter, state: &mut AsyncDelegate, kind: u8, input: JsValue)
    -> Result<AsyncDelegateStep, Throw>
{
    let iterator = state.iterator.clone();
    let method = match kind {
        0 => state.next.clone(),
        1 => match get_method(i, &iterator, "return")? {
            Some(method) => method,
            // The resumption value was already awaited (AsyncGeneratorUnwrapYieldResumption).
            None => return Ok(AsyncDelegateStep::Return(input)),
        },
        _ => match get_method(i, &iterator, "throw")? {
            Some(method) => method,
            None => {
                if state.sync {
                    close_sync(i, &iterator);
                    return Err(Throw::type_error("delegated iterator does not have a throw method"));
                }
                // AsyncIteratorClose(iteratorRecord, NormalCompletion) then the TypeError.
                let Some(close) = get_method(i, &iterator, "return")? else {
                    return Err(Throw::type_error("delegated iterator does not have a throw method"));
                };
                let result = i.call_value(&close, iterator, Vec::new())?;
                state.phase = AsyncDelegatePhase::AwaitingClose;
                return Ok(AsyncDelegateStep::Await(result));
            }
        },
    };
    let result = i.call_value(&method, iterator, vec![input])?;
    if state.sync {
        if !result.is_object() {
            return Err(Throw::type_error("iterator result is not an object"));
        }
        let done = i.get_prop(&result, "done")?.to_boolean();
        let value = i.get_prop(&result, "value")?;
        state.phase = AsyncDelegatePhase::AwaitingSyncValue { kind, done };
        return Ok(AsyncDelegateStep::Await(value));
    }
    state.phase = AsyncDelegatePhase::AwaitingResult { kind };
    Ok(AsyncDelegateStep::Await(result))
}

/// Act on an (awaited) inner iterator result for the resumption `kind` that produced it.
fn inner_result(i: &mut Interpreter, state: &mut AsyncDelegate, kind: u8, done: bool, result: &JsValue)
    -> Result<AsyncDelegateStep, Throw>
{
    let value = i.get_prop(result, "value")?;
    if !done {
        state.phase = AsyncDelegatePhase::Received;
        return Ok(AsyncDelegateStep::Yield(value));
    }
    if kind == 1 {
        state.phase = AsyncDelegatePhase::AwaitingReturnValue;
        return Ok(AsyncDelegateStep::Await(value));
    }
    Ok(AsyncDelegateStep::Complete(value))
}
