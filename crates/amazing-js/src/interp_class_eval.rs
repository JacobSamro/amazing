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
        let decorators = self.evaluate_decorators(&class.decorators, outer_env)?;
        let class_env = EnvHandle::new(EnvKind::Block, Some(outer_env.clone()));
        if let Some(name) = &class.name { class_env.declare_uninitialized(&name.name, false)?; }
        // Heritage uses the enclosing private environment, before class-local brands exist.
        let superclass = class.super_class.as_ref().map(|expression| self.eval_expr(expression, &class_env)).transpose()?;
        let prototype_parent = match &superclass {
            None => Some(self.object_proto.clone()),
            Some(JsValue::Null) => None,
            Some(parent) if class_is_constructor(parent) => match self.get_prop(parent, "prototype")? {
                JsValue::Object(prototype) => Some(prototype),
                JsValue::Null => None,
                _ => return Err(Throw::type_error("super constructor prototype is not an object or null")),
            },
            _ => return Err(Throw::type_error("super class is not a constructor")),
        };
        class_env.set_class_private(collect_private_names(&class.members));
        let constructor = class.members.iter().find_map(|member| match member {
            ast::ClassMember::Method(method) if method.kind == ast::MethodKind::Constructor => Some(method),
            _ => None,
        });
        let original = if let Some(method) = constructor { self.method_value(method, &class_env, true) } else {
            self.make_function(&ast::FunctionItem {
                name: class.name.clone(), params: Vec::new(),
                body: ast::Block { body: Vec::new(), span: crate::tokens::Span::new(crate::tokens::SourceLocation::new(1, 1), crate::tokens::SourceLocation::new(1, 1)) },
                is_async: false, is_generator: false,
            }, &class_env)
        };
        let JsValue::Object(object) = &original else { unreachable!("class constructor") };
        let prototype = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        prototype.borrow_mut().proto = prototype_parent;
        prototype.borrow_mut().define("constructor", symbol_data(original.clone(), true, true)).unwrap();
        let class_name = class.name.as_ref().map(|name| crate::value::JsString::from(name.name.clone()))
            .or_else(|| inferred_name.cloned()).unwrap_or_else(|| "".into());
        {
            let mut object = object.borrow_mut();
            object.class_constructor = true;
            object.constructible = true;
            object.define("prototype", Property { accessor: false, value: JsValue::Object(prototype.clone()), writable: false, enumerable: false, configurable: false, getter: None, setter: None }).unwrap();
            if let Some(parent) = &superclass {
                if let JsValue::Object(parent) = parent { object.proto = Some(parent.clone()); }
                object.derived_constructor = Some(DerivedConstructor { parent: parent.clone(), implicit: constructor.is_none() });
            }
            let function = object.func.as_mut().unwrap();
            function.home = Some(prototype.clone());
            function.proto_for_new = Some(prototype.clone());
            function.name = class_name.to_utf8_lossy();
            object.define("name", Property { accessor: false, value: JsValue::str(class_name.clone()), writable: false, enumerable: false, configurable: true, getter: None, setter: None }).unwrap();
        }
        let mut definitions = Vec::new();
        for member in &class.members {
            match member {
                ast::ClassMember::Method(method) if method.kind != ast::MethodKind::Constructor => {
                    let decorators = self.evaluate_decorators(&method.decorators, &class_env)?;
                    let key = self.class_element_key(&method.key, &method.key_utf16, &class_env, method.is_static)?;
                    let home = if method.is_static { object.clone() } else { prototype.clone() };
                    let mut value = self.method_value(method, &class_env, true);
                    if let JsValue::Object(function) = &value { function.borrow_mut().func.as_mut().unwrap().home = Some(home); }
                    let name = match &key {
                        FieldKey::Public(key) => Self::property_function_name(key),
                        FieldKey::Private(name) => name.text().into(),
                    };
                    Self::name_class_method(&mut value, &name, method.kind, &method.params);
                    let kind = match method.kind { ast::MethodKind::Get => "getter", ast::MethodKind::Set => "setter", _ => "method" };
                    let get = if kind == "getter" { value.clone() } else { JsValue::Undefined };
                    let set = if kind == "setter" { value.clone() } else { JsValue::Undefined };
                    definitions.push(ClassDefinitionElement::Element(DecoratedElement {
                        key, storage: None, kind, is_static: method.is_static, decorators, value, get, set,
                        init: None, initializers: Vec::new(), extra_initializers: Vec::new(),
                    }));
                }
                ast::ClassMember::Method(_) => {}
                ast::ClassMember::Field(field) => {
                    let decorators = self.evaluate_decorators(&field.decorators, &class_env)?;
                    let key = self.class_element_key(&field.key, &field.key_utf16, &class_env, field.is_static)?;
                    let (storage, get, set) = if field.is_accessor {
                        let (storage, get, set) = self.auto_accessor_pair(&key, &class_env);
                        (Some(storage), get, set)
                    } else { (None, JsValue::Undefined, JsValue::Undefined) };
                    definitions.push(ClassDefinitionElement::Element(DecoratedElement {
                        key, storage, kind: if field.is_accessor { "accessor" } else { "field" },
                        is_static: field.is_static, decorators, value: JsValue::Undefined, get, set,
                        init: field.init.clone(), initializers: Vec::new(), extra_initializers: Vec::new(),
                    }));
                }
                ast::ClassMember::StaticBlock(block) => definitions.push(ClassDefinitionElement::Block(block.clone())),
            }
        }
        let mut instance = Vec::new();
        let mut statics = Vec::new();
        let mut instance_extras = Vec::new();
        let mut static_extras = Vec::new();
        // Decorate static methods, instance methods, static fields, instance fields.
        for (is_static, is_field) in [(true, false), (false, false), (true, true), (false, true)] {
            for definition in &mut definitions {
                let ClassDefinitionElement::Element(element) = definition else { continue; };
                if element.is_static != is_static || (element.kind == "field") != is_field { continue; }
                let extras = if is_static { &mut static_extras } else { &mut instance_extras };
                self.apply_element_decorators(element, &class_env, extras)?;
                if !is_field {
                    self.define_decorated_method(element, if is_static { object } else { &prototype }, &mut instance, &mut statics)?;
                }
            }
        }
        // Fields, auto-accessor storage and static blocks retain declaration order.
        for definition in definitions {
            match definition {
                ClassDefinitionElement::Element(element) if matches!(element.kind, "field" | "accessor") => {
                    let name_key = element.storage.as_ref().map(|_| element.key.clone());
                    let key = element.storage.unwrap_or(element.key);
                    if element.is_static {
                        statics.push(StaticElement::Field { key, name_key, init: element.init, initializers: element.initializers, extra_initializers: element.extra_initializers });
                    } else {
                        instance.push(InstanceElement::Field { key, name_key, init: element.init, initializers: element.initializers, extra_initializers: element.extra_initializers });
                    }
                }
                ClassDefinitionElement::Block(block) => statics.push(StaticElement::Block(block)),
                _ => {}
            }
        }
        if !instance.is_empty() || !instance_extras.is_empty() {
            object.borrow_mut().class_elements = Some(Rc::new(ClassElements {
                instance, method_extra_initializers: instance_extras, class_env: class_env.clone(), home: prototype,
            }));
        }
        initialize_static_private_elements(&original, &statics)?;
        let static_home = object.clone();
        let mut value = original;
        let mut class_extras = Vec::new();
        let class_key = FieldKey::Public(PropertyKey::String(class_name));
        for decorator in decorators.iter().rev() {
            let replacement = self.invoke_decorator(decorator, value.clone(), "class", &class_key, false, &class_env, &mut class_extras)?;
            if !replacement.is_undefined() {
                if callable_function(&replacement).is_none() { return Err(Throw::type_error("class decorator must return a function or undefined")); }
                value = replacement;
            }
        }
        if let Some(name) = &class.name { class_env.declare_lexical(&name.name, value.clone(), false)?; }
        run_decorator_initializers(self, &value, &static_extras)?;
        // This follows the replacement, while super keeps the original [[HomeObject]].
        run_static_elements(self, &value, static_home, &class_env, &statics)?;
        run_decorator_initializers(self, &value, &class_extras)?;
        Ok(value)
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
