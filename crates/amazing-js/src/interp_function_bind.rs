// Function.prototype.bind and BoundFunctionCreate ported and adapted from
// boa-dev/boa (core/engine/src/builtins/function/{mod,bound}.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT, Copyright (c) 2019 Jason Williams.
// Existing Amazing private home storage supplies the bound target/this/List.

fn native_function_bind(i: &mut Interpreter, target: JsValue,
    args: Vec<JsValue>) -> Result<JsValue, Throw>
{
    let JsValue::Object(target_object) = &target else {
        return Err(Throw::type_error("Function.prototype.bind receiver is not callable"));
    };
    if target_object.borrow().func.is_none() {
        return Err(Throw::type_error("Function.prototype.bind receiver is not callable"));
    }
    let this_argument = args.first().cloned().unwrap_or(JsValue::Undefined);
    let bound_arguments = args.get(1..).unwrap_or(&[]).to_vec();
    let bound_count = bound_arguments.len();
    // BoundFunctionCreate performs the actual target [[GetPrototypeOf]] before
    // HasOwnProperty/Get of length or Get of name. No author borrow is retained.
    let bound = i.create_bound_function(target_object, this_argument, bound_arguments)?;
    let mut length = 0.0;
    if i.default_get_own_property(target_object, &PropertyKey::from("length"))?.is_some() {
        if let JsValue::Num(number) = i.get_prop(&target, "length")? {
            if number == f64::INFINITY {
                length = f64::INFINITY;
            } else if number.is_finite() {
                let difference = number.trunc() - bound_count as f64;
                if difference > 0.0 { length = difference; }
            }
        }
    }
    i.define_bound_metadata(&bound, "length", JsValue::num(length))?;
    let target_name = i.get_prop(&target, "name")?;
    let mut name: Vec<u16> = "bound ".encode_utf16().collect();
    if let JsValue::Str(target_name) = target_name { name.extend_from_slice(target_name.units()); }
    i.define_bound_metadata(&bound, "name", JsValue::Str(crate::value::JsString::from_units(name)))?;
    Ok(JsValue::Object(bound))
}

impl Interpreter {
    fn create_bound_function(&mut self, target: &Obj, receiver: JsValue,
        arguments: Vec<JsValue>) -> Result<Obj, Throw>
    {
        let prototype = match self.default_get_prototype(target)? {
            JsValue::Object(prototype) => Some(prototype),
            JsValue::Null => None,
            _ => return Err(Throw::type_error("invalid function prototype")),
        };
        // IsConstructor inspects the immutable construct capability, including
        // revoked Proxy capabilities; revocation itself is checked on dispatch.
        let constructible = target.borrow().constructible;
        let data = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        data.borrow_mut().set_own("target", JsValue::Object(target.clone()));
        data.borrow_mut().set_own("this", receiver);
        // BoundArguments is an internal List. Its storage is never exposed and
        // must not perform Array Set, invoke indexed setters, or acquire iterators.
        let mut list = JsObject::ordinary();
        list.set_own("length", JsValue::num(arguments.len() as f64));
        for (index, value) in arguments.into_iter().enumerate() {
            list.set_own(index.to_string(), value);
        }
        data.borrow_mut().set_own("args", JsValue::Object(Obj(Rc::new(RefCell::new(list)))));
        let JsValue::Object(bound) = Interpreter::native("bound", native_bound, &self.function_proto)
            else { unreachable!("native function allocation") };
        {
            let mut function = bound.borrow_mut();
            function.proto = prototype;
            function.func.as_mut().unwrap().home = Some(data);
            function.constructible = constructible;
            function.delete_own("prototype");
        }
        // Bound functions create no invocation realm/context. Existing
        // get_function_realm unwraps the private target at actual invocation.
        Ok(bound)
    }

    fn define_bound_metadata(&self, bound: &Obj, name: &str,
        value: JsValue) -> Result<(), Throw>
    {
        bound.borrow_mut().define(name, Property { accessor: false, value,
            writable: false, enumerable: false, configurable: true,
            getter: None, setter: None })
            .map_err(|_| Throw::type_error("cannot define bound function metadata"))
    }
}
