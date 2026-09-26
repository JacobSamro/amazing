use super::*;

#[test]
fn modifier_keys_are_independent_case_sensitive_and_shared_by_mouse_and_keyboard() {
    let modifiers = [("altKey", "Alt"), ("ctrlKey", "Control"), ("metaKey", "Meta"), ("shiftKey", "Shift"),
        ("modifierAltGraph", "AltGraph"), ("modifierCapsLock", "CapsLock"), ("modifierFn", "Fn"),
        ("modifierFnLock", "FnLock"), ("modifierHyper", "Hyper"), ("modifierNumLock", "NumLock"),
        ("modifierScrollLock", "ScrollLock"), ("modifierSuper", "Super"), ("modifierSymbol", "Symbol"),
        ("modifierSymbolLock", "SymbolLock")];
    let mut h = Host::default();
    for (constructor, getter) in [(O::MouseEventConstructor, O::MouseGetModifierState),
        (O::WheelEventConstructor, O::MouseGetModifierState),
        (O::KeyboardEventConstructor, O::KeyboardGetModifierState)] {
        for (field, active_key) in modifiers {
            let event = call(&mut h, constructor, V::Undefined, &[str_value("x"),
                V::Dictionary(vec![(field, V::Number(1.))])]);
            for (_, key) in modifiers {
                assert_eq!(call(&mut h, getter, event.clone(), &[str_value(key)]), V::Bool(key == active_key));
            }
            for key in ["control", "altgraph", "", "Unidentified", "OS"] {
                assert_eq!(call(&mut h, getter, event.clone(), &[str_value(key)]), V::Bool(false));
            }
        }
    }
}

#[test]
fn modifier_state_checks_receiver_and_arity_before_key_conversion() {
    let mut h = Host::default();
    let mouse = call(&mut h, O::MouseEventConstructor, V::Undefined, &[str_value("x")]);
    let keyboard = call(&mut h, O::KeyboardEventConstructor, V::Undefined, &[str_value("x")]);
    h.conversions.clear();
    assert!(invoke(&mut h, O::KeyboardGetModifierState, mouse.clone(), &[V::Coerce("Control")]).is_err());
    assert!(invoke(&mut h, O::MouseGetModifierState, keyboard, &[V::Coerce("Control")]).is_err());
    assert!(h.conversions.is_empty());
    assert!(invoke(&mut h, O::MouseGetModifierState, mouse.clone(), &[]).is_err());
    assert_eq!(invoke(&mut h, O::MouseGetModifierState, mouse, &[V::Throw]), Err("coercion sentinel".into()));
}

#[test]
fn event_numeric_dictionaries_apply_integer_widths_and_preserve_finite_fractions() {
    let mut h = Host::default();
    for (constructor, field, input, expected) in [
        (O::MouseEventConstructor, "button", 65535.9, -1.),
        (O::MouseEventConstructor, "button", 32768., -32768.),
        (O::MouseEventConstructor, "buttons", -1., 65535.),
        (O::MouseEventConstructor, "buttons", f64::INFINITY, 0.),
        (O::KeyboardEventConstructor, "location", -1., 4294967295.),
        (O::KeyboardEventConstructor, "keyCode", 4294967297., 1.),
        (O::KeyboardEventConstructor, "charCode", f64::NAN, 0.),
        (O::UIEventConstructor, "which", -1., 4294967295.),
        (O::WheelEventConstructor, "deltaMode", 4294967298., 2.),
        (O::MouseEventConstructor, "clientX", 3.5, 3.5),
        (O::WheelEventConstructor, "deltaX", -0.5, -0.5),
    ] {
        let event = call(&mut h, constructor, V::Undefined, &[str_value("x"),
            V::Dictionary(vec![(field, V::Number(input))])]);
        assert_eq!(call(&mut h, O::EventExtra(field), event, &[]), V::Number(expected));
    }
    let event = call(&mut h, O::WheelEventConstructor, V::Undefined, &[str_value("x"),
        V::Dictionary(vec![("momentum", V::Bool(true))])]);
    assert_eq!(call(&mut h, O::EventExtra("momentum"), event, &[]), V::Bool(true));
}

#[test]
fn nonfinite_event_doubles_reject_without_leaking_earlier_window_roots() {
    let mut h = Host::default();
    let window = V::Dictionary(vec![]);
    h.window = Some((7, window.clone()));
    for field in ["clientX", "clientY", "screenX", "screenY", "deltaX", "deltaY", "deltaZ"] {
        for input in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(invoke(&mut h, O::WheelEventConstructor, V::Undefined, &[str_value("x"),
                V::Dictionary(vec![("view", window.clone()), (field, V::Number(input))])]).is_err());
            assert!(h.callbacks.is_empty());
        }
    }
}
