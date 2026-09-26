use super::*;
use amazing_dom::events::{EventClass, EventId};

#[test]
fn legacy_factory_preserves_supported_event_classes_for_ascii_aliases() {
    let mut h = Host::default();
    for (name, class) in [("CompositionEvent", EventClass::Composition),
        ("FocusEvent", EventClass::Focus), ("KeyboardEvent", EventClass::Keyboard),
        ("MouseEvent", EventClass::Mouse), ("MouseEvents", EventClass::Mouse),
        ("UIEvent", EventClass::Ui), ("UIEvents", EventClass::Ui),
        ("Event", EventClass::Event), ("Events", EventClass::Event),
        ("HTMLEvents", EventClass::Event), ("SVGEvents", EventClass::Event),
        ("DeviceMotionEvent", EventClass::DeviceMotion),
        ("DeviceOrientationEvent", EventClass::DeviceOrientation),
        ("DragEvent", EventClass::Drag), ("StorageEvent", EventClass::Storage),
        ("TextEvent", EventClass::Text)] {
        for alias in [name.to_string(), name.to_ascii_lowercase(), name.to_ascii_uppercase()] {
            let event = call(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[str_value(&alias)]);
            let V::Platform(key, _) = event.clone() else { panic!("not an event") };
            assert_eq!(h.dom.events().event(EventId::from_slot(key.slot as usize)).class(), class);
            assert_eq!(call(&mut h, O::EventType, event.clone(), &[]), str_value(""));
            assert_eq!(call(&mut h, O::EventIsTrusted, event.clone(), &[]), V::Bool(false));
            let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
            assert!(invoke(&mut h, O::DispatchEvent, target.clone(), &[event.clone()]).is_err());
            call(&mut h, O::EventInitEvent, event.clone(), &[str_value("ready")]);
            assert_eq!(call(&mut h, O::DispatchEvent, target, &[event]), V::Bool(true));
        }
    }
}

#[test]
fn unrecognized_legacy_interfaces_throw_not_supported_for_all_domstrings() {
    let mut h = Host::default();
    for name in [str_value("foo"), str_value("WheelEvent"), str_value("FocusEvents"),
        str_value("UIEvent "), str_value("UİEvent"), str_value("UıEvent"), V::String(vec![0xd800])] {
        assert_eq!(invoke(&mut h, O::CreateEvent, V::Node(NodeIdx(0)), &[name]),
            Err("NotSupportedError".into()));
    }
}
