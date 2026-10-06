// Portions ported from boa-dev/boa (core/engine/src/builtins/typed_array/object.rs)
// @ ef40c63283e1bb73f371d93372e9e95217a36945, MIT.
// Adapted to Amazing's binary side tables and partial descriptors. Immutable
// backing-store rules follow tc39/proposal-immutable-arraybuffer §TypedArrays.

fn binary_canonical_key(key: &PropertyKey) -> Option<Option<usize>> {
    let PropertyKey::String(text) = key else {
        return None;
    };
    // Every canonical number spelling is ASCII. Preserve arbitrary UTF16 names
    // as ordinary keys rather than projecting them through a replacement scalar.
    if text.units().iter().any(|unit| *unit > 0x7f) {
        return None;
    }
    let text: String = text
        .units()
        .iter()
        .map(|unit| *unit as u8 as char)
        .collect();
    if text == "-0" {
        return Some(None);
    }
    let number = crate::value::string_to_number(&text);
    if text != number_to_string(number) {
        return None;
    }
    Some(
        if number.is_finite()
            && number >= 0.0
            && number.fract() == 0.0
            && number < usize::MAX as f64
        {
            Some(number as usize)
        } else {
            None
        },
    )
}

fn binary_index_view(
    object: &Obj,
    key: &PropertyKey,
) -> Option<(BinaryView, BinaryKind, Option<usize>)> {
    let Some(BinaryState::View(view)) = binary_state(object) else {
        return None;
    };
    let kind = view.kind?;
    Some((view, kind, binary_canonical_key(key)?))
}

fn binary_get_index(
    _i: &mut Interpreter,
    object: &Obj,
    key: &PropertyKey,
) -> Option<Result<JsValue, Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some(Ok(
        match index.filter(|index| *index < binary_length(&view)) {
            Some(index) => {
                binary_read(&view, kind, index * kind.size(), true).unwrap_or(JsValue::Undefined)
            }
            None => JsValue::Undefined,
        },
    ))
}

// TypedArraySetElement converts before IsValidIntegerIndex. A conversion can
// detach or resize the buffer: revalidate afterward, then write without another
// callback. DataView's throwing writer deliberately remains a separate operation.
fn binary_set_element(
    i: &mut Interpreter,
    view: &BinaryView,
    kind: BinaryKind,
    index: Option<usize>,
    value: &JsValue,
) -> Result<(), Throw> {
    let bytes = if kind.big() {
        i.to_bigint(value)?.low_u64().to_le_bytes().to_vec()
    } else {
        binary_encode_number(i.to_number(value)?, kind, true)
    };
    if let Some(index) = index.filter(|index| *index < binary_length(view)) {
        let start = view.offset + index * kind.size();
        view.buffer.borrow_mut().bytes[start..start + kind.size()].copy_from_slice(&bytes);
    }
    Ok(())
}

fn binary_set_index(
    i: &mut Interpreter,
    object: &Obj,
    key: &PropertyKey,
    value: JsValue,
) -> Option<Result<bool, Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    let immutable = view.buffer.borrow().immutable;
    Some(if immutable {
        Ok(false)
    } else {
        binary_set_element(i, &view, kind, index, &value).map(|()| true)
    })
}

fn binary_index_descriptor(object: &Obj, key: &PropertyKey) -> Option<Option<Property>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    let immutable = view.buffer.borrow().immutable;
    Some(
        index
            .filter(|index| *index < binary_length(&view))
            .map(|index| Property {
                accessor: false,
                value: binary_read(&view, kind, index * kind.size(), true)
                    .unwrap_or(JsValue::Undefined),
                writable: !immutable,
                enumerable: true,
                configurable: !immutable,
                getter: None,
                setter: None,
            }),
    )
}

fn binary_delete_index(object: &Obj, key: &PropertyKey) -> Option<bool> {
    binary_index_descriptor(object, key).map(|property| property.is_none())
}

fn binary_define_index_fields(
    i: &mut Interpreter,
    object: &Obj,
    key: &PropertyKey,
    fields: &StdDescriptorFields,
) -> Option<Result<bool, Throw>> {
    let (view, kind, index) = binary_index_view(object, key)?;
    Some((|| {
        let Some(index) = index.filter(|index| *index < binary_length(&view)) else {
            return Ok(false);
        };
        if fields.get.is_some() || fields.set.is_some() || fields.enumerable == Some(false) {
            return Ok(false);
        }
        if view.buffer.borrow().immutable {
            if fields.configurable == Some(true) || fields.writable == Some(true) {
                return Ok(false);
            }
            let current = binary_read(&view, kind, index * kind.size(), true)?;
            return Ok(fields
                .value
                .as_ref()
                .is_none_or(|value| std_same_value(value, &current)));
        }
        if fields.configurable == Some(false) || fields.writable == Some(false) {
            return Ok(false);
        }
        if let Some(value) = &fields.value {
            binary_set_element(i, &view, kind, Some(index), value)?;
        }
        Ok(true)
    })())
}

fn binary_define_index_object(
    i: &mut Interpreter,
    object: &Obj,
    key: &PropertyKey,
    descriptor: &Obj,
) -> Option<Result<bool, Throw>> {
    binary_index_view(object, key)?;
    Some((|| {
        let fields = i.std_descriptor_fields(&JsValue::Object(descriptor.clone()))?;
        binary_define_index_fields(i, object, key, &fields).unwrap()
    })())
}

fn binary_own_keys(object: &Obj) -> Option<Vec<PropertyKey>> {
    let Some(BinaryState::View(view)) = binary_state(object) else {
        return None;
    };
    view.kind?;
    let mut keys: Vec<_> = (0..binary_length(&view))
        .map(|index| PropertyKey::from(index.to_string()))
        .collect();
    keys.extend(
        object
            .borrow()
            .own_property_keys()
            .into_iter()
            .filter(|key| binary_canonical_key(key).is_none()),
    );
    Some(keys)
}

include!("builtins_binary_exotic_tests.rs");
