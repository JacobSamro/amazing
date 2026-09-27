use super::*;

// DOM-B-REFLECT: ARIAMixin string reflection, `HTMLInputElement.switch`, the
// minimal `Animation` model and `navigator.clipboard`. Exercised at the
// binding-op level (like the rest of this file) rather than through a full
// JS interpreter, since the mock `Host`'s `promises` vec lets a test read
// back a Promise's resolved/rejected value directly -- `Element.animate()`
// and `Clipboard.readText/writeText` are otherwise only observable through
// `.then()`, which the JS-level test harness in `crates/amazing` cannot
// drive without a real microtask-pumping scheduler.

#[test]
fn aria_attributes_are_nullable_string_reflections_on_any_element() {
    let mut h = Host::default();
    let div = element(&mut h);
    // Absent attribute reflects as null, not "".
    assert_eq!(call(&mut h, O::AriaGet("aria-label"), V::Node(div), &[]), V::Null);
    assert_eq!(call(&mut h, O::AriaGet("role"), V::Node(div), &[]), V::Null);
    // Setting a string value both sets the content attribute and reflects back.
    call(&mut h, O::AriaSet("aria-label"), V::Node(div), &[str_value("Close")]);
    assert_eq!(h.dom.get_attribute_ns(div, None, "aria-label"), Some("Close"));
    assert_eq!(call(&mut h, O::AriaGet("aria-label"), V::Node(div), &[]), str_value("Close"));
    call(&mut h, O::AriaSet("role"), V::Node(div), &[str_value("button")]);
    assert_eq!(call(&mut h, O::AriaGet("role"), V::Node(div), &[]), str_value("button"));
    // Setting null/undefined removes the content attribute (getter goes back to null).
    call(&mut h, O::AriaSet("aria-label"), V::Node(div), &[V::Null]);
    assert_eq!(h.dom.get_attribute_ns(div, None, "aria-label"), None);
    assert_eq!(call(&mut h, O::AriaGet("aria-label"), V::Node(div), &[]), V::Null);
    call(&mut h, O::AriaSet("role"), V::Node(div), &[str_value("dialog")]);
    call(&mut h, O::AriaSet("role"), V::Node(div), &[V::Undefined]);
    assert_eq!(h.dom.get_attribute_ns(div, None, "role"), None);
    // Applies to every Element regardless of namespace -- not gated to HTML
    // like `element_reflection.rs`'s tag/interface-keyed reflections.
    let svg = h.dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "g").unwrap();
    call(&mut h, O::AriaSet("aria-hidden"), V::Node(svg), &[str_value("true")]);
    assert_eq!(call(&mut h, O::AriaGet("aria-hidden"), V::Node(svg), &[]), str_value("true"));
    // Only Elements (and only via the getter/setter arg-count contract).
    let text = h.dom.create_text(NodeIdx(0), "x");
    assert!(invoke(&mut h, O::AriaGet("aria-label"), V::Node(text), &[]).is_err());
    assert!(invoke(&mut h, O::AriaSet("aria-label"), V::Node(div), &[]).is_err());
}

#[test]
fn input_switch_reflects_as_a_boolean_content_attribute() {
    let mut h = Host::default();
    let input = h.dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "input").unwrap();
    let get = O::LegacyReflectGet(Interface::HTMLInputElement, "switch", amazing_bindings_kind());
    let set = O::LegacyReflectSet(Interface::HTMLInputElement, "switch", amazing_bindings_kind());
    assert_eq!(call(&mut h, get, V::Node(input), &[]), V::Bool(false));
    call(&mut h, set, V::Node(input), &[V::Bool(true)]);
    assert_eq!(h.dom.get_attribute_ns(input, None, "switch"), Some(""));
    assert_eq!(call(&mut h, get, V::Node(input), &[]), V::Bool(true));
    call(&mut h, set, V::Node(input), &[V::Bool(false)]);
    assert_eq!(h.dom.get_attribute_ns(input, None, "switch"), None);
    // Gated on the HTMLInputElement interface: an element whose tag does not
    // map to it (e.g. a plain div) rejects both accessors.
    let div = element(&mut h);
    assert!(invoke(&mut h, get, V::Node(div), &[]).is_err());
    assert!(invoke(&mut h, set, V::Node(div), &[V::Bool(true)]).is_err());
}
// Local alias: `LegacyReflectionKind` is not otherwise imported by this
// test module, and `crate::host::LegacyReflectionKind::Boolean` reads oddly
// three times over; this keeps the two call sites above readable.
fn amazing_bindings_kind() -> crate::host::LegacyReflectionKind { crate::host::LegacyReflectionKind::Boolean }

#[test]
fn element_animate_returns_a_finished_animation_whose_promises_resolve_to_it() {
    let mut h = Host::default();
    let div = element(&mut h);
    let animation = call(&mut h, O::ElementAnimate, V::Node(div), &[]);
    assert!(matches!(animation, V::Platform(_, InterfaceBrand::ANIMATION)));
    let finished = call(&mut h, O::AnimationFinished, animation.clone(), &[]);
    let ready = call(&mut h, O::AnimationReady, animation.clone(), &[]);
    assert!(matches!(finished, V::Promise(_)) && matches!(ready, V::Promise(_)));
    assert_ne!(finished, ready);
    let V::Promise(finished_id) = finished else { unreachable!() };
    let V::Promise(ready_id) = ready else { unreachable!() };
    assert_eq!(h.promises[finished_id as usize].1, Some(animation.clone()));
    assert_eq!(h.promises[ready_id as usize].1, Some(animation.clone()));
    // `animate()` on a non-Element (or a stale/foreign receiver on the other
    // Animation methods) is a TypeError, not a panic.
    let text = h.dom.create_text(NodeIdx(0), "x");
    assert!(invoke(&mut h, O::ElementAnimate, V::Node(text), &[]).is_err());
    assert!(invoke(&mut h, O::AnimationFinished, V::Node(div), &[]).is_err());
}

#[test]
fn animation_onfinish_runs_synchronously_and_cancel_replaces_finished_with_a_rejection() {
    let mut h = Host::default();
    let div = element(&mut h);
    let animation = call(&mut h, O::ElementAnimate, V::Node(div), &[]);
    call(&mut h, O::AnimationSetOnFinish, animation.clone(), &[V::Callable(1)]);
    assert_eq!(call(&mut h, O::AnimationOnFinish, animation.clone(), &[]), V::Callable(1));
    call(&mut h, O::AnimationFinish, animation.clone(), &[]);
    assert_eq!(h.calls, vec![1]);
    assert_eq!(h.call_receivers.last().unwrap().0, animation);
    // play()/pause() are plain play-state setters; neither touches onfinish.
    call(&mut h, O::AnimationPlay, animation.clone(), &[]);
    call(&mut h, O::AnimationPause, animation.clone(), &[]);
    assert_eq!(h.calls, vec![1]);
    // Clearing onfinish with a non-callable value is accepted (stored as
    // "no handler"), matching `EventHandlerNonNull`-style non-throwing sets.
    call(&mut h, O::AnimationSetOnFinish, animation.clone(), &[V::Null]);
    assert_eq!(call(&mut h, O::AnimationOnFinish, animation.clone(), &[]), V::Null);
    // cancel(): the getter observes a freshly rejected promise afterwards,
    // and a second cancel() on an already-idle animation is a no-op (no
    // further promise replacement, no error).
    let before = call(&mut h, O::AnimationFinished, animation.clone(), &[]);
    call(&mut h, O::AnimationCancel, animation.clone(), &[]);
    let after = call(&mut h, O::AnimationFinished, animation.clone(), &[]);
    assert_ne!(before, after);
    let V::Promise(after_id) = after.clone() else { unreachable!() };
    assert!(h.promises[after_id as usize].2.is_some());
    call(&mut h, O::AnimationCancel, animation.clone(), &[]);
    assert_eq!(call(&mut h, O::AnimationFinished, animation, &[]), after);
}

#[test]
fn navigator_clipboard_is_a_stable_singleton_round_tripping_in_memory_text() {
    let mut h = Host::default();
    let clipboard = call(&mut h, O::NavigatorClipboard, V::Undefined, &[]);
    assert!(matches!(clipboard, V::Platform(_, InterfaceBrand::CLIPBOARD)));
    // Every access wraps the same per-document singleton key.
    assert_eq!(call(&mut h, O::NavigatorClipboard, V::Undefined, &[]), clipboard);
    let write = call(&mut h, O::ClipboardWriteText, clipboard.clone(), &[str_value("hello")]);
    assert!(matches!(write, V::Promise(_)));
    assert_eq!(h.dom.clipboard_text(), "hello".encode_utf16().collect::<Vec<_>>().as_slice());
    let read = call(&mut h, O::ClipboardReadText, clipboard.clone(), &[]);
    let V::Promise(read_id) = read else { unreachable!() };
    assert_eq!(h.promises[read_id as usize].1, Some(str_value("hello")));
    // A receiver that isn't the Clipboard singleton is rejected.
    assert!(invoke(&mut h, O::ClipboardReadText, V::Undefined, &[]).is_err());
}
