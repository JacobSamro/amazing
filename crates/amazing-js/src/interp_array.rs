// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Generic length/index/trap and FlattenIntoArray algorithms; explicit local
// traversal frames replace upstream Rust recursion without an observable depth cap.
// Generic Array.prototype operations. Each method reads through the receiver's
// ordinary property path so inherited indices and host hooks remain visible.

fn array_receiver(i: &mut Interpreter, this: JsValue) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() { return Err(Throw::type_error("Array method called on null or undefined")); }
    native_object(i, JsValue::Undefined, vec![this])
}

fn array_like_length(i: &mut Interpreter, object: &JsValue) -> Result<usize, Throw> {
    array_search_length(i, object)
}

// Searches can start close to the end of a very large sparse array-like object.
// The same normative LengthOfArrayLike bound applies to every generic operation.
fn array_search_length(i: &mut Interpreter, object: &JsValue) -> Result<usize, Throw> {
    let value = i.get_prop(object, "length")?;
    let number = i.to_number(&value)?;
    let length = if number.is_nan() || number <= 0.0 { 0.0 }
        else { number.trunc().min(9_007_199_254_740_991.0) };
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
    let JsValue::Object(handle) = object else { return Err(Throw::type_error("array receiver is not an object")); };
    let key = index.to_string();
    if let Some(descriptor) = binary_index_descriptor(handle, &PropertyKey::from(key.clone())) {
        return Ok(descriptor.is_some());
    }
    i.legacy_has_property(handle, key)
}

fn array_get_index(i: &mut Interpreter, object: &JsValue, index: usize) -> Result<JsValue, Throw> {
    i.get_prop(object, index.to_string())
}

fn array_delete(i: &mut Interpreter, object: &JsValue, index: usize) -> Result<(), Throw> {
    if !std_reflect_delete_property(i, JsValue::Undefined,
        vec![object.clone(), JsValue::str(index.to_string())])?.to_boolean() {
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

fn array_create_data_properties(i: &mut Interpreter, target: &JsValue, values: Vec<JsValue>) -> Result<(), Throw> {
    for (index, value) in values.into_iter().enumerate() { array_create_data_property(i, target, index, value)?; }
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
    let copied = if copy { Some(array_create(i, length)?) } else { None };
    let mut values = Vec::new();
    for index in 0..length {
        if copy || array_has(i, &object, index)? {
            values.push(i.get_prop(&object, index.to_string())?);
        }
    }
    let values = array_merge_sort(i, values, &comparator)?;
    if let Some(result) = copied {
        for (index, value) in values.into_iter().enumerate() {
            array_create_data_property(i, &result, index, value)?;
        }
        Ok(result)
    } else {
        let count = values.len();
        for (index, value) in values.into_iter().enumerate() {
            array_set(i, &object, index.to_string(), value)?;
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
    let end = array_index(i, args.get(2).filter(|value| !value.is_undefined()), length, length as f64)?;
    let count = end.saturating_sub(start).min(length.saturating_sub(target));
    if start < target && target < start + count {
        for offset in (0..count).rev() {
            let source = start + offset;
            let destination = target + offset;
            if array_has(i, &object, source)? {
                let value = i.get_prop(&object, source.to_string())?;
                array_set(i, &object, destination.to_string(), value)?;
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
                array_set(i, &object, destination.to_string(), value)?;
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
    let end = array_index(i, args.get(2).filter(|value| !value.is_undefined()), length, length as f64)?;
    let value = args.first().cloned().unwrap_or(JsValue::Undefined);
    for index in start..end {
        array_set(i, &object, index.to_string(), value.clone())?;
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
    for index in (0..=start).rev() {
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
        let lower_value = if lower_exists {
            Some(i.get_prop(&object, lower.to_string())?)
        } else {
            None
        };
        let upper_exists = array_has(i, &object, upper)?;
        let upper_value = if upper_exists {
            Some(i.get_prop(&object, upper.to_string())?)
        } else {
            None
        };
        match (lower_value, upper_value) {
            (Some(lower_value), Some(upper_value)) => {
                array_set(i, &object, lower.to_string(), upper_value)?;
                array_set(i, &object, upper.to_string(), lower_value)?;
            }
            (None, Some(upper_value)) => {
                array_set(i, &object, lower.to_string(), upper_value)?;
                array_delete(i, &object, upper)?;
            }
            (Some(lower_value), None) => {
                array_delete(i, &object, lower)?;
                array_set(i, &object, upper.to_string(), lower_value)?;
            }
            (None, None) => (),
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
        array_set(i, &object, "length", JsValue::num(0.0))?;
        return Ok(JsValue::Undefined);
    }
    let first = i.get_prop(&object, "0")?;
    for index in 1..length {
        if array_has(i, &object, index)? {
            let value = i.get_prop(&object, index.to_string())?;
            array_set(i, &object, (index - 1).to_string(), value)?;
        } else {
            array_delete(i, &object, index - 1)?;
        }
    }
    array_delete(i, &object, length - 1)?;
    array_set(i, &object, "length", JsValue::num((length - 1) as f64))?;
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
    let length = length as usize;
    let new_length = length
        .checked_add(count)
        .ok_or_else(|| Throw::range_error("array length overflow"))?;
    if count > 0 {
        for index in (0..length).rev() {
            if array_has(i, &object, index)? {
                let value = i.get_prop(&object, index.to_string())?;
                array_set(i, &object, (index + count).to_string(), value)?;
            } else {
                array_delete(i, &object, index + count)?;
            }
        }
    }
    for (index, value) in args.into_iter().enumerate() {
        array_set(i, &object, index.to_string(), value)?;
    }
    array_set(i, &object, "length", JsValue::num(new_length as f64))?;
    Ok(JsValue::num(new_length as f64))
}

fn native_array_to_reversed(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let result = array_create(i, length)?;
    for index in 0..length {
        let value = i.get_prop(&object, (length - index - 1).to_string())?;
        array_create_data_property(i, &result, index, value)?;
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
    if new_length > 9_007_199_254_740_991 { return Err(Throw::type_error("toSpliced length exceeds maximum safe integer")); }
    let result = array_create(i, new_length)?;
    for index in 0..start {
        let value = i.get_prop(&object, index.to_string())?;
        array_create_data_property(i, &result, index, value)?;
    }
    for (offset, value) in args.into_iter().skip(2).enumerate() {
        array_create_data_property(i, &result, (start + offset), value)?;
    }
    for index in start + skip..length {
        let value = i.get_prop(&object, index.to_string())?;
        array_create_data_property(i, &result, (index - skip + inserted), value)?;
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
    let result = array_create(i, length)?;
    for position in 0..length {
        let value = if position == index as usize {
            replacement.clone()
        } else {
            i.get_prop(&object, position.to_string())?
        };
        array_create_data_property(i, &result, position, value)?;
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

fn array_flatten_into(
    i: &mut Interpreter, target: &JsValue, source: JsValue, source_length: usize,
    depth: f64, mapper: Option<JsValue>, receiver: JsValue,
) -> Result<(), Throw> {
    struct Frame { source: JsValue, length: usize, index: usize, depth: f64, mapped: bool }
    let mut frames = vec![Frame { source, length: source_length, index: 0, depth, mapped: mapper.is_some() }];
    let mut target_index = 0usize;
    while let Some(frame) = frames.last_mut() {
        if frame.index >= frame.length { frames.pop(); continue; }
        let source = frame.source.clone();
        let index = frame.index;
        frame.index += 1;
        let depth = frame.depth;
        let mapped = frame.mapped;
        if !array_has(i, &source, index)? { continue; }
        let mut value = i.get_prop(&source, index.to_string())?;
        if mapped {
            value = i.call_value(mapper.as_ref().expect("root mapper"), receiver.clone(),
                vec![value, JsValue::num(index as f64), source])?;
        }
        if depth > 0.0 && is_array_value(i, &value)? {
            let length = array_like_length(i, &value)?;
            frames.push(Frame { source: value, length, index: 0,
                depth: if depth.is_infinite() { depth } else { depth - 1.0 }, mapped: false });
        } else {
            if target_index >= 9_007_199_254_740_991 { return Err(Throw::type_error("flatten target index exceeds maximum safe integer")); }
            array_create_data_property(i, target, target_index, value)?;
            target_index += 1;
        }
    }
    Ok(())
}
fn native_array_flat(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let depth = match args.first().filter(|value| !value.is_undefined()) {
        Some(value) => to_integer_or_infinity(i, value)?.max(0.0),
        None => 1.0,
    };
    let result = array_species_create(i, &object, 0)?;
    array_flatten_into(i, &result, object, length, depth, None, JsValue::Undefined)?;
    Ok(result)
}
fn native_array_flat_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_like_length(i, &object)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() { return Err(Throw::type_error("flatMap callback is not callable")); }
    let receiver = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let result = array_species_create(i, &object, 0)?;
    array_flatten_into(i, &result, object, length, 1.0, Some(callback), receiver)?;
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
                        accessor: true,
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
                symbol_data(values.clone(), true, true),
            )
            .unwrap();
        self.original_array_iterator = values;
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
                .define(name, Property::data(JsValue::Bool(true)))
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

#[cfg(test)]
mod generic_array_protocol_tests {
    use super::*;
    fn check(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}\n{source}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
    #[test]
    fn huge_generic_search_and_predicate_operations_observe_the_first_index() {
        check(r#"
          var valid=true,max=2**53-1,object={length:max,0:7};
          valid=valid && Array.prototype.includes.call(object,7) && Array.prototype.indexOf.call(object,7)===0;
          var count=0;valid=valid && !Array.prototype.every.call(object,function(v,k,o){count++;return false;}) && count===1;
          var object={length:Infinity};object[max-1]=9;
          valid=valid && Array.prototype.findLast.call(object,function(v,k){valid=valid && k===max-1;return true;})===9;
          valid=valid && Array.prototype.findLastIndex.call(object,function(){return true;})===max-1;
          valid=valid && Array.prototype.lastIndexOf.call(object,9)===max-1;
          var error={},calls=0;try{Array.prototype.reduceRight.call(object,function(a,v,k,o){calls++;valid=valid && a===1 && v===9 && k===max-1 && o===object;throw error;},1);valid=false;}catch(e){valid=valid && e===error && calls===1;}
          var object={length:2**32+1,0:3};try{Array.prototype.reduce.call(object,function(a,v,k){valid=valid && a===0 && v===3 && k===0;throw error;},0);valid=false;}catch(e){valid=valid && e===error;}
          var reads=0,from={valueOf(){reads++;throw'unused';}};valid=valid && Array.prototype.indexOf.call({length:0},1,from)===-1 && !Array.prototype.includes.call({length:0},1,from) && reads===0;valid;
        "#);
    }
    #[test]
    fn huge_sparse_mutators_reach_observable_traps_and_zero_unshift_only_sets_length() {
        check(r#"
          var valid=true,max=2**53-1,error={};
          for(var method of ['copyWithin','fill','reverse','shift','unshift','splice']){
            var log=[],p=new Proxy({length:max},{get(t,k,r){log.push('get:'+k);if(k==='length')return max;throw error;},has(t,k){log.push('has:'+k);throw error;},set(t,k,v,r){log.push('set:'+k);throw error;},deleteProperty(t,k){log.push('delete:'+k);throw error;}});
            try{
              if(method==='copyWithin')Array.prototype.copyWithin.call(p,max-1,max-2,max);
              else if(method==='fill')Array.prototype.fill.call(p,1,max-1);
              else if(method==='unshift')Array.prototype.unshift.call(p,1);
              else if(method==='splice')Array.prototype.splice.call(p,max,0);
              else Array.prototype[method].call(p);
              valid=false;
            }catch(e){valid=valid && (method==='unshift'?e instanceof TypeError:e===error);}
            if(method!=='unshift')valid=valid && log.length===2 && log[0]==='get:length';
          }
          var log=[],p=new Proxy({length:Infinity},{get(t,k){log.push('get:'+k);return t[k];},set(t,k,v){log.push('set:'+k+':'+v);return true;},has(){throw'no movement';},deleteProperty(){throw'no movement';}});
          valid=valid && Array.prototype.unshift.call(p)===max && log.join(',')==='get:length,set:length:'+max;
          var log=[],p=new Proxy({length:2**32},{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);throw error;}});
          try{Array.prototype.sort.call(p);valid=false;}catch(e){valid=valid && e===error && log.join(',')==='get:length,has:0';}valid;
        "#);
    }
    #[test]
    fn relative_indices_have_correct_undefined_defaults_and_coercion_order() {
        check(r#"
          var valid=true,a=[0,1,2,3];valid=valid && a.copyWithin(0,1,undefined)===a && a.join(',')==='1,2,3,3';
          var a=[0,0];valid=valid && a.fill(1,0,undefined)===a && a.join(',')==='1,1';
          for(var method of ['copyWithin','fill']){
            var log=[],target={length:3,0:0,1:1,2:2},p=new Proxy(target,{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);return Reflect.has(t,k);},set(t,k,v){log.push('set:'+k);t[k]=v;return true;}});
            var begin={valueOf(){log.push('begin');return -2;}},end={valueOf(){log.push('end');return -1;}};
            if(method==='copyWithin')Array.prototype.copyWithin.call(p,{valueOf(){log.push('target');return 0;}},begin,end);else Array.prototype.fill.call(p,9,begin,end);
            valid=valid && log.slice(0,method==='copyWithin'?4:3).join(',')===(method==='copyWithin'?'get:length,target,begin,end':'get:length,begin,end');
          }
          valid=valid && [1,2,3].fill(9,NaN,-Infinity).join(',')==='1,2,3' && [1,2,3].copyWithin(Infinity,0).join(',')==='1,2,3';
          var count=0,from={valueOf(){count++;return-Infinity;}},a=[1];valid=valid && a.indexOf(1,from)===0 && count===1;valid;
        "#);
    }
    #[test]
    fn reverse_orders_has_get_then_mutation_and_does_not_delete_two_holes() {
        check(r#"
          var valid=true,log=[],target={length:2,0:'a',1:'b'},p=new Proxy(target,{get(t,k,r){log.push('get:'+k);if(k==='0')delete t[1];return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);return Reflect.has(t,k);},set(t,k,v){log.push('set:'+k);t[k]=v;return true;},deleteProperty(t,k){log.push('delete:'+k);return delete t[k];}});
          valid=valid && Array.prototype.reverse.call(p)===p && log.join(',')==='get:length,has:0,get:0,has:1,delete:0,set:1' && !Object.hasOwn(target,'0') && target[1]==='a';
          var log=[],p=new Proxy({length:2},{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);return false;},deleteProperty(){throw'unused deletion';},set(){throw'unused set';}});
          Array.prototype.reverse.call(p);valid=valid && log.join(',')==='get:length,has:0,has:1';
          var log=[],target={length:2,1:'b'},p=new Proxy(target,{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);return Reflect.has(t,k);},set(t,k,v){log.push('set:'+k);t[k]=v;return true;},deleteProperty(t,k){log.push('delete:'+k);return delete t[k];}});
          Array.prototype.reverse.call(p);valid=valid && log.join(',')==='get:length,has:0,has:1,get:1,set:0,delete:1' && target[0]==='b' && !Object.hasOwn(target,'1');valid;
        "#);
    }
    #[test]
    fn concat_uses_semantic_array_brand_and_maximum_counter_before_index_reads() {
        check(r#"
          var valid=true,log=[],array=[1,2],p=new Proxy(array,{get(t,k,r){log.push(k===Symbol.isConcatSpreadable?'spread':String(k));return Reflect.get(t,k,r);},has(t,k){log.push('has:'+k);return Reflect.has(t,k);}});
          valid=valid && [].concat(p).join(',')==='1,2' && log.join(',')==='spread,length,has:0,0,has:1,1';
          var revoked=Proxy.revocable([], {get(t,k,r){if(k===Symbol.isConcatSpreadable){revoked.revoke();return undefined;}return Reflect.get(t,k,r);}});
          try{[].concat(revoked.proxy);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var error={},source={length:2**53-1,[Symbol.isConcatSpreadable]:true,get 0(){throw error;}};
          try{[1].concat(source);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var p=new Proxy(source,{has(){throw error;}});try{[].concat(p);valid=false;}catch(e){valid=valid && e===error;}
          var object={[Symbol.isConcatSpreadable]:false,length:2,get 0(){throw'unused';}};valid=valid && [].concat(object)[0]===object;valid;
        "#);
    }
    #[test]
    fn flatten_reads_root_length_once_before_depth_species_and_nested_proxy_lengths() {
        check(r#"
          var valid=true,log=[],nested=new Proxy([1,2],{get(t,k,r){log.push('nested:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('nestedHas:'+k);return Reflect.has(t,k);}}),a=[nested,3],p=new Proxy(a,{get(t,k,r){log.push('root:'+k);return Reflect.get(t,k,r);},has(t,k){log.push('rootHas:'+k);return Reflect.has(t,k);}});
          var depth={valueOf(){log.push('depth');return 1;}};var result=Array.prototype.flat.call(p,depth);
          valid=valid && result.join(',')==='1,2,3' && log.join(',')==='root:length,depth,root:constructor,rootHas:0,root:0,nested:length,nestedHas:0,nested:0,nestedHas:1,nested:1,rootHas:1,root:1';
          valid=valid && [[1],[2]].flat(undefined).join(',')==='1,2' && [[1]].flat(NaN)[0][0]===1 && [[1]].flat(-Infinity)[0][0]===1;
          var nested=1;for(var k=0;k<300;k++)nested=[nested];valid=valid && [nested].flat(Infinity).join(',')==='1' && [nested].flat(301)[0]===1;
          var revoked=Proxy.revocable([],{});revoked.revoke();valid=valid && [revoked.proxy].flat(0)[0]===revoked.proxy;
          try{[revoked.proxy].flat(1);valid=false;}catch(e){valid=valid && e instanceof TypeError;}valid;
        "#);
    }
    #[test]
    fn flatten_species_writes_are_streamed_before_later_source_and_mapper_observations() {
        check(r#"
          var valid=true;
          for(var method of ['flat','flatMap']){
            var log=[],error={},target=new Proxy({}, {defineProperty(t,k,d){log.push('define:'+k+':'+d.value);valid=valid && d.writable && d.enumerable && d.configurable;throw error;}}),a=[1,2];
            Object.defineProperty(a,1,{get(){log.push('later');throw'unused';},configurable:true});
            a.constructor={[Symbol.species]:function(n){log.push('species:'+n);return target;}};
            try{if(method==='flat')a.flat();else a.flatMap(function(v,k){log.push('map:'+k);return[v];});valid=false;}catch(e){valid=valid && e===error;}
            valid=valid && log.join(',')===(method==='flat'?'species:0,define:0:1':'species:0,map:0,define:0:1');
          }
          var a=[[1,2],[3]];a.constructor={[Symbol.species]:function(){return a;}};
          valid=valid && a.flat()===a && a.join(',')==='1,2,2';
          var input=[[1],[2]],desc=Object.getOwnPropertyDescriptor(Array.prototype,'0'),writes=0;Object.defineProperty(Array.prototype,'0',{set(){writes++;throw'inherited';},configurable:true});
          try{var result=input.flat();}finally{if(desc)Object.defineProperty(Array.prototype,'0',desc);else delete Array.prototype[0];}
          valid=valid && writes===0 && result.join(',')==='1,2' && Object.hasOwn(result,'0');valid;
        "#);
    }
    #[test]
    fn copy_array_creation_bounds_precede_reads_but_index_and_new_length_errors_keep_order() {
        check(r#"
          var valid=true,max=2**53-1;
          for(var method of ['toSorted','toReversed','toSpliced','with']){
            var log=[],p=new Proxy({length:2**32},{get(t,k,r){log.push('get:'+k);if(k!=='length')throw'unused index';return Reflect.get(t,k,r);}});
            try{if(method==='with')Array.prototype.with.call(p,0,1);else Array.prototype[method].call(p);valid=false;}catch(e){valid=valid && e instanceof RangeError && log.join(',')==='get:length';}
          }
          var log=[],p=new Proxy({length:2**32},{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);}});
          try{Array.prototype.with.call(p,{valueOf(){log.push('index');return-Infinity;}},1);valid=false;}catch(e){valid=valid && e instanceof RangeError && log.join(',')==='get:length,index';}
          var object={length:Infinity};var result=Array.prototype.toSpliced.call(object,0,max);valid=valid && result.length===0;
          try{Array.prototype.toSpliced.call(object,0,0,1);valid=false;}catch(e){valid=valid && e instanceof TypeError;}
          var order=[],error={};try{Array.prototype.toSorted.call({get length(){order.push('length');return 3;},get 0(){order.push('0');throw error;}});valid=false;}catch(e){valid=valid && e===error && order.join(',')==='length,0';}valid;
        "#);
    }
    #[test]
    fn splice_species_and_mutation_limits_precede_copy_or_shift_operations() {
        check(r#"
          var valid=true,max=2**53-1,error={},log=[],target={length:max,constructor:{[Symbol.species]:function(){throw'unused ordinary species';}}},p=new Proxy(target,{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},set(t,k,v){log.push('set:'+k+':'+v);throw error;},has(){throw'unused index';}});
          try{Array.prototype.splice.call(p,max,0);valid=false;}catch(e){valid=valid && e===error && log.join(',')==='get:length,set:length:'+max;}
          var a=[1,2],log=[],ctor=function(n){log.push('species:'+n);throw error;};a.constructor={[Symbol.species]:ctor};
          try{a.splice(0,2);valid=false;}catch(e){valid=valid && e===error && log.join(',')==='species:2' && a.join(',')==='1,2';}
          var log=[],p=new Proxy({length:max},{get(t,k,r){log.push('get:'+k);return Reflect.get(t,k,r);},has(){throw'unused';}});
          try{Array.prototype.splice.call(p,max,0,1);valid=false;}catch(e){valid=valid && e instanceof TypeError && log.join(',')==='get:length';}
          var a=[1,2,3,4];valid=valid && a.splice(-3,1.9,9,8).join(',')==='2' && a.join(',')==='1,9,8,3,4';valid;
        "#);
    }
    #[test]
    fn unscopables_fields_are_enumerable_and_proxy_date_fallback_does_not_leak_brand() {
        check(r#"
          var valid=true,u=Array.prototype[Symbol.unscopables],d=Object.getOwnPropertyDescriptor(Array.prototype,Symbol.unscopables);
          valid=valid && Object.getPrototypeOf(u)===null && !d.writable && !d.enumerable && d.configurable;
          for(var name of ['at','copyWithin','entries','fill','find','findIndex','findLast','findLastIndex','flat','flatMap','includes','keys','toReversed','toSorted','toSpliced','values']){
            var p=Object.getOwnPropertyDescriptor(u,name);valid=valid && p.value===true && p.writable && p.enumerable && p.configurable;
          }
          valid=valid && Object.keys(u).length===16;
          // Remove the legacy Date tag so this tests the fallback brand, not an inherited explicit tag.
          delete Date.prototype[Symbol.toStringTag];
          var date=new Date(0),proxy=new Proxy(date,{});date.join=undefined;
          valid=valid && Array.prototype.toString.call(date)==='[object Date]' && Array.prototype.toString.call(proxy)==='[object Object]';
          date[Symbol.toStringTag]='Custom';valid=valid && Array.prototype.toString.call(proxy)==='[object Custom]';
          valid=valid && Object.prototype.toString.call(new Proxy(function(){},{}))==='[object Function]';valid;
        "#);
    }
    #[test]
    fn foreign_array_methods_preserve_creation_range_and_species_descriptor_realms() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();let global=i.binding_create_object(None).unwrap();let realm=i.binding_create_realm(global).unwrap();
            for(name,src) in [("foreignFlat","Array.prototype.flat"),("foreignFlatMap","Array.prototype.flatMap"),("foreignSorted","Array.prototype.toSorted"),("foreignReversed","Array.prototype.toReversed"),("foreignSpliced","Array.prototype.toSpliced"),("foreignArray","Array.prototype"),("foreignObject","Object.prototype"),("foreignRange","RangeError.prototype"),("foreignType","TypeError.prototype")] {
                let v=i.binding_eval_in_realm(realm,src).unwrap();i.global_env.declare_var(name,v).unwrap();
            }
            let source=r#"
              var valid=true;
              for(var method of [foreignSorted,foreignReversed,foreignSpliced]){
                try{method.call({length:2**32});valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignRange;}
              }
              valid=valid && Object.getPrototypeOf(foreignFlat.call([[1]]))===foreignArray && Object.getPrototypeOf(foreignFlatMap.call([1],function(x){return[x];}))===foreignArray;
              var descriptorProto,source=[1];source.constructor={[Symbol.species]:function(){return new Proxy({}, {defineProperty(t,k,d){descriptorProto=Object.getPrototypeOf(d);return true;}});}};
              foreignFlat.call(source);valid=valid && descriptorProto===foreignObject;
              try{foreignFlatMap.call([],3);valid=false;}catch(e){valid=valid && Object.getPrototypeOf(e)===foreignType;}
              var error={},depth={valueOf(){throw error;}};try{foreignFlat.call([],depth);valid=false;}catch(e){valid=valid && e===error;}
              var source={length:2**53-1,0:1},count=0;try{foreignFlatMap.call(source,function(){count++;throw error;});valid=false;}catch(e){valid=valid && e===error && count===1;}valid;
            "#;
            assert_eq!(i.run_script_with_backend(source,backend).unwrap_or_else(|e|panic!("{backend:?}: {}",e.describe())),JsValue::Bool(true),"{backend:?}");
        }
    }
}
