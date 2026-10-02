// Portions ported from boa-dev/boa
// (core/engine/src/bytecompiler/expression/object_literal.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Method-definition SetFunctionName/SetHomeObject operations are adapted
// onto Amazing's existing method factory and property/function representation.

impl Interpreter {
    fn resolve_super_target(
        &mut self,
        target: &ast::Expr,
        env: &EnvHandle,
    ) -> Result<Option<Reference>, Throw> {
        let mut target = target;
        while let ExprKind::Paren(inner) = &target.kind {
            target = inner;
        }
        if !matches!(&target.kind, ExprKind::Member { obj, .. } if matches!(obj.kind, ExprKind::Super))
        {
            return Ok(None);
        }
        match self.eval_chain_reference(target, env)? {
            ChainEvaluation::Property {
                base,
                receiver,
                key,
                ..
            } => Ok(Some(Reference::SuperProperty(base, key, receiver))),
            _ => unreachable!("super property reference"),
        }
    }

    fn object_method_value(
        &self,
        method: &ast::ClassMethod,
        env: &EnvHandle,
        home: &JsValue,
        key: &PropertyKey,
    ) -> JsValue {
        let value = self.method_value(method, env, false);
        self.configure_object_method(&value, home, key, method.kind, &method.params);
        value
    }

    fn configure_object_method(
        &self,
        value: &JsValue,
        home: &JsValue,
        key: &PropertyKey,
        kind: ast::MethodKind,
        params: &[ast::Param],
    ) {
        let JsValue::Object(function) = value else {
            unreachable!()
        };
        let JsValue::Object(home) = home else {
            unreachable!()
        };
        let base_name = Self::property_function_name(key);
        let mut units: Vec<u16> = match kind {
            ast::MethodKind::Get => "get ".encode_utf16().collect(),
            ast::MethodKind::Set => "set ".encode_utf16().collect(),
            _ => Vec::new(),
        };
        units.extend_from_slice(base_name.units());
        let name = crate::value::JsString::from_units(units);
        let length = params
            .iter()
            .take_while(|p| !p.rest && p.default.is_none())
            .count();
        let mut function = function.borrow_mut();
        let body = function
            .func
            .as_mut()
            .expect("method factory returns a function");
        body.name = name.to_utf8_lossy();
        body.home = Some(home.clone());
        function.constructible = false;
        let _ = function.define(
            "name",
            Property {
                value: JsValue::str(name),
                writable: false,
                enumerable: false,
                configurable: true,
                getter: None,
                setter: None,
            },
        );
        let _ = function.define(
            "length",
            Property {
                value: JsValue::num(length as f64),
                writable: false,
                enumerable: false,
                configurable: true,
                getter: None,
                setter: None,
            },
        );
    }
}

#[cfg(test)]
mod object_method_tests {
    use super::*;

    #[test]
    fn object_methods_and_accessors_keep_home_and_actual_receiver() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var reads=0,parent={get value(){reads++;return this.marker;},set value(v){this.marker=v;},m(){return this.marker;}};
                var o={__proto__:parent,marker:7,m(){return super.m();},
                    get value(){return super.value;},set value(v){super.value=v;},
                    later(){return ()=>super.m();},*g(){yield super.m();},async a(){return super.m();}};
                var detached=o.m, receiver={marker:9};
                var before=o.value; o.value=11; var writeOnly=reads===1;
                var lexical=o.later();
                var asyncValue; o.a().then(v=>asyncValue=v);
                var direct=detached.call(receiver),generator=o.g().next().value;
                Object.setPrototypeOf(o,{m(){return this.marker+1;}});
                before===7 && writeOnly && o.marker===11 && direct===9 && generator===11 && lexical()===12;
            "#, backend).unwrap();
            i.drain_microtasks().unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
            assert_eq!(i.run_script("asyncValue").unwrap(), JsValue::num(11.0));
        }
    }

    #[test]
    fn object_method_names_lengths_and_descriptors_preserve_exact_property_keys() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            let result = i.run_script_with_backend(r#"
                var named=Symbol('s'),empty=Symbol(),o={m(a,b=0,c){},[named](){},[empty](){},
                    get value(){},set value(v){},['\uD800'](){},get ['\uD801'](){}};
                var getter=Object.getOwnPropertyDescriptor(o,'value').get;
                var setter=Object.getOwnPropertyDescriptor(o,'value').set;
                var loneGetter=Object.getOwnPropertyDescriptor(o,'\uD801').get;
                var name=Object.getOwnPropertyDescriptor(o.m,'name'),length=Object.getOwnPropertyDescriptor(o.m,'length');
                var property=Object.getOwnPropertyDescriptor(o,'m');
                o.m.name==='m' && o.m.length===1 && o[named].name==='[s]' && o[empty].name==='' &&
                getter.name==='get value' && getter.length===0 && setter.name==='set value' && setter.length===1 &&
                o['\uD800'].name.charCodeAt(0)===0xD800 && loneGetter.name.length===5 && loneGetter.name.charCodeAt(4)===0xD801 &&
                !name.writable && !name.enumerable && name.configurable && !length.writable && !length.enumerable && length.configurable &&
                property.writable && property.enumerable && property.configurable && !Object.hasOwn(o.m,'prototype');
            "#, backend).unwrap();
            assert_eq!(result, JsValue::Bool(true), "{backend:?}");
        }
    }
}
