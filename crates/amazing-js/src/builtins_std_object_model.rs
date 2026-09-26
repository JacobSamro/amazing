// Ordinary object operations shared by the standard-library built-ins.
// These live beside the built-ins until the runtime adopts them.

#[derive(Default)]
struct StdDescriptorFields {
    value: Option<JsValue>,
    writable: Option<bool>,
    get: Option<JsValue>,
    set: Option<JsValue>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
}

fn std_same_value(left: &JsValue, right: &JsValue) -> bool {
    match (left, right) {
        (JsValue::Num(a), JsValue::Num(b)) => {
            (a.is_nan() && b.is_nan()) || (a == b && (a != &0.0 || a.is_sign_negative() == b.is_sign_negative()))
        }
        _ => left == right,
    }
}

fn std_callable(value: &JsValue) -> bool {
    matches!(value, JsValue::Object(object) if object.borrow().func.is_some())
}

impl Interpreter {
    fn std_descriptor_fields(&mut self, descriptor: &JsValue) -> Result<StdDescriptorFields, Throw> {
        let JsValue::Object(object) = descriptor else {
            return Err(Throw::type_error("property descriptor must be an object"));
        };
        let mut fields = StdDescriptorFields::default();
        for name in ["enumerable", "configurable", "value", "writable", "get", "set"] {
            if !object.borrow().has_property(name) { continue; }
            let value = self.get_prop(descriptor, name)?;
            match name {
                "enumerable" => fields.enumerable = Some(value.to_boolean()),
                "configurable" => fields.configurable = Some(value.to_boolean()),
                "value" => fields.value = Some(value),
                "writable" => fields.writable = Some(value.to_boolean()),
                "get" => fields.get = Some(value),
                "set" => fields.set = Some(value),
                _ => unreachable!(),
            }
        }
        if (fields.get.is_some() || fields.set.is_some())
            && (fields.value.is_some() || fields.writable.is_some()) {
            return Err(Throw::type_error("descriptor mixes data and accessor fields"));
        }
        for accessor in [&fields.get, &fields.set] {
            if let Some(value) = accessor {
                if !value.is_undefined() && !std_callable(value) {
                    return Err(Throw::type_error("accessor is not callable"));
                }
            }
        }
        Ok(fields)
    }

    fn define_own_property(&mut self, target: &Obj, key: &PropertyKey, descriptor: &JsValue) -> Result<bool, Throw> {
        if let Some(result) = self.window_proxy_define(target, key, descriptor) { return result; }
        if let Some(result) = self.proxy_define(target, key, descriptor) { return result; }
        if matches!(target.borrow().exotic, Exotic::ModuleNamespace(_)) {
            let JsValue::Object(fields) = descriptor else { return Err(Throw::type_error("property descriptor must be an object")); };
            return namespace_define_allowed(self, target, key, fields);
        }
        if let JsValue::Object(descriptor_object) = descriptor {
            if let Some(result) = binary_define_index_object(self, target, key, descriptor_object) {
                return result;
            }
        }

        let mut fields = self.std_descriptor_fields(descriptor)?;
        let is_array = matches!(target.borrow().exotic, Exotic::Array);
        if is_array && key.as_string().is_some_and(|name| name.to_utf8_lossy() == "length") {
            if let Some(value) = fields.value.take() {
                let number = self.to_number(&value)?;
                if !number.is_finite() || number < 0.0 || number.fract() != 0.0 || number > u32::MAX as f64 {
                    return Err(Throw::range_error("invalid array length"));
                }
                if target.borrow().own_prop("length").is_some_and(|old| !old.writable && old.value != JsValue::num(number)) {
                    return Ok(false);
                }
                if !array_prepare_length_change(target, number as u32) { return Ok(false); }
                fields.value = Some(JsValue::num(number));
            }
        }
        let extending_array_index = if is_array {
            array_index_key(key).and_then(|index| {
                let length = target.borrow().get_own("length")?.to_number().ok()?;
                (index as f64 >= length).then_some(index)
            })
        } else { None };
        if extending_array_index.is_some() && target.borrow().own_prop("length").is_some_and(|p| !p.writable) {
            return Ok(false);
        }

        let original = if let Some(name) = key.as_string() {
            self.legacy_own_property(target, name, false)?
        } else { target.borrow().own_prop(key).cloned() }.or_else(|| {
            if key.as_string().is_some_and(|name| name.to_utf8_lossy() == "length") {
                target.borrow().func.as_ref().map(|function| Property {
                    value: JsValue::num(function.params.len() as f64),
                    writable: false, enumerable: false, configurable: true,
                    getter: None, setter: None,
                })
            } else { None }
        });
        let new_accessor = fields.get.is_some() || fields.set.is_some();
        let new_data = fields.value.is_some() || fields.writable.is_some();
        let Some(mut property) = original.clone() else {
            if !target.borrow().extensible { return Ok(false); }
            let mut property = Property {
                value: JsValue::Undefined, writable: false, enumerable: fields.enumerable.unwrap_or(false),
                configurable: fields.configurable.unwrap_or(false), getter: None, setter: None,
            };
            if new_accessor {
                property.getter = fields.get.as_ref().filter(|value| callable_function(value).is_some()).cloned();
                property.setter = fields.set.as_ref().filter(|value| callable_function(value).is_some()).cloned();
            } else {
                property.value = fields.value.unwrap_or(JsValue::Undefined);
                property.writable = fields.writable.unwrap_or(false);
            }
            if let Some(name) = key.as_string() {
                if self.legacy_define(target, name, &property)? { return Ok(true); }
            }
            target.borrow_mut().define(key, property).unwrap();
            if let Some(index) = extending_array_index {
                let mut length = target.borrow().own_prop("length").cloned().unwrap();
                length.value = JsValue::num(index as f64 + 1.0);
                target.borrow_mut().define("length", length).unwrap();
            }
            return Ok(true);
        };
        let old_accessor = property.getter.is_some() || property.setter.is_some();
        if !property.configurable {
            if fields.configurable == Some(true) || fields.enumerable.is_some_and(|v| v != property.enumerable) {
                return Ok(false);
            }
            if (new_accessor && !old_accessor) || (new_data && old_accessor) { return Ok(false); }
            if old_accessor {
                for (requested, old) in [(&fields.get, &property.getter), (&fields.set, &property.setter)] {
                    if let Some(requested) = requested {
                        let same = match (requested, old) {
                            (JsValue::Undefined, None) => true,
                            (requested, Some(function)) => std_same_value(requested, function),
                            _ => false,
                        };
                        if !same { return Ok(false); }
                    }
                }
            } else if !property.writable {
                if fields.writable == Some(true) || fields.value.as_ref().is_some_and(|value| !std_same_value(value, &property.value)) {
                    return Ok(false);
                }
            }
        }
        if new_accessor && !old_accessor {
            property.value = JsValue::Undefined;
            property.writable = false;
        } else if new_data && old_accessor {
            property.getter = None;
            property.setter = None;
            property.value = JsValue::Undefined;
            property.writable = false;
        }
        if let Some(value) = fields.value { property.value = value; }
        if let Some(writable) = fields.writable { property.writable = writable; }
        if let Some(enumerable) = fields.enumerable { property.enumerable = enumerable; }
        if let Some(configurable) = fields.configurable { property.configurable = configurable; }
        if let Some(get) = fields.get { property.getter = callable_function(&get).map(|_| get); }
        if let Some(set) = fields.set { property.setter = callable_function(&set).map(|_| set); }
        if let Some(name) = key.as_string() {
            if self.legacy_define(target, name, &property)? { return Ok(true); }
        }
        target.borrow_mut().define(key, property).unwrap();
        if let Some(index) = extending_array_index {
            let mut length = target.borrow().own_prop("length").cloned().unwrap();
            length.value = JsValue::num(index as f64 + 1.0);
            target.borrow_mut().define("length", length).unwrap();
        }
        Ok(true)
    }

    fn set_with_receiver(&mut self, target: &Obj, key: &PropertyKey, value: JsValue, receiver: JsValue) -> Result<bool, Throw> {
        if let Some(result) = self.window_proxy_set(target, key, value.clone(), receiver.clone()) { return result; }
        if let Some(result) = self.proxy_set(target, key, value.clone(), receiver.clone()) { return result; }
        if matches!(target.borrow().exotic, Exotic::ModuleNamespace(_)) {
            return Ok(false);
        }
        if let Some(name) = key.as_string() {
            let name = name.to_utf8_lossy();
            if let Some(forward_to) = self.binding_put_forwards_target(target, &name) {
                let forwarded = self.get_prop(&receiver, &name)?;
                self.set_prop(&forwarded, forward_to, value)?;
                return Ok(true);
            }
        }
        if let JsValue::Object(object) = &receiver {
            if Rc::ptr_eq(&object.0, &target.0) {
                if let Some(result) = binary_set_index(self, target, key, value.clone()) {
                    result?;
                    return Ok(true);
                }
            }
        }
        let mut cursor = Some(target.clone());
        while let Some(object) = cursor {
            if let Some(result) = self.window_proxy_set(&object, key, value.clone(), receiver.clone()) { return result; }
            if let Some(result) = self.proxy_set(&object, key, value.clone(), receiver.clone()) { return result; }
            let property = if let Some(name) = key.as_string() {
                // Legacy named properties belong to the collection itself.
                // They do not prevent an ordinary receiver from shadowing one.
                let inherited = !matches!(&receiver, JsValue::Object(receiver_object)
                    if Rc::ptr_eq(&receiver_object.0, &object.0));
                self.legacy_own_property(&object, name, inherited)?
            } else { object.borrow().own_prop(key).cloned() };
            if let Some(property) = property {
                if let Some(setter) = property.setter {
                    self.call_value(&setter, receiver, vec![value])?;
                    return Ok(true);
                }
                if property.getter.is_some() || !property.writable { return Ok(false); }
                break;
            }
            cursor = if let Some(result) = self.window_proxy_prototype(&object) {
                match result? { JsValue::Object(p) => Some(p), _ => None }
            } else if let Some(result) = self.proxy_get_prototype(&object) {
                match result? { JsValue::Object(p) => Some(p), _ => None }
            } else { object.borrow().proto.clone() };
        }
        let JsValue::Object(receiver_object) = receiver else { return Ok(false); };
        let existing = match self.window_proxy_own_property(&receiver_object, key) {
            Some(result) => result?,
            None => match self.proxy_get_own_property(&receiver_object, key) {
                Some(result) => result?, None => receiver_object.borrow().own_prop(key).cloned(),
            },
        };
        if let Some(property) = &existing {
            if property.getter.is_some() || property.setter.is_some() || !property.writable { return Ok(false); }
        } else if !receiver_object.borrow().extensible { return Ok(false); }
        if let Some(result) = binary_set_index(self, &receiver_object, key, value.clone()) {
            result?;
            return Ok(true);
        }
        if self.legacy_for(&receiver_object).is_some()
            || Self::dom_style_id(&receiver_object).is_some()
            || Self::dom_dataset_id(&receiver_object).is_some()
            || dom_node_id(&receiver_object).is_some()
        {
            self.set_prop(&JsValue::Object(receiver_object), key, value)?;
            return Ok(true);
        }
        let mut descriptor = JsObject::ordinary();
        descriptor.set_own("value", value);
        if existing.is_none() {
            for name in ["writable", "enumerable", "configurable"] {
                descriptor.set_own(name, JsValue::Bool(true));
            }
        }
        self.define_own_property(&receiver_object, key, &JsValue::object(descriptor))
    }

    fn create_list_from_array_like(&mut self, value: &JsValue) -> Result<Vec<JsValue>, Throw> {
        if !value.is_object() { return Err(Throw::type_error("argumentsList must be an object")); }
        let length_value = self.get_prop(value, "length")?;
        let length = self.to_number(&length_value)?;
        let length = if length.is_nan() || length <= 0.0 { 0 }
            else if length >= 9_007_199_254_740_991.0 { 9_007_199_254_740_991usize }
            else { length.trunc() as usize };
        if length > 1_000_000 { return Err(Throw::range_error("argument list too large")); }
        (0..length).map(|index| self.get_prop(value, index.to_string())).collect()
    }
}

// Annex B.2.2.2-5: Object.prototype.__defineGetter__ / __defineSetter__ /
// __lookupGetter__ / __lookupSetter__. RequireObjectCoercible(this) then
// ToObject(this); primitives are boxed so the define/lookup operate on the
// wrapper's own properties exactly as the spec describes.

fn std_legacy_to_object(i: &mut Interpreter, this: JsValue) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Object.prototype accessor called on null or undefined"));
    }
    native_object(i, JsValue::Undefined, vec![this])
}

fn std_object_define_accessor(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, is_getter: bool,
) -> Result<JsValue, Throw> {
    let object = std_legacy_to_object(i, this)?;
    let accessor = std_reflect_arg(&args, 1);
    if callable_function(&accessor).is_none() {
        return Err(Throw::type_error(if is_getter { "__defineGetter__ requires a callable" } else { "__defineSetter__ requires a callable" }));
    }
    let key = i.to_property_key(&std_reflect_arg(&args, 0))?;
    let mut descriptor = JsObject::ordinary();
    descriptor.set_own(if is_getter { "get" } else { "set" }, accessor);
    descriptor.set_own("enumerable", JsValue::Bool(true));
    descriptor.set_own("configurable", JsValue::Bool(true));
    let JsValue::Object(target) = object else { return Err(Throw::type_error("receiver is not an object")); };
    if !i.define_own_property(&target, &key, &JsValue::object(descriptor))? {
        return Err(Throw::type_error("cannot define property"));
    }
    Ok(JsValue::Undefined)
}

fn std_object_lookup_accessor(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>, want_getter: bool,
) -> Result<JsValue, Throw> {
    let object = std_legacy_to_object(i, this)?;
    let key = i.to_property_key(&std_reflect_arg(&args, 0))?;
    let mut cursor = match &object { JsValue::Object(o) => Some(o.clone()), _ => None };
    while let Some(target) = cursor {
        let property = match key.as_string() {
            Some(name) => i.legacy_own_property(&target, name, false)?,
            None => target.borrow().own_prop(&key).cloned(),
        };
        if let Some(property) = property {
            if property.getter.is_some() || property.setter.is_some() {
                let accessor = if want_getter { property.getter } else { property.setter };
                return Ok(accessor.unwrap_or(JsValue::Undefined));
            }
            return Ok(JsValue::Undefined);
        }
        cursor = target.borrow().proto.clone();
    }
    Ok(JsValue::Undefined)
}

fn native_object_define_getter(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    std_object_define_accessor(i, this, args, true)
}

fn native_object_define_setter(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    std_object_define_accessor(i, this, args, false)
}

fn native_object_lookup_getter(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    std_object_lookup_accessor(i, this, args, true)
}

fn native_object_lookup_setter(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    std_object_lookup_accessor(i, this, args, false)
}

impl Interpreter {
    /// Register an embedding host brand whose objects carry the [[ErrorData]]
    /// brand (for example DOMException). `Error.isError` consults this set, so
    /// a writable JS property can never forge the brand.
    pub fn binding_register_error_brand(&mut self, brand: u64) {
        self.error_brands.insert(brand);
    }

    fn host_error_brand(&self, object: &JsObject) -> bool {
        object
            .host_identity
            .as_ref()
            .is_some_and(|identity| self.error_brands.contains(&identity.handle.brand))
    }
}
