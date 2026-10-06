// Portions ported from boa-dev/boa (core/engine/src/builtins/array_buffer/mod.rs)
// @ ef40c63283e1bb73f371d93372e9e95217a36945, MIT.
// Copyright (c) 2019 Jason Williams; license retained in binary_resizable.LICENSE-MIT.

fn binary_resize_buffer(
    i: &mut Interpreter,
    buffer: &Rc<RefCell<BinaryBuffer>>,
    requested: &JsValue,
) -> Result<JsValue, Throw> {
    // Internal-slot/brand admission precedes author conversion. Detachment and
    // current shared length are witnessed only after that conversion returns.
    {
        let b = buffer.borrow();
        if !b.resizable || b.immutable {
            return Err(Throw::type_error("buffer is not resizable"));
        }
    }
    let length = binary_index(i, requested)?;
    let mut b = buffer.borrow_mut();
    if b.detached {
        return Err(Throw::type_error("detached ArrayBuffer"));
    }
    if length > b.max || (b.shared && length < b.bytes.len()) {
        return Err(Throw::range_error("invalid buffer size"));
    }
    let additional = length.saturating_sub(b.bytes.len());
    b.bytes
        .try_reserve(additional)
        .map_err(|_| Throw::range_error("buffer allocation failed"))?;
    b.bytes.resize(length, 0);
    Ok(JsValue::Undefined)
}

fn binary_owns_constructor_allocation(native: crate::value::NativeFn) -> bool {
    [
        binary_array_buffer_ctor as crate::value::NativeFn,
        binary_shared_buffer_ctor,
        binary_data_view_ctor,
    ]
    .iter()
    .any(|candidate| std::ptr::fn_addr_eq(native, *candidate))
}
fn binary_constructor_object(i: &mut Interpreter) -> Result<Obj, Throw> {
    let new_target = i
        .current_new_target()
        .ok_or_else(|| Throw::type_error("binary data constructor requires new"))?;
    let constructor = JsValue::Object(
        i.native_stack
            .last()
            .cloned()
            .ok_or_else(|| Throw::type_error("missing binary constructor context"))?,
    );
    let prototype = match i.get_prop(&new_target, "prototype")? {
        JsValue::Object(prototype) => prototype,
        _ => i.constructor_fallback_prototype(&constructor, &new_target)?,
    };
    let mut raw = JsObject::ordinary();
    raw.proto = Some(prototype);
    raw.constructed = true;
    Ok(Obj(Rc::new(RefCell::new(raw))))
}
fn binary_allocate_buffer(
    i: &mut Interpreter,
    length: usize,
    max: usize,
    resizable: bool,
    shared: bool,
) -> Result<JsValue, Throw> {
    let object = binary_constructor_object(i)?;
    if length > 256 * 1024 * 1024 || max > 256 * 1024 * 1024 {
        return Err(Throw::range_error(
            "buffer allocation exceeds supported capacity",
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Throw::range_error("buffer allocation failed"))?;
    bytes.resize(length, 0);
    binary_register(
        &object,
        BinaryState::Buffer(Rc::new(RefCell::new(BinaryBuffer {
            bytes,
            max,
            resizable,
            shared,
            detached: false,
            immutable: false,
        }))),
    );
    Ok(JsValue::Object(object))
}

fn binary_maximum_length(
    i: &mut Interpreter,
    options: Option<&JsValue>,
    length: usize,
) -> Result<(usize, bool), Throw> {
    let Some(options) = options.filter(|options| !options.is_undefined()) else {
        return Ok((length, false));
    };
    if !options.is_object() {
        return Ok((length, false));
    }
    let maximum = i.get_prop(options, "maxByteLength")?;
    if maximum.is_undefined() {
        return Ok((length, false));
    }
    let max = binary_index(i, &maximum)?;
    if max < length {
        return Err(Throw::range_error("invalid maxByteLength"));
    }
    Ok((max, true))
}

fn binary_copy_bytes(source: &[u8]) -> Result<Vec<u8>, Throw> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(source.len())
        .map_err(|_| Throw::range_error("buffer allocation failed"))?;
    bytes.extend_from_slice(source);
    Ok(bytes)
}

fn binary_iterator_values(
    i: &mut Interpreter,
    source: &JsValue,
    method: &JsValue,
) -> Result<Vec<JsValue>, Throw> {
    let iterator = i.call_value(method, source.clone(), Vec::new())?;
    let record = i.std_iterator_direct(iterator)?;
    let mut values = Vec::new();
    // Preserve the existing eager bridge's limit for general iterators. Its
    // ordinary Array intrinsic path historically had no such iteration cap.
    let ordinary_array = matches!(source, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Array))
        && matches!((method, &i.original_array_iterator), (JsValue::Object(a), JsValue::Object(b)) if Rc::ptr_eq(&a.0, &b.0));
    loop {
        let Some(value) = std_iterator_step(i, &record)? else {
            return Ok(values);
        };
        if !ordinary_array && values.len() == 100_000 {
            return Err(Throw::range_error("iterator source too large"));
        }
        values
            .try_reserve(1)
            .map_err(|_| Throw::range_error("iterator list allocation failed"))?;
        values.push(value);
    }
}
fn binary_source_method(i: &mut Interpreter, source: &JsValue) -> Result<JsValue, Throw> {
    let method = i.get_prop(source, WellKnownSymbol::Iterator.symbol())?;
    if !method.is_undefined() && !method.is_null() && callable_function(&method).is_none() {
        return Err(Throw::type_error("source @@iterator is not callable"));
    }
    Ok(method)
}
fn binary_array_like_length(i: &mut Interpreter, source: &JsValue) -> Result<usize, Throw> {
    let length = array_search_length(i, source)?;
    if length > 1_000_000 {
        return Err(Throw::range_error("array-like source too large"));
    }
    Ok(length)
}
