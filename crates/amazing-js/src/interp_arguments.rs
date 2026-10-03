// Portions ported from boa-dev/boa (core/engine/src/builtins/function/arguments.rs)
// @ 39cd11214ecb366200157dab96b3919e61e9a391, MIT.
// Arguments creation and exotic operations adapted to Amazing's binding cells.

fn arguments_mapped_cell(object: &Obj, key: &PropertyKey) -> Option<crate::env::BindingCell> {
    match &object.borrow().exotic {
        Exotic::Arguments(map) => map.get(key).cloned(),
        _ => None,
    }
}

fn arguments_own_property(object: &Obj, key: &PropertyKey)
    -> Option<Result<Option<Property>, Throw>> {
    if !matches!(&object.borrow().exotic, Exotic::Arguments(_)) { return None; }
    Some((|| {
        let mut property = object.borrow().own_prop(key).cloned();
        if let (Some(property), Some(cell)) = (&mut property, arguments_mapped_cell(object, key)) {
            property.value = cell.read()?;
        }
        Ok(property)
    })())
}

fn arguments_write_mapping(object: &Obj, key: &PropertyKey, value: JsValue) -> Result<(), Throw> {
    if let Some(cell) = arguments_mapped_cell(object, key) { cell.write_mutable(value)?; }
    Ok(())
}

fn arguments_finish_define(object: &Obj, key: &PropertyKey,
    cell: Option<crate::env::BindingCell>, value: Option<JsValue>, detach: bool)
    -> Result<(), Throw> {
    if let Some(cell) = cell {
        if let Some(value) = value { cell.write_mutable(value)?; }
        if detach {
            if let Exotic::Arguments(map) = &mut object.borrow_mut().exotic { map.remove(key); }
        }
    }
    Ok(())
}

impl Interpreter {
    fn create_arguments_object(&self, env: &EnvHandle, params: &[ast::Param],
        args: &[JsValue], strict: bool) -> JsValue {
        let simple = params.iter().all(|param|
            !param.rest && param.default.is_none() && matches!(param.binding, Binding::Ident(_)));
        let mapped = !strict && simple;
        let mut object = JsObject::ordinary();
        object.proto = Some(self.object_proto.clone());
        object.arguments_object = true;
        object.define("length", Property {
            value: JsValue::num(args.len() as f64), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        for (index, value) in args.iter().enumerate() { object.set_own(index.to_string(), value.clone()); }
        object.define(WellKnownSymbol::Iterator.symbol(), Property {
            value: self.original_array_iterator.clone(), writable: true, enumerable: false,
            configurable: true, getter: None, setter: None,
        }).unwrap();
        if mapped {
            let mut names = HashSet::new();
            let mut map = HashMap::new();
            // The rightmost occurrence claims a name even when no argument was supplied.
            for (index, param) in params.iter().enumerate().rev() {
                if let Binding::Ident(name) = &param.binding {
                    if names.insert(name.name.clone()) && index < args.len() {
                        if let Some(cell) = env.local_cell(&name.name) {
                            map.insert(PropertyKey::from(index.to_string()), cell);
                        }
                    }
                }
            }
            object.exotic = Exotic::Arguments(map);
            if let Some(callee) = self.frames.last().and_then(|frame| frame.own_function.clone()) {
                object.define("callee", Property {
                    value: JsValue::Object(callee), writable: true, enumerable: false,
                    configurable: true, getter: None, setter: None,
                }).unwrap();
            }
        } else {
            object.define("callee", Property {
                value: JsValue::Undefined, writable: false, enumerable: false, configurable: false,
                getter: Some(self.intrinsic_arguments_thrower.clone()),
                setter: Some(self.intrinsic_arguments_thrower.clone()),
            }).unwrap();
        }
        JsValue::object(object)
    }
}

#[cfg(test)]
mod arguments_exotic_tests {
    use super::*;

    fn check(cases: &[&str]) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in cases {
                let mut i = Interpreter::new();
                let result = i.run_script_with_backend(source, backend);
                assert_eq!(result.unwrap_or_else(|error| panic!("{backend:?}: {source}: {error:?}")),
                    JsValue::Bool(true), "{backend:?}: {source}");
            }
        }
    }

    #[test]
    fn mapped_arguments_alias_cells_duplicates_and_escaped_calls() {
        check(&[
            "function f(a){arguments[0]=7;return a===7&&arguments.length===2&&arguments[1]===2};f(1,2)",
            "function f(a){a=8;return arguments[0]===8&&Object.getOwnPropertyDescriptor(arguments,'0').value===8};f(1)",
            "function f(a,a){a=9;return arguments[0]===1&&arguments[1]===9};f(1,2)",
            "function f(a,a){a=9;return arguments[0]===1&&!Object.hasOwn(arguments,'1')};f(1)",
            "function f(a){return {args:arguments,read:()=>a,write:v=>a=v}};var x=f(1);x.args[0]=5;var ok=x.read()===5;x.write(6);ok&&x.args[0]===6",
            "function f(a){return Object.prototype.toString.call(arguments)==='[object Arguments]'};f(1)",
            "function f(a){function a(){};return arguments[0]===a&&typeof a==='function'};f(1)",
            "function f(a){var args=arguments;var read=()=>a;eval('function a(){}');return args[0]===a&&read()===a&&typeof a==='function'};f(1)",
            "function f(a){var args=arguments;var read=()=>a;eval('var a=3');return {args,read}};var x=f(1);x.args[0]=8;x.read()===8",
        ]);
        // Seed the deliberately unsupported callable on the AST path, then
        // invoke it from either entry backend without compiling its declaration.
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.run_script_with_backend("function f(a){if(true){function forceFallback(){}}a=8;return arguments[0]===8}",
                ExecutionBackend::TreeWalk).unwrap();
            assert_eq!(i.run_script_with_backend("f(1)", backend).unwrap(), JsValue::Bool(true));
        }
    }

    #[test]
    fn mapped_arguments_descriptors_detach_only_after_successful_definition() {
        check(&[
            "function f(a){a=3;Object.defineProperty(arguments,'0',{writable:false});a=4;return arguments[0]===3};f(1)",
            "function f(a){Object.defineProperty(arguments,'0',{value:5});var ok=a===5;a=6;return ok&&arguments[0]===6};f(1)",
            "function f(a){Object.defineProperty(arguments,'0',{value:5,writable:false});a=6;return arguments[0]===5};f(1)",
            "function f(a){Object.defineProperty(arguments,'0',{get(){return 7}});a=8;return arguments[0]===7&&a===8};f(1)",
            "function f(a){Object.defineProperty(arguments,'0',{set(v){},configurable:true});a=8;return arguments[0]===undefined&&a===8};f(1)",
            "function f(a){Object.defineProperty(arguments,'0',{configurable:false});var ok=!Reflect.defineProperty(arguments,'0',{get(){return 2}});a=8;return ok&&arguments[0]===8};f(1)",
            "function f(a){var desc={get value(){a=4;throw 42}};try{Object.defineProperty(arguments,'0',desc)}catch(e){if(e!==42)return false};a=8;return arguments[0]===8};f(1)",
        ]);
    }

    #[test]
    fn mapped_arguments_delete_seal_freeze_and_proxy_reflection() {
        check(&[
            "function f(a){delete arguments[0];a=4;arguments[0]=5;return a===4&&arguments[0]===5};f(1)",
            "function f(a){Object.seal(arguments);var ok=!Reflect.deleteProperty(arguments,'0');a=7;return ok&&arguments[0]===7&&Object.isSealed(arguments)};f(1)",
            "function f(a){a=3;Object.freeze(arguments);a=4;return arguments[0]===3&&Object.isFrozen(arguments)&&!Reflect.set(arguments,'0',5)};f(1)",
            "function f(a){var p=new Proxy(arguments,{});a=4;var ok=Object.getOwnPropertyDescriptor(p,'0').value===4;Object.defineProperty(p,'0',{value:5});return ok&&a===5};f(1)",
            "function f(a){var p=new Proxy(arguments,{});delete p[0];a=4;return p[0]===undefined};f(1)",
        ]);
    }

    #[test]
    fn mapped_arguments_distinguish_get_and_set_receivers() {
        check(&[
            "function f(a){a=4;return Reflect.get(arguments,'0',{})===4};f(1)",
            "function f(a){var other={};var ok=Reflect.set(arguments,'0',5,other);return ok&&a===1&&arguments[0]===1&&other[0]===5};f(1)",
            "function f(a){var base={0:0};var ok=Reflect.set(base,'0',5,arguments);return ok&&a===5&&arguments[0]===5&&base[0]===0};f(1)",
            "function f(a){var other=Object.create(arguments);other[0]=6;return a===1&&arguments[0]===1&&other[0]===6};f(1)",
        ]);
    }

    #[test]
    fn unmapped_arguments_keep_intrinsics_and_brand_after_mutation() {
        check(&[
            "function f(a){'use strict';a=7;return arguments[0]===1&&Object.prototype.toString.call(arguments)==='[object Arguments]'};f(1)",
            "function f(a=1){a=7;return arguments[0]===2};f(2)",
            "function f({a}){a=7;return arguments[0].a===1};f({a:1})",
            "var original=Array.prototype.values;Array.prototype.values=function(){throw 42};function f(){return arguments[Symbol.iterator]===original};f(1)",
            "var original=Object.getOwnPropertyDescriptor(Function.prototype,'arguments').get;Object.defineProperty(Function.prototype,'arguments',{get(){return 7},configurable:true});function f(){'use strict';return Object.getOwnPropertyDescriptor(arguments,'callee').get===original};f()",
            "function f(){'use strict';try{return arguments.callee,false}catch(e){return e instanceof TypeError}};f()",
            "function f(a){arguments[Symbol.toStringTag]='Custom';return Object.prototype.toString.call(arguments)==='[object Custom]'};f(1)",
        ]);
    }
}
