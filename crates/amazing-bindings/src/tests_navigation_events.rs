use super::*;

#[test]
fn navigation_dictionaries_convert_in_inheritance_and_member_order() {
    let mut h = Host::default();
    for (constructor, fields) in [
        (O::HashChangeEventConstructor, vec!["bubbles", "cancelable", "composed", "newURL", "oldURL"]),
        (O::PopStateEventConstructor, vec!["bubbles", "cancelable", "composed", "hasUAVisualTransition", "state"]),
        (O::PageTransitionEventConstructor, vec!["bubbles", "cancelable", "composed", "persisted"]),
    ] {
        h.property_reads.clear();
        call(&mut h, constructor, V::Undefined, &[str_value("x"), V::Dictionary(vec![])]);
        assert_eq!(h.property_reads, fields);
        assert!(invoke(&mut h, constructor, V::Undefined, &[]).is_err());
        assert!(invoke(&mut h, constructor, V::Undefined, &[str_value("x"), V::Bool(true)]).is_err());
    }
    let event = call(&mut h, O::HashChangeEventConstructor, V::Undefined, &[str_value("x"),
        V::Dictionary(vec![("oldURL", V::String(vec![0xd800])), ("newURL", V::Null)])]);
    assert_eq!(call(&mut h, O::HashChangeOldURL, event.clone(), &[]), str_value("�"));
    assert_eq!(call(&mut h, O::HashChangeNewURL, event, &[]), str_value("null"));
}

#[test]
fn navigation_payload_getters_enforce_their_own_interface_and_retain_any_state() {
    let mut h = Host::default();
    let payload = V::Dictionary(vec![("x", V::Number(1.))]);
    let event = call(&mut h, O::PopStateEventConstructor, V::Undefined, &[str_value("x"),
        V::Dictionary(vec![("state", payload.clone()), ("hasUAVisualTransition", V::Number(1.))])]);
    assert_eq!(call(&mut h, O::PopStateState, event.clone(), &[]), payload);
    assert_eq!(call(&mut h, O::PopStateHasUAVisualTransition, event.clone(), &[]), V::Bool(true));
    assert!(invoke(&mut h, O::HashChangeOldURL, event.clone(), &[]).is_err());
    assert!(invoke(&mut h, O::PageTransitionPersisted, event.clone(), &[]).is_err());
    h.conversions.clear();
    assert!(invoke(&mut h, O::BeforeUnloadSetReturnValue, event, &[V::Coerce("x")]).is_err());
    assert!(h.conversions.is_empty());
    let default = call(&mut h, O::PopStateEventConstructor, V::Undefined, &[str_value("x")]);
    assert_eq!(call(&mut h, O::PopStateState, default, &[]), V::Null);
}

#[test]
fn before_unload_string_return_value_is_independent_of_event_cancellation() {
    let mut h = Host::default();
    let event = call(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[str_value("beforeunloadevent")]);
    assert_eq!(call(&mut h, O::BeforeUnloadReturnValue, event.clone(), &[]), str_value(""));
    call(&mut h, O::EventInitEvent, event.clone(), &[str_value("beforeunload"), V::Bool(false), V::Bool(true)]);
    call(&mut h, O::BeforeUnloadSetReturnValue, event.clone(), &[V::Coerce("false")]);
    assert_eq!(call(&mut h, O::BeforeUnloadReturnValue, event.clone(), &[]), str_value("false"));
    assert_eq!(call(&mut h, O::EventReturnValue, event.clone(), &[]), V::Bool(true));
    call(&mut h, O::EventPreventDefault, event.clone(), &[]);
    assert_eq!(call(&mut h, O::EventReturnValue, event.clone(), &[]), V::Bool(false));
    assert_eq!(call(&mut h, O::BeforeUnloadReturnValue, event.clone(), &[]), str_value("false"));
    call(&mut h, O::EventInitEvent, event.clone(), &[str_value("again")]);
    assert_eq!(call(&mut h, O::BeforeUnloadReturnValue, event, &[]), str_value("false"));
}
