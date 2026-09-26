// Generic synchronous Array factories, preserving the observable iterator order.
fn array_factory_result(i: &mut Interpreter, constructor: &JsValue, length: Option<usize>) -> Result<JsValue, Throw> {
    if matches!(constructor, JsValue::Object(object) if object.borrow().func.is_some() && object.borrow().constructible) {
        return i.construct(constructor, length.map(|length| vec![JsValue::num(length as f64)]).unwrap_or_default());
    }
    let length = length.unwrap_or(0);
    if length > u32::MAX as usize { return Err(Throw::range_error("invalid array length")); }
    Ok(array_result(i, length))
}

fn native_array_from(i: &mut Interpreter, constructor: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let source = args.first().cloned().unwrap_or(JsValue::Undefined);
    let mapper = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    let receiver = args.get(2).cloned().unwrap_or(JsValue::Undefined);
    if !mapper.is_undefined() && callable_function(&mapper).is_none() {
        return Err(Throw::type_error("Array.from mapper is not callable"));
    }
    if source.is_null() || source.is_undefined() { return Err(Throw::type_error("Array.from requires an object")); }
    let method = i.get_prop(&source, WellKnownSymbol::Iterator.symbol())?;
    if !method.is_null() && !method.is_undefined() {
        if callable_function(&method).is_none() { return Err(Throw::type_error("Array.from iterator is not callable")); }
        let result = array_factory_result(i, &constructor, None)?;
        let iterator = i.call_value(&method, source, Vec::new())?;
        if !iterator.is_object() { return Err(Throw::type_error("iterator is not an object")); }
        let next = i.get_prop(&iterator, "next")?;
        let record = StdIteratorRecord { iterator, next };
        let mut index = 0usize;
        loop {
            if index >= 9_007_199_254_740_991 {
                return Err(std_iterator_abrupt_close(i, &record, Throw::type_error("Array.from length exceeds maximum safe integer")));
            }
            // Retain the existing eager collector's resource bound while
            // advancing lazily; resource-limit exits still close the iterator.
            if index >= 100_000 {
                return Err(std_iterator_abrupt_close(i, &record, Throw::range_error("iterator exceeded the iteration limit")));
            }
            let Some(value) = std_iterator_step(i, &record)? else {
                array_set(i, &result, "length", JsValue::num(index as f64))?;
                return Ok(result);
            };
            let added = (|| {
                let mapped = if mapper.is_undefined() { value } else {
                    i.call_value(&mapper, receiver.clone(), vec![value, JsValue::num(index as f64)])?
                };
                array_create_data_property(i, &result, index, mapped)
            })();
            if let Err(error) = added { return Err(std_iterator_abrupt_close(i, &record, error)); }
            index += 1;
        }
    }
    let object = array_receiver(i, source)?;
    let length = array_search_length(i, &object)?;
    let result = array_factory_result(i, &constructor, Some(length))?;
    if length > 1_000_000 { return Err(Throw::range_error("Array.from source too large")); }
    for index in 0..length {
        let value = i.get_prop(&object, index.to_string())?;
        let mapped = if mapper.is_undefined() { value } else {
            i.call_value(&mapper, receiver.clone(), vec![value, JsValue::num(index as f64)])?
        };
        array_create_data_property(i, &result, index, mapped)?;
    }
    array_set(i, &result, "length", JsValue::num(length as f64))?;
    Ok(result)
}

fn native_array_of(i: &mut Interpreter, constructor: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let length = args.len();
    let result = array_factory_result(i, &constructor, Some(length))?;
    for (index, value) in args.into_iter().enumerate() { array_create_data_property(i, &result, index, value)?; }
    array_set(i, &result, "length", JsValue::num(length as f64))?;
    Ok(result)
}

#[cfg(test)]
mod array_factory_tests {
    use super::*;

    #[test]
    fn custom_constructors_receive_lengths_and_mapping_reads_live_input() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var lengths = [], source = {0: 'old', 1: 'old', length: 2};
            function Custom(length) { lengths.push(arguments.length + ':' + length); }
            var result = Array.from.call(Custom, source, function(value, index) {
                source[1] = 'new'; return value;
            });
            var iterable = Array.from.call(Custom, [1]);
            var empty = Array.of.call(Custom);
            var two = Array.of.call(Custom, 'a', 'b');
            result instanceof Custom && result[0] === 'old' && result[1] === 'new' &&
            result.length === 2 && iterable.length === 1 && empty.length === 0 &&
            two instanceof Custom && two[1] === 'b' && two.length === 2 &&
            lengths.join(',') === '1:2,0:undefined,1:0,1:2'
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn mapper_failure_closes_iterator_but_next_failure_does_not() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var closed = 0, reads = 0, error = {}, caught = false;
            var source = {get [Symbol.iterator]() { reads++; return function() {
                return {next() { return {value: 1, done: false}; }, return() { closed++; throw 2; }};
            }; }};
            try { Array.from(source, function() { throw error; }); } catch(e) { caught = e === error; }
            var second = {[Symbol.iterator]() {
                return {next() { throw error; }, return() { closed++; return {}; }};
            }};
            try { Array.from(second); } catch(e) { caught = caught && e === error; }
            caught && closed === 1 && reads === 1
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn factories_define_data_and_require_length_set() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var calls = 0, length = -1;
            var prototype = {set 0(value) { calls++; }};
            function Custom() {
                var result = Object.create(prototype);
                Object.defineProperty(result, 'length', {set(value) { length = value; }});
                return result;
            }
            var result = Array.of.call(Custom, 4);
            var sentinel = {}, thrown = false;
            function Locked() { return Object.preventExtensions({}); }
            var source = {[Symbol.iterator]() {
                return {next() { return {value: 1, done: false}; }, return() { thrown = true; return {}; }};
            }};
            try { Array.from.call(Locked, source); } catch(e) { sentinel = e; }
            Object.hasOwn(result, '0') && result[0] === 4 && calls === 0 && length === 1 &&
            thrown && sentinel instanceof TypeError
        "#).unwrap(), JsValue::Bool(true));
    }
}
