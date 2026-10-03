impl Interpreter {
    fn get_prop(&mut self, v: &JsValue, k: impl Into<PropertyKey>) -> Result<JsValue, Throw> {
        self.get_prop_receiver(v, &k.into(), v.clone())
    }

    fn get_prop_receiver(
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
                        Some(property) => match property.getter {
                            Some(getter) => self.call_value(&getter, receiver, Vec::new()),
                            None => Ok(property.value),
                        },
                        None => Ok(JsValue::Undefined),
                    };
                }
                return self.binding_with_realm(realm, |i| i.get_prop_receiver(&JsValue::Object(global), key, receiver));
            }
            if let Some(result) = self.proxy_get(object, key, receiver.clone()) { return result; }
        }
        let text = key.as_string().map(|name| name.to_utf8_lossy());
        let k = text.as_deref().unwrap_or("");
        match v {
            JsValue::Object(o) => {
                if let Exotic::ModuleNamespace(exports) = &o.borrow().exotic {
                    if let Some(name) = key.as_string() {
                        return match exports.get(&name.to_utf8_lossy()).cloned() {
                            Some(cell) => cell.read(), None => Ok(JsValue::Undefined),
                        };
                    }
                }
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
                    if let Some(getter) = property.getter {
                        self.call_value(&getter, receiver, Vec::new())
                    } else {
                        Ok(property.value)
                    }
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
        if matches!(o.borrow().exotic, Exotic::ModuleNamespace(_)) {
            return Err(Throw::type_error("module namespace properties cannot be assigned"));
        }
        if let Some(result) = binary_set_index(self, &o, &key, x.clone()) { return result; }
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
                    if let Some(setter) = descriptor.setter {
                        self.call_value(&setter, JsValue::Object(o.clone()), vec![x])?;
                        return Ok(());
                    }
                    if descriptor.getter.is_some() || !descriptor.writable {
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
        // JSRT-21: fetch the own property once and reuse it below, instead of
        // re-fetching the identical key right after (the old code called
        // `own_prop(&key)` twice in a row with nothing between the two calls
        // that could mutate `o`, on every single indexed set/push). The one
        // path that CAN mutate `o` between the fetch and the write is the
        // `k == "length"` ToNumber conversion below (a hostile valueOf could
        // delete/redefine "length" as a side effect), so that path alone
        // re-fetches after the conversion rather than trusting the cache.
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
            if property.getter.is_some() {
                return Err(Throw::type_error("property has no setter"));
            }
        } else if !o.borrow().extensible {
            return Err(Throw::type_error("object is not extensible"));
        }
        let is_length_set = k == "length" && matches!(o.borrow().exotic, Exotic::Array);
        if is_length_set {
            let n = self.to_number(&x)?;
            if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
                return Err(Throw::range_error("invalid array length"));
            }
        }
        let existing = if is_length_set { o.borrow().own_prop(&key).cloned() } else { cached };
        if let Some(mut property) = existing {
            property.value = x;
            o.borrow_mut().define(&key, property).unwrap();
        } else {
            o.borrow_mut().set_own(&key, x);
        }
        if let Ok(n) = k.parse::<usize>() {
            if matches!(o.borrow().exotic, Exotic::Array) {
                let mut length_property = o.borrow().own_prop("length").cloned().unwrap();
                let len = match length_property.value {
                    JsValue::Num(n) => n,
                    _ => 0.0,
                };
                if n as f64 + 1.0 > len {
                    length_property.value = JsValue::num(n as f64 + 1.0);
                    o.borrow_mut().define("length", length_property).unwrap();
                }
            }
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
