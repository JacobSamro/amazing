// Live GetSuperConstructor and original instance-element binding adapted from
// lucid-softworks/lumen (crates/lumen/src/eval.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Default builtin creation/forwarding follows ECMA-262 ClassDefinitionEvaluation.
// It forwards the original argument list, without synthesizing rest/spread AST.

fn is_default_class_constructor(object: &Obj) -> bool {
    matches!(object.borrow().func.as_ref().map(|function| &function.body),
        Some(FunctionBody::Native(native))
            if std::ptr::fn_addr_eq(*native, native_default_class_constructor as crate::value::NativeFn))
}

impl Interpreter {
    fn make_default_class_constructor(&self, class: &ast::ClassItem, env: &EnvHandle) -> JsValue {
        let mut object = JsObject::function(JsFunction {
            source: class.source.clone(),
            name: class.name.as_ref().map(|name| name.name.clone()).unwrap_or_default(),
            params: Vec::new(),
            body: FunctionBody::Native(native_default_class_constructor),
            is_arrow: false, is_async: false, is_generator: false,
            env: env.clone(), home: None, proto_for_new: None,
        });
        object.proto = Some(self.function_proto.clone());
        object.realm = Some(self.host_realm);
        object.strict_function = true;
        object.constructible = true;
        // Builtin function properties have length before name. Class evaluation
        // supplies the genuine class name/home/source and nonwritable prototype.
        object.delete_own("name");
        object.delete_own("length");
        object.define("length", symbol_data(JsValue::num(0.0), false, true)).unwrap();
        object.define("name", symbol_data(JsValue::str(""), false, true)).unwrap();
        JsValue::object(object)
    }
}

fn native_default_class_constructor(i: &mut Interpreter, _this: JsValue, args: Vec<JsValue>)
    -> Result<JsValue, Throw>
{
    // This rejection executes inside the builtin's own realm. Explicit class
    // [[Call]] likewise rejects after its actual callee realm is installed.
    let new_target = i.current_new_target()
        .ok_or_else(|| Throw::type_error("class constructor requires new"))?;
    let function = i.frames.last().and_then(|frame| frame.own_function.clone())
        .ok_or_else(|| Throw::type_error("missing active default constructor"))?;
    let derived = function.borrow().derived_constructor.is_some();
    let result = if derived {
        let parent = function.borrow().proto.clone().map_or(JsValue::Null, JsValue::Object);
        // Construct performs IsConstructor after the original arguments have
        // already been supplied. It receives the exact NewTarget, including
        // Proxy/bound forwarding, and may return an existing parent object.
        i.construct_with_new_target(&parent, args, new_target)?
    } else {
        let prototype = match i.get_prop(&new_target, "prototype")? {
            JsValue::Object(prototype) => prototype,
            _ => i.object_prototype_for_function(&new_target)?,
        };
        let mut object = JsObject::ordinary();
        object.proto = Some(prototype);
        object.constructed = true;
        JsValue::object(object)
    };
    let elements = function.borrow().class_elements.clone();
    if let Some(elements) = elements {
        initialize_instance_elements(i, &result, &elements)?;
    }
    Ok(result)
}
