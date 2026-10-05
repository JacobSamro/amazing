// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Rooted phases of ClassDefinitionEvaluation shared by AST and resumable VM.

impl Interpreter {
    fn prepare_class_elements(
        &mut self, state: &mut ClassEvaluation, superclass: Option<ClassHeritageOperand>,
    ) -> Result<EnvHandle, Throw> {
        if !matches!(state.phase, ClassEvaluationPhase::Heritage) {
            return Err(Throw::type_error("invalid class element preparation phase"));
        }
        let class = state.class.clone();
        let class_env = state.class_env.clone().ok_or_else(|| Throw::type_error("missing class environment"))?;
        let inferred_name = state.inferred_name.as_ref();
        // The driver has restored the outer lexical/private environment.
        // Only now consume the heritage Reference; its captured binding/key
        // still belongs to the environment where it was evaluated.
        let superclass = superclass.map(|operand| match operand {
            ClassHeritageOperand::Value(value) => Ok(value),
            ClassHeritageOperand::Reference(mut reference) => self.reference_value(&mut reference),
        }).transpose()?;
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
            self.make_default_class_constructor(&class, &class_env)
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
            function.source = class.source.clone();
            function.home = Some(prototype.clone());
            function.proto_for_new = Some(prototype.clone());
            function.name = class_name.to_utf8_lossy();
            object.define("name", Property { accessor: false, value: JsValue::str(class_name.clone()), writable: false, enumerable: false, configurable: true, getter: None, setter: None }).unwrap();
        }
        state.original = Some(original);
        state.prototype = Some(prototype);
        state.class_name = class_name;
        state.phase = if class.members.is_empty() { ClassEvaluationPhase::Finish }
            else { ClassEvaluationPhase::Elements { next: 0 } };
        Ok(class_env)
    }

    fn class_evaluation_key(
        &mut self, key: &ast::PropKey, units: &Option<Vec<u16>>, env: &EnvHandle,
        is_static: bool, computed: Option<PropertyKey>,
    ) -> Result<FieldKey, Throw> {
        let key = if matches!(key, ast::PropKey::Computed(_)) {
            FieldKey::Public(computed.ok_or_else(|| Throw::type_error("missing computed class key"))?)
        } else {
            self.class_element_key(key, units, env, is_static)?
        };
        if is_static && matches!(&key, FieldKey::Public(PropertyKey::String(name)) if *name == "prototype") {
            return Err(Throw::type_error("static class elements cannot be named prototype"));
        }
        Ok(key)
    }

    fn append_class_element(
        &mut self, state: &mut ClassEvaluation, index: usize, computed_key: Option<PropertyKey>,
    ) -> Result<(), Throw> {
        if !matches!(state.phase, ClassEvaluationPhase::Elements { next } if next == index) {
            return Err(Throw::type_error("invalid class element phase"));
        }
        let class = state.class.clone();
        let class_env = state.class_env.clone().ok_or_else(|| Throw::type_error("missing class environment"))?;
        let original = state.original.clone().ok_or_else(|| Throw::type_error("missing class constructor"))?;
        let JsValue::Object(object) = &original else { unreachable!("class constructor") };
        let prototype = state.prototype.clone().ok_or_else(|| Throw::type_error("missing class prototype"))?;
        let member = &class.members[index];
            match member {
                ast::ClassMember::Method(method) if method.kind != ast::MethodKind::Constructor => {
                    let decorators = std::mem::take(&mut state.member_decorators);
                    let key = self.class_evaluation_key(&method.key, &method.key_utf16, &class_env, method.is_static, computed_key)?;
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
                    state.definitions.push(ClassDefinitionElement::Element(DecoratedElement {
                        key, storage: None, kind, is_static: method.is_static, decorators, value, get, set,
                        init: None, initializers: Vec::new(), extra_initializers: Vec::new(),
                    }));
                }
                ast::ClassMember::Method(_) => {}
                ast::ClassMember::Field(field) => {
                    let decorators = std::mem::take(&mut state.member_decorators);
                    let key = self.class_evaluation_key(&field.key, &field.key_utf16, &class_env, field.is_static, computed_key)?;
                    let (storage, get, set) = if field.is_accessor {
                        let (storage, get, set) = self.auto_accessor_pair(&key, &class_env);
                        (Some(storage), get, set)
                    } else { (None, JsValue::Undefined, JsValue::Undefined) };
                    state.definitions.push(ClassDefinitionElement::Element(DecoratedElement {
                        key, storage, kind: if field.is_accessor { "accessor" } else { "field" },
                        is_static: field.is_static, decorators, value: JsValue::Undefined, get, set,
                        init: field.init.clone(), initializers: Vec::new(), extra_initializers: Vec::new(),
                    }));
                }
                ast::ClassMember::StaticBlock(block) => state.definitions.push(ClassDefinitionElement::Block(block.clone())),
            }
        state.member_decorators.clear();
        state.phase = if index + 1 == class.members.len() { ClassEvaluationPhase::Finish }
            else { ClassEvaluationPhase::Elements { next: index + 1 } };
        Ok(())
    }

    fn finish_class_evaluation(&mut self, state: ClassEvaluation) -> Result<JsValue, Throw> {
        if !matches!(state.phase, ClassEvaluationPhase::Finish) {
            return Err(Throw::type_error("incomplete class evaluation"));
        }
        let class = state.class;
        let class_env = state.class_env.ok_or_else(|| Throw::type_error("missing class environment"))?;
        let original = state.original.ok_or_else(|| Throw::type_error("missing class constructor"))?;
        let JsValue::Object(object) = &original else { unreachable!("class constructor") };
        let prototype = state.prototype.ok_or_else(|| Throw::type_error("missing class prototype"))?;
        let class_name = state.class_name;
        let decorators = state.decorators;
        let mut definitions = state.definitions;
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

enum ClassHeritageOperand {
    Value(JsValue),
    Reference(Reference),
}

struct ClassEvaluation {
    class: Rc<ast::ClassItem>,
    outer_env: EnvHandle,
    class_env: Option<EnvHandle>,
    inferred_name: Option<crate::value::JsString>,
    decorators: Vec<DecoratorDefinition>,
    member_decorators: Vec<DecoratorDefinition>,
    original: Option<JsValue>,
    prototype: Option<Obj>,
    class_name: crate::value::JsString,
    definitions: Vec<ClassDefinitionElement>,
    phase: ClassEvaluationPhase,
}

enum ClassEvaluationPhase {
    OuterDecorators,
    Heritage,
    Elements { next: usize },
    Finish,
}

impl Interpreter {
    fn class_heritage_operand(&mut self, expression: &ast::Expr, env: &EnvHandle)
        -> Result<ClassHeritageOperand, Throw>
    {
        if let Some(name) = event_handler::identifier_name(expression) {
            return self.name_reference(env, name).map(ClassHeritageOperand::Reference);
        }
        // The existing chain evaluator preserves final member References,
        // canonical keys, grouping and actual super/private receivers. It
        // reads only intermediate links needed to evaluate the final one.
        Ok(match self.eval_chain_reference(expression, env)? {
            ChainEvaluation::ShortCircuit => ClassHeritageOperand::Value(JsValue::Undefined),
            ChainEvaluation::Value(value) => ClassHeritageOperand::Value(value),
            ChainEvaluation::Property { base, receiver, key, super_reference } => {
                ClassHeritageOperand::Reference(if super_reference {
                    Reference::SuperProperty(base, key, receiver)
                } else { Reference::Property(base, key) })
            }
            ChainEvaluation::Private { receiver, name } => {
                ClassHeritageOperand::Reference(Reference::Private(receiver, name))
            }
        })
    }

    fn begin_class_evaluation(
        &self,
        class: Rc<ast::ClassItem>,
        outer: &EnvHandle,
        inferred_name: Option<crate::value::JsString>,
    ) -> Result<ClassEvaluation, Throw> {
        Ok(ClassEvaluation {
            class,
            outer_env: outer.clone(),
            class_env: None,
            inferred_name,
            decorators: Vec::new(),
            member_decorators: Vec::new(),
            original: None,
            prototype: None,
            class_name: "".into(),
            definitions: Vec::new(),
            phase: ClassEvaluationPhase::OuterDecorators,
        })
    }

    fn append_class_decorator(
        &self,
        state: &mut ClassEvaluation,
        member: Option<usize>,
        value: JsValue,
        receiver: JsValue,
    ) -> Result<(), Throw> {
        let definition = DecoratorDefinition { value, receiver };
        match (&state.phase, member) {
            (ClassEvaluationPhase::OuterDecorators, None) => state.decorators.push(definition),
            (ClassEvaluationPhase::Elements { next }, Some(index)) if *next == index => {
                state.member_decorators.push(definition);
            }
            _ => return Err(Throw::type_error("invalid class decorator phase")),
        }
        Ok(())
    }

    fn enter_class_heritage(&self, state: &mut ClassEvaluation) -> Result<EnvHandle, Throw> {
        if !matches!(state.phase, ClassEvaluationPhase::OuterDecorators) {
            return Err(Throw::type_error("invalid class heritage phase"));
        }
        let class_env = EnvHandle::new(EnvKind::Block, Some(state.outer_env.clone()));
        if let Some(name) = &state.class.name {
            class_env.declare_uninitialized(&name.name, false)?;
        }
        // Keep the inherited private namespace through heritage. Fresh class
        // identities are installed only when element evaluation begins.
        state.class_env = Some(class_env.clone());
        state.phase = ClassEvaluationPhase::Heritage;
        Ok(class_env)
    }
}
