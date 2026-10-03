// Portions ported from lucid-softworks/lumen (crates/lumen/src/modules.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Module namespace operations adapted to live cells and deferred evaluation callbacks.
fn is_module_namespace(object: &Obj) -> bool {
    matches!(
        object.borrow().exotic,
        Exotic::ModuleNamespace(_) | Exotic::DeferredModuleNamespace(_)
    )
}
impl Interpreter {
    fn namespace_exports(
        &mut self,
        object: &Obj,
        key: Option<&PropertyKey>,
    ) -> Option<Result<std::collections::BTreeMap<String, crate::env::BindingCell>, Throw>> {
        let (exports, evaluate) = match &object.borrow().exotic {
            Exotic::ModuleNamespace(exports) => (exports.clone(), None),
            Exotic::DeferredModuleNamespace(state) => {
                if key.is_some_and(|key| {
                    matches!(key, PropertyKey::Symbol(_))
                        || key
                            .as_string()
                            .is_some_and(|name| name.to_utf8_lossy() == "then")
                }) {
                    return None;
                }
                (state.exports.clone(), Some(state.evaluate.clone()))
            }
            _ => return None,
        };
        if key.is_some_and(|key| matches!(key, PropertyKey::Symbol(_))) {
            return None;
        }
        Some(match evaluate {
            Some(evaluate) => evaluate(self).map(|_| exports),
            None => Ok(exports),
        })
    }
    fn namespace_get(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<JsValue, Throw>> {
        self.namespace_exports(object, Some(key)).map(|result| {
            result.and_then(|exports| {
                let name = key.as_string().unwrap().to_utf8_lossy();
                match exports.get(&name) {
                    Some(cell) => cell.read(),
                    None => Ok(JsValue::Undefined),
                }
            })
        })
    }
    fn namespace_own_property(
        &mut self,
        object: &Obj,
        key: &PropertyKey,
    ) -> Option<Result<Option<Property>, Throw>> {
        self.namespace_exports(object, Some(key)).map(|result| {
            result.and_then(|exports| {
                exports
                    .get(&key.as_string().unwrap().to_utf8_lossy())
                    .map(|cell| {
                        cell.read().map(|value| Property {
                            accessor: false,
                            value,
                            writable: true,
                            enumerable: true,
                            configurable: false,
                            getter: None,
                            setter: None,
                        })
                    })
                    .transpose()
            })
        })
    }
    fn namespace_has(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        self.namespace_exports(object, Some(key)).map(|result| {
            result.map(|exports| exports.contains_key(&key.as_string().unwrap().to_utf8_lossy()))
        })
    }
    fn namespace_delete(&mut self, object: &Obj, key: &PropertyKey) -> Option<Result<bool, Throw>> {
        self.namespace_has(object, key)
            .map(|result| result.map(|exists| !exists))
    }
    fn namespace_keys(&mut self, object: &Obj) -> Option<Result<Vec<PropertyKey>, Throw>> {
        self.namespace_exports(object, None).map(|result| {
            result.map(|exports| {
                let mut strings: Vec<crate::value::JsString> =
                    exports.keys().map(|name| name.clone().into()).collect();
                strings.sort();
                let mut keys: Vec<_> = strings.into_iter().map(PropertyKey::String).collect();
                keys.extend(
                    object
                        .borrow()
                        .own_property_keys()
                        .into_iter()
                        .filter(|key| matches!(key, PropertyKey::Symbol(_))),
                );
                keys
            })
        })
    }
}
fn namespace_define_fields(
    i: &mut Interpreter,
    target: &Obj,
    key: &PropertyKey,
    fields: &StdDescriptorFields,
) -> Result<bool, Throw> {
    // [[GetOwnProperty]] must run even when the proposed descriptor will fail.
    let current = match i.namespace_own_property(target, key) {
        Some(result) => result?,
        None => target.borrow().own_prop(key).cloned(),
    };
    let Some(current) = current else {
        return Ok(false);
    };
    if fields.get.is_some()
        || fields.set.is_some()
        || fields.configurable == Some(true)
        || fields
            .enumerable
            .is_some_and(|value| value != current.enumerable)
        || fields
            .writable
            .is_some_and(|value| value != current.writable)
    {
        return Ok(false);
    }
    Ok(fields
        .value
        .as_ref()
        .is_none_or(|value| std_same_value(value, &current.value)))
}
