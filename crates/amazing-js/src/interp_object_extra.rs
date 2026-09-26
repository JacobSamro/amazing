// The remaining Object, Function and Boolean built-ins.

fn object_argument(args: &[JsValue]) -> JsValue {
    args.first().cloned().unwrap_or(JsValue::Undefined)
}

fn object_required(args: &[JsValue]) -> Result<Obj, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(object.clone()),
        _ => Err(Throw::type_error("Object operation requires an object")),
    }
}

fn native_object_entries(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let object = object_argument(&args);
    if object.is_null() || object.is_undefined() {
        return Err(Throw::type_error("Object.entries requires an object"));
    }
    let mut entries = Vec::new();
    for key in i.own_property_keys(&object)? {
        let PropertyKey::String(key) = key else { continue; };
        if !i.object_own_enumerable(&object, &PropertyKey::String(key.clone()))? { continue; }
        let value = i.get_prop(&object, &key)?;
        entries.push(array_values(i, vec![JsValue::Str(key), value])?);
    }
    array_values(i, entries)
}

// These Object operations require an iterable, unlike Iterator.from which
// also accepts an iterator directly. Capture `next` once for the whole walk.
fn object_iterator(i: &mut Interpreter, input: JsValue) -> Result<StdIteratorRecord, Throw> {
    let method = i.get_prop(&input, WellKnownSymbol::Iterator.symbol())?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("value is not iterable"));
    }
    let iterator = i.call_value(&method, input, Vec::new())?;
    i.std_iterator_direct(iterator)
}

fn native_object_from_entries(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let iterator = object_iterator(i, object_argument(&args))?;
    let object = i.object_value();
    let JsValue::Object(result) = &object else { unreachable!() };
    // Iterator protocol failures propagate without closing. Failures while
    // processing an entry close the iterator, preserving the original throw.
    while let Some(entry) = std_iterator_step(i, &iterator)? {
        let added = (|| {
            if !entry.is_object() {
                return Err(Throw::type_error("entry is not an object"));
            }
            let key_value = i.get_prop(&entry, "0")?;
            let value = i.get_prop(&entry, "1")?;
            let key = i.to_property_key(&key_value)?;
            result.borrow_mut().define(key, Property::data(value)).unwrap();
            Ok(())
        })();
        if let Err(error) = added {
            let _ = std_iterator_close(i, &iterator);
            return Err(error);
        }
    }
    Ok(object)
}

fn native_object_is(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(std_same_value(
        args.first().unwrap_or(&JsValue::Undefined),
        args.get(1).unwrap_or(&JsValue::Undefined),
    )))
}

fn native_object_descriptors(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    if target.is_null() || target.is_undefined() {
        return Err(Throw::type_error(
            "Object.getOwnPropertyDescriptors requires an object",
        ));
    }
    let result = i.object_value();
    for key in i.own_property_keys(&target)? {
        let descriptor = native_object_get_own_property_descriptor(
            i,
            JsValue::Undefined,
            vec![
                target.clone(),
                match &key {
                    PropertyKey::String(name) => JsValue::Str(name.clone()),
                    PropertyKey::Symbol(symbol) => JsValue::Symbol(symbol.clone()),
                },
            ],
        )?;
        if !descriptor.is_undefined() {
            let JsValue::Object(result) = &result else { unreachable!() };
            result.borrow_mut().define(key, Property::data(descriptor)).unwrap();
        }
    }
    Ok(result)
}

fn native_object_define_properties(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let target = object_required(&args)?;
    let descriptors = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if descriptors.is_null() || descriptors.is_undefined() {
        return Err(Throw::type_error("property descriptors are nullish"));
    }
    // Validate all descriptors before defining any of them.
    let mut pending = Vec::new();
    for key in i.own_property_keys(&descriptors)? {
        if !i.object_own_enumerable(&descriptors, &key)? { continue; }
        let descriptor = i.get_prop(&descriptors, &key)?;
        let fields = i.std_descriptor_fields(&descriptor)?;
        // Keep the converted record, not the user object whose getters or
        // prototype can change while subsequent descriptors are collected.
        let mut snapshot = JsObject::ordinary();
        for (name, value) in [
            ("value", fields.value), ("writable", fields.writable.map(JsValue::Bool)),
            ("get", fields.get), ("set", fields.set),
            ("enumerable", fields.enumerable.map(JsValue::Bool)),
            ("configurable", fields.configurable.map(JsValue::Bool)),
        ] {
            if let Some(value) = value { snapshot.set_own(name, value); }
        }
        pending.push((key, JsValue::object(snapshot)));
    }
    for (key, descriptor) in pending {
        if !i.define_own_property(&target, &key, &descriptor)? {
            return Err(Throw::type_error("cannot define property"));
        }
    }
    Ok(JsValue::Object(target))
}

fn object_integrity(
    i: &mut Interpreter,
    args: Vec<JsValue>,
    freeze: bool,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    let JsValue::Object(object) = &target else {
        return Ok(target);
    };
    if i.is_window_proxy(object) || i.legacy_for(object).is_some() {
        return Err(Throw::type_error("cannot seal a legacy platform object"));
    }
    if freeze && matches!(object.borrow().exotic, Exotic::TypedArray) {
        let view = binary_view(&target)?;
        if binary_length(&view) != 0 || view.buffer.borrow().resizable {
            return Err(Throw::type_error("cannot freeze a TypedArray with indexed elements"));
        }
    }
    if !i.default_prevent_extensions(object)? {
        return Err(Throw::type_error("cannot prevent extensions on this object"));
    }
    for key in i.own_property_keys(&target)? {
        let existing = i.default_get_own_property(object, &key)?;
        if let Some(mut property) = existing {
            property.configurable = false;
            if freeze && property.getter.is_none() && property.setter.is_none() {
                property.writable = false;
            }
            let descriptor = object_descriptor_value(i, property);
            i.define_own_property(object, &key, &descriptor)?;
        }
    }
    Ok(target)
}
fn native_object_freeze(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity(i, args, true)
}
fn native_object_seal(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity(i, args, false)
}

fn object_integrity_test(
    i: &mut Interpreter,
    args: Vec<JsValue>,
    freeze: bool,
) -> Result<JsValue, Throw> {
    let target = object_argument(&args);
    let JsValue::Object(object) = &target else {
        return Ok(JsValue::Bool(true));
    };
    if i.is_window_proxy(object) || i.legacy_for(object).is_some() || i.default_is_extensible(object)? {
        return Ok(JsValue::Bool(false));
    }
    for key in i.own_property_keys(&target)? {
        if let Some(property) = i.default_get_own_property(object, &key)? {
            if property.configurable
                || (freeze
                    && property.getter.is_none()
                    && property.setter.is_none()
                    && property.writable)
            {
                return Ok(JsValue::Bool(false));
            }
        }
    }
    Ok(JsValue::Bool(true))
}
fn native_object_is_frozen(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity_test(i, args, true)
}
fn native_object_is_sealed(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    object_integrity_test(i, args, false)
}

fn native_object_group_by(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let items = object_argument(&args);
    let callback = args.get(1).cloned().unwrap_or(JsValue::Undefined);
    if callable_function(&callback).is_none() {
        return Err(Throw::type_error("groupBy callback is not callable"));
    }
    let iterator = object_iterator(i, items)?;
    let result = JsValue::object(JsObject::ordinary());
    let mut index = 0u64;
    loop {
        if index >= 9_007_199_254_740_991 {
            let _ = std_iterator_close(i, &iterator);
            return Err(Throw::type_error("too many groupBy items"));
        }
        let Some(value) = std_iterator_step(i, &iterator)? else { break; };
        let added = (|| {
            let group = i.call_value(
                &callback,
                JsValue::Undefined,
                vec![value.clone(), JsValue::num(index as f64)],
            )?;
            let key = i.to_property_key(&group)?;
            let current = i.get_prop(&result, &key)?;
            if current.is_undefined() {
                let group = array_values(i, vec![value])?;
                i.set_prop(&result, key, group)?;
            } else {
                let length = array_like_length(i, &current)?;
                i.set_prop(&current, length.to_string(), value)?;
            }
            Ok(())
        })();
        if let Err(error) = added {
            let _ = std_iterator_close(i, &iterator);
            return Err(error);
        }
        index += 1;
    }
    Ok(result)
}

fn native_object_is_prototype_of(
    _: &mut Interpreter,
    this: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("isPrototypeOf receiver is nullish"));
    }
    let JsValue::Object(prototype) = this else {
        return Ok(JsValue::Bool(false));
    };
    let Some(JsValue::Object(object)) = args.first() else {
        return Ok(JsValue::Bool(false));
    };
    let mut cursor = object.borrow().proto.clone();
    while let Some(current) = cursor {
        if Rc::ptr_eq(&current.0, &prototype.0) {
            return Ok(JsValue::Bool(true));
        }
        cursor = current.borrow().proto.clone();
    }
    Ok(JsValue::Bool(false))
}

fn native_object_to_locale_string(
    i: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("toLocaleString receiver is nullish"));
    }
    let method = i.get_prop(&this, "toString")?;
    if callable_function(&method).is_none() {
        return Err(Throw::type_error("toString is not callable"));
    }
    i.call_value(&method, this, Vec::new())
}

fn native_function_prototype(
    _: &mut Interpreter,
    _: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Undefined)
}

fn native_function_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = this else {
        return Err(Throw::type_error(
            "Function.toString receiver is not callable",
        ));
    };
    let function = object
        .borrow()
        .func
        .clone()
        .ok_or_else(|| Throw::type_error("Function.toString receiver is not callable"))?;
    Ok(JsValue::str(format!(
        "function {}() {{ [native code] }}",
        function.name
    )))
}

fn boolean_value(this: JsValue) -> Result<bool, Throw> {
    match this {
        JsValue::Bool(value) => Ok(value),
        JsValue::Object(object) => match object.borrow().get_own("[[BooleanData]]") {
            Some(JsValue::Bool(value)) => Ok(value),
            _ => Err(Throw::type_error(
                "Boolean method called on incompatible receiver",
            )),
        },
        _ => Err(Throw::type_error(
            "Boolean method called on incompatible receiver",
        )),
    }
}
fn native_boolean_to_string(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::str(if boolean_value(this)? {
        "true"
    } else {
        "false"
    }))
}
fn native_boolean_value_of(
    _: &mut Interpreter,
    this: JsValue,
    _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    Ok(JsValue::Bool(boolean_value(this)?))
}

impl Interpreter {
    fn install_object_extra(&mut self, constructor: &JsValue) {
        let JsValue::Object(object) = constructor else {
            return;
        };
        for (name, length) in [
            ("assign", 2),
            ("create", 2),
            ("defineProperty", 3),
            ("getOwnPropertyDescriptor", 2),
            ("getOwnPropertyNames", 1),
            ("getOwnPropertySymbols", 1),
            ("getPrototypeOf", 1),
            ("hasOwn", 2),
            ("isExtensible", 1),
            ("keys", 1),
            ("preventExtensions", 1),
            ("setPrototypeOf", 2),
            ("values", 1),
        ] {
            let existing = object.borrow().own_prop(name).cloned();
            if let Some(mut property) = existing {
                property.enumerable = false;
                if let JsValue::Object(method) = &property.value {
                    method.borrow_mut().constructible = false;
                    method.borrow_mut().delete_own("prototype");
                }
                symbol_method_length(&property.value, length);
                object.borrow_mut().define(name, property).unwrap();
            }
        }
        for (name, native, length) in [
            (
                "defineProperties",
                native_object_define_properties as crate::value::NativeFn,
                2,
            ),
            ("entries", native_object_entries, 1),
            ("freeze", native_object_freeze, 1),
            ("fromEntries", native_object_from_entries, 1),
            ("getOwnPropertyDescriptors", native_object_descriptors, 1),
            ("groupBy", native_object_group_by, 2),
            ("is", native_object_is, 2),
            ("isFrozen", native_object_is_frozen, 1),
            ("isSealed", native_object_is_sealed, 1),
            ("seal", native_object_seal, 1),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            object
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
        for (name, native, length) in [
            (
                "isPrototypeOf",
                native_object_is_prototype_of as crate::value::NativeFn,
                1,
            ),
            ("toLocaleString", native_object_to_locale_string, 0),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.object_proto
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
        let method = symbol_method(self, "toString", native_function_to_string);
        self.function_proto
            .borrow_mut()
            .define("toString", symbol_data(method, true, true))
            .unwrap();
    }

    fn install_boolean_extra(&mut self, constructor: &JsValue) {
        let JsValue::Object(object) = constructor else {
            return;
        };
        object
            .borrow_mut()
            .define(
                "prototype",
                symbol_data(JsValue::Object(self.boolean_proto.clone()), false, false),
            )
            .unwrap();
        self.boolean_proto
            .borrow_mut()
            .define("constructor", symbol_data(constructor.clone(), true, true))
            .unwrap();
        self.boolean_proto
            .borrow_mut()
            .define(
                "[[BooleanData]]",
                symbol_data(JsValue::Bool(false), false, false),
            )
            .unwrap();
        for (name, native) in [
            (
                "toString",
                native_boolean_to_string as crate::value::NativeFn,
            ),
            ("valueOf", native_boolean_value_of),
        ] {
            let method = symbol_method(self, name, native);
            self.boolean_proto
                .borrow_mut()
                .define(name, symbol_data(method, true, true))
                .unwrap();
        }
    }
}

// EnumerableOwnProperties and ObjectDefineProperties snapshot keys but inspect
// each descriptor immediately before Get, after previous getters have run.
impl Interpreter {
    fn object_own_enumerable(&mut self, value: &JsValue, key: &PropertyKey) -> Result<bool, Throw> {
        match value {
            JsValue::Object(object) => {
                Ok(self.default_get_own_property(object, key)?.is_some_and(|property| property.enumerable))
            }
            JsValue::Str(string) => Ok(array_index_key(key).is_some_and(|index| (index as usize) < string.len())),
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod object_collection_tests {
    use super::*;

    #[test]
    fn descriptor_collection_snapshots_converted_fields_once() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var reads = 0, descriptor = { get value() { reads++; return 7; } };
            var props = { a: descriptor, get b() {
                Object.defineProperty(descriptor, 'value', {value: 9});
                return {value: 2};
            }};
            var target = Object.defineProperties({}, props);
            reads === 1 && target.a === 7 && target.b === 2;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn property_collection_rechecks_enumerability_after_getters() {
        for operation in ["Object.values(source)", "Object.entries(source)", "Object.assign({}, source)"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var reads = 0, source = {{ get a() {{
                    Object.defineProperty(source, 'b', {{enumerable: false}});
                    Object.defineProperty(source, 'c', {{enumerable: true}});
                    return 1;
                }}, get b() {{ reads++; return 2; }} }};
                Object.defineProperty(source, 'c', {{value: 3, configurable: true}});
                var result = {operation};
                reads === 0 && (Array.isArray(result) ? result.length === 2 : result.c === 3 && !Object.hasOwn(result, 'b'));
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn define_properties_rechecks_keys_and_validates_before_writing() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var target = {}, source = { get a() { delete source.b; return {value: 1}; }, b: null };
            Object.defineProperties(target, source);
            var sentinel = {}, failed = false, untouched = {};
            try { Object.defineProperties(untouched, {a: {value: 1}, get b() { throw sentinel; }}); }
            catch (e) { failed = e === sentinel; }
            target.a === 1 && !Object.hasOwn(target, 'b') && failed && !Object.hasOwn(untouched, 'a');
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn primitive_descriptors_preserve_code_units_and_key_conversion() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var descriptors = Object.getOwnPropertyDescriptors('\uD800x');
            var calls = 0, key = { toString() { calls++; return 'absent'; } };
            var absent = Object.getOwnPropertyDescriptor(Symbol(), key);
            descriptors[0].value === '\uD800' && descriptors[1].value === 'x' &&
                descriptors[0].enumerable && !descriptors[0].writable &&
                !descriptors[0].configurable && descriptors.length.value === 2 &&
                !descriptors.length.enumerable && absent === undefined && calls === 1 &&
                Object.getOwnPropertyDescriptor(42, 'x') === undefined;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn descriptors_create_own_proto_property_and_create_requires_prototype() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var source = Object.create(null);
            source.__proto__ = 42;
            var descriptors = Object.getOwnPropertyDescriptors(source), failed = false;
            try { Object.create(); } catch (e) { failed = e instanceof TypeError; }
            Object.hasOwn(descriptors, '__proto__') && descriptors.__proto__.value === 42 &&
                Object.getPrototypeOf(descriptors) === Object.prototype && failed;
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_iterator_tests {
    use super::*;

    #[test]
    fn from_entries_interleaves_iteration_and_entry_conversion() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var effects = [], count = 0;
            var entries = { [Symbol.iterator]() { return this; }, next() {
                effects.push('next');
                if (count++) return {done:true};
                return {done:false, value: {
                    get 0() { effects.push('key'); return {toString() {effects.push('coerce'); return 'x';}}; },
                    get 1() { effects.push('value'); return 7; }
                }};
            }};
            var result = Object.fromEntries(entries);
            result.x === 7 && effects.join(',') === 'next,key,value,coerce,next';
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn object_iterators_close_on_consumer_errors_and_keep_original_throw() {
        for operation in ["Object.fromEntries(iterable)", "Object.groupBy(iterable, function() {throw sentinel;})"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var sentinel = {{}}, closed = 0, caught;
                var iterable = {{ [Symbol.iterator]() {{ return {{
                    next() {{ return {{done:false, value: {{get 0() {{throw sentinel;}}}}}}; }},
                    return() {{ closed++; throw 'close error'; }}
                }}; }} }};
                try {{ {operation}; }} catch (error) {{ caught = error; }}
                closed === 1 && caught === sentinel;
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn object_iterators_do_not_close_after_step_failure() {
        for operation in ["Object.fromEntries(iterable)", "Object.groupBy(iterable, function() {return 0;})"] {
            let mut i = Interpreter::new();
            let script = format!(r#"
                var sentinel = {{}}, closed = 0, caught;
                var iterable = {{ [Symbol.iterator]() {{ return {{
                    next() {{ return {{ get done() {{ throw sentinel; }} }}; }},
                    return() {{ closed++; return {{}}; }}
                }}; }} }};
                try {{ {operation}; }} catch (error) {{ caught = error; }}
                closed === 0 && caught === sentinel;
            "#);
            assert_eq!(i.run_script(&script).unwrap(), JsValue::Bool(true), "{operation}");
        }
    }

    #[test]
    fn group_by_callbacks_observe_and_extend_live_array_iteration() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var input = [1,2], seen = [];
            var groups = Object.groupBy(input, function(value, index) {
                seen.push(index);
                if (index === 0) input.push(3);
                return value % 2;
            });
            groups[1].join(',') === '1,3' && groups[0][0] === 2 &&
                seen.join(',') === '0,1,2' && Object.getPrototypeOf(groups) === null;
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_string_wrapper_tests {
    use super::*;

    #[test]
    fn string_wrappers_expose_immutable_code_units_to_object_operations() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var boxed = Object('a\uD800b');
            var descriptor = Object.getOwnPropertyDescriptor(boxed, '1');
            var length = Object.getOwnPropertyDescriptor(boxed, 'length');
            var copied = Object.assign({}, boxed);
            var before = boxed[1];
            boxed[1] = 'changed';
            var deleted = delete boxed[1];
            Object.keys(boxed).join(',') === '0,1,2' &&
                Object.values(boxed)[1] === '\uD800' &&
                Object.entries(boxed)[1][1] === '\uD800' &&
                copied[1] === '\uD800' && descriptor.value === '\uD800' &&
                descriptor.enumerable && !descriptor.writable && !descriptor.configurable &&
                length.value === 3 && !length.enumerable && !length.writable &&
                !length.configurable && boxed[1] === before && !deleted;
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn from_entries_accepts_boxed_string_entries_and_constructed_strings() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var entries = Object.fromEntries([Object('ab'), new String('cd')]);
            var receiver = {};
            var called = String.call(receiver, 'text');
            var boxed = new String('xy');
            var same = Object.defineProperty(boxed, '0', { value: 'x' });
            var rejected = false;
            try { Object.defineProperty(boxed, '0', { value: 'z' }); }
            catch (error) { rejected = error instanceof TypeError; }
            called === 'text' && !Object.hasOwn(receiver, 'length') &&
                entries.a === 'b' && entries.c === 'd' && same === boxed && rejected &&
                Object.keys(boxed).join(',') === '0,1';
        "#).unwrap(), JsValue::Bool(true));
    }
}

#[cfg(test)]
mod object_assignment_tests {
    use super::*;

    #[test]
    fn unsuccessful_property_assignment_only_throws_for_strict_references() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for prefix in ["", "'use strict';"] {
                let mut i = Interpreter::new();
                let source = format!(r#"{prefix}
                    var object = {{}};
                    Object.defineProperty(object, 'fixed', {{value: 3}});
                    var threw = false;
                    try {{ object.fixed = 4; }} catch (error) {{ threw = error instanceof TypeError; }}
                    threw === {} && object.fixed === 3;
                "#, !prefix.is_empty());
                assert_eq!(i.run_script_with_backend(&source, backend).unwrap(), JsValue::Bool(true),
                    "{backend:?}: {prefix}");
            }
        }
    }
}

// Complete the attributes of the runtime-owned intrinsic constructors after
// installation. Other lanes retain ownership of their constructors/methods.
impl Interpreter {
    fn install_runtime_intrinsic_attributes(&mut self) {
        for name in ["Object", "Function", "Array", "Boolean", "Promise", "Error",
            "EvalError", "RangeError", "ReferenceError", "SyntaxError", "TypeError", "URIError",
            "AggregateError", "SuppressedError"] {
            let Some(JsValue::Object(constructor)) = self.global_env.resolve(name) else { continue; };
            symbol_method_length(&JsValue::Object(constructor.clone()), match name {
                "AggregateError" => 2, "SuppressedError" => 3, _ => 1 });
            let prototype = constructor.borrow().get_own("prototype");
            if let Some(JsValue::Object(prototype)) = prototype {
                constructor.borrow_mut().define("prototype",
                    symbol_data(JsValue::Object(prototype.clone()), false, false)).unwrap();
                for key in ["constructor", "name", "message"] {
                    let property = prototype.borrow().own_prop(key).cloned();
                    if let Some(mut property) = property {
                        property.enumerable = false;
                        prototype.borrow_mut().define(key, property).unwrap();
                    }
                }
            }
        }
        if let Some(JsValue::Object(error)) = self.global_env.resolve("Error") {
            if let Some(JsValue::Object(prototype)) = error.borrow().get_own("prototype") {
                runtime_method_attributes(&prototype, "toString", 0);
            }
        }
        if let Some(JsValue::Object(promise)) = self.global_env.resolve("Promise") {
            for name in ["resolve", "reject", "all", "allSettled", "any", "race"] {
                runtime_method_attributes(&promise, name, 1);
            }
        }
        for (name, length) in [("then", 2), ("catch", 1), ("finally", 1)] {
            runtime_method_attributes(&self.promise_proto, name, length);
        }
    }
}

fn runtime_method_attributes(object: &Obj, name: &str, length: u32) {
    let property = object.borrow().own_prop(name).cloned();
    if let Some(mut property) = property {
        property.enumerable = false;
        if let JsValue::Object(method) = &property.value {
            method.borrow_mut().constructible = false;
            method.borrow_mut().delete_own("prototype");
        }
        symbol_method_length(&property.value, length);
        object.borrow_mut().define(name, property).unwrap();
    }
}
