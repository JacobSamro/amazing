// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs,
// for-of async-from-sync continuation and iterator closing)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
//! Async iteration uses the iterator protocol and the existing Promise job queue.
use super::*;

impl Interpreter {
    pub(super) fn binding_iterator_step(&mut self, iterator: &JsValue, next: &JsValue)
        -> Result<Option<JsValue>, Throw>
    {
        let result = self.call_value(next, iterator.clone(), Vec::new())?;
        if !result.is_object() {
            return Err(Throw::type_error("iterator result is not an object"));
        }
        if self.get_prop(&result, "done")?.to_boolean() {
            Ok(None)
        } else {
            Ok(Some(self.get_prop(&result, "value")?))
        }
    }

    pub(super) fn await_iteration_value(&mut self, mut value: JsValue) -> Result<JsValue, Throw> {
        // Inside a coroutine-backed async function body, suspend for real (interp_coroutine.rs).
        if let Some(result) = coroutine_await(self, value.clone()) {
            return result;
        }
        for _ in 0..1_000 {
            let JsValue::Object(object) = &value else { return Ok(value) };
            let state = match &object.borrow().exotic {
                Exotic::Promise(state) => state.state.clone(),
                _ => return Ok(value.clone()),
            };
            match state {
                PromiseFate::Fulfilled(next) => value = next,
                PromiseFate::Rejected(error) => return Err(Throw(error)),
                PromiseFate::Pending => {
                    self.drain_jobs()?;
                    if matches!(&object.borrow().exotic,
                        Exotic::Promise(PromiseState { state: PromiseFate::Pending, .. })) {
                        return Err(Throw::type_error("async iterator promise is still pending"));
                    }
                }
            }
        }
        Err(Throw::range_error("async iterator promise resolution exceeded the limit"))
    }

    fn close_async_iterator(&mut self, iterator: &JsValue, sync: bool) -> Result<(), Throw> {
        if sync {
            let wrapper = vm::async_from_sync_request(self, iterator.clone(), JsValue::Undefined, 1, None);
            let result = self.await_iteration_value(wrapper)?;
            if !result.is_object() {
                return Err(Throw::type_error("async iterator return result is not an object"));
            }
            return Ok(());
        }
        let method = self.get_prop(iterator, "return")?;
        if method.is_undefined() || method.is_null() { return Ok(()); }
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("async iterator return is not callable"));
        }
        let result = self.call_value(&method, iterator.clone(), Vec::new())?;
        let result = self.await_iteration_value(result)?;
        if !result.is_object() {
            return Err(Throw::type_error("async iterator return result is not an object"));
        }
        Ok(())
    }

    pub(super) fn exec_for_await_of(&mut self, left: &ForLeft, iterable: &JsValue,
        body: &ast::Stmt, env: &EnvHandle) -> Result<Flow, Throw>
    {
        let async_method = self.get_prop(iterable, WellKnownSymbol::AsyncIterator.symbol())?;
        let async_iterator = !async_method.is_undefined() && !async_method.is_null();
        let iterator = if async_iterator {
            if callable_function(&async_method).is_none() {
                return Err(Throw::type_error("@@asyncIterator is not callable"));
            }
            let iterator = self.call_value(&async_method, iterable.clone(), Vec::new())?;
            if !iterator.is_object() {
                return Err(Throw::type_error("async iterator is not an object"));
            }
            iterator
        } else {
            let method = self.get_prop(iterable, WellKnownSymbol::Iterator.symbol())?;
            if callable_function(&method).is_none() {
                return Err(Throw::type_error("value is not async iterable"));
            }
            let iterator = self.call_value(&method, iterable.clone(), Vec::new())?;
            if !iterator.is_object() {
                return Err(Throw::type_error("sync iterator is not an object"));
            }
            iterator
        };
        let next = self.get_prop(&iterator, "next")?;
        if async_iterator && callable_function(&next).is_none() {
            return Err(Throw::type_error("async iterator next is not callable"));
        }
        for _ in 0..100_000 {
            let result = if async_iterator {
                self.call_value(&next, iterator.clone(), Vec::new())?
            } else {
                vm::async_from_sync_request(self, iterator.clone(), next.clone(), 0, None)
            };
            let result = self.await_iteration_value(result)?;
            if !result.is_object() {
                return Err(Throw::type_error("async iterator result is not an object"));
            }
            if self.get_prop(&result, "done")?.to_boolean() { return Ok(Flow::Normal); }
            let value = self.get_prop(&result, "value")?;
            // Sync values have already passed through their wrapper reaction.
            // Genuine async iterator values remain untouched.
            let iteration_env = for_iteration_env(left, env);
            if let Err(error) = self.bind_for(left, value, &iteration_env) {
                let _ = self.close_async_iterator(&iterator, !async_iterator);
                return Err(error);
            }
            match self.exec_loop_body(body, &iteration_env) {
                Ok(Flow::Normal | Flow::Continue(None)) => {}
                Ok(Flow::Break(None)) => {
                    self.close_async_iterator(&iterator, !async_iterator)?;
                    return Ok(Flow::Normal);
                }
                Ok(flow) => {
                    self.close_async_iterator(&iterator, !async_iterator)?;
                    return Ok(flow);
                }
                Err(error) => {
                    let _ = self.close_async_iterator(&iterator, !async_iterator);
                    return Err(error);
                }
            }
        }
        let _ = self.close_async_iterator(&iterator, !async_iterator);
        Err(Throw::range_error("async iteration exceeded the limit"))
    }
}
