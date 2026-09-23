use super::*;
use crate::events::{CallbackId, DispatchOrigin, Event, EventInit, EventTarget, ListenerOptions};
use std::time::Duration;

fn options(mode: ShadowMode, slot_assignment: SlotAssignment) -> ShadowOptions {
    ShadowOptions {
        mode,
        delegates_focus: true,
        slot_assignment,
        clonable: true,
        serializable: true,
    }
}

// Ported assertions: shadow-dom/Element-interface-attachShadow.html and
// shadow-dom/ShadowRoot-interface.html at WPT 53bc812da2ed81179ebd1ba76b2d197cc4747d12.
#[test]
fn attach_checks_hosts_and_exposes_only_open_roots() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let closed_host = dom.create_element(NodeIdx(0), "span");
    let invalid = dom.create_element(NodeIdx(0), "a");
    assert_eq!(
        dom.attach_shadow(invalid, options(ShadowMode::Open, SlotAssignment::Named)),
        Err(DomError::NotSupportedError)
    );
    let custom = dom.create_element(NodeIdx(0), "x-thing");
    assert!(dom.attach_shadow(custom, options(ShadowMode::Open, SlotAssignment::Named)).is_ok());
    let svg = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "g").unwrap();
    assert_eq!(dom.attach_shadow(svg, options(ShadowMode::Open, SlotAssignment::Named)),
        Err(DomError::NotSupportedError));
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    assert_eq!(dom.visible_shadow_root(host), Some(root));
    assert_eq!(dom.shadow_data(root).unwrap().host, host);
    assert_eq!(
        dom.attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named)),
        Err(DomError::NotSupportedError)
    );
    let closed = dom
        .attach_shadow(
            closed_host,
            options(ShadowMode::Closed, SlotAssignment::Named),
        )
        .unwrap();
    assert_eq!(dom.visible_shadow_root(closed_host), None);
    assert_eq!(
        dom.shadow_data(closed).unwrap().options.mode,
        ShadowMode::Closed
    );
    assert_eq!(
        dom.pre_insert(root, host, None),
        Err(DomError::HierarchyRequestError)
    );
}

// Ported assertions: dom/nodes/rootNode.html and
// shadow-dom/ShadowRoot-interface.html at the same WPT revision.
#[test]
fn root_and_connectivity_cross_host_only_when_composed() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Closed, SlotAssignment::Named))
        .unwrap();
    let child = dom.create_element(root, "span");
    assert_eq!(dom.root_node(child, false), Ok(root));
    assert_eq!(dom.root_node(child, true), Ok(NodeIdx(0)));
    assert!(dom.is_connected(child));
    assert_eq!(
        dom.shadow_including_ancestors(child),
        vec![child, root, host, NodeIdx(0)]
    );
    dom.pre_remove(host, NodeIdx(0)).unwrap();
    assert_eq!(dom.root_node(child, true), Ok(host));
    assert!(!dom.is_connected(child));
}

// Ported assertions: shadow-dom/slotchange.html and
// shadow-dom/manual-slot-assignment-no-wrong-unassign.html.
#[test]
fn named_and_manual_slots_assign_in_required_order_and_signal_once() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let a = dom.create_element(host, "span");
    let b = dom.create_element(host, "span");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    let first = dom.create_element(root, "slot");
    let second = dom.create_element(root, "slot");
    assert_eq!(dom.assigned_nodes(first, false, false), Ok(vec![a, b]));
    assert_eq!(dom.assigned_nodes(second, false, false), Ok(vec![]));
    assert_eq!(dom.assigned_slot(a), Some(first));
    assert!(dom.mutation_microtask_pending());
    let mut checkpoint = dom.begin_mutation_checkpoint();
    assert_eq!(dom.next_slotchange(&mut checkpoint), Some(first));
    assert_eq!(dom.next_slotchange(&mut checkpoint), None);
    dom.set_attribute(second, "name", "named").unwrap();
    dom.set_attribute(b, "slot", "named").unwrap();
    assert_eq!(dom.assigned_nodes(first, false, false), Ok(vec![a]));
    assert_eq!(dom.assigned_nodes(second, false, false), Ok(vec![b]));
    let mut checkpoint = dom.begin_mutation_checkpoint();
    assert_eq!(dom.next_slotchange(&mut checkpoint), Some(first));
    assert_eq!(dom.next_slotchange(&mut checkpoint), Some(second));
    let manual_host = dom.create_element(NodeIdx(0), "div");
    let c = dom.create_element(manual_host, "i");
    let d = dom.create_element(manual_host, "b");
    let manual_root = dom
        .attach_shadow(
            manual_host,
            options(ShadowMode::Open, SlotAssignment::Manual),
        )
        .unwrap();
    let manual_slot = dom.create_element(manual_root, "slot");
    dom.assign_slot(manual_slot, &[d, c, d]).unwrap();
    assert_eq!(
        dom.assigned_nodes(manual_slot, false, false),
        Ok(vec![d, c])
    );
}

// Ported assertions: shadow-dom/event-composed-path.html and
// shadow-dom/event-inside-shadow-tree.html at WPT 53bc812d.
#[test]
fn event_path_retargets_and_hides_closed_tree_from_outside_listener() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Closed, SlotAssignment::Named))
        .unwrap();
    let inner = dom.create_element(root, "span");
    for (i, node) in [inner, root, host, NodeIdx(0)].into_iter().enumerate() {
        dom.add_event_listener(
            EventTarget::Node(node),
            "ping",
            Some(CallbackId(i as u64)),
            ListenerOptions::default(),
        );
    }
    let event = dom.events_mut().create_event(Event::new(
        "ping",
        EventInit {
            bubbles: true,
            composed: true,
            cancelable: false,
        },
        Duration::ZERO,
    ));
    let mut dispatch = dom
        .begin_event_dispatch(EventTarget::Node(inner), event, DispatchOrigin::Script)
        .unwrap();
    let mut seen = Vec::new();
    while let Some(invocation) = dom.events_mut().next_listener(&mut dispatch) {
        seen.push((
            invocation.current_target,
            invocation.target,
            dom.events().event(event).composed_path(),
        ));
    }
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].0, EventTarget::Node(inner));
    assert_eq!(seen[0].1, EventTarget::Node(inner));
    assert_eq!(
        seen[0].2,
        vec![
            EventTarget::Node(inner),
            EventTarget::Node(root),
            EventTarget::Node(host),
            EventTarget::Node(NodeIdx(0))
        ]
    );
    assert_eq!(seen[2].0, EventTarget::Node(host));
    assert_eq!(seen[2].1, EventTarget::Node(host));
    assert_eq!(
        seen[2].2,
        vec![EventTarget::Node(host), EventTarget::Node(NodeIdx(0))]
    );
    assert!(dom.events().event(event).composed_path().is_empty());
    let quiet = dom.events_mut().create_event(Event::new(
        "ping",
        EventInit {
            bubbles: true,
            composed: false,
            cancelable: false,
        },
        Duration::ZERO,
    ));
    let mut dispatch = dom
        .begin_event_dispatch(EventTarget::Node(inner), quiet, DispatchOrigin::Script)
        .unwrap();
    let mut targets = Vec::new();
    while let Some(invocation) = dom.events_mut().next_listener(&mut dispatch) {
        targets.push(invocation.current_target);
    }
    assert_eq!(
        targets,
        vec![EventTarget::Node(inner), EventTarget::Node(root)]
    );
}

// Ported assertion: shadow-dom/event-with-related-target.html; the outer host
// sees target and relatedTarget both retargeted to itself and is suppressed.
#[test]
fn related_target_is_retargeted_at_each_boundary() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    let target = dom.create_element(root, "span");
    let related = dom.create_element(root, "i");
    for (i, node) in [target, root, host].into_iter().enumerate() {
        dom.add_event_listener(
            EventTarget::Node(node),
            "over",
            Some(CallbackId(i as u64)),
            ListenerOptions::default(),
        );
    }
    let event = dom.events_mut().create_event(Event::new(
        "over",
        EventInit {
            bubbles: true,
            composed: true,
            cancelable: false,
        },
        Duration::ZERO,
    ));
    dom.events_mut()
        .event_mut(event)
        .set_related_target(Some(EventTarget::Node(related)));
    let mut dispatch = dom
        .begin_event_dispatch(EventTarget::Node(target), event, DispatchOrigin::Script)
        .unwrap();
    let mut seen = Vec::new();
    while let Some(invocation) = dom.events_mut().next_listener(&mut dispatch) {
        seen.push((invocation.current_target, invocation.related_target));
    }
    assert_eq!(
        seen,
        vec![
            (EventTarget::Node(target), Some(EventTarget::Node(related))),
            (EventTarget::Node(root), Some(EventTarget::Node(related)))
        ]
    );
}

// Ported assertions: shadow-dom/shadow-root-clonable.html and
// shadow-dom/Node-prototype-cloneNode.html at WPT 53bc812d.
#[test]
fn clonable_shadow_root_is_deep_copied_even_for_shallow_host_clone() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    let input = dom.create_element(root, "input");
    assert_eq!(dom.clone_node(root, false), Err(DomError::NotSupportedError));
    let copy = dom.clone_node(host, false).unwrap();
    assert_eq!(dom.get(copy).first_child, None);
    let copy_root = dom.visible_shadow_root(copy).unwrap();
    assert_ne!(copy_root, root);
    assert_eq!(
        dom.local_name(dom.get(copy_root).first_child.unwrap()),
        Some("input")
    );
    assert_ne!(dom.get(copy_root).first_child, Some(input));
    assert!(dom.shadow_data(copy_root).unwrap().options.clonable);
    let other = dom.create_element(NodeIdx(0), "span");
    dom.attach_shadow(
        other,
        ShadowOptions {
            clonable: false,
            ..options(ShadowMode::Open, SlotAssignment::Named)
        },
    )
    .unwrap();
    let other_copy = dom.clone_node(other, true).unwrap();
    assert_eq!(dom.shadow_root_of_host(other_copy), None);
}

// Ported assertions: shadow-dom/event-inside-slotted-node.html and
// shadow-dom/event-composed-path-after-dom-mutation.html.
#[test]
fn slotted_path_crosses_shadow_even_when_not_composed_and_is_frozen() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let light = dom.create_element(host, "span");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Closed, SlotAssignment::Named))
        .unwrap();
    let slot = dom.create_element(root, "slot");
    for (i, node) in [light, slot, root, host].into_iter().enumerate() {
        dom.add_event_listener(
            EventTarget::Node(node),
            "ping",
            Some(CallbackId(i as u64)),
            ListenerOptions::default(),
        );
    }
    let event = dom.events_mut().create_event(Event::new(
        "ping",
        EventInit {
            bubbles: true,
            composed: false,
            cancelable: false,
        },
        Duration::ZERO,
    ));
    let mut dispatch = dom
        .begin_event_dispatch(EventTarget::Node(light), event, DispatchOrigin::Script)
        .unwrap();
    let mut targets = Vec::new();
    while let Some(invocation) = dom.events_mut().next_listener(&mut dispatch) {
        targets.push(invocation.current_target);
        if invocation.current_target == EventTarget::Node(light) {
            dom.pre_remove(slot, root).unwrap();
            assert_eq!(
                dom.events().event(event).composed_path(),
                vec![
                    EventTarget::Node(light),
                    EventTarget::Node(host),
                    EventTarget::Node(NodeIdx(0))
                ]
            );
        }
    }
    assert_eq!(
        targets,
        vec![
            EventTarget::Node(light),
            EventTarget::Node(slot),
            EventTarget::Node(root),
            EventTarget::Node(host)
        ]
    );
}

// Ported assertion: shadow-dom/slotchange.html fallback-content changes.
#[test]
fn fallback_change_signals_empty_slot_once_per_checkpoint() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    let slot = dom.create_element(root, "slot");
    let fallback = dom.create_element(slot, "b");
    assert!(dom.mutation_microtask_pending());
    let mut checkpoint = dom.begin_mutation_checkpoint();
    assert_eq!(dom.next_slotchange(&mut checkpoint), Some(slot));
    assert_eq!(dom.next_slotchange(&mut checkpoint), None);
    dom.pre_remove(fallback, slot).unwrap();
    let mut checkpoint = dom.begin_mutation_checkpoint();
    assert_eq!(dom.next_slotchange(&mut checkpoint), Some(slot));
    assert_eq!(dom.next_slotchange(&mut checkpoint), None);
}

// Ported assertions: shadow-dom/slotchange.html and HTMLSlotElement's
// flattened assignedNodes/assignedElements behavior from HTML cd8ac6f1.
#[test]
fn flatten_uses_fallback_and_expands_nested_slots() {
    let mut dom = Dom::new();
    let host = dom.create_element(NodeIdx(0), "div");
    let root = dom
        .attach_shadow(host, options(ShadowMode::Open, SlotAssignment::Named))
        .unwrap();
    let outer = dom.create_element(root, "slot");
    let text = dom.append_child(
        outer,
        NodeKind::Text {
            data: "fallback".into(),
        },
    );
    let nested = dom.create_element(outer, "slot");
    let child = dom.create_element(nested, "i");
    assert_eq!(dom.assigned_nodes(outer, false, false), Ok(vec![]));
    assert_eq!(
        dom.assigned_nodes(outer, true, false),
        Ok(vec![text, child])
    );
    assert_eq!(dom.assigned_nodes(outer, true, true), Ok(vec![child]));
    let light = dom.create_element(host, "span");
    assert_eq!(dom.assigned_nodes(outer, true, false), Ok(vec![light]));
}
