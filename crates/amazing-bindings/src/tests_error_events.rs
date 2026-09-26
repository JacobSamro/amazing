use super::*;

#[test]
fn error_event_conversions_finish_before_retaining_any_payload_roots() {
    let mut h = Host::default();
    let payload = V::Dictionary(vec![]);
    assert!(invoke(&mut h, O::ErrorEventConstructor, V::Undefined, &[str_value("x"),
        V::Dictionary(vec![("error", payload.clone()), ("message", V::Throw)])]).is_err());
    assert!(h.callbacks.is_empty());
    h.property_reads.clear();
    let event = call(&mut h, O::ErrorEventConstructor, V::Undefined, &[str_value("x"), V::Dictionary(vec![
        ("colno", V::Number(-1.)), ("error", V::Null), ("filename", V::String(vec![0xd800])),
        ("lineno", V::Number(f64::INFINITY)), ("message", V::String(vec![0xd800]))])]);
    assert_eq!(h.property_reads, ["bubbles", "cancelable", "composed", "colno", "error", "filename", "lineno", "message"]);
    for (name, expected) in [("colno", V::Number(4294967295.)), ("lineno", V::Number(0.)),
        ("error", V::Null), ("filename", V::String(vec![0xfffd])), ("message", V::String(vec![0xd800]))] {
        assert_eq!(call(&mut h, O::ErrorEventGet(name), event.clone(), &[]), expected);
    }
    assert!(invoke(&mut h, O::PromiseRejectionEventGet("reason"), event, &[]).is_err());
}

#[test]
fn rejection_event_requires_object_and_keeps_undefined_distinct_from_null() {
    let mut h = Host::default();
    let promise = V::Dictionary(vec![]);
    for (extra, expected) in [(vec![], V::Undefined), (vec![("reason", V::Null)], V::Null),
        (vec![("reason", V::Bool(false))], V::Bool(false))] {
        let mut dictionary = vec![("promise", promise.clone())]; dictionary.extend(extra);
        h.property_reads.clear();
        let event = call(&mut h, O::PromiseRejectionEventConstructor, V::Undefined,
            &[V::String(vec![0xd800]), V::Dictionary(dictionary)]);
        assert_eq!(h.property_reads, ["bubbles", "cancelable", "composed", "promise", "reason"]);
        assert_eq!(call(&mut h, O::PromiseRejectionEventGet("reason"), event.clone(), &[]), expected);
        assert_eq!(call(&mut h, O::PromiseRejectionEventGet("promise"), event.clone(), &[]), promise);
        assert_eq!(call(&mut h, O::EventType, event, &[]), V::String(vec![0xd800]));
    }
    for value in [V::Undefined, V::Null, V::Number(1.), str_value("x")] {
        assert!(invoke(&mut h, O::PromiseRejectionEventConstructor, V::Undefined,
            &[str_value("x"), V::Dictionary(vec![("promise", value)])]).is_err());
    }
}
