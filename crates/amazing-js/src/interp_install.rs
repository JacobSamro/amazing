// Realm builtin installation and native function construction.

impl Interpreter {
    fn native(name: &str, f: crate::value::NativeFn, proto: &Obj) -> JsValue {
        let mut o = JsObject::function(JsFunction {
            name: name.into(),
            params: Vec::new(),
            body: FunctionBody::Native(f),
            is_arrow: false,
            is_async: false,
            is_generator: false,
            env: EnvHandle::new(EnvKind::Global, None),
            home: None,
            proto_for_new: None,
        });
        o.proto = Some(proto.clone());
        let handle = Obj(Rc::new(RefCell::new(o)));
        let prototype = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
        handle
            .borrow_mut()
            .set_own("prototype", JsValue::Object(prototype));
        let _ = handle.borrow_mut().define("name", Property { value: JsValue::str(name), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
        let _ = handle.borrow_mut().define("length", Property { value: JsValue::num(0.0), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
        JsValue::Object(handle)
    }
    fn install_builtins(&mut self) {
        self.install_binary_builtins();
        self.install_bigint_builtins();
        self.define_global("undefined", JsValue::Undefined);
        self.define_global("NaN", JsValue::Num(f64::NAN));
        self.define_global("Infinity", JsValue::Num(f64::INFINITY));
        self.define_global(
            "eval",
            Self::native("eval", native_eval, &self.function_proto),
        );
        self.define_global(
            "fetch",
            Self::native("fetch", native_fetch, &self.function_proto),
        );
        self.define_global(
            "setTimeout",
            Self::native("setTimeout", native_set_timeout, &self.function_proto),
        );
        self.define_global(
            "clearTimeout",
            Self::native("clearTimeout", native_clear_timeout, &self.function_proto),
        );
        self.define_global(
            "setInterval",
            Self::native("setInterval", native_set_interval, &self.function_proto),
        );
        self.define_global(
            "clearInterval",
            Self::native("clearInterval", native_clear_timeout, &self.function_proto),
        );
        self.define_global(
            "requestAnimationFrame",
            Self::native(
                "requestAnimationFrame",
                native_request_animation_frame,
                &self.function_proto,
            ),
        );
        self.define_global(
            "cancelAnimationFrame",
            Self::native(
                "cancelAnimationFrame",
                native_cancel_animation_frame,
                &self.function_proto,
            ),
        );
        self.define_global(
            "queueMicrotask",
            Self::native("queueMicrotask", native_queue_microtask, &self.function_proto),
        );
        let error_constructor = Self::native("Error", native_error, &self.function_proto);
        let error_prototype = match &error_constructor {
            JsValue::Object(function) => match function.borrow().get_own("prototype") {
                Some(JsValue::Object(prototype)) => prototype,
                _ => unreachable!("native function has a prototype"),
            },
            _ => unreachable!("native function is an object"),
        };
        {
            let mut prototype = error_prototype.borrow_mut();
            prototype.proto = Some(self.object_proto.clone());
            prototype.set_own("name", JsValue::str("Error"));
            prototype.set_own("message", JsValue::str(""));
            prototype.set_own("toString", Self::native("toString", native_error_to_string, &self.function_proto));
            prototype.set_own("constructor", error_constructor.clone());
        }
        self.define_global("Error", error_constructor);
        for (name, native) in [
            ("TypeError", native_type_error as crate::value::NativeFn),
            ("RangeError", native_range_error as crate::value::NativeFn),
            ("ReferenceError", native_reference_error as crate::value::NativeFn),
            ("SyntaxError", native_syntax_error as crate::value::NativeFn),
        ] {
            let constructor = Self::native(name, native, &self.function_proto);
            if let JsValue::Object(function) = &constructor {
                if let Some(JsValue::Object(prototype)) = function.borrow().get_own("prototype") {
                    let mut prototype = prototype.borrow_mut();
                    prototype.proto = Some(error_prototype.clone());
                    prototype.set_own("name", JsValue::str(name));
                    prototype.set_own("message", JsValue::str(""));
                    prototype.set_own("constructor", constructor.clone());
                }
            }
            self.define_global(name, constructor);
        }
        let function_ctor = Self::native(
            "Function",
            native_function_constructor,
            &self.function_proto,
        );
        if let JsValue::Object(object) = &function_ctor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.function_proto.clone()));
        }
        self.define_global("Function", function_ctor);
        let has_own_property = symbol_method(self, "hasOwnProperty", native_has_own_property);
        symbol_method_length(&has_own_property, 1);
        self.object_proto.borrow_mut().set_own("hasOwnProperty", has_own_property);
        let property_is_enumerable = symbol_method(self, "propertyIsEnumerable", native_property_is_enumerable);
        symbol_method_length(&property_is_enumerable, 1);
        self.object_proto.borrow_mut().set_own("propertyIsEnumerable", property_is_enumerable);
        let object_ctor = Self::native("Object", native_object, &self.function_proto);
        if let JsValue::Object(o) = &object_ctor {
            o.borrow_mut().set_own("prototype", JsValue::Object(self.object_proto.clone()));
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "keys",
                Self::native("keys", native_object_keys, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "defineProperty",
                Self::native(
                    "defineProperty",
                    native_define_property,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "values",
                Self::native("values", native_object_values, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "assign",
                Self::native("assign", native_object_assign, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "preventExtensions",
                Self::native(
                    "preventExtensions",
                    native_prevent_extensions,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "isExtensible",
                Self::native("isExtensible", native_is_extensible, &self.function_proto),
            );
            for (name, function) in [
                ("create", native_object_create as crate::value::NativeFn),
                (
                    "getPrototypeOf",
                    native_object_get_prototype_of as crate::value::NativeFn,
                ),
                (
                    "setPrototypeOf",
                    native_object_set_prototype_of as crate::value::NativeFn,
                ),
                ("hasOwn", native_object_has_own as crate::value::NativeFn),
                ("getOwnPropertyNames", native_object_get_own_property_names as crate::value::NativeFn),
                ("getOwnPropertySymbols", native_object_get_own_property_symbols as crate::value::NativeFn),
                ("getOwnPropertyDescriptor", native_object_get_own_property_descriptor as crate::value::NativeFn),
            ] {
                let _ = self.set_prop(
                    &JsValue::Object(o.clone()),
                    name,
                    Self::native(name, function, &self.function_proto),
                );
            }
        }
        self.define_global("Object", object_ctor);
        let reflect = self.object_value();
        if let JsValue::Object(object) = &reflect {
            for (name, native) in [
                ("ownKeys", native_reflect_own_keys as crate::value::NativeFn),
                ("get", native_reflect_get as crate::value::NativeFn),
                ("set", native_reflect_set as crate::value::NativeFn),
                ("has", native_reflect_has as crate::value::NativeFn),
                ("deleteProperty", native_reflect_delete_property as crate::value::NativeFn),
                ("defineProperty", native_reflect_define_property as crate::value::NativeFn),
                ("construct", native_reflect_construct as crate::value::NativeFn),
                ("getOwnPropertyDescriptor", native_object_get_own_property_descriptor as crate::value::NativeFn),
            ] {
                object.borrow_mut().set_own(name, Self::native(name, native, &self.function_proto));
            }
        }
        self.define_global("Reflect", reflect);
        self.install_std_reflect();
        self.define_global(
            "Array",
            Self::native("Array", native_array, &self.function_proto),
        );
        if let Some(array) = self.global_env.resolve("Array") {
            let _ = self.set_prop(
                &array,
                "prototype",
                JsValue::Object(self.array_proto.clone()),
            );
            let _ = self.set_prop(
                &array,
                "isArray",
                Self::native("isArray", native_array_is_array, &self.function_proto),
            );
            let _ = self.set_prop(
                &array,
                "from",
                Self::native("from", native_array_from, &self.function_proto),
            );
            let _ = self.set_prop(
                &array,
                "of",
                Self::native("of", native_array_of, &self.function_proto),
            );
        }
        self.install_symbols();
        let date_ctor = Self::native("Date", native_date, &self.function_proto);
        if let JsValue::Object(object) = &date_ctor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.date_proto.clone()));
        }
        self.define_global("Date", date_ctor);
        let json = self.object_value();
        let _ = self.set_prop(
            &json,
            "parse",
            Self::native("parse", native_json_parse, &self.function_proto),
        );
        let _ = self.set_prop(
            &json,
            "stringify",
            Self::native("stringify", native_json_stringify, &self.function_proto),
        );
        self.define_global("JSON", json);
        self.define_global(
            "parseInt",
            Self::native("parseInt", native_parse_int, &self.function_proto),
        );
        self.define_global(
            "parseFloat",
            Self::native("parseFloat", native_parse_float, &self.function_proto),
        );
        self.define_global(
            "isFinite",
            Self::native("isFinite", native_is_finite, &self.function_proto),
        );
        let promise_ctor = Self::native("Promise", native_promise, &self.function_proto);
        if let JsValue::Object(o) = &promise_ctor {
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "resolve",
                Self::native("resolve", native_promise_resolve, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "reject",
                Self::native("reject", native_promise_reject, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "all",
                Self::native("all", native_promise_all, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "race",
                Self::native("race", native_promise_race, &self.function_proto),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "allSettled",
                Self::native(
                    "allSettled",
                    native_promise_all_settled,
                    &self.function_proto,
                ),
            );
            let _ = self.set_prop(
                &JsValue::Object(o.clone()),
                "any",
                Self::native("any", native_promise_any, &self.function_proto),
            );
        }
        self.define_global("Promise", promise_ctor);
        let _ = self.set_prop(
            &JsValue::Object(self.date_proto.clone()),
            "toString",
            Self::native("toString", native_date_to_string, &self.function_proto),
        );
        let object_to_string = symbol_method(self, "toString", native_object_to_string);
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "toString",
            object_to_string,
        );
        let object_value_of = symbol_method(self, "valueOf", native_object_value_of);
        let _ = self.set_prop(
            &JsValue::Object(self.object_proto.clone()),
            "valueOf",
            object_value_of,
        );
        for (name, function) in [
            ("call", native_function_call as crate::value::NativeFn),
            ("apply", native_function_apply as crate::value::NativeFn),
            ("bind", native_function_bind as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.function_proto.clone()),
                name,
                Self::native(name, function, &self.function_proto),
            );
        }
        for (n, f) in [
            ("toString", native_array_to_string as crate::value::NativeFn),
            ("push", native_array_push as crate::value::NativeFn),
            ("pop", native_array_pop as crate::value::NativeFn),
            ("join", native_array_join as crate::value::NativeFn),
            ("map", native_array_map as crate::value::NativeFn),
            ("filter", native_array_filter as crate::value::NativeFn),
            ("some", native_array_some as crate::value::NativeFn),
            ("forEach", native_array_for_each as crate::value::NativeFn),
            ("reduce", native_array_reduce as crate::value::NativeFn),
            ("slice", native_array_slice as crate::value::NativeFn),
            ("splice", native_array_splice as crate::value::NativeFn),
            ("concat", native_array_concat as crate::value::NativeFn),
            ("includes", native_array_includes as crate::value::NativeFn),
            ("indexOf", native_array_index_of as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.array_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        for (n, f) in [
            ("then", native_promise_then as crate::value::NativeFn),
            ("catch", native_promise_catch as crate::value::NativeFn),
            ("finally", native_promise_finally as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(
                &JsValue::Object(self.promise_proto.clone()),
                n,
                Self::native(n, f, &self.function_proto),
            );
        }
        self.install_std_collections();
        self.install_std_iterator_helpers();
        self.install_temporal();
        let math = self.object_value();
        for (n, f) in [
            ("abs", native_abs as crate::value::NativeFn),
            ("max", native_max as crate::value::NativeFn),
            ("min", native_min as crate::value::NativeFn),
            ("floor", native_math_floor as crate::value::NativeFn),
            ("ceil", native_math_ceil as crate::value::NativeFn),
            ("round", native_math_round as crate::value::NativeFn),
            ("sqrt", native_math_sqrt as crate::value::NativeFn),
            ("pow", native_math_pow as crate::value::NativeFn),
            ("trunc", native_math_trunc as crate::value::NativeFn),
            ("sign", native_math_sign as crate::value::NativeFn),
            ("random", native_math_random as crate::value::NativeFn),
        ] {
            let _ = self.set_prop(&math, n, Self::native(n, f, &self.function_proto));
        }
        self.define_global("Math", math);
        let console = self.object_value();
        let _ = self.set_prop(
            &console,
            "log",
            Self::native("log", native_log, &self.function_proto),
        );
        self.define_global("console", console);
        self.define_global(
            "Number",
            Self::native("Number", native_number, &self.function_proto),
        );
        let string_ctor = Self::native("String", native_string, &self.function_proto);
        if let JsValue::Object(object) = &string_ctor {
            let _ = object.borrow_mut().define("length", Property {
                value: JsValue::num(1.0), writable: false, enumerable: false,
                configurable: true, getter: None, setter: None,
            });
            let _ = object.borrow_mut().define("prototype", Property {
                value: JsValue::Object(self.string_proto.clone()), writable: false,
                enumerable: false, configurable: false, getter: None, setter: None,
            });
            let from_char_code = Self::native(
                        "fromCharCode",
                        native_string_from_char_code,
                        &self.function_proto,
                    );
            if let JsValue::Object(function) = &from_char_code {
                function.borrow_mut().constructible = false;
                function.borrow_mut().delete_own("prototype");
                let _ = function.borrow_mut().define("length", Property {
                    value: JsValue::num(1.0), writable: false, enumerable: false,
                    configurable: true, getter: None, setter: None,
                });
            }
            let _ = object.borrow_mut().define("fromCharCode", regexp_property(from_char_code, true));
        }
        let _ = self.string_proto.borrow_mut().define("constructor", Property {
            value: string_ctor.clone(), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        let _ = self.string_proto.borrow_mut().define("length", Property {
            value: JsValue::num(0.0), writable: false, enumerable: false,
            configurable: false, getter: None, setter: None,
        });
        for (name, function) in [
            ("match", native_string_match as crate::value::NativeFn),
            ("search", native_string_search as crate::value::NativeFn),
            ("split", native_string_split as crate::value::NativeFn),
            ("trim", native_string_trim as crate::value::NativeFn),
            ("charAt", native_string_char_at as crate::value::NativeFn),
            ("charCodeAt", native_string_char_code_at as crate::value::NativeFn),
            ("codePointAt", native_string_code_point_at as crate::value::NativeFn),
            ("slice", native_string_slice as crate::value::NativeFn),
            ("substring", native_string_substring as crate::value::NativeFn),
            ("indexOf", native_string_index_of as crate::value::NativeFn),
            ("includes", native_string_includes as crate::value::NativeFn),
            ("replace", native_string_replace as crate::value::NativeFn),
            ("startsWith", native_string_starts_with as crate::value::NativeFn),
            ("endsWith", native_string_ends_with as crate::value::NativeFn),
            ("isWellFormed", native_string_is_well_formed as crate::value::NativeFn),
            ("toWellFormed", native_string_to_well_formed as crate::value::NativeFn),
            ("normalize", native_string_normalize as crate::value::NativeFn),
            ("localeCompare", native_string_locale_compare as crate::value::NativeFn),
            ("toString", native_string_value_of as crate::value::NativeFn),
            ("valueOf", native_string_value_of as crate::value::NativeFn),
        ] {
            let arity = match name {
                "slice" | "substring" | "replace" => 2.0,
                "trim" | "isWellFormed" | "toWellFormed" | "normalize" | "toString" | "valueOf" => 0.0,
                _ => 1.0,
            };
            let method = Self::native(name, function, &self.function_proto);
            if let JsValue::Object(object) = &method {
                object.borrow_mut().constructible = false;
                object.borrow_mut().delete_own("prototype");
                let _ = object.borrow_mut().define("length", Property {
                    value: JsValue::num(arity), writable: false, enumerable: false,
                    configurable: true, getter: None, setter: None,
                });
            }
            let _ = self.string_proto.borrow_mut().define(name, Property {
                value: method, writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            });
        }
        self.define_global("String", string_ctor);
        let boolean_ctor = Self::native("Boolean", native_boolean, &self.function_proto);
        if let JsValue::Object(object) = &boolean_ctor {
            let _ = object.borrow_mut().define("prototype", Property {
                value: JsValue::Object(self.boolean_proto.clone()), writable: false,
                enumerable: false, configurable: false, getter: None, setter: None,
            });
        }
        let _ = self.boolean_proto.borrow_mut().define("constructor", Property {
            value: boolean_ctor.clone(), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        for (name, native) in [
            ("toString", native_boolean_to_string as crate::value::NativeFn),
            ("valueOf", native_boolean_value_of as crate::value::NativeFn),
        ] {
            let _ = self.boolean_proto.borrow_mut().define(name, Property {
                value: symbol_method(self, name, native), writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            });
        }
        self.define_global("Boolean", boolean_ctor);
        self.define_global(
            "isNaN",
            Self::native("isNaN", native_is_nan, &self.function_proto),
        );
        if let Some(number) = self.global_env.resolve("Number") {
            let _ = self.set_prop(&number, "POSITIVE_INFINITY", JsValue::Num(f64::INFINITY));
            let _ = self.set_prop(
                &number,
                "NEGATIVE_INFINITY",
                JsValue::Num(f64::NEG_INFINITY),
            );
            let _ = self.set_prop(&number, "NaN", JsValue::Num(f64::NAN));
            let _ = self.set_prop(&number, "MAX_VALUE", JsValue::Num(f64::MAX));
            let _ = self.set_prop(&number, "MIN_VALUE", JsValue::Num(f64::from_bits(1)));
            let _ = self.set_prop(&number, "MAX_SAFE_INTEGER", JsValue::num(9_007_199_254_740_991.0));
            let _ = self.set_prop(
                &number,
                "isNaN",
                Self::native("isNaN", native_number_is_nan, &self.function_proto),
            );
            let _ = self.set_prop(
                &number,
                "isFinite",
                Self::native("isFinite", native_number_is_finite, &self.function_proto),
            );
        }
        self.install_std_string();
        self.install_std_number_math();
        self.install_std_json_global();
    }
    fn define_global(&self, n: &str, v: JsValue) {
        self.global_obj.borrow_mut().set_own(n, v.clone());
        let _ = self.global_env.declare_var(n, v);
    }
}
