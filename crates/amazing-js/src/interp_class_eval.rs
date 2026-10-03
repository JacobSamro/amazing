// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Class decorator integration adapted to Amazing's class records and environments.

impl Interpreter {
    fn class_element_key(&mut self, key: &ast::PropKey, units: &Option<Vec<u16>>, env: &EnvHandle,
        is_static: bool) -> Result<FieldKey, Throw> {
        if let ast::PropKey::Private(identifier) = key {
            return Ok(FieldKey::Private(env.resolve_private(&identifier.name).expect("class private name")));
        }
        let key = match units {
            Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
            None => self.prop_key(key, env)?,
        };
        if is_static && matches!(&key, PropertyKey::String(name) if *name == "prototype") {
            return Err(Throw::type_error("static class elements cannot be named prototype"));
        }
        Ok(FieldKey::Public(key))
    }

    fn auto_accessor_pair(&self, key: &FieldKey, env: &EnvHandle) -> (FieldKey, JsValue, JsValue) {
        let name = match key {
            FieldKey::Public(key) => Self::property_function_name(key),
            FieldKey::Private(name) => name.text().into(),
        };
        let backing = PrivateName::new(format!("{} accessor storage", name.to_utf8_lossy()));
        let state = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        state.borrow_mut().set_own("key", JsValue::str(backing.text()));
        state.borrow_mut().set_own("private", JsValue::Bool(true));
        let access_env = EnvHandle::new(EnvKind::Block, Some(env.clone()));
        access_env.set_class_private(std::collections::HashMap::from([("[[DecoratorAccess]]".into(), backing.clone())]));
        let get = decorator_native(self, &format!("get {}", name.to_utf8_lossy()), auto_accessor_get, 0, &state, &access_env);
        let set = decorator_native(self, &format!("set {}", name.to_utf8_lossy()), auto_accessor_set, 1, &state, &access_env);
        (FieldKey::Private(backing), get, set)
    }

    fn define_decorated_method(&mut self, element: &DecoratedElement, target: &Obj,
        instance: &mut Vec<InstanceElement>, statics: &mut Vec<StaticElement>) -> Result<(), Throw> {
        let accessor = matches!(element.kind, "getter" | "setter" | "accessor");
        if let FieldKey::Private(name) = &element.key {
            if element.kind == "accessor" {
                if element.is_static {
                    statics.push(StaticElement::PrivateAccessor { name: name.clone(), get: Some(element.get.clone()), set: Some(element.set.clone()) });
                } else {
                    instance.push(InstanceElement::PrivateAccessor { name: name.clone(), get: Some(element.get.clone()), set: Some(element.set.clone()) });
                }
            } else {
                let getter = element.kind == "getter";
                let value = if getter { element.get.clone() } else if element.kind == "setter" { element.set.clone() } else { element.value.clone() };
                if element.is_static {
                    upsert_static_private_accessor_or_method(statics, name.clone(), accessor, getter, value);
                } else {
                    upsert_instance_private_accessor_or_method(instance, name.clone(), accessor, getter, value);
                }
            }
            return Ok(());
        }
        let FieldKey::Public(key) = &element.key else { unreachable!() };
        let property = if accessor {
            let mut property = target.borrow().own_prop(key).filter(|property| property.is_accessor()).cloned().unwrap_or(Property {
                accessor: false,
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true, getter: None, setter: None,
            });
            property.accessor = true;
            if matches!(element.kind, "getter" | "accessor") { property.getter = Some(element.get.clone()); }
            if matches!(element.kind, "setter" | "accessor") { property.setter = Some(element.set.clone()); }
            property
        } else { symbol_data(element.value.clone(), true, true) };
        target.borrow_mut().define(key, property).map_err(|_| Throw::type_error("could not define class method"))?;
        Ok(())
    }

    fn eval_class_inner(&mut self, class: &ast::ClassItem, outer_env: &EnvHandle,
        inferred_name: Option<&crate::value::JsString>) -> Result<JsValue, Throw> {
        let mut state = self.begin_class_evaluation(Rc::new(class.clone()), outer_env, inferred_name.cloned())?;
        for decorator in &class.decorators {
            let (value, receiver) = self.chain_callee(decorator, outer_env)?
                .unwrap_or((JsValue::Undefined, JsValue::Undefined));
            self.append_class_decorator(&mut state, None, value, receiver)?;
        }
        let class_env = self.enter_class_heritage(&mut state)?;
        let superclass = class.super_class.as_ref()
            .map(|expression| self.class_heritage_operand(expression, &class_env)).transpose()?;
        let class_env = self.prepare_class_elements(&mut state, superclass)?;
        for (index, member) in class.members.iter().enumerate() {
            let (decorators, key) = match member {
                ast::ClassMember::Method(method) if method.kind != ast::MethodKind::Constructor =>
                    (method.decorators.as_slice(), Some(&method.key)),
                ast::ClassMember::Field(field) => (field.decorators.as_slice(), Some(&field.key)),
                _ => (&[][..], None),
            };
            for decorator in decorators {
                let (value, receiver) = self.chain_callee(decorator, &class_env)?
                    .unwrap_or((JsValue::Undefined, JsValue::Undefined));
                self.append_class_decorator(&mut state, Some(index), value, receiver)?;
            }
            let computed = match key {
                Some(key @ ast::PropKey::Computed(_)) => Some(self.prop_key(key, &class_env)?),
                _ => None,
            };
            self.append_class_element(&mut state, index, computed)?;
        }
        self.finish_class_evaluation(state)
    }
}

fn auto_accessor_get(i: &mut Interpreter, this: JsValue, _: Vec<JsValue>) -> Result<JsValue, Throw> {
    let FieldKey::Private(name) = decorator_access_key(i)? else { unreachable!() };
    private_field_get(i, &this, &name)
}

fn auto_accessor_set(i: &mut Interpreter, this: JsValue, args: Vec<JsValue>) -> Result<JsValue, Throw> {
    let FieldKey::Private(name) = decorator_access_key(i)? else { unreachable!() };
    private_field_set(i, &this, &name, args.first().cloned().unwrap_or(JsValue::Undefined))?;
    Ok(JsValue::Undefined)
}

// Fixed-base function metadata may still mark arrows constructible. Follow only
// internal exotic slots: IsConstructor must never observe a prototype getter.
fn class_is_constructor(value: &JsValue) -> bool {
    let mut value = value.clone();
    loop {
        let JsValue::Object(object) = &value else { return false; };
        let next = {
            let object = object.borrow();
            if !object.constructible { return false; }
            let Some(function) = &object.func else { return false; };
            if function.is_arrow || function.is_async || function.is_generator { return false; }
            if let Exotic::Proxy(proxy) = &object.exotic {
                if proxy.revoked.get() { return object.constructible; }
                Some(proxy.target.borrow().clone())
            } else if Interpreter::is_bound_function(&value) {
                function.home.as_ref().and_then(|data| data.borrow().get_own("target"))
            } else { return true; }
        };
        let Some(next) = next else { return false; };
        value = next;
    }
}
