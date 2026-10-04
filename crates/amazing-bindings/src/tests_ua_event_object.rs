use super::*;
use amazing_dom::events::{DispatchOrigin, EventId, EventTarget};

#[test]
fn existing_event_preserves_identity_payload_and_script_vs_ua_trust() {
    let mut h = Host::default();
    let node = element(&mut h);
    let payload = V::Dictionary(vec![("stable", V::Number(42.))]);
    let event = call(&mut h, O::CustomEventConstructor, V::Undefined,
        &[str_value("sample"), V::Dictionary(vec![("detail", payload.clone()), ("cancelable", V::Bool(true))])]);
    let expected = event.clone();
    let listener = h.script(move |host, receiver, args| {
        assert_eq!(receiver, &V::Node(node));
        assert_eq!(args[0], expected);
        assert_eq!(call(host, O::CustomEventDetail, args[0].clone(), &[]), payload);
        Ok(V::Undefined)
    });
    call(&mut h, O::AddEventListener, V::Node(node), &[str_value("sample"), listener]);
    assert_eq!(call(&mut h, O::DispatchEvent, V::Node(node), &[event.clone()]), V::Bool(true));
    assert_eq!(call(&mut h, O::EventIsTrusted, event.clone(), &[]), V::Bool(false));
    assert!(dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &event).unwrap());
    assert_eq!(call(&mut h, O::EventIsTrusted, event.clone(), &[]), V::Bool(true));
    h.cancel_events = true;
    assert!(!dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &event).unwrap());
    assert_eq!(call(&mut h, O::EventDefaultPrevented, event, &[]), V::Bool(true));
}

#[test]
fn existing_event_rejects_wrong_arena_brand_state_and_nested_dispatch() {
    let mut h = Host::default(); let node = element(&mut h);
    let event = call(&mut h, O::EventConstructor, V::Undefined, &[str_value("sample")]);
    let V::Platform(key, brand) = event.clone() else { panic!("native Event expected") };
    for invalid in [V::Undefined, V::Node(node), V::Platform(PlatformKey { arena:key.arena+1, ..key }, brand),
        V::Platform(PlatformKey { generation:key.generation+1, ..key }, brand),
        V::Platform(PlatformKey { slot:u32::MAX as u64, ..key }, brand)] {
        assert!(dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &invalid).is_err());
    }
    let uninitialized = call(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[str_value("Event")]);
    assert!(dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &uninitialized).is_err());
    let id = EventId::from_slot(key.slot as usize);
    let mut dispatch = h.dom.begin_event_dispatch(EventTarget::Node(node), id, DispatchOrigin::Script).unwrap();
    assert!(dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &event).is_err());
    assert!(h.dom.events_mut().next_listener(&mut dispatch).is_none());
    assert!(dispatch_user_agent_event_object(&mut h, EventTarget::Node(node), &event).unwrap());
}

#[test]
fn html_load_fires_only_associated_window_with_document_target_and_exact_identity() {
    let mut h = Host::default(); let child = h.dom.create_document();
    let root_slot = h.dom.create_window_event_target(); let child_slot = h.dom.create_window_event_target();
    h.dom.associate_document_window_event_target(NodeIdx(0), h.dom.node_generation(NodeIdx(0)).unwrap(), root_slot).unwrap();
    h.dom.associate_document_window_event_target(child, h.dom.node_generation(child).unwrap(), child_slot).unwrap();
    let root = V::Dictionary(vec![("window", str_value("root"))]);
    let child_window = V::Dictionary(vec![("window", str_value("child"))]);
    h.window = Some((root_slot, root.clone())); h.external_targets.push((child_slot, child_window.clone()));
    let expected = child_window.clone();
    let callback = h.script(move |host, receiver, args| {
        assert_eq!(receiver, &expected);
        assert_eq!(call(host,O::EventTargetValue,args[0].clone(),&[]),V::Node(child));
        assert_eq!(call(host,O::EventCurrentTarget,args[0].clone(),&[]),expected);
        assert_eq!(call(host,O::EventComposedPath,args[0].clone(),&[]),V::Array(vec![expected.clone()]));
        assert_eq!(call(host,O::EventIsTrusted,args[0].clone(),&[]),V::Bool(true));
        assert_eq!(call(host,O::EventBubbles,args[0].clone(),&[]),V::Bool(false));
        assert_eq!(call(host,O::EventCancelable,args[0].clone(),&[]),V::Bool(false));
        host.calls.push(1);
        Ok(V::Undefined)
    });
    call(&mut h,O::AddEventListener,child_window,&[str_value("load"),callback]);
    call(&mut h,O::AddEventListener,root,&[str_value("load"),V::Callable(2)]);
    call(&mut h,O::AddEventListener,V::Node(child),&[str_value("load"),V::Callable(3)]);
    dispatch_document_load_event(&mut h,child).unwrap();
    assert_eq!(h.calls,vec![1]);
    let inert = h.dom.create_document(); dispatch_document_load_event(&mut h,inert).unwrap();
    assert_eq!(h.calls,vec![1]);
    assert!(dispatch_document_load_event(&mut h,NodeIdx(u32::MAX)).is_err());
    let ordinary = element(&mut h);
    assert!(dispatch_document_load_event(&mut h,ordinary).is_err());
}
