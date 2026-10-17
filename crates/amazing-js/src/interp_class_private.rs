// Portions ported from lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Private class elements (§15.7, §7.3.29-31, §6.2.10) and instance-element
// initialization shared by base and derived constructors. `eval_class_inner`
// in interp.rs builds the per-class `ClassElements`/`StaticElement` list this
// file consumes; the AST parses private names already (`ExprKind::PrivateName`,
// `PropKey::Private`, `MemberProp::Private`) but the tree-walker previously
// rejected every one of them with a placeholder error.
//
// `define_field`'s own NamedEvaluation call is adapted from lucid-softworks/lumen
// (crates/lumen/src/eval.rs, ClassFieldDefinitionEvaluation's SetFunctionName step)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.

/// Runs one field initializer or static block with `this` bound and `home`
/// set for `super` resolution, in a fresh function-scoped child of the
/// class's own captured environment. Mirrors the frame a real method call
/// pushes (`call_value_with_new_target_inner`) without going through the
/// general call machinery, since neither a field initializer nor a static
/// block is itself a callable value.
fn eval_class_element_scope<T>(
    i: &mut Interpreter,
    this: JsValue,
    home: Option<Obj>,
    class_env: &EnvHandle,
    body: impl FnOnce(&mut Interpreter, &EnvHandle) -> Result<T, Throw>,
) -> Result<T, Throw> {
    i.admit_synchronous_call()?;
    let scope = EnvHandle::new(EnvKind::Function, Some(class_env.clone()));
    scope.bind_this(this.clone());
    i.frames.push(CallFrame { context_id: CallFrame::next_context_id(), this, home, super_constructor: None, new_target: None, own_function: None });
    let completion = i.completion.take();
    let result = body(i, &scope);
    i.completion = completion;
    i.frames.pop();
    result
}

/// DefineField (§15.7.10): evaluates the initializer (or Undefined) with
/// `this` = the field's owner, then creates the field as an own property
/// (public) or PrivateElement (private) — never through an inherited setter.
/// NamedEvaluation (§15.7.14 ClassFieldDefinitionEvaluation) supplies an
/// anonymous function/class initializer with the field's own key as its name.
fn define_field(
    i: &mut Interpreter,
    owner: &JsValue,
    home: Option<Obj>,
    class_env: &EnvHandle,
    key: &FieldKey,
    init: &Option<ast::Expr>,
    name_key: &Option<FieldKey>,
    initializers: &[JsValue],
    extra_initializers: &[JsValue],
) -> Result<(), Throw> {
    let name_key = name_key.as_ref().unwrap_or(key);
    let name: crate::value::JsString = match name_key {
        FieldKey::Public(property_key) => Interpreter::property_function_name(property_key),
        // Private identifier text already includes the leading '#'.
        FieldKey::Private(private_name) => private_name.text().into(),
    };
    let mut value = eval_class_element_scope(i, owner.clone(), home, class_env, |i, scope| {
        match init {
            // ClassFieldDefinitionEvaluation's own NamedEvaluation (spec 15.7.10): an
            // anonymous function/class field initializer is named after the field's own
            // key (already resolved to a PropertyKey/PrivateName by class-definition time,
            // per spec -- computed keys are evaluated once, not per instance).
            Some(expr) => {
                // Same class-self-reference-before-eval need as every other NamedEvaluation
                // call site here: `named_evaluation`/`infer_function_name` deliberately skip
                // class_constructor objects (a class's own name is installed before its own
                // static elements run, not after the fact), so an anonymous class field
                // initializer -- `field = class { static seen = this.name; }` -- needs
                // `eval_named_class_expr` to make that name visible *during* evaluation.
                // Class and ordinary function initializer names share the lossless key.
                let mut v = i.eval_named_class_expr(expr, scope, &name)?;
                i.named_evaluation_utf16(expr, &mut v, &name);
                Ok(v)
            }
            None => Ok(JsValue::Undefined),
        }
    })?;
    for initializer in initializers {
        value = i.call_value(initializer, owner.clone(), vec![value])?;
    }
    let JsValue::Object(object) = owner else {
        return Err(Throw::type_error("class field target is not an object"));
    };
    match key {
        FieldKey::Public(property_key) => {
            let mut descriptor = JsObject::ordinary();
            descriptor.set_own("value", value);
            for attribute in ["writable", "enumerable", "configurable"] {
                descriptor.set_own(attribute, JsValue::Bool(true));
            }
            if !i.define_own_property(object, property_key, &JsValue::object(descriptor))? {
                return Err(Throw::type_error("could not define class field"));
            }
        }
        FieldKey::Private(name) => {
            object.borrow_mut().private_add(name.clone(), PrivateElementKind::Field(value))?;
        }
    }
    run_decorator_initializers(i, owner, extra_initializers)
}

/// InitializeInstanceElements (§15.7.11): replays a class's own instance
/// fields and private methods/accessors onto a freshly-constructed `this`,
/// in declaration order. Called once per class level in an inheritance
/// chain, right when that level's own constructor makes `this` available
/// (base-class allocation, or the return of an explicit/implicit `super()`).
fn initialize_instance_elements(
    i: &mut Interpreter,
    this: &JsValue,
    elements: &ClassElements,
) -> Result<(), Throw> {
    let home = Some(elements.home.clone());
    for element in &elements.instance {
        match element {
            InstanceElement::Field { .. } => {}
            InstanceElement::PrivateMethod { name, value } => {
                let JsValue::Object(object) = this else {
                    return Err(Throw::type_error("class instance target is not an object"));
                };
                object.borrow_mut().private_add(name.clone(), PrivateElementKind::Method(value.clone()))?;
            }
            InstanceElement::PrivateAccessor { name, get, set } => {
                let JsValue::Object(object) = this else {
                    return Err(Throw::type_error("class instance target is not an object"));
                };
                object.borrow_mut().private_add(
                    name.clone(),
                    PrivateElementKind::Accessor { get: get.clone(), set: set.clone() },
                )?;
            }
        }
    }
    run_decorator_initializers(i, this, &elements.method_extra_initializers)?;
    for element in &elements.instance {
        if let InstanceElement::Field { key, init, name_key, initializers, extra_initializers } = element {
            define_field(i, this, home.clone(), &elements.class_env, key, init, name_key, initializers, extra_initializers)?;
        }
    }
    Ok(())
}

fn initialize_static_private_elements(constructor: &JsValue, statics: &[StaticElement]) -> Result<(), Throw> {
    let JsValue::Object(object) = constructor else { unreachable!("class constructor") };
    for element in statics {
        match element {
            StaticElement::PrivateMethod { name, value } => object.borrow_mut().private_add(name.clone(), PrivateElementKind::Method(value.clone()))?,
            StaticElement::PrivateAccessor { name, get, set } => object.borrow_mut().private_add(name.clone(), PrivateElementKind::Accessor { get: get.clone(), set: set.clone() })?,
            _ => {}
        }
    }
    Ok(())
}

/// Evaluates a class's static fields, static private methods/accessors, and
/// static blocks once, in declaration order, right after the class's own
/// prototype methods are installed. `this` inside every one of them is the
/// constructor itself; `home` is the constructor (so `super.x` in a static
/// initializer resolves via the parent *constructor*, matching the static
/// side's own prototype chain).
fn run_static_elements(
    i: &mut Interpreter,
    constructor: &JsValue,
    home: Obj,
    class_env: &EnvHandle,
    statics: &[StaticElement],
) -> Result<(), Throw> {
    for element in statics {
        match element {
            StaticElement::Field { key, init, name_key, initializers, extra_initializers } => {
                define_field(i, constructor, Some(home.clone()), class_env, key, init, name_key, initializers, extra_initializers)?;
            }
            StaticElement::PrivateMethod { .. } | StaticElement::PrivateAccessor { .. } => {}
            StaticElement::Block(block) => {
                eval_class_element_scope(i, constructor.clone(), Some(home.clone()), class_env, |i, scope| {
                    i.hoist(&block.body, scope)?;
                    i.exec_stmts_with_disposal(&block.body, scope).map(|_| ())
                })?;
            }
        }
    }
    Ok(())
}

/// PrivateFieldFind + the TypeError a missing brand throws on read (§7.3.29).
fn private_field_get(i: &mut Interpreter, base: &JsValue, name: &PrivateName) -> Result<JsValue, Throw> {
    let JsValue::Object(object) = base else {
        return Err(Throw::type_error("cannot read a private member from a non-object"));
    };
    let kind = object
        .borrow()
        .private_find(name)
        .map(|element| element.kind.clone())
        .ok_or_else(|| {
            Throw::type_error(format!(
                "cannot read private member {} from an object whose class did not declare it",
                name.text()
            ))
        })?;
    match kind {
        PrivateElementKind::Field(value) => Ok(value),
        PrivateElementKind::Method(value) => Ok(value),
        PrivateElementKind::Accessor { get: Some(getter), .. } => i.call_value(&getter, base.clone(), Vec::new()),
        PrivateElementKind::Accessor { get: None, .. } => {
            Err(Throw::type_error(format!("{} was defined without a getter", name.text())))
        }
    }
}

/// PrivateFieldSet + the TypeError a missing brand, a method, or a
/// getter-only accessor throws on write (§7.3.29).
fn private_field_set(i: &mut Interpreter, base: &JsValue, name: &PrivateName, value: JsValue) -> Result<(), Throw> {
    let JsValue::Object(object) = base else {
        return Err(Throw::type_error("cannot write a private member on a non-object"));
    };
    enum Action {
        Field,
        Setter(JsValue),
        NotWritable,
        NoSetter,
    }
    let action = match object.borrow_mut().private_find_mut(name) {
        None => {
            return Err(Throw::type_error(format!(
                "cannot write private member {} to an object whose class did not declare it",
                name.text()
            )));
        }
        Some(element) => match &mut element.kind {
            PrivateElementKind::Field(slot) => {
                *slot = value.clone();
                Action::Field
            }
            PrivateElementKind::Method(_) => Action::NotWritable,
            PrivateElementKind::Accessor { set: Some(setter), .. } => Action::Setter(setter.clone()),
            PrivateElementKind::Accessor { set: None, .. } => Action::NoSetter,
        },
    };
    match action {
        Action::Field => Ok(()),
        Action::Setter(setter) => i.call_value(&setter, base.clone(), vec![value]).map(|_| ()),
        Action::NotWritable => Err(Throw::type_error(format!("private method {} is not writable", name.text()))),
        Action::NoSetter => Err(Throw::type_error(format!("{} was defined without a setter", name.text()))),
    }
}

/// `#x in obj` brand check (§13.10.1): a non-object right-hand side throws;
/// an unbranded object simply answers `false`.
fn private_field_has(base: &JsValue, name: &PrivateName) -> Result<bool, Throw> {
    match base {
        JsValue::Object(object) => Ok(object.borrow().private_has(name)),
        _ => Err(Throw::type_error("cannot use 'in' operator to search for a private name in a non-object")),
    }
}

/// PrivateMethodOrAccessorAdd's definition-time counterpart: a getter and
/// setter sharing one private name (however far apart they are declared)
/// become ONE `InstanceElement::PrivateAccessor`, keyed by `PrivateName`
/// identity rather than declaration adjacency. A bare private method is
/// pushed as its own element.
fn upsert_instance_private_accessor_or_method(
    elements: &mut Vec<InstanceElement>,
    name: PrivateName,
    is_accessor: bool,
    is_getter: bool,
    method: JsValue,
) {
    if !is_accessor {
        elements.push(InstanceElement::PrivateMethod { name, value: method });
        return;
    }
    if let Some(InstanceElement::PrivateAccessor { get, set, .. }) =
        elements.iter_mut().find(|element| matches!(element, InstanceElement::PrivateAccessor { name: existing, .. } if *existing == name))
    {
        if is_getter { *get = Some(method); } else { *set = Some(method); }
        return;
    }
    elements.push(InstanceElement::PrivateAccessor {
        name,
        get: is_getter.then(|| method.clone()),
        set: (!is_getter).then_some(method),
    });
}

/// Static-side twin of [`upsert_instance_private_accessor_or_method`].
fn upsert_static_private_accessor_or_method(
    elements: &mut Vec<StaticElement>,
    name: PrivateName,
    is_accessor: bool,
    is_getter: bool,
    method: JsValue,
) {
    if !is_accessor {
        elements.push(StaticElement::PrivateMethod { name, value: method });
        return;
    }
    if let Some(StaticElement::PrivateAccessor { get, set, .. }) =
        elements.iter_mut().find(|element| matches!(element, StaticElement::PrivateAccessor { name: existing, .. } if *existing == name))
    {
        if is_getter { *get = Some(method); } else { *set = Some(method); }
        return;
    }
    elements.push(StaticElement::PrivateAccessor {
        name,
        get: is_getter.then(|| method.clone()),
        set: (!is_getter).then_some(method),
    });
}

/// Collects one `PrivateName` per distinct `#name` spelled directly in this
/// class's own member list (methods, accessors, fields; static or not).
/// First occurrence wins the identity; every later element with the same
/// text (a getter/setter pair, or — invalidly — a redeclaration) shares it.
/// Nested classes get their own map layered on top by their own
/// `eval_class_inner` call, so shadowing falls out of `resolve_private`'s
/// parent-chain walk for free.
fn collect_private_names(members: &[ast::ClassMember]) -> HashMap<String, PrivateName> {
    let mut names: HashMap<String, PrivateName> = HashMap::new();
    let mut note = |key: &ast::PropKey| {
        if let ast::PropKey::Private(identifier) = key {
            names.entry(identifier.name.clone()).or_insert_with(|| PrivateName::new(identifier.name.as_str()));
        }
    };
    for member in members {
        match member {
            ast::ClassMember::Method(method) => note(&method.key),
            ast::ClassMember::Field(field) => note(&field.key),
            ast::ClassMember::StaticBlock(_) => {}
        }
    }
    names
}
