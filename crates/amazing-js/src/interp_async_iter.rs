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

    fn close_async_iterator(&mut self, iterator: &JsValue) -> Result<(), Throw> {
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
            Some(iterator)
        } else {
            self.custom_iterator(iterable)?
        };
        let Some(iterator) = iterator else {
            // Arrays and strings use the built-in sync iterator path.
            for value in self.iterate(iterable)? {
                let value = self.await_iteration_value(value)?;
                let iteration_env = for_iteration_env(left, env);
                self.bind_for(left, value, &iteration_env)?;
                match self.exec_stmt(body, &iteration_env)? {
                    Flow::Normal | Flow::Continue(None) => {}
                    Flow::Break(None) => break,
                    flow => return Ok(flow),
                }
            }
            return Ok(Flow::Normal);
        };
        let next = self.get_prop(&iterator, "next")?;
        if callable_function(&next).is_none() {
            return Err(Throw::type_error("async iterator next is not callable"));
        }
        for _ in 0..100_000 {
            let result = self.call_value(&next, iterator.clone(), Vec::new())?;
            let result = if async_iterator {
                self.await_iteration_value(result)?
            } else { result };
            if !result.is_object() {
                return Err(Throw::type_error("async iterator result is not an object"));
            }
            if self.get_prop(&result, "done")?.to_boolean() { return Ok(Flow::Normal); }
            let value = self.get_prop(&result, "value")?;
            let value = self.await_iteration_value(value)?;
            let iteration_env = for_iteration_env(left, env);
            if let Err(error) = self.bind_for(left, value, &iteration_env) {
                let _ = self.close_async_iterator(&iterator);
                return Err(error);
            }
            match self.exec_stmt(body, &iteration_env) {
                Ok(Flow::Normal | Flow::Continue(None)) => {}
                Ok(Flow::Break(None)) => {
                    self.close_async_iterator(&iterator)?;
                    return Ok(Flow::Normal);
                }
                Ok(flow) => {
                    self.close_async_iterator(&iterator)?;
                    return Ok(flow);
                }
                Err(error) => {
                    let _ = self.close_async_iterator(&iterator);
                    return Err(error);
                }
            }
        }
        let _ = self.close_async_iterator(&iterator);
        Err(Throw::range_error("async iteration exceeded the limit"))
    }
}
