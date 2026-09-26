use super::*;

#[test]
fn listener_conversion_precedes_null_duplicate_and_aborted_early_returns() {
    let mut h = Host::default();
    let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
    let options = V::Dictionary(vec![]);
    for callback in [V::Null, V::Callable(1), V::Callable(1)] {
        h.property_reads.clear();
        call(&mut h, O::AddEventListener, target.clone(), &[str_value("x"), callback, options.clone()]);
        assert_eq!(h.property_reads, ["capture", "once", "passive", "signal"]);
    }
    h.property_reads.clear();
    call(&mut h, O::RemoveEventListener, target.clone(), &[str_value("x"), V::Null, options.clone()]);
    assert_eq!(h.property_reads, ["capture"]);
    h.property_reads.clear();
    assert!(invoke(&mut h, O::AddEventListener, target.clone(),
        &[str_value("x"), V::Number(1.), options]).unwrap_err().starts_with("TypeError"));
    assert!(h.property_reads.is_empty());
    assert!(invoke(&mut h, O::AddEventListener, target.clone(),
        &[str_value("x"), V::Null, V::Dictionary(vec![("signal", V::Null)])]).is_err());
    h.conversions.clear();
    assert!(invoke(&mut h, O::AddEventListener, target, &[V::Coerce("arity")]).is_err());
    assert!(h.conversions.is_empty());
}

#[test]
fn signal_removal_releases_only_still_registered_callbacks_and_preserves_duplicate_signal() {
    let mut h = Host::default();
    let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
    let event = call(&mut h, O::EventConstructor, V::Undefined, &[str_value("x")]);
    let first = call(&mut h, O::AbortControllerConstructor, V::Undefined, &[]);
    let first_signal = call(&mut h, O::AbortControllerSignal, first.clone(), &[]);
    let second = call(&mut h, O::AbortControllerConstructor, V::Undefined, &[]);
    let second_signal = call(&mut h, O::AbortControllerSignal, second.clone(), &[]);
    for signal in [first_signal.clone(), second_signal] {
        call(&mut h, O::AddEventListener, target.clone(), &[str_value("x"), V::Callable(7),
            V::Dictionary(vec![("signal", signal)])]);
    }
    call(&mut h, O::AbortControllerAbort, second, &[str_value("second")]);
    call(&mut h, O::DispatchEvent, target.clone(), &[event.clone()]);
    assert_eq!(h.calls, [7]);
    call(&mut h, O::AbortControllerAbort, first, &[str_value("first")]);
    call(&mut h, O::DispatchEvent, target.clone(), &[event]);
    assert_eq!(h.calls, [7]);
    assert!(!h.callbacks.iter().any(|entry| *entry == Some(V::Callable(7))));
    call(&mut h, O::AddEventListener, target, &[str_value("x"), V::Callable(8),
        V::Dictionary(vec![("signal", first_signal)])]);
    assert!(!h.callbacks.iter().any(|entry| *entry == Some(V::Callable(8))));
}

#[test]
fn primitive_options_use_boolean_capture_and_remove_ignores_signal() {
    let mut h = Host::default();
    let target = call(&mut h, O::EventTargetConstructor, V::Undefined, &[]);
    let event = call(&mut h, O::EventConstructor, V::Undefined, &[str_value("x")]);
    call(&mut h, O::AddEventListener, target.clone(), &[str_value("x"), V::Callable(4), V::Number(1.)]);
    call(&mut h, O::RemoveEventListener, target.clone(), &[str_value("x"), V::Callable(4),
        V::Dictionary(vec![("capture", V::Bool(true)), ("signal", V::Null)])]);
    call(&mut h, O::DispatchEvent, target, &[event]);
    assert!(h.calls.is_empty());
}
