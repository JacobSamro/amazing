#[cfg(test)]
mod eval_declaration_protocol_tests {
    use super::*;
    fn matrix(source: &str) {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i = Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            assert_eq!(i.run_script_with_backend(source, backend)
                .unwrap_or_else(|error| panic!("{backend:?}: {source}: {}", error.describe())), JsValue::Bool(true));
        }
    }
    #[test]
    fn global_functions_replace_configurable_descriptors_for_all_four_kinds() {
        for direct in [true, false] {
            for declaration in ["function f(){return 345}", "function* f(){yield 345}",
                "async function f(){return 345}", "async function* f(){yield 345}"] {
                for writable in [true, false] { for enumerable in [true, false] {
                    let call = if direct { "eval" } else { "(0,eval)" };
                    matrix(&format!("Object.defineProperty(globalThis,'f',{{value:1,writable:{writable},enumerable:{enumerable},configurable:true}});{call}('{declaration}');var d=Object.getOwnPropertyDescriptor(globalThis,'f');typeof f==='function'&&d.value===f&&d.writable&&d.enumerable&&d.configurable"));
                }}
                matrix(&format!("var touches=0;Object.defineProperty(globalThis,'f',{{get(){{touches++;throw 1}},set(v){{touches++;throw 2}},configurable:true}});{}('{declaration}');var d=Object.getOwnPropertyDescriptor(globalThis,'f');touches===0&&typeof d.value==='function'&&d.writable&&d.enumerable&&d.configurable",if direct{"eval"}else{"(0,eval)"}));
            }
        }
        matrix("Object.defineProperty(globalThis,'f',{value:1,writable:true,enumerable:true,configurable:false});eval('function f(){return 4}');var d=Object.getOwnPropertyDescriptor(globalThis,'f');f()===4&&d.writable&&d.enumerable&&!d.configurable");
        matrix("eval('function f(){return 1}function f(){return 2}');f()===2");
    }
    #[test]
    fn global_vars_preserve_existing_values_accessors_and_prevalidate_failures() {
        matrix("var hits=0;Object.defineProperty(globalThis,'x',{get(){hits++;return 9},configurable:true});eval('var x');hits===0&&typeof Object.getOwnPropertyDescriptor(globalThis,'x').get==='function'");
        matrix("Object.defineProperty(globalThis,'x',{value:9,writable:false,configurable:false});eval('var x');x===9");
        for descriptor in ["{value:1,writable:false,enumerable:true}", "{value:1,writable:true,enumerable:false}", "{get(){throw 1},enumerable:true}"] {
            matrix(&format!("Object.defineProperty(globalThis,'bad',{descriptor});var caught=false;try{{eval('var untouched;function good(){{}}function bad(){{}}')}}catch(e){{caught=e instanceof TypeError}}caught&&!Object.hasOwn(globalThis,'untouched')&&!Object.hasOwn(globalThis,'good')"));
        }
        matrix("var caught=false;Object.preventExtensions(globalThis);try{eval('var missing;function absent(){}')}catch(e){caught=e instanceof TypeError}caught&&!Object.hasOwn(globalThis,'missing')&&!Object.hasOwn(globalThis,'absent')");
        matrix("let conflict;var ran=false,caught=false;try{eval('var untouched;function conflict(){};ran=true')}catch(e){caught=e instanceof SyntaxError}caught&&!ran&&!Object.hasOwn(globalThis,'untouched')");
        matrix("eval('var read=()=>lexical;let lexical=7');read()===7&&!Object.hasOwn(globalThis,'lexical')");
    }
    #[test]
    fn actual_proxy_global_preserves_internal_operations_and_abrupt_identity() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for reject in [false, true] {
                let mut i=Interpreter::new();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
                let proxy=i.run_script_with_backend(r#"
                    var events=[],sentinel={get name(){throw 2}};
                    new Proxy(globalThis,{
                      getOwnPropertyDescriptor(t,k){if(k==='fresh')events.push('own');return Reflect.getOwnPropertyDescriptor(t,k)},
                      isExtensible(t){events.push('ext');return Reflect.isExtensible(t)},
                      defineProperty(t,k,d){if(k==='fresh')events.push('define');return Reflect.defineProperty(t,k,d)},
                      set(t,k,v,r){if(k==='fresh'){events.push('set');if(reject)throw sentinel}return Reflect.set(t,k,v,r)}
                    })
                "#,backend).unwrap();
                i.global_env.declare_var("reject",JsValue::Bool(reject)).unwrap();
                i.global_env.declare_var("caught",JsValue::Bool(false)).unwrap();
                let JsValue::Object(proxy)=proxy else {panic!("actual proxy global")};
                i.global_obj=proxy.clone();i.global_env.set_global_object(proxy);
                let source=if reject {
                    "caught=false;try{eval('function fresh(){}')}catch(e){caught=e===sentinel}caught&&events.join(',').startsWith('own,ext,own,define,set')&&typeof globalThis.fresh==='function'"
                } else {
                    "eval('var fresh;var fresh');events.join(',').startsWith('own,ext,own,ext,own,ext,define,set')&&Object.getOwnPropertyDescriptor(globalThis,'fresh').configurable"
                };
                assert_eq!(i.run_script_with_backend(source,backend).unwrap(),JsValue::Bool(true),"{backend:?} reject={reject}");
            }
        }
    }
    #[test]
    fn intervening_lexical_records_are_checked_without_object_operations() {
        for declaration in ["let x=1", "const x=1", "class x{}"] {
            matrix(&format!("var caught=false;(function(){{{{{declaration};{{try{{eval('var x')}}catch(e){{caught=e instanceof SyntaxError}}}}}}}})();caught"));
        }
        matrix("var caught=false;for(let x=0;x<1;x++){try{eval('var x')}catch(e){caught=e instanceof SyntaxError}}caught");
        matrix("var caught=false;switch(1){case 1:let x;try{eval('var x')}catch(e){caught=e instanceof SyntaxError}}caught");
        matrix("var hits=0;var obj=new Proxy({}, {has(){hits++;return false}});(function(){with(obj){eval('var fresh')}})();hits===1");
        matrix("(function(){let x=1;eval('\"use strict\";var x=2');return x===1})()");
        matrix("(function(){let x=1;(0,eval)('var x=2');return x===1})()&&globalThis.x===2");
        matrix("var caught=false;try{(function(x=eval('var x')){})()}catch(e){caught=e instanceof SyntaxError}caught");
    }
    #[test]
    fn authentic_catch_parameter_exception_keeps_body_block_barriers() {
        for parameter in ["x", "{x}", "[x]"] {
            let thrown = match parameter { "x"=>"1", "{x}"=>"({x:1})", _=>"[1]" };
            matrix(&format!("(function(){{try{{throw {thrown}}}catch({parameter}){{eval('var x');return x===1}}}})()"));
        }
        matrix("(function(){var caught=false;try{throw 1}catch(x){{let y;try{eval('var y')}catch(e){caught=e instanceof SyntaxError}}}return caught})()");
        matrix("(function(){try{throw 1}catch{let x;try{eval('var x')}catch(e){return e instanceof SyntaxError}}return false})()");
        matrix("(function(){try{throw 1}catch(x){eval('var x=3');return x===3}})()");
    }
    #[test]
    fn annex_candidates_promote_only_the_actual_eligible_occurrence() {
        for body in ["{function f(){return 4}}", "if(true){function f(){return 4}}", "if(true)function f(){return 4}",
            "label:{function f(){return 4}}", "for(var n=0;n<1;n++){function f(){return 4}}",
            "switch(1){case 1:function f(){return 4}}", "try{function f(){return 4}}finally{}",
            "try{throw 1}catch(e){function f(){return 4}}", "try{}finally{function f(){return 4}}"] {
            matrix(&format!("eval('{body}');f()===4&&Object.getOwnPropertyDescriptor(globalThis,'f').configurable"));
        }
        matrix("eval('if(false){function f(){}}');f===undefined&&Object.hasOwn(globalThis,'f')");
        matrix("eval('{function f(){return 1}}{let f=2;{function f(){return 3}}}');f()===1");
        matrix("let f=7;eval('{function f(){return 4}}');f===7&&!Object.hasOwn(globalThis,'f')");
        matrix("Object.defineProperty(globalThis,'f',{value:7,writable:false,configurable:false});eval('{function f(){return 4}}');f===7");
        matrix("eval('\"use strict\";{function f(){}}');!Object.hasOwn(globalThis,'f')");
        matrix("eval('{async function f(){};function* g(){};async function* h(){}}');!Object.hasOwn(globalThis,'f')&&!Object.hasOwn(globalThis,'g')&&!Object.hasOwn(globalThis,'h')");
    }
    #[test]
    fn genuine_later_scripts_can_lexically_shadow_eval_globals() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            for source in ["eval('var x=3')", "eval('function x(){return 3}')", "eval('if(true){function x(){return 3}}')"] {
                for declaration in ["let x=7", "const x=7", "class x{}"] {
                    let mut i=Interpreter::new();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
                    i.run_script_with_backend(source,backend).unwrap();
                    i.run_script_with_backend(declaration,backend).unwrap();
                    assert_eq!(i.run_script_with_backend("x!==globalThis.x&&Object.hasOwn(globalThis,'x')",backend).unwrap(),JsValue::Bool(true));
                    assert!(!i.global_env.has_script_var("x"));
                }
            }
        }
    }
    #[test]
    fn foreign_eval_home_and_arbitrary_abrupt_identity_are_preserved() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
            let realm=i.binding_create_realm(JsValue::object(JsObject::ordinary())).unwrap();
            let foreign_eval=i.binding_eval_in_realm(realm,"eval").unwrap();
            let foreign_syntax=i.binding_eval_in_realm(realm,"SyntaxError.prototype").unwrap();
            i.global_env.declare_var("foreignEval",foreign_eval).unwrap();
            i.global_env.declare_var("foreignSyntax",foreign_syntax).unwrap();
            assert_eq!(i.run_script_with_backend("var caught=false;try{foreignEval('let x;var x')}catch(e){caught=Object.getPrototypeOf(e)===foreignSyntax}caught",backend).unwrap(),JsValue::Bool(true));
            assert_eq!(i.run_script_with_backend("(function(){let x=1;foreignEval('var x=7');return x===1})()",backend).unwrap(),JsValue::Bool(true));
            assert_eq!(i.binding_eval_in_realm(realm,"x===7").unwrap(),JsValue::Bool(true));
        }
        matrix("var sentinel={get name(){throw 2}};var caught=false;try{eval('throw sentinel')}catch(e){caught=e===sentinel}caught");
        matrix("var sentinel={};var caught=false;try{eval({toString(){throw sentinel}})}catch(e){caught=true}caught===false");
    }
    #[test]
    fn actual_ast_and_bytecode_factories_preserve_eval_occurrence_eligibility() {
        for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();i.force_ast_callables=backend==ExecutionBackend::TreeWalk;
            let function=i.run_script_with_backend("(function probe(){return eval('var local=1;local')})",backend).unwrap();
            let JsValue::Object(object)=&function else {panic!("actual function factory")};
            let body=object.borrow();let data=body.func.as_ref().unwrap();
            assert!(match backend {
                ExecutionBackend::TreeWalk=>matches!(data.body,FunctionBody::Block(_)),
                ExecutionBackend::Bytecode=>matches!(data.body,FunctionBody::Bytecode(_)),
            },"{backend:?}: genuine callable backend");
            drop(body);
            assert_eq!(i.call_value(&function,JsValue::Undefined,vec![]).unwrap(),JsValue::Num(1.0));
        }
        for body in ["{function p(){return 3}}", "if(true)function p(){return 3}", "switch(1){case 1:function p(){return 3}}"] {
            matrix(&format!("(function(p){{eval('{body}');return p()===3}})(1)"));
            matrix(&format!("(function(p){{{body};return p===1}})(1)"));
        }
        matrix("(function(p){eval('{function p(){return 3}}');eval('{function p(){return 4}}');return p()===4})(1)");
        matrix("(function(p){eval('(function inner(p){{function p(){return 3}}if(p!==1)throw 4})(1)');return p===1})(1)");
        matrix("(function({p}){eval('{function p(){return 3}}');return p()===3})({p:1})");
        matrix("(function(p=1){eval('{function p(){return 3}}');return p()===3})()");
        matrix("(function(...p){eval('{function p(){return 3}}');return p()===3})(1)");
        matrix("(function(arguments){eval('{function arguments(){return 3}}');return arguments()===3})(1)");
    }
    #[test]
    fn local_delete_strictness_alias_and_source_protocol_guards() {
        matrix("(function(){eval('var x=1');var ok=delete x;return ok&&typeof x==='undefined'})()");
        matrix("(function(){eval('var x=1;x=(delete x,4)');return x===4&&delete x})()");
        matrix("(function(x){eval('var x=7');return x===7&&arguments[0]===7&&!delete x})(1)");
        matrix("(function(){eval('\"use strict\";var x=1');return typeof x==='undefined'})()");
        matrix("var text='function f /* spelling */(){return 7}';eval(text);f.toString()===text");
    }
}
