use super::*;

fn runtimes() -> impl Iterator<Item = (Interpreter, ExecutionBackend)> {
    [ExecutionBackend::TreeWalk, ExecutionBackend::Bytecode]
        .into_iter()
        .map(|backend| {
            let mut i = Interpreter::new();
            i.force_ast_callables = backend == ExecutionBackend::TreeWalk;
            (i, backend)
        })
}
fn symbol(i: &mut Interpreter, backend: ExecutionBackend, source: &str) -> PropertyKey {
    let JsValue::Symbol(key) = i.run_script_with_backend(source, backend).unwrap() else {
        panic!("real Symbol")
    };
    PropertyKey::Symbol(key)
}
#[test]
fn semantic_key_get_preserves_symbol_identity_exact_string_units_and_primitive_boxing() {
    for (mut i, backend) in runtimes() {
        let object=i.run_script_with_backend("var one=Symbol('same'),two=Symbol('same');var object={[one]:1,[two]:2,[Symbol.iterator]:3};object[String.fromCharCode(0xd800)]=4;object",backend).unwrap();
        for (source, expected) in [("one", 1.0), ("two", 2.0), ("Symbol.iterator", 3.0)] {
            let key = symbol(&mut i, backend, source);
            assert_eq!(
                i.binding_get_property_key(&object, key).unwrap(),
                JsValue::Num(expected)
            );
        }
        let JsValue::Str(key) = JsValue::str_units(vec![0xd800]) else {
            unreachable!()
        };
        assert_eq!(
            i.binding_get_property_key(&object, PropertyKey::String(key))
                .unwrap(),
            JsValue::Num(4.0)
        );
        assert_eq!(
            i.binding_get_property_key(&object, "absent".into())
                .unwrap(),
            JsValue::Undefined
        );
        let key = symbol(&mut i, backend, "Symbol.iterator");
        let method = i
            .binding_get_property_key(&JsValue::str("ab"), key)
            .unwrap();
        assert!(i.binding_is_callable(&method));
        for receiver in [JsValue::Null, JsValue::Undefined] {
            // The existing binding gateway admits nullish values as absent.
            // Receiver validation remains the embedding conversion policy.
            assert_eq!(
                i.binding_get_property_key(&receiver, "x".into()).unwrap(),
                JsValue::Undefined
            );
        }
    }
}
#[test]
fn semantic_key_get_observes_only_get_once_and_preserves_original_receiver() {
    for (mut i, backend) in runtimes() {
        let object=i.run_script_with_backend("var key=Symbol('key'),hits=0;var proto={get [key](){hits++;this.extra=7;return this}};var object=Object.create(proto);object",backend).unwrap();
        let key = symbol(&mut i, backend, "key");
        assert_eq!(i.binding_get_property_key(&object, key).unwrap(), object);
        assert_eq!(
            i.run_script_with_backend("hits===1&&object.extra===7", backend)
                .unwrap(),
            JsValue::Bool(true)
        );
        let proxy=i.run_script_with_backend("var gets=0;var proxy=new Proxy(object,{get(t,k,r){gets++;if(k!==key||r!==proxy)throw 2;return Reflect.get(t,k,r)},has(){throw 3},getOwnPropertyDescriptor(){throw 4}});proxy",backend).unwrap();
        // Use a read-only getter here: reentrancy above separately measures
        // receiver writes; the Proxy must observe exactly the semantic Get.
        i.run_script_with_backend(
            "Object.defineProperty(proto,key,{get(){hits++;return this}})",
            backend,
        )
        .unwrap();
        let key = symbol(&mut i, backend, "key");
        assert_eq!(i.binding_get_property_key(&proxy, key).unwrap(), proxy);
        assert_eq!(
            i.run_script_with_backend("gets===1&&hits===2", backend)
                .unwrap(),
            JsValue::Bool(true)
        );
    }
}
#[test]
fn semantic_key_get_preserves_arbitrary_abrupt_identity_revocation_and_foreign_getter_home() {
    for (mut i, backend) in runtimes() {
        let object=i.run_script_with_backend("var sentinel={get name(){throw 9}};var key=Symbol('key');({get [key](){throw sentinel}})",backend).unwrap();
        let sentinel = i.run_script_with_backend("sentinel", backend).unwrap();
        let key = symbol(&mut i, backend, "key");
        assert_eq!(
            i.binding_get_property_key(&object, key).unwrap_err().0,
            sentinel
        );
        let proxy = i
            .run_script_with_backend("new Proxy({},{get(){throw sentinel}})", backend)
            .unwrap();
        let key = symbol(&mut i, backend, "Symbol.iterator");
        assert_eq!(
            i.binding_get_property_key(&proxy, key).unwrap_err().0,
            sentinel
        );
        let revoked = i
            .run_script_with_backend("var r=Proxy.revocable({},{});r.revoke();r.proxy", backend)
            .unwrap();
        let key = symbol(&mut i, backend, "Symbol.iterator");
        assert!(i.binding_get_property_key(&revoked, key).is_err());
        let realm = i
            .binding_create_realm(JsValue::object(JsObject::ordinary()))
            .unwrap();
        let foreign=i.binding_eval_in_realm(realm,"var sentinel={get name(){throw 9}};var hits=0;({get [Symbol.iterator](){hits++;throw sentinel}})").unwrap();
        let sentinel = i.binding_eval_in_realm(realm, "sentinel").unwrap();
        let key = symbol(&mut i, backend, "Symbol.iterator");
        assert_eq!(
            i.binding_get_property_key(&foreign, key).unwrap_err().0,
            sentinel
        );
        assert_eq!(
            i.binding_eval_in_realm(realm, "hits").unwrap(),
            JsValue::Num(1.0)
        );
        assert_eq!(
            i.run_script_with_backend("typeof hits==='undefined'", backend)
                .unwrap(),
            JsValue::Bool(true)
        );
    }
}
