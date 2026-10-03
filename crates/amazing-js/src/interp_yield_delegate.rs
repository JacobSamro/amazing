// Synchronous delegated-yield protocol portions adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, yield_delegate/GetIterator) @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.

/// Strongly retained iterator record shared by VM and coroutine sync delegation.
/// No object borrow is held while calling user code; dropping the record releases both values.
pub(crate) struct SyncYieldDelegate {
    iterator: JsValue,
    next: JsValue,
}

pub(crate) enum YieldDelegateStep {
    /// GeneratorYield forwards this original result without reading its value.
    YieldResult(JsValue),
    Complete(JsValue),
    Return(JsValue),
}

impl Interpreter {
    pub(crate) fn start_sync_yield_delegate(
        &mut self,
        iterable: &JsValue,
    ) -> Result<SyncYieldDelegate, Throw> {
        let method = self
            .yield_delegate_method(iterable, WellKnownSymbol::Iterator.symbol())?
            .ok_or_else(|| Throw::type_error("yield* value is not iterable"))?;
        let iterator = self.call_value(&method, iterable.clone(), Vec::new())?;
        if !iterator.is_object() {
            return Err(Throw::type_error("iterator is not an object"));
        }
        let next = self.get_prop(&iterator, "next")?;
        Ok(SyncYieldDelegate { iterator, next })
    }

    fn yield_delegate_method(
        &mut self,
        iterator: &JsValue,
        key: impl Into<PropertyKey>,
    ) -> Result<Option<JsValue>, Throw> {
        let method = self.get_prop(iterator, key)?;
        if method.is_undefined() || method.is_null() {
            return Ok(None);
        }
        if !self.binding_is_callable(&method) {
            return Err(Throw::type_error("iterator method is not callable"));
        }
        Ok(Some(method))
    }

    pub(crate) fn step_sync_yield_delegate(
        &mut self,
        state: &SyncYieldDelegate,
        kind: u8,
        input: JsValue,
    ) -> Result<YieldDelegateStep, Throw> {
        let method = match kind {
            0 => state.next.clone(),
            1 => match self.yield_delegate_method(&state.iterator, "return")? {
                Some(method) => method,
                None => return Ok(YieldDelegateStep::Return(input)),
            },
            _ => match self.yield_delegate_method(&state.iterator, "throw")? {
                Some(method) => method,
                None => {
                    self.close_iterator(&state.iterator)?;
                    return Err(Throw::type_error(
                        "delegated iterator does not have a throw method",
                    ));
                }
            },
        };
        let result = self.call_value(&method, state.iterator.clone(), vec![input])?;
        if !result.is_object() {
            return Err(Throw::type_error("iterator result is not an object"));
        }
        if self.get_prop(&result, "done")?.to_boolean() {
            let value = self.get_prop(&result, "value")?;
            Ok(if kind == 1 {
                YieldDelegateStep::Return(value)
            } else {
                YieldDelegateStep::Complete(value)
            })
        } else {
            Ok(YieldDelegateStep::YieldResult(result))
        }
    }
}
