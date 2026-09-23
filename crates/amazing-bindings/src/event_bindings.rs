use crate::host::{BindingHost, CallbackId, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{
    abort::AbortSignalId,
    events::{DispatchOrigin, Event, EventId, EventInit, EventTarget, ListenerOptions},
    DomError, NodeKind,
};
use std::time::Duration;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::EventTargetConstructor | O::AddEventListener | O::RemoveEventListener
        | O::DispatchEvent | O::EventConstructor | O::CustomEventConstructor
        | O::CustomEventDetail | O::CustomEventInit | O::CreateEvent | O::EventType
        | O::EventTargetValue | O::EventCurrentTarget | O::EventPhase | O::EventBubbles
        | O::EventCancelable | O::EventComposed | O::EventDefaultPrevented
        | O::EventIsTrusted | O::EventTimeStamp | O::EventStopPropagation
        | O::EventStopImmediatePropagation | O::EventPreventDefault
        | O::EventComposedPath | O::EventInitEvent | O::EventCancelBubble
        | O::EventSetCancelBubble | O::EventReturnValue | O::EventSetReturnValue)
}
fn key<H: BindingHost>(host: &H, slot: u64) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena, slot, generation: 1 }
}
fn utf16_string<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    let units = host.to_dom_string(value)?;
    // The native Event registry still stores event types as Rust String. A
    // separate UTF-16 native migration is required for lone-surrogate types.
    String::from_utf16(&units).map_err(|_| host.type_error("Invalid event type"))
}
fn event_id<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<EventId, H::Error> {
    let key = host.platform_object(value, InterfaceBrand::EVENT)
        .ok_or_else(|| host.type_error("Expected an Event"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid Event"));
    }
    let id = EventId::from_slot(key.slot as usize);
    host.with_dom(|dom| dom.events().event_checked(id).ok_or(DomError::InvalidStateError).map(|_| ()))?;
    Ok(id)
}
fn target<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<EventTarget, H::Error> {
    if let Some(node) = host.node_brand(value) { return Ok(EventTarget::Node(node)); }
    if let Some(key) = host.platform_object(value, InterfaceBrand::ABORT_SIGNAL) {
        if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
            return Err(host.type_error("Invalid AbortSignal"));
        }
        let slot = host.with_dom(|dom| dom.abort_signal_target(AbortSignalId(key.slot as usize)))?;
        return Ok(EventTarget::External(slot));
    }
    let key = host.platform_object(value, InterfaceBrand::EVENT_TARGET)
        .ok_or_else(|| host.type_error("Expected an EventTarget"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid EventTarget"));
    }
    host.with_dom(|dom| if dom.has_event_target(key.slot) { Ok(()) }
        else { Err(DomError::InvalidStateError) })?;
    Ok(EventTarget::External(key.slot))
}
fn wrap_target<H: BindingHost>(host: &mut H, target: Option<EventTarget>) -> Result<H::Value, H::Error> {
    match target {
        Some(EventTarget::Node(node)) => host.wrap_node(node),
        Some(EventTarget::External(slot)) => {
            let signal = host.with_dom(|dom| Ok(dom.abort_signal_for_target(slot)))?;
            match signal {
                Some(id) => host.wrap_platform_object(key(host, id.0 as u64), InterfaceBrand::ABORT_SIGNAL),
                None => host.wrap_platform_object(key(host, slot), InterfaceBrand::EVENT_TARGET),
            }
        }
        None => Ok(host.null()),
    }
}
pub(crate) fn dispatch_abort_event<H: BindingHost>(host: &mut H, signal: AbortSignalId) -> Result<(), H::Error> {
    let target = host.with_dom(|dom| dom.abort_signal_target(signal).map(EventTarget::External))?;
    dispatch_native_event(host, target, "abort", EventInit::default())
}
pub(crate) fn dispatch_slotchange_event<H: BindingHost>(host: &mut H, slot: amazing_dom::NodeIdx) -> Result<(), H::Error> {
    dispatch_native_event(host, EventTarget::Node(slot), "slotchange",
        EventInit { bubbles: true, cancelable: false, composed: false })
}
pub(crate) fn dispatch_invalid_event<H: BindingHost>(host: &mut H, node: amazing_dom::NodeIdx) -> Result<(), H::Error> {
    dispatch_native_event(host, EventTarget::Node(node), "invalid",
        EventInit { bubbles: false, cancelable: true, composed: false })
}
pub(crate) fn dispatch_reset_event<H: BindingHost>(host: &mut H, node: amazing_dom::NodeIdx) -> Result<bool, H::Error> {
    dispatch_native_event_result(host, EventTarget::Node(node), "reset",
        EventInit { bubbles: true, cancelable: true, composed: false })
}
fn dispatch_native_event<H: BindingHost>(host: &mut H, target: EventTarget, kind: &str, init: EventInit) -> Result<(), H::Error> {
    dispatch_native_event_result(host, target, kind, init).map(|_| ())
}
fn dispatch_native_event_result<H: BindingHost>(host: &mut H, target: EventTarget, kind: &str, init: EventInit) -> Result<bool, H::Error> {
    let event = host.with_dom(|dom| Ok(dom.events_mut().create_event(
        Event::new(kind, init, Duration::ZERO))))?;
    let mut dispatch = host.with_dom(|dom| dom.begin_event_dispatch(target, event, DispatchOrigin::UserAgent))?;
    loop {
        let invocation = host.with_dom(|dom| Ok(dom.events_mut().next_listener(&mut dispatch)))?;
        let Some(invocation) = invocation else { break };
        let result = callback_call(host, invocation.callback, invocation.current_target, event);
        if invocation.once { host.release_value(invocation.callback); }
        if result.is_err() { dispatch.report_exception("Native event listener threw".into()); }
    }
    host.with_dom(|dom| dom.events().dispatch_result(event))
}
fn option<H: BindingHost>(host: &mut H, value: Option<&H::Value>, name: &str) -> Result<bool, H::Error> {
    let Some(value) = value else { return Ok(false) };
    if host.is_nullish(value) { return Ok(false); }
    if host.same_value(value, &host.boolean(true)) { return Ok(name == "capture"); }
    if host.same_value(value, &host.boolean(false)) { return Ok(false); }
    let field = host.get_property(value, name)?;
    Ok(host.to_boolean(&field))
}
fn callback_call<H: BindingHost>(
    host: &mut H, callback: CallbackId, this_target: EventTarget, event: EventId,
) -> Result<(), H::Error> {
    let value = host.root_value(callback).ok_or_else(|| host.type_error("Released EventListener"))?;
    let event_value = host.wrap_platform_object(key(host, event.slot() as u64), InterfaceBrand::EVENT)?;
    let (function, this) = if host.is_callable(&value) {
        (value, wrap_target(host, Some(this_target))?)
    } else {
        let function = host.get_property(&value, "handleEvent")?;
        if !host.is_callable(&function) { return Err(host.type_error("EventListener.handleEvent is not callable")); }
        (function, value)
    };
    host.call(&function, &this, &[event_value]).map(|_| ())
}
enum Output { Undefined, Boolean(bool), Number(f64), String(Vec<u16>), Target(Option<EventTarget>), Path(Vec<EventTarget>) }

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::EventTargetConstructor {
        let slot = host.with_dom(|dom| Ok(dom.create_event_target()))?;
        return host.wrap_platform_object(key(host, slot), InterfaceBrand::EVENT_TARGET);
    }
    if matches!(op, O::EventConstructor | O::CustomEventConstructor | O::CreateEvent) {
        if op == O::CreateEvent {
            let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Document"))?;
            host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) { Ok(()) }
                else { Err(DomError::TypeError) })?;
        }
        let raw_type = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let event_type = utf16_string(host, raw_type)?;
        let init = if matches!(op, O::EventConstructor | O::CustomEventConstructor) {
            let dictionary = args.get(1);
            EventInit {
                bubbles: option(host, dictionary, "bubbles")?,
                cancelable: option(host, dictionary, "cancelable")?,
                composed: option(host, dictionary, "composed")?,
            }
        } else {
            if !["Event", "Events", "CustomEvent"].iter().any(|name| name.eq_ignore_ascii_case(&event_type)) {
                return Err(host.type_error("Unsupported legacy event interface"));
            }
            EventInit::default()
        };
        let legacy_custom = op == O::CreateEvent && event_type.eq_ignore_ascii_case("CustomEvent");
        let event = if op == O::CustomEventConstructor {
            let dictionary = args.get(1);
            let detail = match dictionary {
                Some(value) if !host.is_nullish(value) => host.get_property(value, "detail")?,
                _ => host.undefined(),
            };
            let root = if host.is_nullish(&detail) { None } else { Some(host.retain_value(detail)) };
            Event::new_custom(event_type, init, Duration::ZERO, root)
        } else if legacy_custom {
            Event::new_custom("", init, Duration::ZERO, None)
        } else {
            Event::new(if op == O::CreateEvent { "" } else { &event_type }, init, Duration::ZERO)
        };
        let id = host.with_dom(|dom| Ok(dom.events_mut().create_event(event)))?;
        return host.wrap_platform_object(key(host, id.slot() as u64), InterfaceBrand::EVENT);
    }
    if matches!(op, O::AddEventListener | O::RemoveEventListener | O::DispatchEvent) {
        let target = target(host, &receiver)?;
        if op == O::DispatchEvent {
            let event_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let event = event_id(host, event_arg)?;
            let mut dispatch = host.with_dom(|dom| dom.begin_event_dispatch(target, event, DispatchOrigin::Script))?;
            loop {
                let invocation = host.with_dom(|dom| Ok(dom.events_mut().next_listener(&mut dispatch)))?;
                let Some(invocation) = invocation else { break };
                let result = callback_call(host, invocation.callback, invocation.current_target, event);
                if invocation.once { host.release_value(invocation.callback); }
                if result.is_err() { dispatch.report_exception("EventListener callback threw".into()); }
            }
            let result = host.with_dom(|dom| dom.events().dispatch_result(event))?;
            return Ok(host.boolean(result));
        }
        let event_type_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let event_type = utf16_string(host, event_type_arg)?;
        let callback = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
        if host.is_nullish(callback) { return Ok(host.undefined()); }
        let options = args.get(2);
        let capture = option(host, options, "capture")?;
        let existing = host.with_dom(|dom| Ok(dom.events().listener_callbacks(target, &event_type, capture)))?;
        let matched = existing.into_iter().find(|id| host.root_value(*id)
            .is_some_and(|value| host.same_value(&value, callback)));
        if op == O::RemoveEventListener {
            if let Some(id) = matched {
                host.with_dom(|dom| { dom.events_mut().remove_listener(target, &event_type, Some(id), capture); Ok(()) })?;
                host.release_value(id);
            }
            return Ok(host.undefined());
        }
        if matched.is_some() { return Ok(host.undefined()); }
        let listener_options = ListenerOptions {
            capture, once: option(host, options, "once")?,
            passive: options.map(|_| option(host, options, "passive")).transpose()?,
        };
        let id = host.retain_value(callback.clone());
        host.with_dom(|dom| Ok(dom.add_event_listener(target, &event_type, Some(id), listener_options)))?;
        return Ok(host.undefined());
    }
    let id = event_id(host, &receiver)?;
    let new_type = if matches!(op, O::EventInitEvent | O::CustomEventInit) {
        let event_type_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        Some(utf16_string(host, event_type_arg)?)
    } else { None };
    let bubbles = if matches!(op, O::EventInitEvent | O::CustomEventInit) { args.get(1).is_some_and(|v| host.to_boolean(v)) } else { false };
    let cancelable = if matches!(op, O::EventInitEvent | O::CustomEventInit) { args.get(2).is_some_and(|v| host.to_boolean(v)) } else { false };
    let boolean_arg = args.first().is_some_and(|v| host.to_boolean(v));
    if matches!(op, O::CustomEventDetail | O::CustomEventInit) {
        let custom = host.with_dom(|dom| Ok(dom.events().event(id).is_custom()))?;
        if !custom { return Err(host.type_error("Expected a CustomEvent")); }
        if op == O::CustomEventDetail {
            let root = host.with_dom(|dom| Ok(dom.events().event(id).custom_detail()))?;
            return Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()));
        }
        let detail = args.get(3).cloned().unwrap_or_else(|| host.null());
        let root = if host.is_nullish(&detail) { None } else { Some(host.retain_value(detail)) };
        let old = host.with_dom(|dom| Ok(dom.events_mut().event_mut(id)
            .init_custom_event(new_type.unwrap(), bubbles, cancelable, root)))?;
        match old {
            Some(previous) => { if let Some(previous) = previous { host.release_value(previous); } },
            None => { if let Some(root) = root { host.release_value(root); } },
        }
        return Ok(host.undefined());
    }
    let output = host.with_dom(|dom| {
        let event = dom.events_mut().event_mut(id);
        let result = match op {
            O::EventType => Output::String(event.event_type().encode_utf16().collect()),
            O::EventTargetValue => Output::Target(event.target()),
            O::EventCurrentTarget => Output::Target(event.current_target()),
            O::EventPhase => Output::Number(event.phase() as u8 as f64),
            O::EventBubbles => Output::Boolean(event.bubbles()),
            O::EventCancelable => Output::Boolean(event.cancelable()),
            O::EventComposed => Output::Boolean(event.composed()),
            O::EventDefaultPrevented => Output::Boolean(event.default_prevented()),
            O::EventIsTrusted => Output::Boolean(event.is_trusted()),
            O::EventTimeStamp => Output::Number(event.timestamp().as_secs_f64() * 1000.),
            O::EventCancelBubble => Output::Boolean(event.cancel_bubble()),
            O::EventReturnValue => Output::Boolean(event.return_value()),
            O::EventComposedPath => Output::Path(event.composed_path()),
            O::EventStopPropagation => { event.stop_propagation(); Output::Undefined },
            O::EventStopImmediatePropagation => { event.stop_immediate_propagation(); Output::Undefined },
            O::EventPreventDefault => { event.prevent_default(); Output::Undefined },
            O::EventSetCancelBubble => { event.set_cancel_bubble(boolean_arg); Output::Undefined },
            O::EventSetReturnValue => { event.set_return_value(boolean_arg); Output::Undefined },
            O::EventInitEvent => { event.init_event(new_type.unwrap(), bubbles, cancelable); Output::Undefined },
            _ => unreachable!(),
        };
        Ok(result)
    })?;
    Ok(match output {
        Output::Undefined => host.undefined(),
        Output::Boolean(value) => host.boolean(value),
        Output::Number(value) => host.number(value),
        Output::String(value) => host.string(&value)?,
        Output::Target(value) => wrap_target(host, value)?,
        Output::Path(path) => {
            let mut values = Vec::with_capacity(path.len());
            for target in path { values.push(wrap_target(host, Some(target))?); }
            host.create_array(values)?
        }
    })
}
