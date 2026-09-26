// Shared ECMAScript object operations for generic Array algorithms.
fn array_set(i: &mut Interpreter, object: &JsValue, key: impl Into<PropertyKey>, value: JsValue) -> Result<(), Throw> {
    let JsValue::Object(target) = object else { return Err(Throw::type_error("array receiver is not an object")); };
    if !i.set_with_receiver(target, &key.into(), value, object.clone())? {
        return Err(Throw::type_error("cannot set array property"));
    }
    Ok(())
}

fn array_create_data_property(i: &mut Interpreter, target: &JsValue, index: usize, value: JsValue) -> Result<(), Throw> {
    let JsValue::Object(object) = target else { return Err(Throw::type_error("array result is not an object")); };
    let mut descriptor = JsObject::ordinary();
    descriptor.set_own("value", value);
    descriptor.set_own("writable", JsValue::Bool(true));
    descriptor.set_own("enumerable", JsValue::Bool(true));
    descriptor.set_own("configurable", JsValue::Bool(true));
    if !i.define_own_property(object, &PropertyKey::from(index.to_string()), &JsValue::object(descriptor))? {
        return Err(Throw::type_error("cannot create array element"));
    }
    Ok(())
}

fn array_callback_setup(i: &mut Interpreter, this: JsValue, args: &[JsValue]) -> Result<(JsValue, usize, JsValue, JsValue), Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    let callback = args.first().cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() { return Err(Throw::type_error("array callback is not callable")); }
    Ok((object, length, callback, args.get(1).cloned().unwrap_or(JsValue::Undefined)))
}

fn native_array_map(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (object, length, callback, receiver) = array_callback_setup(i, this, &args)?;
    let result = array_species_create(i, &object, length)?;
    for index in 0..length {
        if !array_has(i, &object, index)? { continue; }
        let value = i.get_prop(&object, index.to_string())?;
        let mapped = i.call_value(&callback, receiver.clone(), vec![value, JsValue::num(index as f64), object.clone()])?;
        array_create_data_property(i, &result, index, mapped)?;
    }
    Ok(result)
}

fn native_array_filter(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (object, length, callback, receiver) = array_callback_setup(i, this, &args)?;
    let result = array_species_create(i, &object, 0)?;
    let mut next = 0;
    for index in 0..length {
        if !array_has(i, &object, index)? { continue; }
        let value = i.get_prop(&object, index.to_string())?;
        if i.call_value(&callback, receiver.clone(), vec![value.clone(), JsValue::num(index as f64), object.clone()])?.to_boolean() {
            array_create_data_property(i, &result, next, value)?;
            next += 1;
        }
    }
    Ok(result)
}

fn native_array_for_each(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (object, length, callback, receiver) = array_callback_setup(i, this, &args)?;
    for index in 0..length {
        if !array_has(i, &object, index)? { continue; }
        let value = i.get_prop(&object, index.to_string())?;
        i.call_value(&callback, receiver.clone(), vec![value, JsValue::num(index as f64), object.clone()])?;
    }
    Ok(JsValue::Undefined)
}

fn native_array_some(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let (object, length, callback, receiver) = array_callback_setup(i, this, &args)?;
    for index in 0..length {
        if !array_has(i, &object, index)? { continue; }
        let value = i.get_prop(&object, index.to_string())?;
        if i.call_value(&callback, receiver.clone(), vec![value, JsValue::num(index as f64), object.clone()])?.to_boolean() {
            return Ok(JsValue::Bool(true));
        }
    }
    Ok(JsValue::Bool(false))
}

fn native_array_push(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    let new_length = length.saturating_add(args.len());
    if new_length > 9_007_199_254_740_991 { return Err(Throw::type_error("push length exceeds maximum safe integer")); }
    for (offset, value) in args.into_iter().enumerate() { array_set(i, &object, (length + offset).to_string(), value)?; }
    array_set(i, &object, "length", JsValue::num(new_length as f64))?;
    Ok(JsValue::num(new_length as f64))
}

fn native_array_pop(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    if length == 0 {
        array_set(i, &object, "length", JsValue::num(0.0))?;
        return Ok(JsValue::Undefined);
    }
    let value = i.get_prop(&object, (length - 1).to_string())?;
    array_delete(i, &object, length - 1)?;
    array_set(i, &object, "length", JsValue::num((length - 1) as f64))?;
    Ok(value)
}

fn native_array_slice(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    let start = array_index(i, args.first(), length, 0.0)?;
    let end = array_index(i, args.get(1).filter(|value| !value.is_undefined()), length, length as f64)?;
    let count = end.saturating_sub(start);
    let result = array_species_create(i, &object, count)?;
    for offset in 0..count {
        if array_has(i, &object, start + offset)? {
            let value = i.get_prop(&object, (start + offset).to_string())?;
            array_create_data_property(i, &result, offset, value)?;
        }
    }
    array_set(i, &result, "length", JsValue::num(count as f64))?;
    Ok(result)
}

fn native_array_concat(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let result = array_species_create(i, &object, 0)?;
    let mut next = 0usize;
    for item in std::iter::once(object).chain(args) {
        let spread = if let JsValue::Object(handle) = &item {
            let value = i.get_prop(&item, WellKnownSymbol::IsConcatSpreadable.symbol())?;
            if value.is_undefined() { matches!(handle.borrow().exotic, Exotic::Array) } else { value.to_boolean() }
        } else { false };
        if spread {
            let length = array_like_length(i, &item)?;
            if next.saturating_add(length) > 9_007_199_254_740_991 { return Err(Throw::type_error("concat length exceeds maximum safe integer")); }
            for index in 0..length {
                if array_has(i, &item, index)? {
                    let value = i.get_prop(&item, index.to_string())?;
                    array_create_data_property(i, &result, next, value)?;
                }
                next += 1;
            }
        } else {
            if next >= 9_007_199_254_740_991 { return Err(Throw::type_error("concat length exceeds maximum safe integer")); }
            array_create_data_property(i, &result, next, item)?;
            next += 1;
        }
    }
    array_set(i, &result, "length", JsValue::num(next as f64))?;
    Ok(result)
}

fn native_array_splice(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let object = array_receiver(i, this)?;
    let length = array_search_length(i, &object)?;
    let start = array_index(i, args.first(), length, 0.0)?;
    let delete_count = if args.is_empty() { 0 } else if args.len() == 1 { length - start } else {
        let count = i.to_number(&args[1])?;
        if count.is_nan() || count <= 0.0 { 0 } else { (count.trunc() as usize).min(length - start) }
    };
    let inserted = args.len().saturating_sub(2);
    let new_length = (length - delete_count).saturating_add(inserted);
    if new_length > 9_007_199_254_740_991 { return Err(Throw::type_error("splice length exceeds maximum safe integer")); }
    if length > 1_000_000 || new_length > 1_000_000 { return Err(Throw::range_error("splice array too large")); }
    let removed = array_species_create(i, &object, delete_count)?;
    for index in 0..delete_count {
        if array_has(i, &object, start + index)? {
            let value = i.get_prop(&object, (start + index).to_string())?;
            array_create_data_property(i, &removed, index, value)?;
        }
    }
    array_set(i, &removed, "length", JsValue::num(delete_count as f64))?;
    if inserted < delete_count {
        for index in start..length - delete_count {
            if array_has(i, &object, index + delete_count)? {
                let value = i.get_prop(&object, (index + delete_count).to_string())?;
                array_set(i, &object, (index + inserted).to_string(), value)?;
            } else { array_delete(i, &object, index + inserted)?; }
        }
        for index in (new_length..length).rev() { array_delete(i, &object, index)?; }
    } else if inserted > delete_count {
        for index in (start..length - delete_count).rev() {
            if array_has(i, &object, index + delete_count)? {
                let value = i.get_prop(&object, (index + delete_count).to_string())?;
                array_set(i, &object, (index + inserted).to_string(), value)?;
            } else { array_delete(i, &object, index + inserted)?; }
        }
    }
    for (offset, value) in args.into_iter().skip(2).enumerate() { array_set(i, &object, (start + offset).to_string(), value)?; }
    array_set(i, &object, "length", JsValue::num(new_length as f64))?;
    Ok(removed)
}

#[cfg(test)]
mod array_object_protocol_tests {
    use super::*;

    #[test]
    fn callbacks_box_receivers_once_and_observe_length_before_callback_validation() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            Number.prototype.length = 1;
            Number.prototype[0] = 'inherited';
            var result = Array.prototype.map.call(7, function(value, index, object) {
                return value === 'inherited' && index === 0 && object.valueOf() === 7 &&
                    Object.getPrototypeOf(object) === Number.prototype;
            });
            var string = Array.prototype.map.call('ab', function(value, index, object) {
                return object.valueOf() === 'ab' && Object.getPrototypeOf(object) === String.prototype;
            });
            var read = 0;
            try { Array.prototype.forEach.call({get length() { read++; return 0; }}, null); } catch(e) {}
            result[0] && string[0] && string[1] && read === 1 &&
            Array.prototype.toString.call(true) === '[object Boolean]' &&
            Array.prototype.toString.call(7) === '[object Number]' &&
            Array.prototype.toString.call('ab') === '[object String]'
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn species_results_keep_holes_and_define_data_without_inherited_setters() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var sparse = [1, , ];
            var mapped = sparse.map(x => x);
            var sliced = sparse.slice(0, undefined);
            var calls = 0, requested = -1;
            var prototype = {};
            Object.defineProperty(prototype, '0', {set: function() { calls++; }});
            sparse.constructor = {[Symbol.species]: function(length) {
                requested = length; return Object.create(prototype);
            }};
            var custom = sparse.map(x => x + 1);
            mapped.length === 2 && !(1 in mapped) && sliced.length === 2 && !(1 in sliced) &&
            custom[0] === 2 && Object.hasOwn(custom, '0') && requested === 2 && calls === 0
        "#).unwrap(), JsValue::Bool(true));
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let foreign = i.binding_eval_in_realm(child, "[1,2]").unwrap();
        i.global_env.declare_var("foreign", foreign).unwrap();
        assert_eq!(i.run_script("Object.getPrototypeOf(Array.prototype.map.call(foreign, x => x)) === Array.prototype").unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn mutations_use_throwing_set_and_keep_observable_splice_order() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var writes = [], count = 0;
            var object = {get length() { return count; }, set length(v) { count = v; writes.push(v); }};
            Array.prototype.push.call(object);
            Array.prototype.push.call(object, 'x');
            var popped = Array.prototype.pop.call(object);
            var prototype = {};
            Object.defineProperty(prototype, '0', {value: 1, writable: false});
            var locked = Object.create(prototype); locked.length = 0;
            var rejected = false;
            try { Array.prototype.push.call(locked, 2); } catch(e) { rejected = e instanceof TypeError; }
            var changing = {length: 3, get 0() { delete this[0]; this[1] = 'changed'; return 'first'; }, 1: 'old', 2: 'last'};
            var removed = Array.prototype.splice.call(changing, 0, 1);
            writes.join(',') === '0,1,0' && popped === 'x' && rejected &&
            removed[0] === 'first' && changing[0] === 'changed' && changing[1] === 'last' && changing.length === 2
        "#).unwrap(), JsValue::Bool(true));
    }
}
