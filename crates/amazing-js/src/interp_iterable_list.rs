// Portions ported/adapted from boa-dev/boa (core/engine/src/builtins/iterable/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// GetIteratorFromMethod and IteratorStepValue use Amazing's existing genuine
// StdIteratorRecord/direct/step operations; eager consumers share one protocol.

impl Interpreter {
    fn iterate(&mut self, value: &JsValue) -> Result<Vec<JsValue>, Throw> {
        let method = self.get_prop(value, WellKnownSymbol::Iterator.symbol())?;
        self.iterate_from_method(value, &method)
    }

    // A caller that has already performed GetMethod must use the captured
    // method, rather than observing an authored getter a second time.
    fn iterate_from_method(&mut self, value: &JsValue, method: &JsValue) -> Result<Vec<JsValue>, Throw> {
        if callable_function(method).is_none() {
            return Err(Throw::type_error("value is not iterable"));
        }
        let iterator = self.call_value(method, value.clone(), Vec::new())?;
        let record = self.std_iterator_direct(iterator)?;
        let mut values = Vec::new();
        // Keep the existing finite eager-collection resource policy. The
        // observable iterator protocol itself has no Array/String shortcuts.
        for _ in 0..100_000usize {
            match std_iterator_step(self, &record)? {
                Some(value) => values.push(value),
                None => return Ok(values),
            }
        }
        // This is our resource-limit completion, not a failed IteratorStep.
        // Preserve the existing original-error precedence on resource close.
        let _ = std_iterator_close(self, &record);
        Err(Throw::range_error("iterator exceeded the iteration limit"))
    }
}
