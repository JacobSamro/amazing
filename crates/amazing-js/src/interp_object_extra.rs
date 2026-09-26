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

fn native_object_from_entries(
    i: &mut Interpreter,
    _: JsValue,
    args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let input = object_argument(&args);
    let entries = i.iterate(&input)?;
    let object = i.object_value();
    let JsValue::Object(result) = &object else { unreachable!() };
    for entry in entries {
        if !entry.is_object() {
            return Err(Throw::type_error("entry is not an object"));
        }
        let key_value = i.get_prop(&entry, "0")?;
        let value = i.get_prop(&entry, "1")?;
        let key = i.to_property_key(&key_value)?;
        result.borrow_mut().define(key, Property {
            value, writable: true, enumerable: true, configurable: true,
            getter: None, setter: None,
        }).unwrap();
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
    if i.legacy_for(object).is_some() {
        return Err(Throw::type_error("cannot seal a legacy platform object"));
    }
    if freeze && matches!(object.borrow().exotic, Exotic::TypedArray) {
        let view = binary_view(&target)?;
        if binary_length(&view) != 0 || view.buffer.borrow().resizable {
            return Err(Throw::type_error("cannot freeze a TypedArray with indexed elements"));
        }
    }
    object.borrow_mut().extensible = false;
    for key in i.own_property_keys(&target)? {
        let existing = object.borrow().own_prop(&key).cloned();
        if let Some(mut property) = existing {
            property.configurable = false;
            if freeze && property.getter.is_none() && property.setter.is_none() {
                property.writable = false;
            }
            object.borrow_mut().define(key, property).unwrap();
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
    if i.legacy_for(object).is_some() || object.borrow().extensible {
        return Ok(JsValue::Bool(false));
    }
    for key in i.own_property_keys(&target)? {
        if let Some(property) = object.borrow().own_prop(&key) {
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
    let entries = i.iterate(&items)?;
    let result = JsValue::object(JsObject::ordinary());
    for (index, value) in entries.into_iter().enumerate() {
        let group = i.call_value(
            &callback,
            JsValue::Undefined,
            vec![value.clone(), JsValue::num(index as f64)],
        )?;
        let key = i.to_property_key(&group)?;
        let current = i.get_prop(&result, &key)?;
        if current.is_undefined() {
            let group = array_values(i, vec![value])?;
            i.set_prop(&result, key.clone(), group)?;
        } else {
            let length = array_like_length(i, &current)?;
            i.set_prop(&current, length.to_string(), value)?;
        }
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
                let property = match binary_index_descriptor(object, key) {
                    Some(property) => property,
                    None => match key {
                        PropertyKey::String(name) => self.legacy_own_property(object, name.clone(), false)?,
                        PropertyKey::Symbol(_) => object.borrow().own_prop(key).cloned(),
                    },
                };
                Ok(property.is_some_and(|property| property.enumerable))
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
