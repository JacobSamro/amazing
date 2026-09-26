use super::*;
use amazing_dom::events::{DispatchOrigin, EventId, EventTarget};

fn message(host: &mut Host, init: V) -> V {
    call(host, O::MessageEventConstructor, V::Undefined, &[str_value("message"), init])
}
fn get(host: &mut Host, event: &V, name: &'static str) -> V {
    call(host, O::MessageEventGet(name), event.clone(), &[])
}
fn port(host: &mut Host, name: &str) -> (u64, V) {
    let slot = host.dom.create_event_target();
    let value = V::Dictionary(vec![("name", str_value(name))]);
    host.external_targets.push((slot, value.clone()));
    (slot, value)
}
fn event_id(value: &V) -> EventId {
    let V::Platform(key, _) = value else { panic!("not a native event") };
    EventId::from_slot(key.slot as usize)
}

#[test]
fn defaults_brand_and_dictionary_read_order() {
    let mut h = Host::default();
    let event = message(&mut h, V::Dictionary(vec![]));
    assert_eq!(h.property_reads, ["bubbles", "cancelable", "composed", "data", "lastEventId", "origin", "ports", "source"]);
    assert_eq!(get(&mut h, &event, "data"), V::Null);
    assert_eq!(get(&mut h, &event, "source"), V::Null);
    assert_eq!(get(&mut h, &event, "origin"), str_value(""));
    assert_eq!(get(&mut h, &event, "lastEventId"), str_value(""));
    assert_eq!(get(&mut h, &event, "ports"), V::Array(vec![]));
    assert_eq!(h.dom.events().event(event_id(&event)).class(), amazing_dom::events::EventClass::Message);
    let plain = call(&mut h, O::EventConstructor, V::Undefined, &[str_value("message")]);
    assert!(invoke(&mut h, O::MessageEventGet("data"), plain, &[]).is_err());
    assert!(invoke(&mut h, O::MessageEventConstructor, V::Undefined, &[]).is_err());
    assert!(invoke(&mut h, O::MessageEventConstructor, V::Undefined,
        &[str_value("message"), V::Number(1.)]).is_err());
}

#[test]
fn payload_identity_utf16_and_invalid_sources() {
    let mut h = Host::default();
    let (_, p) = port(&mut h, "port");
    let payload = V::Dictionary(vec![("value", V::Number(42.))]);
    let event = message(&mut h, V::Dictionary(vec![
        ("data", payload.clone()), ("source", p.clone()), ("ports", V::Array(vec![p.clone()])),
        ("origin", V::String(vec![0xd800])), ("lastEventId", V::String(vec![0xd800])),
    ]));
    assert_eq!(get(&mut h, &event, "data"), payload);
    assert_eq!(get(&mut h, &event, "source"), p.clone());
    assert_eq!(get(&mut h, &event, "ports"), V::Array(vec![p]));
    assert_eq!(get(&mut h, &event, "origin"), V::String(vec![0xfffd]));
    assert_eq!(get(&mut h, &event, "lastEventId"), V::String(vec![0xd800]));
    for (field, value) in [("source", V::Dictionary(vec![])), ("ports", V::Null),
        ("ports", V::Array(vec![V::Dictionary(vec![])]))] {
        let roots = h.callbacks.len();
        assert!(invoke(&mut h, O::MessageEventConstructor, V::Undefined,
            &[str_value("message"), V::Dictionary(vec![(field, value)])]).is_err());
        assert_eq!(h.callbacks.len(), roots, "failed conversion retains no roots");
    }
}

#[test]
fn legacy_init_releases_replaced_values_and_is_ignored_during_dispatch() {
    let mut h = Host::default();
    let event = call(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[str_value("mEsSaGeEvEnT")]);
    assert_eq!(get(&mut h, &event, "data"), V::Null);
    assert_eq!(get(&mut h, &event, "ports"), V::Array(vec![]));
    let (slot, p) = port(&mut h, "port");
    assert!(invoke(&mut h, O::DispatchEvent, p.clone(), &[event.clone()]).is_err());
    call(&mut h, O::MessageEventInit, event.clone(), &[str_value("message"), V::Bool(true),
        V::Bool(true), str_value("payload")]);
    assert_eq!(get(&mut h, &event, "data"), str_value("payload"));
    let live = h.callbacks.iter().flatten().count();
    let mut dispatch = h.dom.begin_event_dispatch(EventTarget::External(slot), event_id(&event), DispatchOrigin::Script).unwrap();
    call(&mut h, O::MessageEventInit, event.clone(), &[str_value("ignored"), V::Bool(false),
        V::Bool(false), str_value("discarded")]);
    assert_eq!(get(&mut h, &event, "data"), str_value("payload"));
    assert_eq!(h.callbacks.iter().flatten().count(), live);
    while h.dom.events_mut().next_listener(&mut dispatch).is_some() {}
    call(&mut h, O::MessageEventInit, event.clone(), &[str_value("new")]);
    assert_eq!(get(&mut h, &event, "data"), V::Null);
    assert_eq!(h.callbacks.iter().flatten().count(), live);
    assert!(!h.callbacks.iter().flatten().any(|value| value == &str_value("payload")));
}

#[test]
fn external_targets_share_registry_and_preserve_identity_for_trusted_delivery() {
    let mut h = Host::default();
    let (slot, p) = port(&mut h, "port");
    for (id, options) in [(1, V::Dictionary(vec![("once", V::Bool(true))])),
        (2, V::Bool(true)), (3, V::Bool(false))] {
        call(&mut h, O::AddEventListener, p.clone(), &[str_value("message"), V::Callable(id), options]);
    }
    call(&mut h, O::RemoveEventListener, p.clone(), &[str_value("message"), V::Callable(3)]);
    for _ in 0..2 {
        crate::dispatch_message_event(&mut h, EventTarget::External(slot), "message",
            crate::MessageEventPayload { data: str_value("payload"), origin: vec![],
                last_event_id: vec![], source: Some(p.clone()), ports: vec![p.clone()] }).unwrap();
    }
    assert_eq!(h.calls, vec![2, 1, 2]);
    assert_eq!(h.callback_targets, vec![Some(EventTarget::External(slot)); 3]);
    let callbacks = h.call_receivers.clone();
    for (receiver, args) in callbacks {
        assert_eq!(receiver, p);
        let event = args[0].clone();
        assert_eq!(call(&mut h, O::EventTargetValue, event.clone(), &[]), p);
        assert_eq!(call(&mut h, O::EventIsTrusted, event.clone(), &[]), V::Bool(true));
        assert_eq!(get(&mut h, &event, "data"), str_value("payload"));
    }
    assert!(!h.callbacks.iter().flatten().any(|value| value == &V::Callable(1)));
}

#[test]
fn external_dispatch_obeys_passive_and_capture_removal_and_rejects_dead_slots() {
    let mut h = Host::default();
    let (_, p) = port(&mut h, "port");
    h.cancel_events = true;
    let options = V::Dictionary(vec![("capture", V::Bool(true)), ("passive", V::Bool(true))]);
    call(&mut h, O::AddEventListener, p.clone(), &[str_value("message"), V::Callable(1), options]);
    let event = message(&mut h, V::Dictionary(vec![("cancelable", V::Bool(true))]));
    assert_eq!(call(&mut h, O::DispatchEvent, p.clone(), &[event.clone()]), V::Bool(true));
    assert_eq!(call(&mut h, O::EventIsTrusted, event.clone(), &[]), V::Bool(false));
    call(&mut h, O::RemoveEventListener, p.clone(), &[str_value("message"), V::Callable(1), V::Bool(false)]);
    call(&mut h, O::DispatchEvent, p.clone(), &[event.clone()]);
    assert_eq!(h.calls.len(), 2, "capture mismatch retains the listener");
    call(&mut h, O::RemoveEventListener, p.clone(), &[str_value("message"), V::Callable(1), V::Bool(true)]);
    call(&mut h, O::AddEventListener, p.clone(), &[str_value("message"), V::Callable(2)]);
    assert_eq!(call(&mut h, O::DispatchEvent, p.clone(), &[event.clone()]), V::Bool(false));
    h.external_targets[0].0 = u64::MAX;
    assert!(invoke(&mut h, O::DispatchEvent, p, &[event]).is_err());
}
