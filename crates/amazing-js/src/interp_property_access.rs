// Portions ported from boa-dev/boa (core/engine/src/builtins/array/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT: Array own-write exotic dispatch.
impl Interpreter {
    fn get_prop(&mut self, v: &JsValue, k: impl Into<PropertyKey>) -> Result<JsValue, Throw> {
        self.get_prop_receiver(v, &k.into(), v.clone())
    }

    #[inline(never)]
    fn get_prop_receiver(
        &mut self,
        v: &JsValue,
        key: &PropertyKey,
        receiver: JsValue,
    ) -> Result<JsValue, Throw> {
        // Trap lookup and callable metadata lookup can traverse Proxy and
        // ordinary prototype chains without creating another JS call frame.
        self.admit_synchronous_call()?;
        self.get_prop_receiver_admitted(v, key, receiver)
    }

    #[inline(never)]
    fn get_prop_receiver_admitted(
        &mut self,
        v: &JsValue,
        key: &PropertyKey,
        receiver: JsValue,
    ) -> Result<JsValue, Throw> {
        if let JsValue::Object(object) = v {
            if let Some(target) = self.window_proxy_property_target(object, WindowProxyOperation::Get, Some(key)) {
                let (realm, global, property) = target?;
                if let WindowProxyProperty::Descriptor(property) = property {
                    return match property {
                        Some(property) => if property.is_accessor() {
                            match property.getter {
                                Some(getter) => self.call_value(&getter, receiver, Vec::new()),
                                None => Ok(JsValue::Undefined),
                            }
                        } else { Ok(property.value) },
                        None => Ok(JsValue::Undefined),
                    };
                }
                return self.binding_with_realm(realm, |i| i.get_prop_receiver(&JsValue::Object(global), key, receiver));
            }
            if let Some(result) = self.proxy_get(object, key, receiver.clone()) { return result; }
            if let Some(cell) = arguments_mapped_cell(object, key) { return cell.read(); }
        }
        let text = key.as_string().map(|name| name.to_utf8_lossy());
        let k = text.as_deref().unwrap_or("");
        match v {
            JsValue::Object(o) => {
                if let Some(result)=self.namespace_get(o,key) {return result;}
                if let Some(result) = binary_get_index(self, o, key) { return result; }
                if key.as_string().is_some() {
                if let Some(node) = Self::dom_dataset_id(o) {
                    let name = Self::dataset_attribute_name(k);
                    return Ok(dom_host(self)?
                        .borrow()
                        .get_attribute(node, &name)
                        .flatten()
                        .map(JsValue::str)
                        .unwrap_or(JsValue::Undefined));
                }
                if let Some(node) = Self::dom_style_id(o) {
                    if let Some(value) = dom_host(self)?.borrow().get_style(node, k) {
                        return Ok(JsValue::str(value));
                    }
                }
                if let Some(node) = dom_node_id(o) {
                    if node == 0 && (k == "body" || k == "documentElement") {
                        let selector = if k == "body" { "body" } else { "html" };
                        let id = dom_host(self)?.borrow_mut().query_selector(selector);
                        return Ok(id
                            .map(|id| self.make_dom_element(id))
                            .unwrap_or(JsValue::Null));
                    }
                    if node != 0 && (k == "id" || k == "className") {
                        let name = if k == "className" { "class" } else { "id" };
                        let value = dom_host(self)?
                            .borrow()
                            .get_attribute(node, name)
                            .flatten()
                            .unwrap_or_default();
                        return Ok(JsValue::str(value));
                    }
                }
                if k == "size" {
                    let size = match &o.borrow().exotic {
                        Exotic::Map(state) => Some(state.entries.len()),
                        Exotic::Set(state) => Some(state.entries.len()),
                        _ => None,
                    };
                    if let Some(size) = size {
                        return Ok(JsValue::num(size as f64));
                    }
                }
                }
                let property = match key.as_string() {
                    Some(name) if self.legacy_for(o).is_some() => self.legacy_own_property(o, name, false)?,
                    _ => o.borrow().own_prop(key).cloned(),
                };
                let prototype = o.borrow().proto.clone();
                if let Some(property) = property {
                    if property.is_accessor() {
                        if let Some(getter) = property.getter {
                            self.call_value(&getter, receiver, Vec::new())
                        } else { Ok(JsValue::Undefined) }
                    } else { Ok(property.value) }
                } else if let Some(p) = prototype {
                    self.get_prop_receiver(&JsValue::Object(p), key, receiver)
                } else {
                    Ok(JsValue::Undefined)
                }
            }
            JsValue::Str(s) => {
                if k == "length" {
                    Ok(JsValue::num(s.len() as f64))
                } else if let Ok(n) = k.parse::<usize>() {
                    Ok(s.units().get(n).map(|u| JsValue::str_units(vec![*u]))
                        .unwrap_or(JsValue::Undefined))
                } else {
                    self.get_prop_receiver(&JsValue::Object(self.string_proto.clone()), key, receiver)
                }
            }
            JsValue::Symbol(_) => self.get_prop_receiver(&JsValue::Object(self.symbol_proto.clone()), key, receiver),
            JsValue::Num(_) => self.get_prop_receiver(&JsValue::Object(self.number_proto.clone()), key, receiver),
            JsValue::Bool(_) => self.get_prop_receiver(&JsValue::Object(self.boolean_proto.clone()), key, receiver),
            JsValue::BigInt(_) => self.get_prop_receiver(&JsValue::Object(self.bigint_proto.clone()), key, receiver),
            JsValue::Null | JsValue::Undefined => Ok(JsValue::Undefined),
        }
    }
    fn set_prop(&mut self, v: &JsValue, key: impl Into<PropertyKey>, x: JsValue) -> Result<(), Throw> {
        let key = key.into();
        let text = key.as_string().map(|name| name.to_utf8_lossy());
        let k = text.as_deref().unwrap_or("");
        let o = match v {
            JsValue::Object(o) => o.clone(),
            _ => return Err(Throw::type_error("cannot set property on primitive")),
        };
        if let Some(result) = self.window_proxy_set(&o, &key, x.clone(), v.clone()) {
            return if result? { Ok(()) } else { Err(Throw::type_error("cannot set WindowProxy property")) };
        }
        if let Some(result) = self.proxy_set(&o, &key, x.clone(), v.clone()) {
            return if result? { Ok(()) } else if self.strict_eval_context {
                Err(Throw::type_error("cannot set proxy property"))
            } else { Ok(()) };
        }
        if is_module_namespace(&o) {
            return Err(Throw::type_error("module namespace properties cannot be assigned"));
        }
        if let Some(result) = binary_set_index(self, &o, &key, x.clone()) { return result; }
        arguments_write_mapping(&o, &key, x.clone())?;
        if let Some(forward_to) = self.binding_put_forwards_target(&o, k) {
            let target = self.get_prop(v, k)?;
            self.set_prop(&target, forward_to, x)?;
            return Ok(());
        }
        if let Some(name) = key.as_string() {
        if let Some(node) = Self::dom_style_id(&o) {
            let value = self.to_string(&x)?;
            dom_mutation(self, |host| host.set_style(node, k, &value))?;
            return Ok(());
        }
        if let Some(node) = Self::dom_dataset_id(&o) {
            let name = Self::dataset_attribute_name(k);
            let value = self.to_string(&x)?;
            dom_mutation(self, |host| host.set_attribute(node, &name, &value))?;
            return Ok(());
        }
        if let Some(node) = dom_node_id(&o) {
            if node != 0 && (k == "id" || k == "className") {
                let name = if k == "className" { "class" } else { "id" };
                let value = self.to_string(&x)?;
                dom_mutation(self, |host| host.set_attribute(node, name, &value))?;
                return Ok(());
            }
        }
        if self.legacy_set(&o, name, x.clone())? { return Ok(()); }
        }
        if o.borrow().own_prop(&key).is_none()
            && (matches!(&key, PropertyKey::Symbol(_)) || self.legacy_for(&o).is_some()) {
            let mut cursor = o.borrow().proto.clone();
            while let Some(prototype) = cursor {
                let descriptor = prototype.borrow().own_prop(&key).cloned();
                if let Some(descriptor) = descriptor {
                    if let Some(setter) = descriptor.setter.clone() {
                        self.call_value(&setter, JsValue::Object(o.clone()), vec![x])?;
                        return Ok(());
                    }
                    if descriptor.is_accessor() || !descriptor.writable {
                        return Err(Throw::type_error("property has no writable setter"));
                    }
                    break;
                }
                // A Proxy mid-chain must have its getPrototypeOf trap observed
                // when walking further up looking for an inherited setter.
                cursor = if let Some(result) = self.window_proxy_prototype(&prototype) {
                    match result? { JsValue::Object(p) => Some(p), _ => None }
                } else if let Some(result) = self.proxy_get_prototype(&prototype) {
                    match result? { JsValue::Object(p) => Some(p), _ => None }
                } else { prototype.borrow().proto.clone() };
            }
        }
        // JSRT-21: fetch the own property once and reuse it for ordinary
        // writes, since no user code runs between the fetch and the write.
        // Array length conversion happens in ArraySetLength, which re-reads
        // the current length descriptor after both numeric conversions.
        let cached = o.borrow().own_prop(&key).cloned();
        if let Some(property) = &cached {
            if let Some(setter) = &property.setter {
                let setter = setter.clone();
                self.call_value(&setter, JsValue::Object(o.clone()), vec![x])?;
                return Ok(());
            }
            if !property.writable {
                return Err(Throw::type_error("cannot assign to a read-only property"));
            }
            if property.is_accessor() {
                return Err(Throw::type_error("property has no setter"));
            }
        } else if !o.borrow().extensible {
            return Err(Throw::type_error("object is not extensible"));
        }
        if matches!(o.borrow().exotic, Exotic::Array) && (k == "length" || array_index_key(&key).is_some()) {
            let mut fields = StdDescriptorFields { value: Some(x), ..StdDescriptorFields::default() };
            if cached.is_none() {
                fields.writable = Some(true);
                fields.enumerable = Some(true);
                fields.configurable = Some(true);
            }
            return if self.array_define_fields(&o, &key, fields)? { Ok(()) }
                else { Err(Throw::type_error("cannot define Array property")) };
        }
        if let Some(mut property) = cached {
            property.value = x;
            o.borrow_mut().define(&key, property).unwrap();
        } else {
            o.borrow_mut().set_own(&key, x);
        }
        Ok(())
    }
    fn enumerable_keys(&mut self, v: &JsValue) -> Result<Vec<crate::value::JsString>, Throw> {
        if let JsValue::Object(object) = v {
            if self.is_window_proxy(object) {
                let mut keys = Vec::new();
                for key in self.own_property_keys(v)? {
                    if let PropertyKey::String(name) = &key {
                        if self.window_proxy_own_property(object, &key).unwrap()?.is_some_and(|p| p.enumerable) {
                            keys.push(name.clone());
                        }
                    }
                }
                return Ok(keys);
            }
            if self.proxy_parts(object).is_some() {
                let mut keys = Vec::new();
                for key in self.own_property_keys(v)? {
                    if let PropertyKey::String(name) = &key {
                        if self.default_get_own_property(object, &key)?.is_some_and(|p| p.enumerable) {
                            keys.push(name.clone());
                        }
                    }
                }
                return Ok(keys);
            }
            if is_module_namespace(object) {
                let mut keys = Vec::new();
                for key in self.own_property_keys(v)? {
                    if let PropertyKey::String(name) = &key {
                        if self.namespace_own_property(object, &key).unwrap()?.is_some_and(|p| p.enumerable) {
                            keys.push(name.clone());
                        }
                    }
                }
                return Ok(keys);
            }
        }
        match v {
            JsValue::Object(o) if binary_own_keys(o).is_some() => Ok(binary_own_keys(o).unwrap().into_iter().filter_map(|key| match key {
                PropertyKey::String(name) if binary_index_descriptor(o, &PropertyKey::String(name.clone())).flatten().is_some()
                    || o.borrow().own_prop(&name).is_some_and(|p| p.enumerable) => Some(name), _ => None,
            }).collect()),
            JsValue::Object(o) => self.legacy_own_keys(o, true),
            JsValue::Str(s) => Ok((0..s.len()).map(|i| i.to_string().into()).collect()),
            _ => Ok(Vec::new()),
        }
    }
    fn next_for_in_key(&mut self, state: &mut ForInState) -> Result<Option<crate::value::JsString>, Throw> {
        while let Some(object) = state.object.clone() {
            if !state.object_was_visited {
                state.remaining_keys = self.own_property_keys(&JsValue::Object(object.clone()))?
                    .into_iter().filter_map(|key| match key {
                        PropertyKey::String(name) => Some(name),
                        PropertyKey::Symbol(_) => None,
                    }).collect();
                state.object_was_visited = true;
            }
            while let Some(key) = state.remaining_keys.pop_front() {
                if state.visited_keys.contains(&key) { continue; }
                let property = if let Some(result) = self.window_proxy_own_property(&object, &PropertyKey::String(key.clone())) { result? }
                    else if let Some(result) = self.proxy_get_own_property(&object, &PropertyKey::String(key.clone())) { result? }
                    else if let Some(result) = self.namespace_own_property(&object, &PropertyKey::String(key.clone())) { result? }
                    else { match binary_index_descriptor(&object, &PropertyKey::String(key.clone())) {
                    Some(descriptor) => descriptor,
                    None => self.legacy_own_property(&object, key.clone(), false)?,
                }};
                if let Some(property) = property {
                    state.visited_keys.insert(key.clone());
                    if property.enumerable { return Ok(Some(key)); }
                }
            }
            state.object = if let Some(result) = self.window_proxy_prototype(&object) {
                match result? { JsValue::Object(proto) => Some(proto), _ => None }
            } else if let Some(result) = self.proxy_get_prototype(&object) {
                match result? { JsValue::Object(proto) => Some(proto), _ => None }
            } else { object.borrow().proto.clone() };
            state.object_was_visited = false;
        }
        Ok(None)
    }
    fn own_property_keys(&mut self, v: &JsValue) -> Result<Vec<PropertyKey>, Throw> {
        if let JsValue::Object(object) = v {
            if let Some(target) = self.window_proxy_target(object, WindowProxyOperation::OwnKeys, None) {
                let (realm, global) = target?;
                return self.binding_with_realm(realm, |i| i.own_property_keys(&JsValue::Object(global)));
            }
            if let Some(result) = self.proxy_own_keys(object) { return result; }
            if let Some(result)=self.namespace_keys(object) {return result;}
        }
        match v {
            JsValue::Object(object) if binary_own_keys(object).is_some() => Ok(binary_own_keys(object).unwrap()),
            JsValue::Object(object) => {
                let strings = self.legacy_own_keys(object, false)?
                    .into_iter().map(PropertyKey::String);
                let symbols = object.borrow().own_property_keys().into_iter()
                    .filter(|key| matches!(key, PropertyKey::Symbol(_)));
                Ok(strings.chain(symbols).collect())
            }
            JsValue::Str(value) => {
                let mut keys: Vec<_> = (0..value.len())
                    .map(|index| PropertyKey::from(index.to_string())).collect();
                keys.push(PropertyKey::from("length"));
                Ok(keys)
            }
            _ => Ok(Vec::new()),
        }
    }
    fn enumerable_property_keys(&mut self, v: &JsValue) -> Result<Vec<PropertyKey>, Throw> {
        if let JsValue::Object(object) = v {
            if self.is_window_proxy(object) {
                let mut keys = Vec::new();
                for key in self.own_property_keys(v)? {
                    if self.window_proxy_own_property(object, &key).unwrap()?.is_some_and(|p| p.enumerable) {
                        keys.push(key);
                    }
                }
                return Ok(keys);
            }
            if self.proxy_parts(object).is_some() {
                let mut keys = Vec::new();
                for key in self.own_property_keys(v)? {
                    if self.default_get_own_property(object, &key)?.is_some_and(|p| p.enumerable) {
                        keys.push(key);
                    }
                }
                return Ok(keys);
            }
        }
        let mut keys: Vec<_> = self.enumerable_keys(v)?.into_iter()
            .map(PropertyKey::String).collect();
        if let JsValue::Object(object) = v {
            keys.extend(object.borrow().own_property_keys().into_iter().filter(|key| {
                matches!(key, PropertyKey::Symbol(_))
                    && object.borrow().own_prop(key).is_some_and(|property| property.enumerable)
            }));
        }
        Ok(keys)
    }
}
