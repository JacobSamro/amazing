// Portions ported from boa-dev/boa (core/engine/src/builtins/typed_array/builtin.rs)
// @ ef40c63283e1bb73f371d93372e9e95217a36945, MIT.
// Whole species/create, map, slice and change-by-copy algorithms adapted to
// Amazing's real BinaryView/Obj state; immutable access modes follow TC39's draft.

#[derive(Clone, Copy, PartialEq, Eq)]
enum BinaryTypedAccess {
    Read,
    Write,
}

fn binary_validate_typed_result(
    value: &JsValue,
    access: BinaryTypedAccess,
    minimum: Option<usize>,
    source_kind: Option<BinaryKind>,
) -> Result<(BinaryView, BinaryKind), Throw> {
    let (view, kind) = binary_typed(value)?;
    if access == BinaryTypedAccess::Write && view.buffer.borrow().immutable {
        return Err(Throw::type_error("immutable TypedArray destination"));
    }
    if binary_view_out_of_bounds(&view) {
        return Err(Throw::type_error("out-of-bounds TypedArray result"));
    }
    if minimum.is_some_and(|length| binary_length(&view) < length) {
        return Err(Throw::type_error(
            "TypedArray constructor returned a short array",
        ));
    }
    if source_kind.is_some_and(|source| source.big() != kind.big()) {
        return Err(Throw::type_error(
            "incompatible TypedArray species content type",
        ));
    }
    Ok((view, kind))
}

fn binary_construct_typed_object(
    i: &mut Interpreter,
    constructor: &JsValue,
    length: usize,
) -> Result<(JsValue, BinaryView, BinaryKind), Throw> {
    let result = i.construct(constructor, vec![JsValue::num(length as f64)])?;
    let (view, kind) =
        binary_validate_typed_result(&result, BinaryTypedAccess::Write, Some(length), None)?;
    Ok((result, view, kind))
}

fn binary_typed_species(
    i: &mut Interpreter,
    source: &JsValue,
    kind: BinaryKind,
) -> Result<JsValue, Throw> {
    let default = i
        .intrinsic_constructors
        .get(kind.name())
        .map(|(constructor, _)| JsValue::Object(constructor.clone()))
        .ok_or_else(|| Throw::type_error("missing TypedArray intrinsic"))?;
    let constructor = i.get_prop(source, "constructor")?;
    if constructor.is_undefined() {
        return Ok(default);
    }
    if !constructor.is_object() {
        return Err(Throw::type_error("TypedArray constructor is not an object"));
    }
    let species = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
    Ok(if species.is_null() || species.is_undefined() {
        default
    } else {
        species
    })
}

fn binary_typed_species_create(
    i: &mut Interpreter,
    source: &JsValue,
    kind: BinaryKind,
    arguments: Vec<JsValue>,
    access: BinaryTypedAccess,
) -> Result<(JsValue, BinaryView, BinaryKind), Throw> {
    let minimum = if arguments.len() == 1 {
        match arguments[0] {
            JsValue::Num(number) => Some(number as usize),
            _ => None,
        }
    } else {
        None
    };
    let constructor = binary_typed_species(i, source, kind)?;
    let result = i.construct(&constructor, arguments)?;
    let (view, result_kind) = binary_validate_typed_result(&result, access, minimum, Some(kind))?;
    Ok((result, view, result_kind))
}

fn binary_typed_species_from_values(
    i: &mut Interpreter,
    source: &JsValue,
    kind: BinaryKind,
    values: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let (result, view, result_kind) = binary_typed_species_create(
        i,
        source,
        kind,
        vec![JsValue::num(values.len() as f64)],
        BinaryTypedAccess::Write,
    )?;
    for (index, value) in values.iter().enumerate() {
        binary_set_element(i, &view, result_kind, Some(index), value)?;
    }
    Ok(result)
}

fn binary_typed_intrinsic_length(
    i: &mut Interpreter,
    kind: BinaryKind,
    length: usize,
) -> Result<(JsValue, BinaryView), Throw> {
    let prototype = i
        .intrinsic_constructors
        .get(kind.name())
        .map(|(_, prototype)| prototype.clone())
        .ok_or_else(|| Throw::type_error("missing TypedArray intrinsic"))?;
    // An initialized native realm owns both real intrinsic prototypes. Never
    // silently substitute Object.prototype for a missing backing-buffer realm.
    if !i.intrinsic_constructors.contains_key("ArrayBuffer") {
        return Err(Throw::type_error("missing ArrayBuffer intrinsic"));
    }
    let bytes = length
        .checked_mul(kind.size())
        .ok_or_else(|| Throw::range_error("TypedArray too large"))?;
    if bytes > 256 * 1024 * 1024 {
        return Err(Throw::range_error("TypedArray too large"));
    }
    let mut raw = JsObject::ordinary();
    raw.proto = Some(prototype);
    let object = Obj(Rc::new(RefCell::new(raw)));
    let buffer = i.binary_new_buffer(vec![0; bytes], bytes, false, false);
    let result = i.binary_view_on(object, binary_obj(&buffer)?, 0, Some(bytes), Some(kind))?;
    let view = binary_typed(&result)?.0;
    Ok((result, view))
}

fn binary_typed_with(
    i: &mut Interpreter,
    source: &JsValue,
    view: &BinaryView,
    kind: BinaryKind,
    length: usize,
    arguments: &[JsValue],
) -> Result<JsValue, Throw> {
    let raw = i.to_number(arguments.first().unwrap_or(&JsValue::Undefined))?;
    let relative = if raw.is_nan() { 0.0 } else { raw.trunc() };
    let index = if relative < 0.0 {
        length as f64 + relative
    } else {
        relative
    };
    let replacement = arguments.get(1).unwrap_or(&JsValue::Undefined);
    let replacement = if kind.big() {
        Interpreter::bigint_value(i.to_bigint(replacement)?)
    } else {
        JsValue::num(i.to_number(replacement)?)
    };
    if !index.is_finite() || index < 0.0 || index >= binary_length(view) as f64 {
        return Err(Throw::range_error("index outside TypedArray"));
    }
    let (result, target) = binary_typed_intrinsic_length(i, kind, length)?;
    let source = binary_obj(source)?;
    // The original length governs allocation/copy even when coercion grew the
    // source enough to make a formerly invalid index valid beyond that length.
    for k in 0..length {
        let value = if k == index as usize {
            replacement.clone()
        } else {
            binary_get_index(i, &source, &PropertyKey::from(k.to_string())).unwrap()?
        };
        binary_set_element(i, &target, kind, Some(k), &value)?;
    }
    Ok(result)
}

fn binary_typed_map(
    i: &mut Interpreter,
    source: &JsValue,
    _view: &BinaryView,
    kind: BinaryKind,
    length: usize,
    callback: JsValue,
    receiver: JsValue,
) -> Result<JsValue, Throw> {
    let (result, target, target_kind) = binary_typed_species_create(
        i,
        source,
        kind,
        vec![JsValue::num(length as f64)],
        BinaryTypedAccess::Write,
    )?;
    let object = binary_obj(source)?;
    for index in 0..length {
        let value = binary_get_index(i, &object, &PropertyKey::from(index.to_string())).unwrap()?;
        let mapped = i.call_value(
            &callback,
            receiver.clone(),
            vec![value, JsValue::num(index as f64), source.clone()],
        )?;
        binary_set_element(i, &target, target_kind, Some(index), &mapped)?;
    }
    Ok(result)
}

fn binary_typed_slice(
    i: &mut Interpreter,
    source: &JsValue,
    view: &BinaryView,
    kind: BinaryKind,
    length: usize,
    arguments: &[JsValue],
) -> Result<JsValue, Throw> {
    let start = if arguments.first().is_some_and(|value| !value.is_undefined()) {
        binary_relative(i, arguments.first(), length as i64)?
    } else {
        0
    };
    let end = binary_relative(i, arguments.get(1), length as i64)?;
    let requested = end.saturating_sub(start);
    let (result, target, target_kind) = binary_typed_species_create(
        i,
        source,
        kind,
        vec![JsValue::num(requested as f64)],
        BinaryTypedAccess::Write,
    )?;
    if requested == 0 {
        return Ok(result);
    }
    if binary_view_out_of_bounds(view) {
        return Err(Throw::type_error("view changed during slice"));
    }
    let current_end = end.min(binary_length(view));
    let count = current_end.saturating_sub(start);
    // A tracking view can still be in bounds after shrinking below start.
    // A zero-byte copy must not even form an empty slice at the old offset.
    if count == 0 {
        return Ok(result);
    }
    if kind == target_kind {
        let from = view.offset + start * kind.size();
        let to = target.offset;
        let bytes = count * kind.size();
        // Slice deliberately streams forward on an overlapping shared store;
        // unlike .set it must not snapshot or use memmove. Preserve NaN bits.
        if Rc::ptr_eq(&view.buffer, &target.buffer) {
            let mut buffer = view.buffer.borrow_mut();
            for byte in 0..bytes {
                let value = buffer.bytes[from + byte];
                buffer.bytes[to + byte] = value;
            }
        } else {
            let buffer = view.buffer.borrow();
            target.buffer.borrow_mut().bytes[to..to + bytes]
                .copy_from_slice(&buffer.bytes[from..from + bytes]);
        }
    } else {
        for (index, original) in (start..current_end).enumerate() {
            let value = binary_read(view, kind, original * kind.size(), true)?;
            binary_set_element(i, &target, target_kind, Some(index), &value)?;
        }
    }
    Ok(result)
}

impl Interpreter {
    fn install_binary_array_to_string_alias(&mut self) {
        let JsValue::Object(constructor) = self
            .global_env
            .resolve("TypedArray")
            .expect("initialized TypedArray intrinsic")
        else {
            unreachable!()
        };
        let JsValue::Object(prototype) = constructor
            .borrow()
            .get_own("prototype")
            .expect("native TypedArray prototype")
        else {
            unreachable!()
        };
        let method = self
            .array_proto
            .borrow()
            .get_own("toString")
            .expect("initialized canonical Array.toString");
        prototype
            .borrow_mut()
            .define("toString", binary_writable_property(method))
            .unwrap();
    }
}

include!("runtime_typedarray_prototype_tests.rs");
