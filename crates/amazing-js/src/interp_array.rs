// Generic Array.prototype operations. Each method reads through the receiver's
// ordinary property path so inherited indices and host hooks remain visible.

fn array_receiver(i: &Interpreter, this: JsValue) -> Result<JsValue, Throw> {
    match this {
        JsValue::Null | JsValue::Undefined => Err(Throw::type_error(
            "Array method called on null or undefined",
        )),
        JsValue::Object(_) => Ok(this),
        JsValue::Str(text) => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.string_proto.clone());
            for (index, unit) in text.units().iter().enumerate() {
                object
                    .define(
                        index.to_string(),
                        Property {
                            value: JsValue::str_units(vec![*unit]),
                            writable: false,
                            enumerable: true,
                            configurable: false,
                            getter: None,
                            setter: None,
                        },
                    )
                    .unwrap();
            }
            object
                .define(
                    "length",
                    Property {
                        value: JsValue::num(text.len() as f64),
                        writable: false,
                        enumerable: false,
                        configurable: false,
                        getter: None,
                        setter: None,
                    },
                )
                .unwrap();
            Ok(JsValue::object(object))
        }
        _ => {
            let mut object = JsObject::ordinary();
            object.proto = Some(i.object_proto.clone());
            Ok(JsValue::object(object))
        }
    }
}

fn array_like_length(i: &mut Interpreter, object: &JsValue) -> Result<usize, Throw> {
    let length = array_search_length(i, object)?;
    if length > 1_000_000 {
        return Err(Throw::range_error(
            "array operation exceeds supported length",
        ));
    }
    Ok(length)
}

// Searches can start close to the end of a very large sparse array-like object.
// Their scan bound is applied after fromIndex has selected the range to visit.
fn array_search_length(i: &mut Interpreter, object: &JsValue) -> Result<usize, Throw> {
    if let JsValue::Object(handle) = object {
        if let Some(JsValue::Str(text)) = handle.borrow().get_own("[[StringData]]") {
            return Ok(text.len());
        }
    }
    let value = i.get_prop(object, "length")?;
    let number = i.to_number(&value)?;
    let length = if number.is_nan() || number <= 0.0 {
        0.0
    } else {
        number.trunc().min(9_007_199_254_740_991.0)
    };
    Ok(length as usize)
}

fn array_index(
    i: &mut Interpreter,
    argument: Option<&JsValue>,
    length: usize,
    default: f64,
) -> Result<usize, Throw> {
    let number = match argument {
        Some(value) => i.to_number(value)?,
        None => default,
    };
    let number = if number.is_nan() { 0.0 } else { number.trunc() };
    Ok(if number < 0.0 {
        (length as f64 + number).max(0.0) as usize
    } else {
        number.min(length as f64) as usize
    })
}

fn array_has(i: &mut Interpreter, object: &JsValue, index: usize) -> Result<bool, Throw> {
    let JsValue::Object(handle) = object else {
        return Ok(false);
    };
    if let Some(JsValue::Str(text)) = handle.borrow().get_own("[[StringData]]") {
        if index < text.len() { return Ok(true); }
    }
    if let Some(descriptor) = binary_index_descriptor(handle, &PropertyKey::from(index.to_string())) {
        return Ok(descriptor.is_some());
    }
    i.legacy_has_property(handle, index.to_string())
}

fn array_get_index(i: &mut Interpreter, object: &JsValue, index: usize) -> Result<JsValue, Throw> {
    if let JsValue::Object(handle) = object {
        if let Some(JsValue::Str(text)) = handle.borrow().get_own("[[StringData]]") {
            if let Some(unit) = text.units().get(index) {
                return Ok(JsValue::str_units(vec![*unit]));
            }
        }
    }
    i.get_prop(object, index.to_string())
}

fn array_delete(i: &mut Interpreter, object: &JsValue, index: usize) -> Result<(), Throw> {
    let JsValue::Object(handle) = object else {
        return Err(Throw::type_error("array receiver is not an object"));
    };
    let key = index.to_string();
    let deleted = i
        .legacy_delete(handle, &key)?
        .unwrap_or_else(|| handle.borrow_mut().delete_own(key));
    if !deleted {
        return Err(Throw::type_error("cannot delete array element"));
    }
    Ok(())
}

fn array_result(i: &Interpreter, length: usize) -> JsValue {
    let mut object = JsObject::array();
    object.proto = Some(i.array_proto.clone());
    let mut property = object.own_prop("length").cloned().unwrap();
    property.value = JsValue::num(length as f64);
    object.define("length", property).unwrap();
    JsValue::object(object)
}

fn array_index_key(key: &PropertyKey) -> Option<u32> {
    let name = key.as_string()?.to_utf8_lossy();
    let index = name.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == name).then_some(index)
}

fn array_prepare_length_change(object: &Obj, new_length: u32) -> bool {
    let old_length = match object.borrow().get_own("length") {
        Some(JsValue::Num(length)) => length as u32,
        _ => 0,
    };
    if new_length >= old_length { return true; }
    let mut indices: Vec<_> = object.borrow().own_property_keys().into_iter()
        .filter_map(|key| array_index_key(&key).filter(|index| *index >= new_length).map(|index| (index, key)))
        .collect();
    indices.sort_by(|a, b| b.0.cmp(&a.0));
    for (index, key) in indices {
        if !object.borrow_mut().delete_own(&key) {
            let mut length = object.borrow().own_prop("length").cloned().unwrap();
            length.value = JsValue::num(index as f64 + 1.0);
            object.borrow_mut().define("length", length).unwrap();
            return false;
        }
    }
    true
}

fn array_species_create(i: &mut Interpreter, source: &JsValue, length: usize) -> Result<JsValue, Throw> {
    if !matches!(source, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Array)) {
        return Ok(array_result(i, length));
    }
    let constructor = i.get_prop(source, "constructor")?;
    if constructor.is_undefined() { return Ok(array_result(i, length)); }
    let JsValue::Object(_) = constructor else {
        return Err(Throw::type_error("array constructor is not an object"));
    };
    let species = i.get_prop(&constructor, WellKnownSymbol::Species.symbol())?;
    if species.is_null() || species.is_undefined() { return Ok(array_result(i, length)); }
    i.construct(&species, vec![JsValue::num(length as f64)])
}

fn array_create_data_properties(i: &mut Interpreter, target: &JsValue, values: Vec<JsValue>) -> Result<(), Throw> {
    let JsValue::Object(object) = target else {
        return Err(Throw::type_error("array species constructor did not return an object"));
    };
    for (index, value) in values.into_iter().enumerate() {
        let mut descriptor = JsObject::ordinary();
        descriptor.set_own("value", value);
        descriptor.set_own("writable", JsValue::Bool(true));
        descriptor.set_own("enumerable", JsValue::Bool(true));
        descriptor.set_own("configurable", JsValue::Bool(true));
        if !i.define_own_property(object, &PropertyKey::from(index.to_string()), &JsValue::object(descriptor))? {
            return Err(Throw::type_error("cannot create array species element"));
        }
    }
    Ok(())
}

fn array_compare(
    i: &mut Interpreter,
    left: &JsValue,
    right: &JsValue,
    comparator: &JsValue,
) -> Result<std::cmp::Ordering, Throw> {
    use std::cmp::Ordering;
    if left.is_undefined() {
        return Ok(if right.is_undefined() {
            Ordering::Equal
        } else {
            Ordering::Greater
        });
    }
    if right.is_undefined() {
        return Ok(Ordering::Less);
    }
    if !comparator.is_undefined() {
        let result = i.call_value(
            comparator,
            JsValue::Undefined,
            vec![left.clone(), right.clone()],
        )?;
        let number = i.to_number(&result)?;
        return Ok(if number < 0.0 {
            Ordering::Less
        } else if number > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        });
    }
    let left = i.to_js_string(left)?;
    let right = i.to_js_string(right)?;
    Ok(left.units().cmp(right.units()))
}

fn array_merge_sort(
    i: &mut Interpreter,
    values: Vec<JsValue>,
    comparator: &JsValue,
) -> Result<Vec<JsValue>, Throw> {
    if values.len() < 2 {
        return Ok(values);
    }
    let mut left = values;
    let right = left.split_off(left.len() / 2);
    let left = array_merge_sort(i, left, comparator)?;
    let right = array_merge_sort(i, right, comparator)?;
    let mut merged = Vec::with_capacity(left.len() + right.len());
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        if array_compare(i, &left[a], &right[b], comparator)? != std::cmp::Ordering::Greater {
            merged.push(left[a].clone());
            a += 1;
        } else {
            merged.push(right[b].clone());
            b += 1;
        }
    }
    merged.extend_from_slice(&left[a..]);
    merged.extend_from_slice(&right[b..]);
    Ok(merged)
}

fn array_sorted(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
    copy: bool,
) -> Result<JsValue, Throw> {
    let comparator = args.first().cloned().unwrap_or(JsValue::Undefined);
    if !comparator.is_undefined() && callable_function(&comparator).is_none() {
        return Err(Throw::type_error("sort comparator is not callable"));
    }
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let mut values = Vec::with_capacity(length);
    for index in 0..length {
        if copy || array_has(i, &object, index)? {
            values.push(i.get_prop(&object, index.to_string())?);
        }
    }
    let values = array_merge_sort(i, values, &comparator)?;
    if copy {
        let result = array_result(i, length);
        for (index, value) in values.into_iter().enumerate() {
            i.set_prop(&result, index.to_string(), value)?;
        }
        Ok(result)
    } else {
        let count = values.len();
        for (index, value) in values.into_iter().enumerate() {
            i.set_prop(&object, index.to_string(), value)?;
        }
        for index in count..length {
            array_delete(i, &object, index)?;
        }
        Ok(object)
    }
}
fn native_array_sort(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_sorted(i, this, args, false)
}
fn native_array_to_sorted(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_sorted(i, this, args, true)
}
fn native_array_species(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(this)
}

fn array_await_ready(value: JsValue) -> Result<JsValue, Throw> {
    if let JsValue::Object(object) = &value {
        if let Exotic::Promise(state) = &object.borrow().exotic {
            return match &state.state {
                PromiseFate::Fulfilled(value) => Ok(value.clone()),
                PromiseFate::Rejected(value) => Err(Throw(value.clone())),
                PromiseFate::Pending => Err(Throw::type_error("pending async input is not yet supported")),
            };
        }
    }
    Ok(value)
}

fn native_array_from_async(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let result = (|| -> Result<JsValue, Throw> {
        let source = array_receiver(i, args.first().cloned().unwrap_or(JsValue::Undefined))?;
        let async_iterator = i.get_prop(&source, WellKnownSymbol::AsyncIterator.symbol())?;
        if !async_iterator.is_undefined() && !async_iterator.is_null() {
            return Err(Throw::type_error("async iterator is not yet supported"));
        }
        let mapper = args.get(1).cloned().unwrap_or(JsValue::Undefined);
        if !mapper.is_undefined() && callable_function(&mapper).is_none() {
            return Err(Throw::type_error("Array.fromAsync mapFn is not callable"));
        }
        let iterator = i.get_prop(&source, WellKnownSymbol::Iterator.symbol())?;
        let items = if iterator.is_undefined() || iterator.is_null() {
            let length = array_like_length(i, &source)?;
            (0..length).map(|index| i.get_prop(&source, index.to_string())).collect::<Result<Vec<_>, _>>()?
        } else {
            i.iterate(&source)?
        };
        let mut values = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let item = array_await_ready(item)?;
            let value = if mapper.is_undefined() { item } else {
                i.call_value(&mapper, args.get(2).cloned().unwrap_or(JsValue::Undefined),
                    vec![item, JsValue::num(index as f64)])?
            };
            values.push(array_await_ready(value)?);
        }
        array_values(i, values)
    })();
    Ok(new_promise(i, match result {
        Ok(value) => PromiseFate::Fulfilled(value),
        Err(Throw(value)) => PromiseFate::Rejected(value),
    }))
}

fn native_array_at(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)? as f64;
    let position = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let position = if position.is_nan() {
        0.0
    } else {
        position.trunc()
    };
    let index = if position < 0.0 {
        length + position
    } else {
        position
    };
    if index < 0.0 || index >= length {
        Ok(JsValue::Undefined)
    } else {
        i.get_prop(&object, (index as usize).to_string())
    }
}

fn native_array_copy_within(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let target = array_index(i, args.first(), length, 0.0)?;
    let start = array_index(i, args.get(1), length, 0.0)?;
    let end = array_index(i, args.get(2), length, length as f64)?;
    let count = end.saturating_sub(start).min(length.saturating_sub(target));
    if start < target && target < start + count {
        for offset in (0..count).rev() {
            let source = start + offset;
            let destination = target + offset;
            if array_has(i, &object, source)? {
                let value = i.get_prop(&object, source.to_string())?;
                i.set_prop(&object, destination.to_string(), value)?;
            } else {
                array_delete(i, &object, destination)?;
            }
        }
    } else {
        for offset in 0..count {
            let source = start + offset;
            let destination = target + offset;
            if array_has(i, &object, source)? {
                let value = i.get_prop(&object, source.to_string())?;
                i.set_prop(&object, destination.to_string(), value)?;
            } else {
                array_delete(i, &object, destination)?;
            }
        }
    }
    Ok(object)
}

fn native_array_fill(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let start = array_index(i, args.get(1), length, 0.0)?;
    let end = array_index(i, args.get(2), length, length as f64)?;
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    for index in start..end {
        i.set_prop(&object, index.to_string(), value.clone())?;
    }
    Ok(object)
}

fn array_predicate(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
    reverse: bool,
    all: bool,
    return_index: bool,
    skip_holes: bool,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("array callback is not callable"));
    }
    let receiver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    for step in 0..length {
        let index = if reverse { length - step - 1 } else { step };
        if skip_holes && !array_has(i, &object, index)? {
            continue;
        }
        let value = i.get_prop(&object, index.to_string())?;
        let selected = i
            .call_value(
                &callback,
                receiver.clone(),
                vec![value.clone(), JsValue::num(index as f64), object.clone()],
            )?
            .to_boolean();
        if all && !selected {
            return Ok(JsValue::Bool(false));
        }
        if !all && selected {
            return Ok(if return_index {
                JsValue::num(index as f64)
            } else {
                value
            });
        }
    }
    Ok(if all {
        JsValue::Bool(true)
    } else if return_index {
        JsValue::num(-1.0)
    } else {
        JsValue::Undefined
    })
}
fn native_array_every(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_predicate(i, this, args, false, true, false, true)
}
fn native_array_find(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_predicate(i, this, args, false, false, false, false)
}
fn native_array_find_index(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_predicate(i, this, args, false, false, true, false)
}
fn native_array_find_last(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_predicate(i, this, args, true, false, false, false)
}
fn native_array_find_last_index(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    array_predicate(i, this, args, true, false, true, false)
}

fn native_array_last_index_of(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    if length == 0 {
        return Ok(JsValue::num(-1.0));
    }
    let search = args.first().cloned().unwrap_or(JsValue::Undefined);
    let start = if let Some(value) = args.get(1) {
        let number = i.to_number(value)?;
        if number.is_nan() {
            0
        } else if number < 0.0 {
            (length as f64 + number.trunc()).max(-1.0) as isize
        } else {
            number.trunc().min((length - 1) as f64) as isize
        }
    } else {
        (length - 1) as isize
    };
    for (visited, index) in (0..=start).rev().enumerate() {
        if visited == 1_000_000 {
            return Err(Throw::range_error("array search exceeds iteration limit"));
        }
        let index = index as usize;
        if array_has(i, &object, index)? && array_get_index(i, &object, index)? == search {
            return Ok(JsValue::num(index as f64));
        }
    }
    Ok(JsValue::num(-1.0))
}

fn native_array_reduce_right(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("array reducer is not callable"));
    }
    let mut accumulator = args.get(1).cloned();
    for step in 0..length {
        let index = length - step - 1;
        if !array_has(i, &object, index)? {
            continue;
        }
        let value = i.get_prop(&object, index.to_string())?;
        accumulator = Some(if let Some(current) = accumulator {
            i.call_value(
                &callback,
                JsValue::Undefined,
                vec![current, value, JsValue::num(index as f64), object.clone()],
            )?
        } else {
            value
        });
    }
    accumulator.ok_or_else(|| Throw::type_error("reduce of empty array with no initial value"))
}

fn native_array_reverse(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    for lower in 0..length / 2 {
        let upper = length - lower - 1;
        let lower_exists = array_has(i, &object, lower)?;
        let upper_exists = array_has(i, &object, upper)?;
        let lower_value = if lower_exists {
            Some(i.get_prop(&object, lower.to_string())?)
        } else {
            None
        };
        let upper_value = if upper_exists {
            Some(i.get_prop(&object, upper.to_string())?)
        } else {
            None
        };
        match upper_value {
            Some(value) => i.set_prop(&object, lower.to_string(), value)?,
            None => array_delete(i, &object, lower)?,
        }
        match lower_value {
            Some(value) => i.set_prop(&object, upper.to_string(), value)?,
            None => array_delete(i, &object, upper)?,
        }
    }
    Ok(object)
}

fn native_array_shift(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    if length == 0 {
        i.set_prop(&object, "length", JsValue::num(0.0))?;
        return Ok(JsValue::Undefined);
    }
    let first = i.get_prop(&object, "0")?;
    for index in 1..length {
        if array_has(i, &object, index)? {
            let value = i.get_prop(&object, index.to_string())?;
            i.set_prop(&object, (index - 1).to_string(), value)?;
        } else {
            array_delete(i, &object, index - 1)?;
        }
    }
    array_delete(i, &object, length - 1)?;
    i.set_prop(&object, "length", JsValue::num((length - 1) as f64))?;
    Ok(first)
}

fn native_array_unshift(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let raw_length = i.get_prop(&object, "length")?;
    let raw_length = i.to_number(&raw_length)?;
    let length = if raw_length.is_nan() || raw_length <= 0.0 {
        0.0
    } else {
        raw_length.trunc().min(9_007_199_254_740_991.0)
    };
    let count = args.len();
    if length + count as f64 > 9_007_199_254_740_991.0 {
        return Err(Throw::type_error("unshift length exceeds maximum safe integer"));
    }
    if length > 1_000_000.0 {
        return Err(Throw::range_error("array operation exceeds supported length"));
    }
    let length = length as usize;
    let new_length = length
        .checked_add(count)
        .ok_or_else(|| Throw::range_error("array length overflow"))?;
    if new_length > 1_000_000 {
        return Err(Throw::range_error(
            "array operation exceeds supported length",
        ));
    }
    for index in (0..length).rev() {
        if array_has(i, &object, index)? {
            let value = i.get_prop(&object, index.to_string())?;
            i.set_prop(&object, (index + count).to_string(), value)?;
        } else {
            array_delete(i, &object, index + count)?;
        }
    }
    for (index, value) in args.into_iter().enumerate() {
        i.set_prop(&object, index.to_string(), value)?;
    }
    i.set_prop(&object, "length", JsValue::num(new_length as f64))?;
    Ok(JsValue::num(new_length as f64))
}

fn native_array_to_reversed(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let result = array_result(i, length);
    for index in 0..length {
        let value = i.get_prop(&object, (length - index - 1).to_string())?;
        i.set_prop(&result, index.to_string(), value)?;
    }
    Ok(result)
}

fn native_array_to_spliced(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let start = array_index(i, args.first(), length, 0.0)?;
    let skip = if args.is_empty() {
        0
    } else if args.len() == 1 {
        length - start
    } else {
        let number = i.to_number(&args[1])?;
        if number.is_nan() || number <= 0.0 {
            0
        } else {
            (number.trunc() as usize).min(length - start)
        }
    };
    let inserted = args.len().saturating_sub(2);
    let new_length = length - skip + inserted;
    if new_length > 1_000_000 {
        return Err(Throw::range_error(
            "array operation exceeds supported length",
        ));
    }
    let result = array_result(i, new_length);
    for index in 0..start {
        let value = i.get_prop(&object, index.to_string())?;
        i.set_prop(&result, index.to_string(), value)?;
    }
    for (offset, value) in args.into_iter().skip(2).enumerate() {
        i.set_prop(&result, (start + offset).to_string(), value)?;
    }
    for index in start + skip..length {
        let value = i.get_prop(&object, index.to_string())?;
        i.set_prop(&result, (index - skip + inserted).to_string(), value)?;
    }
    Ok(result)
}

fn native_array_with(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let number = i.to_number(args.first().unwrap_or(&JsValue::Undefined))?;
    let number = if number.is_nan() { 0.0 } else { number.trunc() };
    let index = if number < 0.0 {
        length as f64 + number
    } else {
        number
    };
    if index < 0.0 || index >= length as f64 {
        return Err(Throw::range_error("array index is out of range"));
    }
    let replacement = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let result = array_result(i, length);
    for position in 0..length {
        let value = if position == index as usize {
            replacement.clone()
        } else {
            i.get_prop(&object, position.to_string())?
        };
        i.set_prop(&result, position.to_string(), value)?;
    }
    Ok(result)
}

fn native_array_to_locale_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let mut units = Vec::new();
    for index in 0..length {
        if index > 0 {
            units.push(b',' as u16);
        }
        let value = i.get_prop(&object, index.to_string())?;
        if value.is_null() || value.is_undefined() {
            continue;
        }
        if matches!(value, JsValue::Num(_)) {
            units.extend_from_slice(i.to_js_string(&value)?.units());
            continue;
        }
        let method = i.get_prop(&value, "toLocaleString")?;
        if callable_function(&method).is_none() {
            return Err(Throw::type_error("toLocaleString is not callable"));
        }
        let text = i.call_value(&method, value, Vec::new())?;
        units.extend_from_slice(i.to_js_string(&text)?.units());
    }
    Ok(JsValue::str_units(units))
}

fn array_flatten(
    i: &mut Interpreter,
    source: &JsValue,
    depth: usize,
    out: &mut Vec<JsValue>,
) -> Result<(), Throw> {
    let length = array_like_length(i, source)?;
    for index in 0..length {
        if !array_has(i, source, index)? {
            continue;
        }
        let value = i.get_prop(source, index.to_string())?;
        let nested = matches!(&value, JsValue::Object(object) if matches!(object.borrow().exotic, Exotic::Array));
        if nested && depth > 0 {
            array_flatten(i, &value, depth - 1, out)?;
        } else {
            if out.len() >= 1_000_000 {
                return Err(Throw::range_error("flattened array is too large"));
            }
            out.push(value);
        }
    }
    Ok(())
}

fn native_array_flat(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let number = match args.first() {
        Some(value) => i.to_number(value)?,
        None => 1.0,
    };
    let depth = if number.is_nan() || number <= 0.0 {
        0
    } else {
        (number.trunc() as usize).min(128)
    };
    let result = array_species_create(i, &object, 0)?;
    let mut values = Vec::new();
    array_flatten(i, &object, depth, &mut values)?;
    array_create_data_properties(i, &result, values)?;
    Ok(result)
}

fn native_array_flat_map(
    i: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("flatMap callback is not callable"));
    }
    let receiver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let result = array_species_create(i, &object, 0)?;
    let mut values = Vec::new();
    for index in 0..length {
        if !array_has(i, &object, index)? {
            continue;
        }
        let value = i.get_prop(&object, index.to_string())?;
        let mapped = i.call_value(
            &callback,
            receiver.clone(),
            vec![value, JsValue::num(index as f64), object.clone()],
        )?;
        if matches!(&mapped, JsValue::Object(handle) if matches!(handle.borrow().exotic, Exotic::Array))
        {
            array_flatten(i, &mapped, 0, &mut values)?;
        } else {
            if values.len() >= 1_000_000 {
                return Err(Throw::range_error("flattened array is too large"));
            }
            values.push(mapped);
        }
    }
    array_create_data_properties(i, &result, values)?;
    Ok(result)
}

impl Interpreter {
    fn install_array_method(
        &mut self,
        name: &str,
        native: crate::value::NativeFn,
        length: u32,
    ) -> JsValue {
        let method = symbol_method(self, name, native);
        symbol_method_length(&method, length);
        self.array_proto
            .borrow_mut()
            .define(name, symbol_data(method.clone(), true, true))
            .unwrap();
        method
    }

    fn install_array_extra(&mut self) {
        if let Some(JsValue::Object(constructor)) = self.global_env.resolve("Array") {
            let from_async = symbol_method(self, "fromAsync", native_array_from_async);
            symbol_method_length(&from_async, 1);
            constructor.borrow_mut().define("fromAsync", symbol_data(from_async, true, true)).unwrap();
            for (name, length) in [("from", 1), ("isArray", 1), ("of", 0)] {
                let existing = constructor.borrow().own_prop(name).cloned();
                if let Some(mut property) = existing {
                    property.enumerable = false;
                    if let JsValue::Object(method) = &property.value {
                        method.borrow_mut().constructible = false;
                        method.borrow_mut().delete_own("prototype");
                    }
                    symbol_method_length(&property.value, length);
                    constructor.borrow_mut().define(name, property).unwrap();
                }
            }
            let getter = symbol_method(self, "get [Symbol.species]", native_array_species);
            constructor
                .borrow_mut()
                .define(
                    WellKnownSymbol::Species.symbol(),
                    Property {
                        value: JsValue::Undefined,
                        writable: false,
                        enumerable: false,
                        configurable: true,
                        getter: Some(getter),
                        setter: None,
                    },
                )
                .unwrap();
        }
        for (name, length) in [
            ("toString", 0),
            ("push", 1),
            ("pop", 0),
            ("join", 1),
            ("map", 1),
            ("filter", 1),
            ("some", 1),
            ("forEach", 1),
            ("reduce", 1),
            ("slice", 2),
            ("splice", 2),
            ("concat", 1),
            ("includes", 1),
            ("indexOf", 1),
        ] {
            if let Some(method) = self.array_proto.borrow().get_own(name) {
                symbol_method_length(&method, length);
            }
        }
        for (name, native, length) in [
            ("at", native_array_at as crate::value::NativeFn, 1),
            ("copyWithin", native_array_copy_within, 2),
            ("every", native_array_every, 1),
            ("fill", native_array_fill, 1),
            ("find", native_array_find, 1),
            ("findIndex", native_array_find_index, 1),
            ("findLast", native_array_find_last, 1),
            ("findLastIndex", native_array_find_last_index, 1),
            ("flat", native_array_flat, 0),
            ("flatMap", native_array_flat_map, 1),
            ("lastIndexOf", native_array_last_index_of, 1),
            ("reduceRight", native_array_reduce_right, 1),
            ("reverse", native_array_reverse, 0),
            ("shift", native_array_shift, 0),
            ("sort", native_array_sort, 1),
            ("toLocaleString", native_array_to_locale_string, 0),
            ("toReversed", native_array_to_reversed, 0),
            ("toSorted", native_array_to_sorted, 1),
            ("toSpliced", native_array_to_spliced, 2),
            ("unshift", native_array_unshift, 1),
            ("with", native_array_with, 2),
        ] {
            self.install_array_method(name, native, length);
        }
        self.install_array_method("entries", native_array_iterator_entries, 0);
        self.install_array_method("keys", native_array_iterator_keys, 0);
        let values = self.install_array_method("values", native_array_iterator_values, 0);
        self.array_proto
            .borrow_mut()
            .define(
                WellKnownSymbol::Iterator.symbol(),
                symbol_data(values, true, true),
            )
            .unwrap();
        let mut unscopables = JsObject::ordinary();
        for name in [
            "at",
            "copyWithin",
            "entries",
            "fill",
            "find",
            "findIndex",
            "findLast",
            "findLastIndex",
            "flat",
            "flatMap",
            "includes",
            "keys",
            "toReversed",
            "toSorted",
            "toSpliced",
            "values",
        ] {
            unscopables
                .define(name, symbol_data(JsValue::Bool(true), true, true))
                .unwrap();
        }
        self.array_proto
            .borrow_mut()
            .define(
                WellKnownSymbol::Unscopables.symbol(),
                symbol_data(JsValue::object(unscopables), false, true),
            )
            .unwrap();
    }
}
