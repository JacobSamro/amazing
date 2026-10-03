// Portions ported from boa-dev/boa (core/engine/src/builtins/math/mod.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Math intrinsic method names, lengths and property attributes adapted to local installers.
// Realm builtin installation and native function construction.

include!("builtins_date.rs");

impl Interpreter {
    fn native(name: &str, f: crate::value::NativeFn, proto: &Obj) -> JsValue {
        Self::native_arity(name, f, proto, 0)
    }
    pub(crate) fn native_arity(name: &str, f: crate::value::NativeFn, proto: &Obj, arity: usize) -> JsValue {
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
        o.realm = proto.borrow().realm;
        o.constructible = false;
        let handle = Obj(Rc::new(RefCell::new(o)));
        // JsObject::function seeds "name" then "length"; built-in functions
        // must expose "length" before "name" in property order.
        let _ = handle.borrow_mut().delete_own("name");
        let _ = handle.borrow_mut().delete_own("length");
        let _ = handle.borrow_mut().define("length", Property { accessor: false, value: JsValue::num(arity as f64), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
        let _ = handle.borrow_mut().define("name", Property { accessor: false, value: JsValue::str(name), writable: false, enumerable: false, configurable: true, getter: None, setter: None });
        JsValue::Object(handle)
    }
    /// Only builtins with a spec [[Construct]] method opt in here.
    fn native_constructor(name: &str, f: crate::value::NativeFn, proto: &Obj) -> JsValue {
        Self::native_constructor_arity(name, f, proto, 0)
    }
    fn native_constructor_arity(name: &str, f: crate::value::NativeFn, proto: &Obj, arity: usize) -> JsValue {
        let value = Self::native_arity(name, f, proto, arity);
        if let JsValue::Object(o) = &value {
            o.borrow_mut().constructible = true;
            let prototype = Obj(Rc::new(RefCell::new(JsObject::ordinary())));
            o.borrow_mut().define("prototype", Property {
                accessor: false,
                value: JsValue::Object(prototype), writable: true, enumerable: false,
                configurable: true, getter: None, setter: None,
            }).unwrap();
        }
        value
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
        let error_constructor = Self::native_constructor("Error", native_error, &self.function_proto);
        // Reused as the stored %Error.prototype% (self.error_proto) rather than
        // the fresh ordinary object `Self::native_constructor` allocates, so the `stack`
        // setter's SameValue-with-home-object check has a stable identity to
        // compare against at call time.
        let error_prototype = self.error_proto.clone();
        if let JsValue::Object(function) = &error_constructor {
            function.borrow_mut().define("prototype", Property {
                accessor: false,
                value: JsValue::Object(error_prototype.clone()), writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            }).unwrap();
        }
        {
            let mut prototype = error_prototype.borrow_mut();
            prototype.proto = Some(self.object_proto.clone());
            prototype.set_own("name", JsValue::str("Error"));
            prototype.set_own("message", JsValue::str(""));
            prototype.set_own("toString", Self::native("toString", native_error_to_string, &self.function_proto));
            prototype.set_own("constructor", error_constructor.clone());
        }
        {
            // `Error.prototype.stack` accessor (proposal error-stack-accessor,
            // pinned in this Test262 revision); see native_error_stack_getter
            // and native_error_stack_setter for the exact algorithm. Built via
            // symbol_method so the accessor functions are non-constructible
            // with no own "prototype", matching every other built-in accessor
            // pair (verifyPrimordialAccessorProperty in prop-desc.js).
            let getter = symbol_method(self, "get stack", native_error_stack_getter);
            let setter = symbol_method(self, "set stack", native_error_stack_setter);
            symbol_method_length(&setter, 1);
            error_prototype.borrow_mut().define("stack", Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                getter: Some(getter), setter: Some(setter),
            }).unwrap();
        }
        if let JsValue::Object(error) = &error_constructor {
            error.borrow_mut().define("isError", symbol_data(
                Self::native("isError", native_error_is_error, &self.function_proto), true, true)).unwrap();
            runtime_method_attributes(error, "isError", 1);
        }
        self.define_global("Error", error_constructor.clone());
        for (name, native) in [
            ("EvalError", native_eval_error as crate::value::NativeFn),
            ("TypeError", native_type_error as crate::value::NativeFn),
            ("RangeError", native_range_error as crate::value::NativeFn),
            ("ReferenceError", native_reference_error as crate::value::NativeFn),
            ("SyntaxError", native_syntax_error as crate::value::NativeFn),
            ("URIError", native_uri_error as crate::value::NativeFn),
            ("AggregateError", native_aggregate_error as crate::value::NativeFn),
            ("SuppressedError", native_suppressed_error as crate::value::NativeFn),
        ] {
            let constructor = Self::native_constructor(name, native, &self.function_proto);
            if let JsValue::Object(function) = &constructor {
                if let JsValue::Object(error_function) = &error_constructor {
                    function.borrow_mut().proto = Some(error_function.clone());
                }
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
        let function_ctor = Self::native_constructor(
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
        let function_constructor = if let Some(constructor) = self.global_env.resolve("Function") {
            self.function_proto.borrow_mut().define("constructor", symbol_data(constructor.clone(), true, true)).unwrap();
            constructor
        } else {
            unreachable!("Function was just defined above")
        };
        // %AsyncFunction%, %GeneratorFunction%, %AsyncGeneratorFunction%:
        // never exposed as globals (CreateDynamicFunction kinds "async",
        // "generator", "async generator"), but reachable from any matching
        // function/method/arrow's `.constructor`, since make_function/
        // method_value/make_arrow already give every such value the shared
        // async_function_proto/generator_function_proto/
        // async_generator_function_proto as its `[[Prototype]]`. Each
        // constructor's own `[[Prototype]]` is %Function% (it "is a
        // subclass of Function"), and its `.prototype` link is
        // {writable:false, enumerable:false, configurable:false} --
        // overwriting native_constructor_arity's ordinary-constructor default of
        // {writable:true, configurable:true} -- matching
        // AsyncFunction-prototype.js. `constructor`/@@toStringTag on the
        // shared prototype are {writable:false, enumerable:false,
        // configurable:true}, per GeneratorFunction/prototype/constructor.js
        // and *-to-string.js/Symbol.toStringTag.js (this differs from an
        // ordinary function's writable:true `.prototype.constructor`,
        // because these three use direct property definition rather than
        // MakeConstructor).
        if let JsValue::Object(function_constructor) = &function_constructor {
            for (name, native, proto, tag, instance_proto) in [
                ("AsyncFunction", native_async_function_constructor as crate::value::NativeFn, self.async_function_proto.clone(), "AsyncFunction", None),
                ("GeneratorFunction", native_generator_function_constructor as crate::value::NativeFn, self.generator_function_proto.clone(), "GeneratorFunction", Some(self.generator_proto.clone())),
                ("AsyncGeneratorFunction", native_async_generator_function_constructor as crate::value::NativeFn, self.async_generator_function_proto.clone(), "AsyncGeneratorFunction", Some(self.async_generator_proto.clone())),
            ] {
                let ctor = Self::native_constructor_arity(name, native, &self.function_proto, 1);
                let JsValue::Object(ctor_obj) = &ctor else { unreachable!("native_constructor_arity always returns an object") };
                ctor_obj.borrow_mut().proto = Some(function_constructor.clone());
                ctor_obj.borrow_mut().define("prototype", Property {
                    accessor: false,
                    value: JsValue::Object(proto.clone()), writable: false, enumerable: false,
                    configurable: false, getter: None, setter: None,
                }).unwrap();
                proto.borrow_mut().define("constructor", symbol_data(ctor.clone(), false, true)).unwrap();
                proto.borrow_mut().define(WellKnownSymbol::ToStringTag.symbol(), symbol_data(JsValue::str(tag), false, true)).unwrap();
                // For the generator kinds, %XFunction.prototype%'s own
                // "prototype" is the shared %[Async]GeneratorPrototype%
                // singleton every constructed generator's own `.prototype`
                // object inherits from (GeneratorFunction/prototype/
                // prototype.js): {writable:false, enumerable:false,
                // configurable:true}.
                if let Some(instance_proto) = instance_proto {
                    proto.borrow_mut().define("prototype", symbol_data(JsValue::Object(instance_proto), false, true)).unwrap();
                }
            }
        }
        let has_own_property = symbol_method(self, "hasOwnProperty", native_has_own_property);
        symbol_method_length(&has_own_property, 1);
        self.object_proto.borrow_mut().define("hasOwnProperty", symbol_data(has_own_property, true, true)).unwrap();
        let property_is_enumerable = symbol_method(self, "propertyIsEnumerable", native_property_is_enumerable);
        symbol_method_length(&property_is_enumerable, 1);
        self.object_proto.borrow_mut().define("propertyIsEnumerable", symbol_data(property_is_enumerable, true, true)).unwrap();
        let proto_getter = symbol_method(self, "get __proto__", native_object_proto_getter);
        let proto_setter = symbol_method(self, "set __proto__", native_object_proto_setter);
        symbol_method_length(&proto_setter, 1);
        self.object_proto.borrow_mut().define("__proto__", Property {
            accessor: true,
            value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
            getter: Some(proto_getter), setter: Some(proto_setter),
        }).unwrap();
        for (name, length, native) in [
            ("__defineGetter__", 2, native_object_define_getter as crate::value::NativeFn),
            ("__defineSetter__", 2, native_object_define_setter as crate::value::NativeFn),
            ("__lookupGetter__", 1, native_object_lookup_getter as crate::value::NativeFn),
            ("__lookupSetter__", 1, native_object_lookup_setter as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, name, native);
            symbol_method_length(&method, length);
            self.object_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        let object_ctor = Self::native_constructor("Object", native_object, &self.function_proto);
        if let JsValue::Object(o) = &object_ctor {
            o.borrow_mut().set_own("prototype", JsValue::Object(self.object_proto.clone()));
            self.object_proto.borrow_mut().define("constructor", symbol_data(object_ctor.clone(), true, true)).unwrap();
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
        self.install_object_extra(&object_ctor);
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
        self.install_proxy();
        self.define_global(
            "Array",
            Self::native_constructor("Array", native_array, &self.function_proto),
        );
        if let Some(array) = self.global_env.resolve("Array") {
            self.array_proto.borrow_mut().define("constructor", symbol_data(array.clone(), true, true)).unwrap();
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
        let date_ctor = Self::native_constructor("Date", native_date, &self.function_proto);
        if let JsValue::Object(object) = &date_ctor {
            object
                .borrow_mut()
                .set_own("prototype", JsValue::Object(self.date_proto.clone()));
        }
        self.install_date_engine(&date_ctor);
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
        let promise_ctor = Self::native_constructor("Promise", native_promise, &self.function_proto);
        self.intrinsic_promise_constructor = promise_ctor.clone();
        if let JsValue::Object(o) = &promise_ctor {
            let getter = symbol_method(self, "get [Symbol.species]", native_array_species);
            o.borrow_mut().define(WellKnownSymbol::Species.symbol(), Property {
                accessor: true,
                value: JsValue::Undefined, writable: false, enumerable: false,
                configurable: true, getter: Some(getter), setter: None,
            }).unwrap();
            let _ = self.set_prop(&promise_ctor, "prototype", JsValue::Object(self.promise_proto.clone()));
            self.promise_proto.borrow_mut().define("constructor", symbol_data(promise_ctor.clone(), true, true)).unwrap();
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
            o.borrow_mut().define("allKeyed", symbol_data(
                Self::native_arity("allKeyed", native_promise_all_keyed, &self.function_proto, 1), true, true)).unwrap();
            o.borrow_mut().define("allSettledKeyed", symbol_data(
                Self::native_arity("allSettledKeyed", native_promise_all_settled_keyed, &self.function_proto, 1), true, true)).unwrap();
        }
        self.define_global("Promise", promise_ctor);
        let _ = self.set_prop(
            &JsValue::Object(self.date_proto.clone()),
            "toString",
            Self::native("toString", native_date_to_string, &self.function_proto),
        );
        self.install_date();
        let object_to_string = symbol_method(self, "toString", native_object_to_string);
        self.object_proto.borrow_mut().define("toString", symbol_data(object_to_string, true, true)).unwrap();
        let object_value_of = symbol_method(self, "valueOf", native_object_value_of);
        self.object_proto.borrow_mut().define("valueOf", symbol_data(object_value_of, true, true)).unwrap();
        for (name, function, length) in [
            ("call", native_function_call as crate::value::NativeFn, 1),
            ("apply", native_function_apply as crate::value::NativeFn, 2),
            ("bind", native_function_bind as crate::value::NativeFn, 1),
        ] {
            let method = symbol_method(self, name, function);
            symbol_method_length(&method, length);
            self.function_proto.borrow_mut().define(name, symbol_data(method, true, true)).unwrap();
        }
        {
            // 9.2.7 AddRestrictedFunctionProperties, applied here to
            // %Function.prototype% itself (steps 2-4): `caller` and
            // `arguments` are accessor properties sharing one %ThrowTypeError%
            // getter/setter, {enumerable: false, configurable: true}. Built via
            // symbol_method (name "", length 0, non-constructible, no own
            // "prototype") so it matches the anonymous, poison-pill shape
            // every other well-known-intrinsic check in the corpus expects.
            //
            // Re-landed here (2026-09-27) after cd669e67's original installation was
            // reverted (c0de6390) along with the getter/setter that made every function
            // throw unconditionally; make_function (interp_stmt.rs) now installs an OWN,
            // non-throwing "caller" on plain sloppy functions that shadows this inherited
            // pill, so the two no longer collide.
            let thrower = symbol_method(self, "", native_throw_type_error_intrinsic);
            self.intrinsic_arguments_thrower = thrower.clone();
            // 10.2.4 %ThrowTypeError% ( ): unlike an ordinary anonymous built-in function
            // (which `symbol_method` otherwise models correctly, with configurable "name"/
            // "length"), this ONE intrinsic is explicitly unforgeable and frozen: [[Extensible]]
            // false, and both "length" and "name" non-configurable (as well as non-writable,
            // already true) -- found chasing a JS-19 zero-loss regression
            // (built-ins/ThrowTypeError/{frozen,extensible,length,name}.js), unmasked the same
            // way as the arguments-callee gaps above: `Object.isFrozen(undefined)` is
            // trivially true, so before `caller`/`arguments` resolved to a real function here,
            // these tests passed on an entirely different, accidental value.
            if let JsValue::Object(object) = &thrower {
                let mut o = object.borrow_mut();
                o.define("length", Property {
                    accessor: false,
                    value: JsValue::num(0.0), writable: false, enumerable: false,
                    configurable: false, getter: None, setter: None,
                }).unwrap();
                o.define("name", Property {
                    accessor: false,
                    value: JsValue::str(""), writable: false, enumerable: false,
                    configurable: false, getter: None, setter: None,
                }).unwrap();
                o.extensible = false;
            }
            for name in ["caller", "arguments"] {
                self.function_proto.borrow_mut().define(name, Property {
                    accessor: true,
                    value: JsValue::Undefined, writable: false, enumerable: false, configurable: true,
                    getter: Some(thrower.clone()), setter: Some(thrower.clone()),
                }).unwrap();
            }
        }
        for (n, f) in [
            ("toString", native_array_to_string as crate::value::NativeFn),
            ("push", native_array_push as crate::value::NativeFn),
            ("pop", native_array_pop as crate::value::NativeFn),
            ("join", native_array_join as crate::value::NativeFn),
            ("map", native_array_map as crate::value::NativeFn),
            ("filter", native_array_filter as crate::value::NativeFn),
            ("some", native_array_some as crate::value::NativeFn),
            ("find", native_array_find as crate::value::NativeFn),
            ("every", native_array_every as crate::value::NativeFn),
            ("sort", native_array_sort as crate::value::NativeFn),
            ("forEach", native_array_for_each as crate::value::NativeFn),
            ("reduce", native_array_reduce as crate::value::NativeFn),
            ("slice", native_array_slice as crate::value::NativeFn),
            ("splice", native_array_splice as crate::value::NativeFn),
            ("concat", native_array_concat as crate::value::NativeFn),
            ("includes", native_array_includes as crate::value::NativeFn),
            ("indexOf", native_array_index_of as crate::value::NativeFn),
        ] {
            let method = symbol_method(self, n, f);
            self.array_proto.borrow_mut().define(n, symbol_data(method, true, true)).unwrap();
        }
        self.install_array_extra();
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
        self.install_thread_generator_methods();
        self.install_temporal();
        let math = self.object_value();
        let JsValue::Object(math_object) = &math else { unreachable!() };
        for (name, length, native) in [
            ("abs", 1, native_abs as crate::value::NativeFn),
            ("max", 2, native_max as crate::value::NativeFn),
            ("min", 2, native_min as crate::value::NativeFn),
            ("floor", 1, native_math_floor as crate::value::NativeFn),
            ("ceil", 1, native_math_ceil as crate::value::NativeFn),
            ("round", 1, native_math_round as crate::value::NativeFn),
            ("sqrt", 1, native_math_sqrt as crate::value::NativeFn),
            ("pow", 2, native_math_pow as crate::value::NativeFn),
            ("trunc", 1, native_math_trunc as crate::value::NativeFn),
            ("sign", 1, native_math_sign as crate::value::NativeFn),
            ("random", 0, native_math_random as crate::value::NativeFn),
        ] {
            std_core_define(self, math_object, name, length, native);
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
            Self::native_constructor("Number", native_number, &self.function_proto),
        );
        let string_ctor = Self::native_constructor("String", native_string, &self.function_proto);
        if let JsValue::Object(object) = &string_ctor {
            let _ = object.borrow_mut().define("length", Property {
                accessor: false,
                value: JsValue::num(1.0), writable: false, enumerable: false,
                configurable: true, getter: None, setter: None,
            });
            let _ = object.borrow_mut().define("prototype", Property {
                accessor: false,
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
                    accessor: false,
                    value: JsValue::num(1.0), writable: false, enumerable: false,
                    configurable: true, getter: None, setter: None,
                });
            }
            let _ = object.borrow_mut().define("fromCharCode", regexp_property(from_char_code, true));
        }
        let _ = self.string_proto.borrow_mut().define("constructor", Property {
            accessor: false,
            value: string_ctor.clone(), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        });
        let _ = self.string_proto.borrow_mut().define("length", Property {
            accessor: false,
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
                    accessor: false,
                    value: JsValue::num(arity), writable: false, enumerable: false,
                    configurable: true, getter: None, setter: None,
                });
            }
            let _ = self.string_proto.borrow_mut().define(name, Property {
                accessor: false,
                value: method, writable: true,
                enumerable: false, configurable: true, getter: None, setter: None,
            });
        }
        self.define_global("String", string_ctor);
        let boolean_ctor = Self::native_constructor("Boolean", native_boolean, &self.function_proto);

        self.install_boolean_extra(&boolean_ctor);
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
        self.install_structured_clone();
        self.install_intl_builtins();
        self.install_runtime_intrinsic_attributes();
    }
    fn define_global(&self, n: &str, v: JsValue) {
        let constant = matches!(n, "undefined" | "NaN" | "Infinity");
        let _ = self.global_obj.borrow_mut().define(n, Property {
            accessor: false,
            value: v.clone(), writable: !constant, enumerable: false,
            configurable: !constant, getter: None, setter: None,
        });
        let _ = self.global_env.declare_var(n, v);
    }
}
