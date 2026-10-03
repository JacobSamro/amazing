use super::*;
use crate::interp::ExecutionBackend;

fn check(i: &mut Interpreter, backend: ExecutionBackend, source: &str) {
    let result = i
        .run_script_with_backend(source, backend)
        .unwrap_or_else(|e| panic!("{backend:?}: {}", e.describe()));
    assert_eq!(result, JsValue::Bool(true), "{backend:?}");
}

#[test]
fn namespace_enumeration_and_super_receiver_observe_uninitialized_exports() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let mut linker = ModuleLinker::new(|_: &str, _: &str| Err("unresolved".into()));
        linker.link("leaf", "export let value=7;").unwrap();
        linker.instantiate_pending("leaf", &mut i).unwrap();
        let ns = linker.namespace_object("leaf").unwrap();
        i.global_env.declare_var("ns", ns).unwrap();
        check(
            &mut i,
            backend,
            r#"
            var caught=0;
            try{Object.keys(ns)}catch(e){if(e instanceof ReferenceError)caught++}
            try{for(var key in ns){throw 8}}catch(e){if(e instanceof ReferenceError)caught++}
            class Base{constructor(){return ns}}
            class Derived extends Base{constructor(){super();super.value=7}}
            try{new Derived()}catch(e){if(e instanceof ReferenceError)caught++}
            Reflect.ownKeys(ns).length===2&&Object.getOwnPropertyNames(ns).join(',')==='value'&&caught===3;
        "#,
        );
    }
}

#[test]
fn ordinary_super_set_to_namespace_receiver_uses_minimal_define_and_samevalue() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        let mut i = Interpreter::new();
        let mut linker = ModuleLinker::new(|_: &str, _: &str| Err("unresolved".into()));
        linker.link("leaf", "export let value=7;").unwrap();
        linker.evaluate("leaf", &mut i).unwrap();
        i.global_env
            .declare_var("ns", linker.namespace_object("leaf").unwrap())
            .unwrap();
        check(
            &mut i,
            backend,
            r#"
            var base={},same=Reflect.set(base,'value',7,ns),different=Reflect.set(base,'value',8,ns);
            var missing=Reflect.set(base,'missing',9,ns);
            class Base{constructor(){return ns}}
            class Good extends Base{constructor(){super();super.value=7}}
            class Bad extends Base{constructor(){super();super.value=8}}
            var good=new Good()===ns,bad=false;
            try{new Bad()}catch(e){bad=e instanceof TypeError}
            same&&!different&&!missing&&good&&bad&&ns.value===7;
        "#,
        );
    }
}

#[test]
fn deferred_receiver_operations_evaluate_once_while_direct_set_and_private_do_not() {
    for backend in [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode] {
        for operation in [
            "Object.keys(ns)",
            "for(var key in ns){}",
            "Reflect.set({},'value',7,ns)",
            "Reflect.set({},'missing',7,ns)",
        ] {
            let linker = Rc::new(RefCell::new(ModuleLinker::new(|name: &str, _: &str| {
                if name == "leaf" {
                    Ok((name.into(), "count++;export const value=7;".into()))
                } else {
                    Err("unresolved".into())
                }
            })));
            let mut i = Interpreter::new();
            i.run_script("var count=0").unwrap();
            linker
                .borrow_mut()
                .link(
                    "root",
                    "import defer * as ns from 'leaf';globalThis.saved=ns;",
                )
                .unwrap();
            ModuleLinker::evaluate_shared(&linker, "root", &mut i).unwrap();
            check(
                &mut i,
                backend,
                r#"
                var ns=saved;
                class Marker extends function(value){return value}{#mark=1;static has(value){return #mark in value}}
                var failed=false;try{new Marker(ns)}catch(e){failed=e instanceof TypeError}
                var plain=Object.preventExtensions({});new Marker(plain);
                failed&&!Marker.has(ns)&&Marker.has(plain)&&!Reflect.set(ns,'value',8)&&count===0;
            "#,
            );
            i.run_script_with_backend(operation, backend).unwrap();
            check(&mut i, backend, "count===1&&ns.value===7");
        }
    }
}
