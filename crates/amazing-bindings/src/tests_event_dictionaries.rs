use super::*;

#[test]
fn ui_event_view_requires_window_for_every_derived_constructor() {
    let mut h = Host::default();
    let window = V::Dictionary(vec![("window", V::Bool(true))]);
    h.window = Some((h.dom.create_event_target(), window.clone()));
    for op in [O::UIEventConstructor, O::FocusEventConstructor, O::MouseEventConstructor,
        O::WheelEventConstructor, O::KeyboardEventConstructor, O::CompositionEventConstructor] {
        for invalid in [V::Number(7.), V::Bool(true), V::Node(NodeIdx(0)), V::Dictionary(vec![])] {
            assert!(invoke(&mut h, op, V::Undefined,
                &[str_value("event"), V::Dictionary(vec![("view", invalid)])]).is_err());
        }
        for valid in [V::Null, V::Undefined, window.clone()] {
            let event = call(&mut h, op, V::Undefined, &[str_value("event"),
                V::Dictionary(vec![("view", valid.clone())])]);
            let expected = if valid == V::Undefined { V::Null } else { valid };
            assert_eq!(call(&mut h, O::EventExtra("view"), event, &[]), expected);
        }
    }
}

#[test]
fn event_dictionary_conversion_is_ordered_and_ui_detail_is_a_signed_long() {
    let mut h = Host::default();
    for (input, output) in [(3.9, 3.), (-3.9, -3.), (4294967297., 1.),
        (2147483648., -2147483648.), (f64::INFINITY, 0.), (f64::NAN, 0.)] {
        h.property_reads.clear();
        let event = call(&mut h, O::UIEventConstructor, V::Undefined, &[str_value("event"),
            V::Dictionary(vec![("detail", V::Number(input))])]);
        assert_eq!(call(&mut h, O::EventExtra("detail"), event, &[]), V::Number(output));
        assert_eq!(h.property_reads, ["bubbles", "cancelable", "composed", "detail", "view"]);
    }
    for op in [O::EventConstructor, O::CustomEventConstructor, O::UIEventConstructor] {
        assert!(invoke(&mut h, op, V::Undefined, &[str_value("event"), V::Bool(true)]).is_err());
    }
    h.property_reads.clear();
    assert!(invoke(&mut h, O::UIEventConstructor, V::Undefined, &[str_value("event"),
        V::Dictionary(vec![("detail", V::Throw)])]).is_err());
    assert_eq!(h.property_reads, ["bubbles", "cancelable", "composed", "detail"]);
}

#[test]
fn invalid_related_target_does_not_retain_earlier_window_root() {
    let mut h = Host::default();
    let window = V::Dictionary(vec![("window", V::Bool(true))]);
    h.window = Some((h.dom.create_event_target(), window.clone()));
    assert!(invoke(&mut h, O::FocusEventConstructor, V::Undefined, &[str_value("focus"),
        V::Dictionary(vec![("view", window), ("relatedTarget", V::Dictionary(vec![]))])]).is_err());
    assert!(h.callbacks.is_empty());
    let event = call(&mut h, O::FocusEventConstructor, V::Undefined, &[str_value("focus"),
        V::Dictionary(vec![("relatedTarget", V::Node(NodeIdx(0)))])]);
    assert_eq!(call(&mut h, O::EventExtra("relatedTarget"), event, &[]), V::Node(NodeIdx(0)));
}
