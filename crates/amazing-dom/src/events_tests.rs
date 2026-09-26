use super::*;
fn fixture() -> (Dom, EventTargets, [EventTarget; 3]) {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let t = d.create_element(p, "t");
    (
        d,
        EventTargets::default(),
        [
            EventTarget::Node(NodeIdx(0)),
            EventTarget::Node(p),
            EventTarget::Node(t),
        ],
    )
}
fn event(r: &mut EventTargets, bubbles: bool, cancelable: bool) -> EventId {
    r.create_event(Event::new(
        "test",
        EventInit {
            bubbles,
            cancelable,
            composed: false,
        },
        Duration::from_millis(123),
    ))
}
fn add(r: &mut EventTargets, d: &Dom, t: EventTarget, n: u64, capture: bool) {
    r.add_listener(
        d,
        t,
        "test",
        Some(CallbackId(n)),
        ListenerOptions {
            capture,
            ..Default::default()
        },
    );
}
fn drain(r: &mut EventTargets, dispatch: &mut Dispatch) -> Vec<EventInvocation> {
    let mut out = vec![];
    while let Some(i) = r.next_listener(dispatch) {
        out.push(i);
    }
    out
}
#[test]
fn wpt_stop_immediate_sets_cancel_bubble_and_suppresses_second_listener() {
    // dom/events/Event-stopImmediatePropagation.html at WPT c2a2fce.
    let (d, mut registry, targets) = fixture();
    add(&mut registry, &d, targets[2], 1, false);
    add(&mut registry, &d, targets[2], 2, false);
    let id = event(&mut registry, false, false);
    let mut dispatch = registry.begin_dispatch(&d, targets[2], id, DispatchOrigin::Script).unwrap();
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    registry.event_mut(id).stop_immediate_propagation();
    assert!(registry.event(id).cancel_bubble());
    assert!(registry.next_listener(&mut dispatch).is_none());
}

#[test]
fn wpt_once_listener_is_removed_before_nested_dispatch() {
    // dom/events/AddEventListenerOptions-once.any.js, nested dispatch site.
    let (d, mut registry, targets) = fixture();
    registry.add_listener(&d, targets[2], "test", Some(CallbackId(1)),
        ListenerOptions { once: true, ..Default::default() });
    let first = event(&mut registry, false, false);
    let mut outer = registry.begin_dispatch(&d, targets[2], first, DispatchOrigin::Script).unwrap();
    assert_eq!(registry.next_listener(&mut outer).unwrap().callback, CallbackId(1));
    let nested = event(&mut registry, false, false);
    let mut inner = registry.begin_dispatch(&d, targets[2], nested, DispatchOrigin::Script).unwrap();
    assert!(registry.next_listener(&mut inner).is_none());
    assert!(registry.next_listener(&mut outer).is_none());
}
#[test]
fn capture_target_and_bubble_order_and_cleanup() {
    let (d, mut r, t) = fixture();
    for (i, n) in t.iter().enumerate() {
        add(&mut r, &d, *n, i as u64 * 2, true);
        add(&mut r, &d, *n, i as u64 * 2 + 1, false);
    }
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    let calls = drain(&mut r, &mut s);
    assert_eq!(
        calls.iter().map(|c| c.callback.0).collect::<Vec<_>>(),
        vec![0, 2, 4, 5, 3, 1]
    );
    assert_eq!(
        calls.iter().map(|c| c.phase).collect::<Vec<_>>(),
        vec![
            EventPhase::Capturing,
            EventPhase::Capturing,
            EventPhase::AtTarget,
            EventPhase::AtTarget,
            EventPhase::Bubbling,
            EventPhase::Bubbling
        ]
    );
    assert_eq!(r.event(e).phase(), EventPhase::None);
    assert_eq!(r.event(e).current_target(), None);
    assert!(r.event(e).composed_path().is_empty());
    assert_eq!(r.event(e).target(), Some(t[2]));
    assert_eq!(r.dispatch_result(e), Ok(true));
}
#[test]
fn non_bubbling_events_still_capture_and_invoke_both_target_groups() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[0], 1, true);
    add(&mut r, &d, t[2], 2, true);
    add(&mut r, &d, t[2], 3, false);
    add(&mut r, &d, t[0], 4, false);
    let e = event(&mut r, false, false);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    assert_eq!(
        drain(&mut r, &mut s)
            .iter()
            .map(|c| c.callback.0)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}
#[test]
fn stop_propagation_keeps_current_listener_snapshot_only() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[1], 1, true);
    add(&mut r, &d, t[1], 2, true);
    add(&mut r, &d, t[2], 3, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    assert_eq!(r.next_listener(&mut s).unwrap().callback, CallbackId(1));
    r.event_mut(e).stop_propagation();
    assert_eq!(
        drain(&mut r, &mut s)
            .iter()
            .map(|c| c.callback.0)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert!(!r.event(e).cancel_bubble());
}
#[test]
fn immediate_stop_skips_remaining_listeners_and_ancestors() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, false);
    add(&mut r, &d, t[2], 2, false);
    add(&mut r, &d, t[0], 3, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).stop_immediate_propagation();
    assert!(r.next_listener(&mut s).is_none());
}
#[test]
fn stop_at_target_capture_prevents_target_bubble_invocation() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, true);
    add(&mut r, &d, t[2], 2, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).stop_propagation();
    assert!(r.next_listener(&mut s).is_none());
}
#[test]
fn cancellation_requires_cancelable_and_non_passive() {
    let (d, mut r, t) = fixture();
    r.add_listener(
        &d,
        t[2],
        "test",
        Some(CallbackId(1)),
        ListenerOptions {
            passive: Some(true),
            ..Default::default()
        },
    );
    add(&mut r, &d, t[2], 2, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).prevent_default();
    assert!(!r.event(e).default_prevented());
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).prevent_default();
    drain(&mut r, &mut s);
    assert_eq!(r.dispatch_result(e), Ok(false));
    let e = event(&mut r, true, false);
    r.event_mut(e).prevent_default();
    assert!(!r.event(e).default_prevented());
}
#[test]
fn duplicate_identity_ignores_once_and_passive_but_includes_capture() {
    let (d, mut r, t) = fixture();
    let id = r.add_listener(
        &d,
        t[2],
        "test",
        Some(CallbackId(1)),
        ListenerOptions::default(),
    );
    assert_eq!(
        id,
        r.add_listener(
            &d,
            t[2],
            "test",
            Some(CallbackId(1)),
            ListenerOptions {
                once: true,
                passive: Some(true),
                capture: false
            }
        )
    );
    add(&mut r, &d, t[2], 1, true);
    assert_eq!(
        r.add_listener(&d, t[2], "test", None, ListenerOptions::default()),
        None
    );
    for _ in 0..2 {
        let e = event(&mut r, true, true);
        let mut s = r
            .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
            .unwrap();
        assert_eq!(drain(&mut r, &mut s).len(), 2);
    }
}
#[test]
fn once_removed_before_nested_dispatch() {
    let (d, mut r, t) = fixture();
    r.add_listener(
        &d,
        t[2],
        "test",
        Some(CallbackId(1)),
        ListenerOptions {
            once: true,
            ..Default::default()
        },
    );
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    let nested = event(&mut r, true, true);
    let mut n = r
        .begin_dispatch(&d, t[2], nested, DispatchOrigin::Script)
        .unwrap();
    assert!(r.next_listener(&mut n).is_none());
    assert!(r.next_listener(&mut s).is_none());
}
#[test]
fn listener_removed_after_snapshot_is_not_called_even_if_readded() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, false);
    add(&mut r, &d, t[2], 2, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    r.remove_listener(t[2], "test", Some(CallbackId(2)), false);
    add(&mut r, &d, t[2], 2, false);
    assert!(r.next_listener(&mut s).is_none());
}
#[test]
fn listeners_added_during_capture_can_run_in_later_bubble_snapshot() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, true);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    add(&mut r, &d, t[2], 2, true);
    add(&mut r, &d, t[2], 3, false);
    assert_eq!(
        drain(&mut r, &mut s)
            .iter()
            .map(|c| c.callback.0)
            .collect::<Vec<_>>(),
        vec![3]
    );
}
#[test]
fn topology_changes_do_not_change_current_event_path() {
    let (mut d, mut r, t) = fixture();
    add(&mut r, &d, t[0], 1, true);
    add(&mut r, &d, t[1], 2, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    let EventTarget::Node(node) = t[2] else {
        unreachable!()
    };
    d.detach(node);
    assert_eq!(r.event(e).composed_path(), vec![t[2], t[1], t[0]]);
    assert_eq!(
        drain(&mut r, &mut s)
            .iter()
            .map(|c| c.callback.0)
            .collect::<Vec<_>>(),
        vec![2]
    );
}
#[test]
fn same_event_redispatch_rejected_and_reuse_resets_trust_not_cancellation() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::UserAgent)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    assert!(r.event(e).is_trusted());
    assert!(matches!(
        r.begin_dispatch(&d, t[2], e, DispatchOrigin::Script),
        Err(DomError::InvalidStateError)
    ));
    r.event_mut(e).prevent_default();
    drain(&mut r, &mut s);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    assert!(!r.event(e).is_trusted());
    drain(&mut r, &mut s);
    assert_eq!(r.dispatch_result(e), Ok(false));
    assert_eq!(r.event(e).timestamp(), Duration::from_millis(123));
}
#[test]
fn init_event_is_noop_during_dispatch_then_resets_flags() {
    let (d, mut r, t) = fixture();
    let e = event(&mut r, true, true);
    let s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::UserAgent)
        .unwrap();
    r.event_mut(e).init_event("changed", false, false);
    assert_eq!(r.event(e).event_type(), "test");
    r.abort_dispatch(s);
    r.event_mut(e).prevent_default();
    r.event_mut(e).init_event("changed", false, false);
    assert_eq!(r.event(e).event_type(), "changed");
    assert!(!r.event(e).is_trusted());
    assert!(!r.event(e).default_prevented());
    assert_eq!(r.event(e).target(), None);
}
#[test]
fn callback_exceptions_are_reported_without_ending_dispatch() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, false);
    add(&mut r, &d, t[2], 2, false);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    s.report_exception("binding exception".into());
    assert_eq!(drain(&mut r, &mut s).len(), 1);
    assert_eq!(s.exceptions(), &["binding exception"]);
    assert_eq!(r.dispatch_result(e), Ok(true));
}
#[test]
fn window_target_is_document_event_parent_and_passive_root() {
    let mut dom = Dom::new();
    let slot = dom.create_event_target();
    dom.set_window_event_target(slot).unwrap();
    let event = Event::new("wheel", EventInit { bubbles: true, cancelable: true, composed: false }, Duration::ZERO);
    assert_eq!(EventTree::event_parent(&dom, EventTarget::Node(NodeIdx(0)), &event),
        Some(EventTarget::External(slot)));
    assert!(EventTree::default_passive(&dom, EventTarget::External(slot), "wheel"));
    assert!(!EventTree::default_passive(&dom, EventTarget::External(slot), "touchend"));
}

#[test]
fn blocking_listener_query_covers_ancestors_and_window_with_generation() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let child = dom.create_element(root, "div");
    let window = dom.create_event_target();
    dom.set_window_event_target(window).unwrap();
    let at_child = EventTarget::Node(child);
    let at_root = EventTarget::Node(root);
    let at_window = EventTarget::External(window);
    let types = &["wheel", "mousewheel", "touchstart", "touchmove"];
    assert!(!dom.has_blocking_listener(at_child, types));
    let initial = dom.events().listener_generation();
    dom.add_event_listener(at_root, "wheel", Some(CallbackId(1)), ListenerOptions::default());
    assert!(!dom.has_blocking_listener(at_child, types)); // root default is passive
    let passive_generation = dom.events().listener_generation();
    assert_ne!(initial, passive_generation);
    dom.add_event_listener(at_window, "touchmove", Some(CallbackId(2)),
        ListenerOptions { passive: Some(false), ..Default::default() });
    assert!(dom.has_blocking_listener(at_child, types));
    assert!(!dom.has_blocking_listener(at_child, &["wheel"]));
    let blocking_generation = dom.events().listener_generation();
    dom.events_mut().remove_listener(at_window, "touchmove", Some(CallbackId(2)), false);
    assert!(!dom.has_blocking_listener(at_child, types));
    assert_ne!(blocking_generation, dom.events().listener_generation());
}

#[test]
fn default_passive_depends_on_type_and_document_target() {
    let (d, mut r, t) = fixture();
    r.add_listener(
        &d,
        t[0],
        "wheel",
        Some(CallbackId(1)),
        ListenerOptions::default(),
    );
    r.add_listener(
        &d,
        t[0],
        "wheel",
        Some(CallbackId(2)),
        ListenerOptions {
            passive: Some(false),
            ..Default::default()
        },
    );
    let e = r.create_event(Event::new(
        "wheel",
        EventInit {
            bubbles: true,
            cancelable: true,
            composed: false,
        },
        Duration::ZERO,
    ));
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).prevent_default();
    assert!(!r.event(e).default_prevented());
    r.next_listener(&mut s).unwrap();
    r.event_mut(e).prevent_default();
    drain(&mut r, &mut s);
    assert!(r.event(e).default_prevented());
}
#[test]
fn external_targets_and_retarget_hooks_are_bindable() {
    struct Tree;
    impl EventTree for Tree {
        fn event_parent(&self, t: EventTarget, e: &Event) -> Option<EventTarget> {
            if t == EventTarget::External(1) && e.composed() {
                Some(EventTarget::External(2))
            } else {
                None
            }
        }
        fn retarget(&self, _: EventTarget, against: EventTarget) -> EventTarget {
            against
        }
    }
    let mut r = EventTargets::default();
    for t in [1, 2] {
        r.add_listener(
            &Tree,
            EventTarget::External(t),
            "x",
            Some(CallbackId(t)),
            ListenerOptions::default(),
        );
    }
    let e = r.create_event(Event::new(
        "x",
        EventInit {
            composed: true,
            ..Default::default()
        },
        Duration::ZERO,
    ));
    let mut s = r
        .begin_dispatch(&Tree, EventTarget::External(1), e, DispatchOrigin::Script)
        .unwrap();
    let calls = drain(&mut r, &mut s);
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|c| c.phase == EventPhase::AtTarget && c.target == c.current_target)
    );
}
#[test]
fn removal_capture_flag_and_legacy_flags_follow_spec() {
    let (d, mut r, t) = fixture();
    add(&mut r, &d, t[2], 1, true);
    add(&mut r, &d, t[2], 1, false);
    r.remove_listener(t[2], "test", Some(CallbackId(1)), true);
    let e = event(&mut r, true, true);
    let mut s = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    assert_eq!(r.next_listener(&mut s).unwrap().phase, EventPhase::AtTarget);
    r.event_mut(e).set_cancel_bubble(true);
    r.event_mut(e).set_cancel_bubble(false);
    assert!(r.event(e).cancel_bubble());
    r.event_mut(e).set_return_value(false);
    r.event_mut(e).set_return_value(true);
    assert!(!r.event(e).return_value());
    drain(&mut r, &mut s);
    assert!(!r.event(e).cancel_bubble());
}
#[test]
fn cyclic_parent_hook_fails_without_setting_dispatch_flag() {
    struct Cycle;
    impl EventTree for Cycle {
        fn event_parent(&self, t: EventTarget, _: &Event) -> Option<EventTarget> {
            Some(t)
        }
    }
    let mut r = EventTargets::default();
    let e = event(&mut r, false, false);
    assert!(matches!(
        r.begin_dispatch(&Cycle, EventTarget::External(1), e, DispatchOrigin::Script),
        Err(DomError::InvalidStateError)
    ));
    assert_eq!(r.dispatch_result(e), Ok(true));
}
#[test]
fn aborting_completed_continuation_does_not_clear_a_new_dispatch() {
    let (d, mut r, t) = fixture();
    let e = event(&mut r, false, false);
    let mut old = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    drain(&mut r, &mut old);
    let current = r
        .begin_dispatch(&d, t[2], e, DispatchOrigin::Script)
        .unwrap();
    r.abort_dispatch(old);
    assert_eq!(r.dispatch_result(e), Err(DomError::InvalidStateError));
    r.abort_dispatch(current);
    assert_eq!(r.dispatch_result(e), Ok(true));
}
#[test]
fn non_composed_hook_does_not_cross_boundary() {
    struct Boundary;
    impl EventTree for Boundary {
        fn event_parent(&self, t: EventTarget, e: &Event) -> Option<EventTarget> {
            if t == EventTarget::External(1) && e.composed() {
                Some(EventTarget::External(2))
            } else {
                None
            }
        }
    }
    let mut r = EventTargets::default();
    r.add_listener(
        &Boundary,
        EventTarget::External(2),
        "test",
        Some(CallbackId(1)),
        ListenerOptions {
            capture: true,
            ..Default::default()
        },
    );
    let e = event(&mut r, true, false);
    let mut s = r
        .begin_dispatch(
            &Boundary,
            EventTarget::External(1),
            e,
            DispatchOrigin::Script,
        )
        .unwrap();
    assert!(r.next_listener(&mut s).is_none());
}

#[test]
fn utf16_listener_keys_distinguish_lone_surrogates_from_replacement_characters() {
    let (d, mut registry, targets) = fixture();
    let lone = [0xd800];
    let replacement = [0xfffd];
    let second_lone = [0xd801];
    for (name, callback) in [(&lone, 1), (&replacement, 2), (&second_lone, 3)] {
        registry.add_listener_utf16(&d, targets[2], name, Some(CallbackId(callback)), ListenerOptions::default());
    }
    let id = registry.create_event(Event::new_utf16(&lone, EventInit::default(), Duration::ZERO));
    assert_eq!(registry.event(id).event_type_utf16(), lone);
    let mut dispatch = registry.begin_dispatch(&d, targets[2], id, DispatchOrigin::Script).unwrap();
    assert_eq!(drain(&mut registry, &mut dispatch).iter().map(|i| i.callback).collect::<Vec<_>>(), vec![CallbackId(1)]);
    registry.event_mut(id).init_event("�", false, false);
    let mut dispatch = registry.begin_dispatch(&d, targets[2], id, DispatchOrigin::Script).unwrap();
    assert_eq!(drain(&mut registry, &mut dispatch).iter().map(|i| i.callback).collect::<Vec<_>>(), vec![CallbackId(2)]);
    assert!(registry.remove_listener(targets[2], "�", Some(CallbackId(1)), false).is_empty());
    assert_eq!(registry.remove_listener_utf16(targets[2], &lone, Some(CallbackId(1)), false), vec![CallbackId(1)]);
}

#[test]
fn utf16_init_during_dispatch_is_ignored_and_once_removes_exact_listener() {
    let (d, mut registry, targets) = fixture();
    let lone = [0xdc00];
    registry.add_listener_utf16(&d, targets[2], &lone, Some(CallbackId(1)), ListenerOptions { once: true, ..Default::default() });
    registry.add_listener(&d, targets[2], "�", Some(CallbackId(2)), ListenerOptions::default());
    let id = registry.create_event(Event::new_custom_utf16(&lone, EventInit::default(), Duration::ZERO, None));
    let mut dispatch = registry.begin_dispatch(&d, targets[2], id, DispatchOrigin::Script).unwrap();
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    assert_eq!(registry.event_mut(id).init_custom_event_utf16(&[0xfffd], false, false, None), None);
    assert_eq!(registry.event(id).event_type_utf16(), lone);
    assert!(registry.next_listener(&mut dispatch).is_none());
    assert!(registry.listener_callbacks_utf16(targets[2], &lone, false).is_empty());
    assert_eq!(registry.listener_callbacks(targets[2], "�", false), vec![CallbackId(2)]);
}
