use super::*;
#[test]
fn every_native_event_constructor_preserves_domstring_type_code_units() {
    let mut h = Host::default();
    for constructor in [O::EventConstructor, O::CustomEventConstructor, O::UIEventConstructor,
        O::FocusEventConstructor, O::MouseEventConstructor, O::WheelEventConstructor,
        O::KeyboardEventConstructor, O::CompositionEventConstructor, O::MessageEventConstructor,
        O::HashChangeEventConstructor, O::PopStateEventConstructor, O::PageTransitionEventConstructor] {
        for units in [vec![0xd800], vec![0xdc00], vec![0xfffd], vec![0xd83d, 0xde00], vec![0xd800, 120, 0xdc00]] {
            let value = V::String(units);
            let event = call(&mut h, constructor, V::Undefined, &[value.clone()]);
            assert_eq!(call(&mut h, O::EventType, event, &[]), value);
        }
    }
}
