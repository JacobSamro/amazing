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
    for key in i.enumerable_keys(&object)? {
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
    for entry in entries {
        if !entry.is_object() {
            return Err(Throw::type_error("entry is not an object"));
        }
        let key_value = i.get_prop(&entry, "0")?;
        let value = i.get_prop(&entry, "1")?;
        let key = i.to_property_key(&key_value)?;
        i.set_prop(&object, key, value)?;
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
            i.set_prop(&result, key, descriptor)?;
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
    for key in i.enumerable_property_keys(&descriptors)? {
        let descriptor = i.get_prop(&descriptors, &key)?;
        i.std_descriptor_fields(&descriptor)?;
        pending.push((key, descriptor));
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
