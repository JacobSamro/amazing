//! AddDisposableResource metadata for the resumable VM. The async hint is
//! distinct from the resolved method: a synchronous fallback still awaits an
//! undefined completion, rather than adopting the synchronous method's result.
use super::*;

pub(super) struct Resource {
    pub value: JsValue,
    pub method: JsValue,
    pub await_result: bool,
    pub use_result: bool,
}

pub(super) fn capture(i: &mut Interpreter, value: JsValue, is_async: bool) -> Result<Option<Resource>, Throw> {
    if value.is_null() || value.is_undefined() {
        return Ok(is_async.then_some(Resource { value, method: JsValue::Undefined, await_result: true, use_result: false }));
    }
    if !value.is_object() { return Err(Throw::type_error("value is not disposable")); }
    let mut method = if is_async { i.get_prop(&value, WellKnownSymbol::AsyncDispose.symbol())? } else { JsValue::Undefined };
    let use_result = !method.is_null() && !method.is_undefined();
    if !use_result { method = i.get_prop(&value, WellKnownSymbol::Dispose.symbol())?; }
    if !i.binding_is_callable(&method) { return Err(Throw::type_error("value is not disposable")); }
    Ok(Some(Resource { value, method, await_result: is_async, use_result: !is_async || use_result }))
}
