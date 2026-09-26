use super::*;

#[test]
fn dom_exception_defaults_null_conversion_and_coercion_order() {
    let mut h = Host::default();
    for args in [vec![], vec![V::Undefined], vec![V::Undefined, V::Undefined]] {
        let value = call(&mut h, O::DOMExceptionConstructor, V::Undefined, &args);
        assert_eq!(call(&mut h, O::DOMExceptionMessage, value.clone(), &[]), str_value(""));
        assert_eq!(call(&mut h, O::DOMExceptionName, value.clone(), &[]), str_value("Error"));
        assert_eq!(call(&mut h, O::DOMExceptionCode, value, &[]), V::Number(0.));
    }
    let value = call(&mut h, O::DOMExceptionConstructor, V::Undefined, &[V::Null, V::Null]);
    assert_eq!(call(&mut h, O::DOMExceptionMessage, value.clone(), &[]), str_value("null"));
    assert_eq!(call(&mut h, O::DOMExceptionName, value, &[]), str_value("null"));
    h.conversions.clear();
    call(&mut h, O::DOMExceptionConstructor, V::Undefined, &[V::Coerce("message"), V::Coerce("name")]);
    assert_eq!(h.conversions, ["message", "name"]);
    h.conversions.clear();
    assert!(invoke(&mut h, O::DOMExceptionConstructor, V::Undefined, &[V::Throw, V::Coerce("name")]).is_err());
    assert!(h.conversions.is_empty());
}

#[test]
fn dom_exception_getters_reject_wrong_brands_realms_and_generations() {
    let mut h = Host::default();
    let value = call(&mut h, O::DOMExceptionConstructor, V::Undefined, &[]);
    let V::Platform(key, _) = value else { panic!("missing platform identity") };
    for op in [O::DOMExceptionName, O::DOMExceptionMessage, O::DOMExceptionCode] {
        for receiver in [V::Undefined, V::Null, V::Dictionary(vec![]),
            V::Platform(key, InterfaceBrand::EVENT),
            V::Platform(PlatformKey { arena: key.arena + 1, ..key }, InterfaceBrand::DOM_EXCEPTION),
            V::Platform(PlatformKey { generation: key.generation + 1, ..key }, InterfaceBrand::DOM_EXCEPTION)] {
            assert!(invoke(&mut h, op, receiver, &[]).is_err());
        }
    }
}
