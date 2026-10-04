use super::*;
use crate::events::{CallbackId, DispatchOrigin, Event, EventInit, EventTarget, EventTargets, EventTree, ListenerOptions};
use std::time::Duration;

fn associate(dom: &mut Dom, document: NodeIdx) -> u64 {
    let slot = dom.create_window_event_target();
    dom.associate_document_window_event_target(document, dom.node_generation(document).unwrap(), slot).unwrap();
    slot
}
fn event(registry: &mut EventTargets, kind: &str) -> crate::events::EventId {
    registry.create_event(Event::new(kind, EventInit { bubbles: true, cancelable: true, composed: true }, Duration::ZERO))
}
fn add(registry: &mut EventTargets, dom: &Dom, target: EventTarget, callback: u64) {
    registry.add_listener(dom, target, "test", Some(CallbackId(callback)), ListenerOptions::default());
}

#[test]
fn root_child_sibling_and_inert_documents_have_distinct_actual_window_paths() {
    let mut dom = Dom::new(); let root = NodeIdx(0); let child = dom.create_document();
    let sibling = dom.create_document(); let inert = dom.create_document();
    let root_window = associate(&mut dom, root); let child_window = associate(&mut dom, child);
    let sibling_window = associate(&mut dom, sibling); let node = dom.create_element(child, "div");
    let mut registry = EventTargets::default();
    for (target, callback) in [(EventTarget::External(root_window), 9), (EventTarget::External(sibling_window), 8),
        (EventTarget::Node(root), 7), (EventTarget::Node(node), 3), (EventTarget::Node(child), 2),
        (EventTarget::External(child_window), 1)] { add(&mut registry, &dom, target, callback); }
    let id = event(&mut registry, "test");
    let mut dispatch = registry.begin_dispatch(&dom, EventTarget::Node(node), id, DispatchOrigin::UserAgent).unwrap();
    assert_eq!(registry.event(id).raw_path(), &[EventTarget::Node(node), EventTarget::Node(child), EventTarget::External(child_window)]);
    assert!(registry.event(id).composed_path().is_empty());
    let mut callbacks = vec![]; while let Some(call) = registry.next_listener(&mut dispatch) {
        assert_eq!(registry.event(id).composed_path(), vec![EventTarget::Node(node), EventTarget::Node(child), EventTarget::External(child_window)]);
        callbacks.push(call.callback.0);
    }
    assert_eq!(callbacks, vec![3, 2, 1]);
    let direct = event(&mut registry, "test");
    let mut dispatch = registry.begin_dispatch(&dom, EventTarget::External(child_window), direct, DispatchOrigin::Script).unwrap();
    assert_eq!(registry.event(direct).raw_path(), &[EventTarget::External(child_window)]);
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    assert!(registry.next_listener(&mut dispatch).is_none());
    assert_eq!(dom.document_window_event_target(inert), None);
    assert_eq!(EventTree::event_parent(&dom, EventTarget::Node(inert), registry.event(direct)), None);
    let load = event(&mut registry, "load");
    registry.begin_dispatch(&dom, EventTarget::Node(node), load, DispatchOrigin::UserAgent).unwrap();
    assert_eq!(registry.event(load).raw_path(), &[EventTarget::Node(node), EventTarget::Node(child)]);
}

#[test]
fn association_validation_is_atomic_and_identical_registration_does_not_invalidate_caches() {
    let mut dom = Dom::new(); let doc = NodeIdx(0); let child = dom.create_document();
    let generation = dom.node_generation(doc).unwrap(); let window = associate(&mut dom, doc);
    let epoch = dom.window_event_association_generation();
    dom.associate_document_window_event_target(doc, generation, window).unwrap();
    assert_eq!(dom.window_event_association_generation(), epoch);
    let ordinary = dom.create_event_target(); let another = dom.create_window_event_target();
    for result in [dom.associate_document_window_event_target(doc, generation + 1, window),
        dom.associate_document_window_event_target(doc, generation, ordinary),
        dom.associate_document_window_event_target(doc, generation, another),
        dom.associate_document_window_event_target(child, dom.node_generation(child).unwrap(), window),
        dom.associate_document_window_event_target(doc, generation, u64::MAX)] {
        assert_eq!(result, Err(DomError::InvalidStateError));
    }
    let element = dom.create_element(child, "div");
    assert_eq!(dom.associate_document_window_event_target(element, dom.node_generation(element).unwrap(), another), Err(DomError::TypeError));
    assert_eq!(dom.window_event_association_generation(), epoch);
    assert_eq!(dom.document_window_event_target(doc), Some(window));
    assert_eq!(dom.document_window_event_target(child), None);
    assert!(!dom.is_window_event_target(ordinary));
}

#[test]
fn document_recycling_and_stale_retirement_never_retarget_retained_window_identity() {
    let mut dom = Dom::new(); let old = dom.create_document(); let old_gen = dom.node_generation(old).unwrap();
    let old_window = associate(&mut dom, old);
    dom.retire_unreachable_node(old).unwrap();
    let replacement = dom.create_document(); assert_eq!(replacement, old);
    let new_gen = dom.node_generation(replacement).unwrap(); assert_ne!(new_gen, old_gen);
    assert_eq!(dom.document_window_event_target(replacement), None);
    let new_window = associate(&mut dom, replacement); assert_ne!(old_window, new_window);
    assert!(!dom.dissociate_document_window_event_target(old, old_gen, old_window));
    assert_eq!(dom.document_window_event_target(replacement), Some(new_window));
    assert!(dom.is_window_event_target(old_window));
    assert!(EventTree::default_passive(&dom, EventTarget::External(old_window), "wheel"));
    let epoch = dom.window_event_association_generation();
    assert!(dom.dissociate_document_window_event_target(replacement, new_gen, new_window));
    assert_ne!(dom.window_event_association_generation(), epoch);
    assert!(dom.is_window_event_target(new_window));
    assert_eq!(dom.document_window_event_target(replacement), None);
}

#[test]
fn passive_defaults_use_real_window_kind_and_each_documents_element_and_body() {
    let mut dom = Dom::new(); let root = NodeIdx(0); let child = dom.create_document();
    let root_window = associate(&mut dom, root); let child_window = associate(&mut dom, child);
    let external = dom.create_event_target(); let root_html = dom.create_element(root, "html");
    let root_body = dom.create_element(root_html, "body"); let child_html = dom.create_element(child, "html");
    let child_frameset = dom.create_element(child_html, "frameset"); let descendant = dom.create_element(child_frameset, "div");
    for kind in ["touchstart", "touchmove", "wheel", "mousewheel"] {
        for target in [EventTarget::External(root_window), EventTarget::External(child_window), EventTarget::Node(root),
            EventTarget::Node(child), EventTarget::Node(root_html), EventTarget::Node(root_body),
            EventTarget::Node(child_html), EventTarget::Node(child_frameset)] {
            assert!(EventTree::default_passive(&dom, target, kind), "{target:?} {kind}");
        }
        assert!(!EventTree::default_passive(&dom, EventTarget::External(external), kind));
        assert!(!EventTree::default_passive(&dom, EventTarget::Node(descendant), kind));
    }
    for kind in ["click", "touchend", "load", "Wheel"] {
        assert!(!EventTree::default_passive(&dom, EventTarget::External(child_window), kind));
    }
    let mut registry = EventTargets::default();
    registry.add_listener(&dom, EventTarget::External(child_window), "wheel", Some(CallbackId(1)), ListenerOptions::default());
    registry.add_listener(&dom, EventTarget::External(child_window), "wheel", Some(CallbackId(2)), ListenerOptions { passive: Some(false), ..Default::default() });
    let id = event(&mut registry, "wheel");
    let mut dispatch = registry.begin_dispatch(&dom, EventTarget::External(child_window), id, DispatchOrigin::UserAgent).unwrap();
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    registry.event_mut(id).prevent_default(); assert!(!registry.event(id).default_prevented());
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(2));
    registry.event_mut(id).prevent_default(); assert!(registry.event(id).default_prevented());
}

#[test]
fn retirement_changes_next_path_and_blocking_policy_but_not_captured_dispatch() {
    let mut dom = Dom::new(); let child = dom.create_document(); let generation = dom.node_generation(child).unwrap();
    let window = associate(&mut dom, child); let node = dom.create_element(child, "div");
    dom.add_event_listener(EventTarget::External(window), "wheel", Some(CallbackId(1)), ListenerOptions { passive: Some(false), ..Default::default() });
    assert!(dom.has_blocking_listener(EventTarget::Node(node), &["wheel"]));
    assert!(!dom.has_blocking_listener(EventTarget::Node(NodeIdx(0)), &["wheel"]));
    let mut registry = EventTargets::default(); add(&mut registry, &dom, EventTarget::External(window), 1);
    let id = event(&mut registry, "test");
    let mut dispatch = registry.begin_dispatch(&dom, EventTarget::Node(node), id, DispatchOrigin::Script).unwrap();
    let epoch = dom.window_event_association_generation();
    assert!(dom.dissociate_document_window_event_target(child, generation, window));
    assert_ne!(dom.window_event_association_generation(), epoch);
    assert!(!dom.has_blocking_listener(EventTarget::Node(node), &["wheel"]));
    assert!(dom.has_blocking_listener(EventTarget::External(window), &["wheel"]));
    let new_window = associate(&mut dom, child); add(&mut registry, &dom, EventTarget::External(new_window), 2);
    assert_eq!(registry.next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    assert!(registry.next_listener(&mut dispatch).is_none());
    let id = event(&mut registry, "test");
    let mut next = registry.begin_dispatch(&dom, EventTarget::Node(node), id, DispatchOrigin::Script).unwrap();
    assert_eq!(registry.next_listener(&mut next).unwrap().callback, CallbackId(2));
    assert!(registry.next_listener(&mut next).is_none());
}

#[test]
fn genuine_window_load_overrides_target_without_fabricating_document_bubbling() {
    let mut dom = Dom::new(); let child = dom.create_document(); let generation = dom.node_generation(child).unwrap();
    let window = associate(&mut dom, child); let root = associate(&mut dom, NodeIdx(0));
    for (target, callback) in [(EventTarget::Node(child), 2), (EventTarget::External(root), 3), (EventTarget::External(window), 1)] {
        dom.add_event_listener(target, "load", Some(CallbackId(callback)), ListenerOptions::default());
    }
    let event = dom.events_mut().create_event(Event::new("load", EventInit::default(), Duration::ZERO));
    assert!(dom.begin_document_load_dispatch(child, generation + 1, event).is_err());
    assert_eq!(dom.events().event(event).target(), None);
    let mut dispatch = dom.begin_document_load_dispatch(child, generation, event).unwrap();
    assert_eq!(dom.events_mut().next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    let live = dom.events().event(event);
    assert_eq!(live.target(), Some(EventTarget::Node(child)));
    assert_eq!(live.current_target(), Some(EventTarget::External(window)));
    assert_eq!(live.composed_path(), vec![EventTarget::External(window)]);
    assert_eq!(live.phase(), crate::events::EventPhase::AtTarget);
    assert!(live.is_trusted()); assert!(!live.bubbles()); assert!(!live.cancelable());
    assert!(dom.events_mut().next_listener(&mut dispatch).is_none());
    let public = dom.events_mut().create_event(Event::new("load", EventInit::default(), Duration::ZERO));
    let mut dispatch = dom.begin_event_dispatch(EventTarget::External(window), public, DispatchOrigin::Script).unwrap();
    assert_eq!(dom.events_mut().next_listener(&mut dispatch).unwrap().callback, CallbackId(1));
    assert_eq!(dom.events().event(public).target(), Some(EventTarget::External(window)));
    assert!(!dom.events().event(public).is_trusted());
    assert!(dom.events_mut().next_listener(&mut dispatch).is_none());
    assert!(dom.dissociate_document_window_event_target(child, generation, window));
    assert!(dom.begin_document_load_dispatch(child, generation, event).is_err());
}

#[test]
fn body_frameset_handler_targets_share_active_document_window_rules() {
    let mut dom = Dom::new(); let child = dom.create_document(); let inert = dom.create_document();
    let root_window = associate(&mut dom, NodeIdx(0)); let child_window = associate(&mut dom, child);
    let body = dom.create_element(NodeIdx(0), "body"); let second = dom.create_element(body, "body");
    dom.detach(second);
    let frameset = dom.create_element(child, "frameset"); let inactive = dom.create_element(inert, "body");
    let ordinary = dom.create_element(frameset, "div"); let svg = dom.create_element_ns(frameset,Some("http://www.w3.org/2000/svg"),"body").unwrap();
    for kind in ["load", "blur", "error", "focus", "resize", "scroll", "message", "storage", "pageswap", "pagereveal", "unload"] {
        assert_eq!(dom.event_handler_target(body,kind), Ok(Some(EventTarget::External(root_window))));
        assert_eq!(dom.event_handler_target(second,kind), Ok(Some(EventTarget::External(root_window))));
        assert_eq!(dom.event_handler_target(frameset,kind), Ok(Some(EventTarget::External(child_window))));
        assert_eq!(dom.event_handler_target(inactive,kind), Ok(None));
        assert_eq!(dom.event_handler_target(ordinary,kind), Ok(Some(EventTarget::Node(ordinary))));
        assert_eq!(dom.event_handler_target(svg,kind), Ok(Some(EventTarget::Node(svg))));
    }
    assert_eq!(dom.event_handler_target(inactive,"click"), Ok(Some(EventTarget::Node(inactive))));
    dom.pre_insert(second, frameset, None).unwrap();
    assert_eq!(dom.event_handler_target(second,"load"), Ok(Some(EventTarget::External(child_window))));
    let generation = dom.node_generation(child).unwrap();
    dom.dissociate_document_window_event_target(child,generation,child_window);
    assert_eq!(dom.event_handler_target(second,"load"), Ok(None));
    assert!(dom.event_handler_target(NodeIdx(u32::MAX),"load").is_err());
}
