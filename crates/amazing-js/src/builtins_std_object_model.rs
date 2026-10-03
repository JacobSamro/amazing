// Portions ported from boa-dev/boa (core/engine/src/builtins/function/arguments.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Portions ported from boa-dev/boa (core/engine/src/builtins/object/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: legacy accessor lookup.
// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: Array exotic definition dispatch.
// Ordinary object operations shared by the standard-library built-ins.
// These live beside the built-ins until the runtime adopts them.

#[derive(Clone, Default)]
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
        if is_module_namespace(target) {
            let fields=self.std_descriptor_fields(descriptor)?;
            return namespace_define_fields(self,target,key,&fields);
        }
        if let JsValue::Object(descriptor_object) = descriptor {
            if let Some(result) = binary_define_index_object(self, target, key, descriptor_object) {
                return result;
            }
        }

        let fields = self.std_descriptor_fields(descriptor)?;
        if matches!(target.borrow().exotic, Exotic::Array) {
            return self.array_define_fields(target, key, fields);
        }
        self.ordinary_define_fields(target, key, fields)
    }

    // Complete the existing ordinary descriptor algorithm without dispatching
    // through Array exotic methods again. Arguments and legacy overlays remain
    // here so their descriptor/alias behavior is unchanged.
    fn ordinary_define_fields(&mut self, target: &Obj, key: &PropertyKey,
        mut fields: StdDescriptorFields) -> Result<bool, Throw> {
        let mapped_cell = arguments_mapped_cell(target, key);
        let mapped_value = fields.value.clone();
        let mapped_detach = fields.get.is_some() || fields.set.is_some() || fields.writable == Some(false);
        if fields.get.is_none() && fields.set.is_none() && fields.value.is_none()
            && fields.writable == Some(false) {
            if let Some(cell) = &mapped_cell { fields.value = Some(cell.read()?); }
        }
        let original = if let Some(result) = arguments_own_property(target, key) {
            result?
        } else if let Some(name) = key.as_string() {
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
        arguments_finish_define(target, key, mapped_cell, mapped_value, mapped_detach)?;
        Ok(true)
    }

    fn set_with_receiver(&mut self, target: &Obj, key: &PropertyKey, value: JsValue, receiver: JsValue) -> Result<bool, Throw> {
        if let Some(result) = self.window_proxy_set(target, key, value.clone(), receiver.clone()) { return result; }
        if let Some(result) = self.proxy_set(target, key, value.clone(), receiver.clone()) { return result; }
        if matches!(&receiver, JsValue::Object(object) if Rc::ptr_eq(&object.0, &target.0)) {
            arguments_write_mapping(target, key, value.clone())?;
        }
        if is_module_namespace(target) {
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
        // [[Set]] with a distinct Receiver on a canonical numeric index of an
        // integer-indexed exotic `target`: IsValidIntegerIndex decides the
        // outcome directly (10.4.5.5). The generic prototype walk below reads
        // `own_prop`, which can't see virtual typed-array indices, and would
        // otherwise walk straight past `target` to a shadowing prototype
        // accessor that the spec says is unreachable here.
        let target_index_view = binary_index_view(target, key);
        if let Some((view, _kind, index)) = &target_index_view {
            if index.filter(|idx| *idx < binary_length(view)).is_none() {
                return Ok(true);
            }
        }
        let mut cursor = if target_index_view.is_some() { None } else { Some(target.clone()) };
        while let Some(object) = cursor {
            if let Some(result) = self.window_proxy_set(&object, key, value.clone(), receiver.clone()) { return result; }
            if let Some(result) = self.proxy_set(&object, key, value.clone(), receiver.clone()) { return result; }
            if is_module_namespace(&object) { return Ok(false); }
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
                Some(result) => result?,
                None => match self.namespace_own_property(&receiver_object, key) {
                    Some(result) => result?,
                    None => receiver_object.borrow().own_prop(key).cloned(),
                },
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
        if is_module_namespace(&receiver_object) {
            return namespace_define_fields(self, &receiver_object, key,
                &StdDescriptorFields { value: Some(value), ..Default::default() });
        }
        self.ordinary_set_data_property(&receiver_object, key, value, existing)
    }

    // JSRT-21: array push/index-assign fast path for OrdinarySet's final step
    // (10.1.9.2 step 4.d, "Perform ? CreateDataProperty(Receiver, P, V)" /
    // the existing-writable-data-property value replacement in step 3). The
    // code this replaces built a throwaway 3-4-field JsObject descriptor and
    // round-tripped it through define_own_property, which re-parses those
    // fields one at a time via std_descriptor_fields (a has_property +
    // get_prop pair per field, each walking the descriptor object's own
    // lookup path) and re-fetches the same own-property `existing` that
    // set_with_receiver had just looked up. Every one of those re-lookups
    // observes the same object with no intervening call that can run user
    // code, so they are pure overhead: this writes the identical Property
    // this Interpreter and value.rs would have produced, directly.
    fn ordinary_set_data_property(&mut self, target: &Obj, key: &PropertyKey, value: JsValue, existing: Option<Property>) -> Result<bool, Throw> {
        // define_own_property's own entry point checks these two traps before
        // touching `target`; skipping straight to the ordinary algorithm below
        // would silently bypass a Proxy's/WindowProxy's [[DefineOwnProperty]]
        // trap when `receiver_object` (passed in here as `target`) is itself
        // one, which is the general path this fast path replaces. Traps need a
        // real descriptor object handed to arbitrary trap code, so build one
        // and fall back for this (rare -- ordinary objects are the common
        // case) branch only.
        if self.is_window_proxy(target) || self.proxy_parts(target).is_some()
            || matches!(&target.borrow().exotic, Exotic::Arguments(_)) {
            let mut descriptor = JsObject::ordinary();
            descriptor.set_own("value", value);
            if existing.is_none() {
                for name in ["writable", "enumerable", "configurable"] {
                    descriptor.set_own(name, JsValue::Bool(true));
                }
            }
            return self.define_own_property(target, key, &JsValue::object(descriptor));
        }
        if let Some(mut property) = existing {
            // Length assignment uses the same ArraySetLength operation as
            // DefineOwnProperty, including both observable numeric conversions.
            if matches!(target.borrow().exotic, Exotic::Array)
                && key.as_string().is_some_and(|name| name.to_utf8_lossy() == "length") {
                return self.array_set_length_fields(target, StdDescriptorFields {
                    value: Some(value), ..StdDescriptorFields::default()
                });
            }
            // Caller (set_with_receiver) already confirmed this property has
            // no getter/setter and is writable, so only .value changes here;
            // a plain [[Set]] never touches writable/enumerable/configurable.
            property.value = value;
            if let Some(name) = key.as_string() {
                if self.legacy_define(target, name, &property)? { return Ok(true); }
            }
            target.borrow_mut().define(key, property).unwrap();
            return Ok(true);
        }
        if !target.borrow().extensible { return Ok(false); }
        let is_array = matches!(target.borrow().exotic, Exotic::Array);
        let extending_array_index = if is_array {
            array_index_key(key).and_then(|index| {
                let length = target.borrow().get_own("length")?.to_number().ok()?;
                (index as f64 >= length).then_some(index)
            })
        } else { None };
        if extending_array_index.is_some() && target.borrow().own_prop("length").is_some_and(|p| !p.writable) {
            return Ok(false);
        }
        let property = Property {
            value, writable: true, enumerable: true, configurable: true, getter: None, setter: None,
        };
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

// Annex B Object.prototype.__proto__: unlike Object.setPrototypeOf, an invalid
// prototype argument is silently ignored, after RequireObjectCoercible(this).
fn native_object_proto_getter(
    i: &mut Interpreter, this: JsValue, _: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = std_legacy_to_object(i, this)? else { unreachable!() };
    i.default_get_prototype(&object)
}

fn native_object_proto_setter(
    i: &mut Interpreter, this: JsValue, args: Vec<JsValue>,
) -> Result<JsValue, Throw> {
    if this.is_null() || this.is_undefined() {
        return Err(Throw::type_error("Object.prototype accessor called on null or undefined"));
    }
    let prototype = match args.first() {
        Some(JsValue::Object(object)) => Some(object.clone()),
        Some(JsValue::Null) => None,
        _ => return Ok(JsValue::Undefined),
    };
    if let JsValue::Object(object) = this {
        if !i.default_set_prototype(&object, &prototype)? {
            return Err(Throw::type_error("cannot change object prototype"));
        }
    }
    Ok(JsValue::Undefined)
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
        let property = i.default_get_own_property(&target, &key)?;
        if let Some(property) = property {
            if property.getter.is_some() || property.setter.is_some() {
                let accessor = if want_getter { property.getter } else { property.setter };
                return Ok(accessor.unwrap_or(JsValue::Undefined));
            }
            return Ok(JsValue::Undefined);
        }
        cursor = match i.default_get_prototype(&target)? { JsValue::Object(proto) => Some(proto), _ => None };
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
