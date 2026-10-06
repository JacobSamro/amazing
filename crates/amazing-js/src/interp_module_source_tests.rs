#[cfg(test)]
mod module_source_intrinsic_tests {
    use super::*;
    fn install_values(i: &mut Interpreter) {
        let ctor=i.binding_abstract_module_source_constructor();
        i.global_env.declare_var("AMS",ctor).unwrap();
        let record=crate::module::CompiledModuleSource::compile("native",&"export const value=42".encode_utf16().collect::<Vec<_>>()).unwrap();
        let source=i.binding_create_compiled_module_source(Rc::new(record));
        i.global_env.declare_var("source",source).unwrap();
    }
    #[test]
    fn private_intrinsic_metadata_and_prototype_chain_are_authentic() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);install_values(&mut i);
            let result=i.run_script_with_backend(r#"
                var p=AMS.prototype,d=Object.getOwnPropertyDescriptor(AMS,'prototype');
                var tag=Object.getOwnPropertyDescriptor(p,Symbol.toStringTag);
                typeof AbstractModuleSource==='undefined' && AMS.name==='AbstractModuleSource' && AMS.length===0 &&
                Object.getPrototypeOf(AMS)===Function.prototype && Object.getPrototypeOf(p)===Object.prototype &&
                p.constructor===AMS && !d.writable && !d.enumerable && !d.configurable &&
                !tag.enumerable && tag.configurable && tag.set===undefined &&
                tag.get.name==='get [Symbol.toStringTag]' && tag.get.length===0 &&
                Object.getPrototypeOf(Object.getPrototypeOf(source))===p && source instanceof AMS &&
                source[Symbol.toStringTag]==='CompiledModuleSource' && Object.prototype.toString.call(source)==='[object CompiledModuleSource]'
            "#,backend).unwrap();assert_eq!(result,JsValue::Bool(true));
        }
    }
    #[test]
    fn abstract_call_and_construct_reject_in_the_intrinsic_realm() {
        let mut i=Interpreter::new();install_values(&mut i);
        assert_eq!(i.run_script("var a=false,b=false;try{AMS()}catch(e){a=e instanceof TypeError}try{new AMS()}catch(e){b=e instanceof TypeError}a&&b").unwrap(),JsValue::Bool(true));
        let parent=i.binding_current_realm();let global=i.binding_create_object(None).unwrap();let child=i.binding_create_realm(global).unwrap();
        let foreign=i.binding_with_realm(child,|i|Ok(i.binding_abstract_module_source_constructor())).unwrap();
        let foreign_type=i.binding_with_realm(child,|i|i.run_script("TypeError")).unwrap();
        i.global_env.declare_var("foreignAMS",foreign).unwrap();i.global_env.declare_var("foreignType",foreign_type).unwrap();
        assert_eq!(i.run_script("var a=false,b=false;try{foreignAMS()}catch(e){a=e instanceof foreignType&&!(e instanceof TypeError)}try{new foreignAMS()}catch(e){b=e instanceof foreignType&&!(e instanceof TypeError)}a&&b").unwrap(),JsValue::Bool(true));
        assert_eq!(i.run_script("var reads=0,nt=new Proxy(function(){},{get(t,k){if(k==='prototype'){reads++;throw 'prototype'}return Reflect.get(t,k)}});var rejected=false;try{Reflect.construct(foreignAMS,[],nt)}catch(e){rejected=e instanceof foreignType}rejected&&reads===0").unwrap(),JsValue::Bool(true));
        assert_eq!(i.binding_current_realm(),parent);
    }
    #[test]
    fn tag_getter_has_no_authored_brand_reads_and_returns_undefined_for_nonsources() {
        for backend in [ExecutionBackend::TreeWalk,ExecutionBackend::Bytecode] {
            let mut i=Interpreter::new();i.test_force_ast_callables(backend==ExecutionBackend::TreeWalk);install_values(&mut i);
            assert_eq!(i.run_script_with_backend(r#"
                var get=Object.getOwnPropertyDescriptor(AMS.prototype,Symbol.toStringTag).get;
                var seen=0,hostile=new Proxy(source,{get(){seen++;throw 'trap'}});
                var revoked=Proxy.revocable(source,{});revoked.revoke();
                var values=[undefined,null,false,0,'text',Symbol(),{},Object.create(AMS.prototype),AMS.prototype,hostile,revoked.proxy];
                var valid=values.every(v=>get.call(v)===undefined);
                Object.defineProperty(source,'module',{get(){seen++;throw 'bad'}});
                valid&&seen===0&&get.call(source)==='CompiledModuleSource'
            "#,backend).unwrap(),JsValue::Bool(true));
        }
    }
    #[test]
    fn private_factory_ignores_poisoned_global_names_and_object_prototype() {
        let mut i=Interpreter::new();install_values(&mut i);
        i.run_script("globalThis.AbstractModuleSource=function(){throw 'poison'};globalThis.CompiledModuleSource=function(){throw 'poison'}").unwrap();
        let record=crate::module::CompiledModuleSource::compile("native2",&"export const value=2".encode_utf16().collect::<Vec<_>>()).unwrap();
        let source=i.binding_create_compiled_module_source(Rc::new(record));i.global_env.declare_var("second",source).unwrap();
        assert_eq!(i.run_script("second instanceof AMS&&Object.getPrototypeOf(second)===Object.getPrototypeOf(source)&&second!==source").unwrap(),JsValue::Bool(true));
    }
}
