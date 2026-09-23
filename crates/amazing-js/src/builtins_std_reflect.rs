// ECMAScript Reflect methods. Keep the object-target validation here: unlike
// the corresponding Object methods, Reflect never boxes primitive targets.

impl Interpreter {
    fn install_std_reflect(&mut self) {
        let Some(JsValue::Object(reflect)) = self.global_env.resolve("Reflect") else { return };
        for (name, length, native) in [
            ("apply", 3, std_reflect_apply as crate::value::NativeFn),
            ("construct", 2, std_reflect_construct as crate::value::NativeFn),
            ("defineProperty", 3, std_reflect_define_property as crate::value::NativeFn),
            ("deleteProperty", 2, std_reflect_delete_property as crate::value::NativeFn),
            ("get", 2, std_reflect_get as crate::value::NativeFn),
            ("getOwnPropertyDescriptor", 2, std_reflect_get_own_property_descriptor as crate::value::NativeFn),
            ("getPrototypeOf", 1, std_reflect_get_prototype_of as crate::value::NativeFn),
            ("has", 2, std_reflect_has as crate::value::NativeFn),
            ("isExtensible", 1, std_reflect_is_extensible as crate::value::NativeFn),
            ("ownKeys", 1, std_reflect_own_keys as crate::value::NativeFn),
            ("preventExtensions", 1, std_reflect_prevent_extensions as crate::value::NativeFn),
            ("set", 3, std_reflect_set as crate::value::NativeFn),
            ("setPrototypeOf", 2, std_reflect_set_prototype_of as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            reflect.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        reflect.borrow_mut().define(
            WellKnownSymbol::ToStringTag.symbol(),
            symbol_data(JsValue::str("Reflect"), false, true),
        ).unwrap();
        self.global_obj.borrow_mut().define("Reflect", symbol_data(JsValue::Object(reflect), true, true)).unwrap();
    }
}

fn std_reflect_target(args: &[JsValue]) -> Result<Obj, Throw> {
    match args.first() {
        Some(JsValue::Object(object)) => Ok(object.clone()),
        _ => Err(Throw::type_error("Reflect target must be an object")),
    }
}

fn std_reflect_arg(args: &[JsValue], index: usize) -> JsValue {
    args.get(index).cloned().unwrap_or(JsValue::Undefined)
}

fn std_reflect_apply(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_arg(&args, 0);
    if !matches!(&target, JsValue::Object(object) if object.borrow().func.is_some()) {
        return Err(Throw::type_error("Reflect.apply target is not callable"));
    }
    let list = i.create_list_from_array_like(&std_reflect_arg(&args, 2))?;
    i.call_value(&target, std_reflect_arg(&args, 1), list)
}

fn std_reflect_construct(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_arg(&args, 0);
    let new_target = args.get(2).cloned().unwrap_or_else(|| target.clone());
    for value in [&target, &new_target] {
        if !matches!(value, JsValue::Object(object) if object.borrow().func.is_some() && object.borrow().constructible) {
            return Err(Throw::type_error("Reflect.construct requires constructors"));
        }
    }
    let list = i.create_list_from_array_like(&std_reflect_arg(&args, 1))?;
    i.construct_with_new_target(&target, list, new_target)
}

fn std_reflect_define_property(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    if !matches!(args.get(2), Some(JsValue::Object(_))) {
        return Err(Throw::type_error("property descriptor must be an object"));
    }
    if let Some(JsValue::Object(descriptor)) = args.get(2) {
        if let Some(result) = binary_define_index_object(i, &target, &key, descriptor) {
            return Ok(JsValue::Bool(result?));
        }
    }
    Ok(JsValue::Bool(i.define_own_property(&target, &key, &std_reflect_arg(&args, 2))?))
}

fn std_reflect_delete_property(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    if let Some(deleted) = binary_delete_index(&target, &key) { return Ok(JsValue::Bool(deleted)); }
    let deleted = if let Some(name) = key.as_string() {
        i.legacy_delete(&target, name)?.unwrap_or_else(|| target.borrow_mut().delete_own(&key))
    } else { target.borrow_mut().delete_own(&key) };
    Ok(JsValue::Bool(deleted))
}

fn std_reflect_get(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    let receiver = args.get(2).cloned().unwrap_or_else(|| JsValue::Object(target.clone()));
    i.get_prop_receiver(&JsValue::Object(target), &key, receiver)
}

fn std_reflect_get_own_property_descriptor(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    if matches!(target.borrow().exotic, Exotic::ModuleNamespace(_)) {
        return native_object_get_own_property_descriptor(i, JsValue::Undefined, args);
    }
    let property = if let Some(property) = binary_index_descriptor(&target, &key) {
        property
    } else if let Some(name) = key.as_string() {
        i.legacy_own_property(&target, name, false)?
    } else { target.borrow().own_prop(&key).cloned() };
    let Some(property) = property else { return Ok(JsValue::Undefined) };
    let mut descriptor = JsObject::ordinary();
    descriptor.proto = Some(i.object_proto.clone());
    if property.getter.is_some() || property.setter.is_some() {
        let accessor = |function: Option<Box<JsFunction>>| function.map(|function| {
            let mut object = JsObject::function(*function);
            object.proto = Some(i.function_proto.clone());
            JsValue::object(object)
        }).unwrap_or(JsValue::Undefined);
        descriptor.set_own("get", accessor(property.getter));
        descriptor.set_own("set", accessor(property.setter));
    } else {
        descriptor.set_own("value", property.value);
        descriptor.set_own("writable", JsValue::Bool(property.writable));
    }
    descriptor.set_own("enumerable", JsValue::Bool(property.enumerable));
    descriptor.set_own("configurable", JsValue::Bool(property.configurable));
    Ok(JsValue::object(descriptor))
}

fn std_reflect_get_prototype_of(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let prototype = target.borrow().proto.clone();
    Ok(prototype.map(JsValue::Object).unwrap_or(JsValue::Null))
}

fn std_reflect_has(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    if let Some(property) = binary_index_descriptor(&target, &key) { return Ok(JsValue::Bool(property.is_some())); }
    let has = if let Some(name) = key.as_string() {
        i.legacy_has_property(&target, name)?
    } else { target.borrow().has_property(&key) };
    Ok(JsValue::Bool(has))
}

fn std_reflect_is_extensible(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    Ok(JsValue::Bool(i.legacy_for(&target).is_some() || target.borrow().extensible))
}

fn std_reflect_own_keys(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let keys = i.own_property_keys(&JsValue::Object(target))?.into_iter().map(|key| match key {
        PropertyKey::String(name) => JsValue::Str(name),
        PropertyKey::Symbol(symbol) => JsValue::Symbol(symbol),
    }).collect();
    array_values(i, keys)
}

fn std_reflect_prevent_extensions(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    if i.legacy_for(&target).is_some() { return Ok(JsValue::Bool(false)); }
    target.borrow_mut().extensible = false;
    Ok(JsValue::Bool(true))
}

fn std_reflect_set(i: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 1))?;
    let value = std_reflect_arg(&args, 2);
    let receiver = args.get(3).cloned().unwrap_or_else(|| JsValue::Object(target.clone()));
    Ok(JsValue::Bool(i.set_with_receiver(&target, &key, value, receiver)?))
}

fn std_reflect_set_prototype_of(_: &mut Interpreter, _: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let target = std_reflect_target(&args)?;
    let prototype = match args.get(1) {
        Some(JsValue::Object(object)) => Some(object.clone()),
        Some(JsValue::Null) => None,
        _ => return Err(Throw::type_error("prototype must be an object or null")),
    };
    let current = target.borrow().proto.clone();
    if current.as_ref().zip(prototype.as_ref()).is_some_and(|(a, b)| Rc::ptr_eq(&a.0, &b.0))
        || (current.is_none() && prototype.is_none()) { return Ok(JsValue::Bool(true)); }
    if !target.borrow().extensible { return Ok(JsValue::Bool(false)); }
    let mut cursor = prototype.clone();
    while let Some(object) = cursor {
        if Rc::ptr_eq(&object.0, &target.0) { return Ok(JsValue::Bool(false)); }
        cursor = object.borrow().proto.clone();
    }
    target.borrow_mut().proto = prototype;
    Ok(JsValue::Bool(true))
}
