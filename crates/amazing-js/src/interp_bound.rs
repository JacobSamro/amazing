// Proper-tail outcome/redispatch adaptation from lucid-softworks/lumen
// (crates/lumen/src/{eval,interpreter}.rs and interpreter/call_entry.rs)
// @ f6c9860d44fa52249cfd647122edea184d32c863, MIT.
// Bound function exotic [[Call]] and [[Construct]] do not create a function
// execution context of their own. Enter only the ultimate target's realm.
impl Interpreter {
    fn is_bound_function(value: &JsValue) -> bool {
        matches!(value, JsValue::Object(object) if object.borrow().func.as_ref().is_some_and(|function|
            matches!(&function.body, FunctionBody::Native(native)
                if std::ptr::fn_addr_eq(*native, native_bound as crate::value::NativeFn))))
    }

    fn bound_call_record(&mut self, function: &JsValue,
        args: Vec<JsValue>) -> Result<TailCallRecord, Throw> {
        let JsValue::Object(object) = function else { return Err(Throw::type_error("invalid bound function")); };
        let data = object.borrow().func.as_ref().and_then(|function| function.home.clone())
            .ok_or_else(|| Throw::type_error("invalid bound function data"))?;
        let target = data.borrow().get_own("target").ok_or_else(|| Throw::type_error("missing bound target"))?;
        let receiver = data.borrow().get_own("this").unwrap_or(JsValue::Undefined);
        let stored = data.borrow().get_own("args").ok_or_else(|| Throw::type_error("missing bound arguments"))?;
        let JsValue::Object(stored) = stored else { return Err(Throw::type_error("invalid bound arguments")); };
        // This is private runtime storage for an argument List, not iterable input.
        // Neither the array iterator nor prototype indexed getters are consulted.
        let count = stored.borrow().get_own("length").unwrap_or(JsValue::num(0.0)).to_number()? as usize;
        let mut arguments = Vec::with_capacity(count + args.len());
        for index in 0..count {
            arguments.push(stored.borrow().get_own(index.to_string()).unwrap_or(JsValue::Undefined));
        }
        arguments.extend(args);
        TailCallRecord::new(target, receiver, arguments)
    }

    fn invoke_bound_function(&mut self, function: &JsValue, args: Vec<JsValue>,
        new_target: Option<JsValue>) -> Result<JsValue, Throw> {
        let TailCallRecord { callee: target, receiver, arguments, .. } = self.bound_call_record(function, args)?;
        if self.depth >= MAX_CALL_DEPTH { return Err(Throw::range_error("maximum call stack size exceeded")); }
        self.depth += 1;
        let result = if let Some(new_target) = new_target {
            let new_target = if new_target == *function { target.clone() } else { new_target };
            self.construct_with_new_target(&target, arguments, new_target)
        } else {
            let direct = std::mem::replace(&mut self.direct_eval_call, false);
            let environment = self.direct_eval_env.take();
            let result = self.call_value(&target, receiver, arguments);
            self.direct_eval_call = direct;
            self.direct_eval_env = environment;
            result
        };
        self.depth -= 1;
        result
    }
}

#[cfg(test)]
mod bound_function_tests {
    use super::*;

    #[test]
    fn bound_argument_lists_do_not_invoke_iterators_and_construct_forwards_new_target() {
        let mut i = Interpreter::new();
        assert_eq!(i.run_script(r#"
            var f = (function(a,b,c) { return a+b+c; }).bind(null, 1, 2);
            var saved = Array.prototype[Symbol.iterator];
            Array.prototype[Symbol.iterator] = function() { throw new Error('iterator'); };
            var sum = f(3);
            Array.prototype[Symbol.iterator] = saved;
            var Bound = Array.bind(null, 1, 2);
            function Target() {}
            Target.prototype = null;
            var a = Reflect.construct(Bound, [3], Target);
            sum === 6 && a.join(',') === '1,2,3' && Object.getPrototypeOf(a) === Array.prototype
        "#).unwrap(), JsValue::Bool(true));
    }

    #[test]
    fn foreign_bound_class_call_rejects_in_the_caller_realm() {
        let mut i = Interpreter::new();
        let child = i.binding_create_realm(i.binding_create_object(None).unwrap()).unwrap();
        let bound = i.binding_eval_in_realm(child, "(class {}).bind(null)").unwrap();
        let error = i.call_value(&bound, JsValue::Undefined, Vec::new()).unwrap_err();
        let expected = i.run_script("TypeError.prototype").unwrap();
        assert_eq!(native_object_get_prototype_of(&mut i, JsValue::Undefined, vec![error.0]).unwrap(), expected);
    }
}
