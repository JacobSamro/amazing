//! Class definition and private element evaluation.
use super::*;
use crate::value::{
    ClassFieldDefinition, ClassFieldKey, ClassInstanceElements, PrivateElement,
};

enum StaticWork {
    Field(ClassFieldDefinition),
    Block(ast::Block),
}

fn add_private_method(slots: &mut Vec<(JsSymbol, PrivateElement)>, id: JsSymbol,
    kind: ast::MethodKind, value: JsValue)
{
    if let Some((_, PrivateElement::Accessor { getter, setter })) =
        slots.iter_mut().find(|(name, _)| name == &id)
    {
        if kind == ast::MethodKind::Get { *getter = Some(value); }
        else { *setter = Some(value); }
        return;
    }
    let element = match kind {
        ast::MethodKind::Get => PrivateElement::Accessor { getter: Some(value), setter: None },
        ast::MethodKind::Set => PrivateElement::Accessor { getter: None, setter: Some(value) },
        _ => PrivateElement::Method(value),
    };
    slots.push((id, element));
}

fn private_binding(name: &str) -> String {
    format!("\0class-private:{name}")
}

impl Interpreter {
    pub(super) fn private_name(&self, env: &EnvHandle, name: &str) -> Result<JsSymbol, Throw> {
        match env.resolve(&private_binding(name)) {
            Some(JsValue::Symbol(id)) => Ok(id),
            _ => Err(Throw::syntax_error(format!("unknown private name {name}"))),
        }
    }

    fn private_element(&self, receiver: &JsValue, id: &JsSymbol) -> Result<PrivateElement, Throw> {
        let JsValue::Object(object) = receiver else {
            return Err(Throw::type_error("private member receiver is not an object"));
        };
        object.borrow().private_elements.iter()
            .find(|(name, _)| name == id)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| Throw::type_error("private member is not present on this object"))
    }

    pub(super) fn private_get(&mut self, receiver: &JsValue, id: &JsSymbol) -> Result<JsValue, Throw> {
        match self.private_element(receiver, id)? {
            PrivateElement::Field(value) | PrivateElement::Method(value) => Ok(value),
            PrivateElement::Accessor { getter: Some(getter), .. } =>
                self.call_value(&getter, receiver.clone(), Vec::new()),
            PrivateElement::Accessor { getter: None, .. } =>
                Err(Throw::type_error("private accessor has no getter")),
        }
    }

    pub(super) fn private_set(&mut self, receiver: &JsValue, id: &JsSymbol, value: JsValue) -> Result<(), Throw> {
        match self.private_element(receiver, id)? {
            PrivateElement::Field(_) => {
                let JsValue::Object(object) = receiver else { unreachable!() };
                let mut object = object.borrow_mut();
                let (_, slot) = object.private_elements.iter_mut()
                    .find(|(name, _)| name == id).expect("checked private slot");
                *slot = PrivateElement::Field(value);
                Ok(())
            }
            PrivateElement::Method(_) => Err(Throw::type_error("private method is read-only")),
            PrivateElement::Accessor { setter: Some(setter), .. } => {
                self.call_value(&setter, receiver.clone(), vec![value])?;
                Ok(())
            }
            PrivateElement::Accessor { setter: None, .. } =>
                Err(Throw::type_error("private accessor has no setter")),
        }
    }

    pub(super) fn private_has(&self, receiver: &JsValue, id: &JsSymbol) -> Result<bool, Throw> {
        let JsValue::Object(object) = receiver else {
            return Err(Throw::type_error("right operand of private in is not an object"));
        };
        Ok(object.borrow().private_elements.iter().any(|(name, _)| name == id))
    }

    fn add_private_element(&self, receiver: &JsValue, id: JsSymbol, element: PrivateElement) -> Result<(), Throw> {
        let JsValue::Object(object) = receiver else { unreachable!() };
        let mut object = object.borrow_mut();
        if object.private_elements.iter().any(|(name, _)| name == &id) {
            return Err(Throw::type_error("private element already initialized"));
        }
        object.private_elements.push((id, element));
        Ok(())
    }

    fn class_element_key(&mut self, key: &ast::PropKey, utf16: &Option<Vec<u16>>, env: &EnvHandle)
        -> Result<ClassFieldKey, Throw>
    {
        match key {
            ast::PropKey::Private(name) => Ok(ClassFieldKey::Private(self.private_name(env, &name.name)?)),
            _ => Ok(ClassFieldKey::Public(match utf16 {
                Some(units) => PropertyKey::String(crate::value::JsString::from_units(units.clone())),
                None => self.prop_key(key, env)?,
            })),
        }
    }

    fn class_method(&self, method: &ast::ClassMethod, env: &EnvHandle, home: &Obj,
        key: &ClassFieldKey) -> JsValue
    {
        let value = self.method_value(method, env, true);
        let JsValue::Object(function) = &value else { unreachable!() };
        let name = match key {
            ClassFieldKey::Public(key) => match key {
                PropertyKey::String(name) => name.to_utf8_lossy(),
                PropertyKey::Symbol(symbol) => symbol.descriptive_string(),
            },
            ClassFieldKey::Private(name) => name.description()
                .map(|description| description.to_utf8_lossy()).unwrap_or_default(),
        };
        let name = match method.kind {
            ast::MethodKind::Get => format!("get {name}"),
            ast::MethodKind::Set => format!("set {name}"),
            _ => name,
        };
        let length = method.params.iter().take_while(|param|
            !param.rest && param.default.is_none()).count();
        {
            let mut function = function.borrow_mut();
            function.func.as_mut().unwrap().home = Some(home.clone());
            function.func.as_mut().unwrap().name = name.clone();
            function.constructible = false;
            function.define("name", Property {
                value: JsValue::str(name), writable: false, enumerable: false,
                configurable: true, getter: None, setter: None,
            }).expect("method name property");
            function.define("length", Property {
                value: JsValue::num(length as f64), writable: false, enumerable: false,
                configurable: true, getter: None, setter: None,
            }).expect("method length property");
        }
        value
    }

    pub(super) fn define_class_method(&mut self, target: &JsValue, key: &PropertyKey,
        kind: ast::MethodKind, value: JsValue) -> Result<(), Throw>
    {
        let JsValue::Object(object) = target else { unreachable!() };
        let mut descriptor = object.borrow().own_prop(key).cloned().unwrap_or(Property {
            value: JsValue::Undefined, writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        if matches!(kind, ast::MethodKind::Get | ast::MethodKind::Set) {
            descriptor.writable = false;
            if kind == ast::MethodKind::Get { descriptor.getter = Some(value); }
            else { descriptor.setter = Some(value); }
        } else {
            descriptor = Property {
                value, writable: true, enumerable: false, configurable: true,
                getter: None, setter: None,
            };
        }
        object.borrow_mut().define(key, descriptor)
            .map_err(|_| Throw::type_error("could not define class method"))
    }

    fn class_initializer(&mut self, expression: &ast::Expr, env: &EnvHandle,
        receiver: &JsValue, home: &Obj) -> Result<JsValue, Throw>
    {
        let field_env = EnvHandle::new(EnvKind::Function, Some(env.clone()));
        field_env.bind_this(receiver.clone());
        self.frames.push(CallFrame {
            this: receiver.clone(), home: Some(home.clone()),
            super_constructor: None, new_target: None, class_constructor: None,
        });
        let result = self.eval_expr(expression, &field_env);
        self.frames.pop();
        result
    }

    fn define_class_field(&mut self, receiver: &JsValue, field: &ClassFieldDefinition,
        env: &EnvHandle, home: &Obj) -> Result<(), Throw>
    {
        let mut value = match &field.initializer {
            Some(expression) => self.class_initializer(expression, env, receiver, home)?,
            None => JsValue::Undefined,
        };
        if field.initializer.is_some() {
            let name = match &field.key {
                ClassFieldKey::Public(PropertyKey::String(name)) => name.to_utf8_lossy(),
                ClassFieldKey::Public(PropertyKey::Symbol(symbol)) => symbol.descriptive_string(),
                ClassFieldKey::Private(symbol) => format!("#{}", symbol.description()
                    .map(|name| name.to_utf8_lossy()).unwrap_or_default()),
            };
            self.infer_function_name(&mut value, &name);
        }
        match &field.key {
            ClassFieldKey::Private(id) =>
                self.add_private_element(receiver, id.clone(), PrivateElement::Field(value)),
            ClassFieldKey::Public(key) => {
                let JsValue::Object(object) = receiver else { unreachable!() };
                object.borrow_mut().define(key, Property::data(value))
                    .map_err(|_| Throw::type_error("could not define class field"))
            }
        }
    }

    pub(super) fn initialize_instance_elements(&mut self, constructor: &Obj, receiver: &JsValue) -> Result<(), Throw> {
        let Some(elements) = constructor.borrow().class_instance_elements.clone() else {
            return Ok(());
        };
        for (id, element) in &elements.methods {
            self.add_private_element(receiver, id.clone(), element.clone())?;
        }
        for field in &elements.fields {
            self.define_class_field(receiver, field, &elements.env, &elements.home)?;
        }
        Ok(())
    }

    pub(super) fn eval_class(&mut self, class: &ast::ClassItem, outer: &EnvHandle)
        -> Result<JsValue, Throw>
    {
        let class_env = EnvHandle::new(EnvKind::Block, Some(outer.clone()));
        let mut private_names = Vec::<String>::new();
        for member in &class.members {
            let key = match member {
                ast::ClassMember::Method(method) => Some(&method.key),
                ast::ClassMember::Field(field) => Some(&field.key),
                ast::ClassMember::StaticBlock(_) => None,
            };
            if let Some(ast::PropKey::Private(name)) = key {
                if !private_names.contains(&name.name) {
                    private_names.push(name.name.clone());
                    class_env.declare_lexical(&private_binding(&name.name),
                        JsValue::Symbol(JsSymbol::new(Some(name.name.clone().into()))), false)?;
                }
            }
        }
        let super_class = class.super_class.as_ref()
            .map(|expression| self.eval_expr(expression, outer)).transpose()?;
        let super_proto = match &super_class {
            None => Some(self.object_proto.clone()),
            Some(JsValue::Null) => None,
            Some(JsValue::Object(parent)) => match parent.borrow().get_own("prototype") {
                Some(JsValue::Object(proto)) => Some(proto),
                Some(JsValue::Null) => None,
                _ => return Err(Throw::type_error("super constructor has no prototype")),
            },
            _ => return Err(Throw::type_error("super class is not a constructor")),
        };
        let constructor = class.members.iter().find_map(|member| match member {
            ast::ClassMember::Method(method) if method.kind == ast::MethodKind::Constructor => Some(method),
            _ => None,
        });
        let value = if let Some(method) = constructor {
            self.method_value(method, &class_env, true)
        } else {
            self.make_function(&ast::FunctionItem {
                name: class.name.clone(), params: Vec::new(),
                body: ast::Block { body: Vec::new(), span: crate::tokens::Span::new(
                    crate::tokens::SourceLocation::new(1, 1),
                    crate::tokens::SourceLocation::new(1, 1),
                ) },
                is_async: false, is_generator: false,
            }, &class_env)
        };
        let JsValue::Object(function) = &value else { unreachable!() };
        let prototype = Obj(Rc::new(RefCell::new({
            let mut object = JsObject::ordinary();
            object.proto = super_proto;
            object
        })));
        function.borrow_mut().define("prototype", Property {
            value: JsValue::Object(prototype.clone()), writable: false,
            enumerable: false, configurable: false, getter: None, setter: None,
        }).map_err(|_| Throw::type_error("could not define class prototype"))?;
        prototype.borrow_mut().define("constructor", Property {
            value: value.clone(), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).map_err(|_| Throw::type_error("could not define class constructor"))?;
        {
            let mut function = function.borrow_mut();
            function.class_constructor = true;
            function.func.as_mut().unwrap().proto_for_new = Some(prototype.clone());
            function.func.as_mut().unwrap().home = Some(prototype.clone());
            if let Some(parent) = &super_class {
                if let JsValue::Object(parent_object) = parent {
                    function.proto = Some(parent_object.clone());
                }
                function.derived_constructor = Some(DerivedConstructor {
                    parent: parent.clone(), implicit: constructor.is_none(),
                });
            }
        }
        if let Some(name) = &class.name {
            class_env.declare_lexical(&name.name, value.clone(), false)?;
            function.borrow_mut().func.as_mut().unwrap().name = name.name.clone();
            function.borrow_mut().define("name", Property {
                value: JsValue::str(&name.name), writable: false, enumerable: false,
                configurable: true, getter: None, setter: None,
            }).expect("class name property");
        }
        function.borrow_mut().define("length", Property {
            value: JsValue::num(constructor.map_or(0, |method| method.params.iter()
                .take_while(|param| !param.rest && param.default.is_none()).count()) as f64),
            writable: false, enumerable: false, configurable: true,
            getter: None, setter: None,
        }).expect("class length property");
        let mut instance_fields = Vec::new();
        let mut instance_methods = Vec::<(JsSymbol, PrivateElement)>::new();
        let mut static_work = Vec::new();
        for member in &class.members {
            match member {
                ast::ClassMember::Method(method) if method.kind == ast::MethodKind::Constructor => {}
                ast::ClassMember::Method(method) => {
                    let key = self.class_element_key(&method.key, &method.key_utf16, &class_env)?;
                    let home = if method.is_static { function } else { &prototype };
                    let target = if method.is_static { value.clone() }
                        else { JsValue::Object(prototype.clone()) };
                    let method_value = self.class_method(method, &class_env, home, &key);
                    match key {
                        ClassFieldKey::Public(key) => self.define_class_method(&target, &key, method.kind, method_value)?,
                        ClassFieldKey::Private(id) => {
                            if method.is_static {
                                add_private_method(&mut function.borrow_mut().private_elements,
                                    id, method.kind, method_value);
                            } else {
                                add_private_method(&mut instance_methods, id, method.kind, method_value);
                            }
                        }
                    }
                }
                ast::ClassMember::Field(field) => {
                    let key = self.class_element_key(&field.key, &field.key_utf16, &class_env)?;
                    let definition = ClassFieldDefinition {
                        key, initializer: field.init.clone(), accessor: field.is_accessor,
                    };
                    if field.is_static { static_work.push(StaticWork::Field(definition)); }
                    else { instance_fields.push(definition); }
                }
                ast::ClassMember::StaticBlock(block) => static_work.push(StaticWork::Block(block.clone())),
            }
        }
        function.borrow_mut().class_instance_elements = Some(ClassInstanceElements {
            fields: instance_fields, methods: instance_methods, env: class_env.clone(), home: prototype,
        });
        for work in static_work {
            match work {
                StaticWork::Field(field) =>
                    self.define_class_field(&value, &field, &class_env, function)?,
                StaticWork::Block(block) => {
                    let block_env = EnvHandle::new(EnvKind::Function, Some(class_env.clone()));
                    block_env.bind_this(value.clone());
                    self.frames.push(CallFrame {
                        this: value.clone(), home: Some(function.clone()),
                        super_constructor: None, new_target: None, class_constructor: None,
                    });
                    let result = self.hoist(&block.body, &block_env)
                        .and_then(|()| self.exec_stmts(&block.body, &block_env));
                    self.frames.pop();
                    result?;
                }
            }
        }
        Ok(value)
    }
}
