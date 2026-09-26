use crate::host::{BindingHost, CallbackId, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{
    abort::AbortSignalId,
    events::{DispatchOrigin, Event, EventClass, EventExtra, EventId, EventInit, EventTarget, ListenerOptions},
    DomError, NodeKind,
};
use std::time::Duration;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::MessageEventConstructor | O::MessageEventGet(_) | O::MessageEventInit | O::EventExtra(_) | O::UIEventConstructor | O::FocusEventConstructor
        | O::MouseEventConstructor | O::WheelEventConstructor | O::KeyboardEventConstructor
        | O::CompositionEventConstructor | O::EventTargetConstructor | O::AddEventListener | O::RemoveEventListener
        | O::DispatchEvent | O::EventConstructor | O::CustomEventConstructor
        | O::CustomEventDetail | O::CustomEventInit | O::CreateEvent | O::EventType
        | O::EventTargetValue | O::EventCurrentTarget | O::EventPhase | O::EventBubbles
        | O::EventCancelable | O::EventComposed | O::EventDefaultPrevented
        | O::EventIsTrusted | O::EventTimeStamp | O::EventStopPropagation
        | O::EventStopImmediatePropagation | O::EventPreventDefault
        | O::EventComposedPath | O::EventInitEvent | O::EventCancelBubble
        | O::EventSetCancelBubble | O::EventReturnValue | O::EventSetReturnValue)
}
pub(crate) fn key<H: BindingHost>(host: &H, slot: u64) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena, slot, generation: 1 }
}
fn utf16_string<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    let units = host.to_dom_string(value)?;
    // The native Event registry still stores event types as Rust String. A
    // separate UTF-16 native migration is required for lone-surrogate types.
    String::from_utf16(&units).map_err(|_| host.type_error("Invalid event type"))
}
pub(crate) fn event_id<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<EventId, H::Error> {
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
    if let Some(slot) = host.window_event_target_slot(value) {
        return Ok(EventTarget::External(slot));
    }
    if let Some(slot) = host.external_event_target_slot(value) {
        host.with_dom(|dom| if dom.has_event_target(slot) { Ok(()) }
            else { Err(DomError::InvalidStateError) })?;
        return Ok(EventTarget::External(slot));
    }
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
            if let Some(window) = host.window_event_target_value(slot) { return Ok(window); }
            if let Some(value) = host.external_event_target_value(slot) { return Ok(value); }
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
/// Dispatch a browser-generated event at a DOM node or the Window event target.
/// The event is trusted; callers supply its bubbling, cancelability and composed flags.
pub fn dispatch_user_agent_event<H: BindingHost>(host: &mut H, target: EventTarget,
    kind: &str, init: EventInit) -> Result<(), H::Error> {
    dispatch_native_event(host, target, kind, init)
}
fn dispatch_native_event_result<H: BindingHost>(host: &mut H, target: EventTarget, kind: &str, init: EventInit) -> Result<bool, H::Error> {
    let event = host.with_dom(|dom| Ok(dom.events_mut().create_event(
        Event::new(kind, init, Duration::ZERO))))?;
    dispatch_created_event(host, target, event)
}
pub(crate) fn dispatch_created_event<H: BindingHost>(host: &mut H, target: EventTarget,
    event: EventId) -> Result<bool, H::Error> {
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
pub(crate) fn dispatch_focus_event<H: BindingHost>(host: &mut H, node: amazing_dom::NodeIdx,
    kind: &str, related: Option<amazing_dom::NodeIdx>) -> Result<(), H::Error> {
    let init = EventInit { bubbles: matches!(kind, "focusin" | "focusout"),
        cancelable: false, composed: true };
    let related_value = match related { Some(node) => Some(host.wrap_node(node)?), None => None };
    let related_root = related_value.map(|value| host.retain_value(value));
    let event = host.with_dom(|dom| {
        let mut event = Event::new(kind, init, Duration::ZERO);
        event.set_class(EventClass::Focus);
        event.set_related_target(related.map(EventTarget::Node));
        event.set_extra("relatedTarget", EventExtra::Root(related_root));
        Ok(dom.events_mut().create_event(event))
    })?;
    let mut dispatch = host.with_dom(|dom| dom.begin_event_dispatch(EventTarget::Node(node), event, DispatchOrigin::UserAgent))?;
    loop {
        let invocation = host.with_dom(|dom| Ok(dom.events_mut().next_listener(&mut dispatch)))?;
        let Some(invocation) = invocation else { break };
        let result = callback_call(host, invocation.callback, invocation.current_target, event);
        if invocation.once { host.release_value(invocation.callback); }
        if result.is_err() { dispatch.report_exception("Focus event listener threw".into()); }
    }
    host.with_dom(|dom| dom.events().dispatch_result(event)).map(|_| ())
}
fn option<H: BindingHost>(host: &mut H, value: Option<&H::Value>, name: &str) -> Result<bool, H::Error> {
    let Some(value) = value else { return Ok(false) };
    if host.is_nullish(value) { return Ok(false); }
    if host.same_value(value, &host.boolean(true)) { return Ok(name == "capture"); }
    if host.same_value(value, &host.boolean(false)) { return Ok(false); }
    let field = host.get_property(value, name)?;
    Ok(host.to_boolean(&field))
}
pub(crate) fn passive_option<H: BindingHost>(host: &mut H, options: Option<&H::Value>) -> Result<Option<bool>, H::Error> {
    let Some(options) = options else { return Ok(None) };
    if host.is_nullish(options) { return Ok(None); }
    if host.same_value(options, &host.boolean(true)) || host.same_value(options, &host.boolean(false)) {
        return Ok(None);
    }
    let passive = host.get_property(options, "passive")?;
    Ok((!host.is_undefined(&passive)).then(|| host.to_boolean(&passive)))
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

fn extra_spec(name: &'static str) -> EventExtra {
    match name {
        "view" | "relatedTarget" => EventExtra::Root(None),
        "ctrlKey" | "shiftKey" | "altKey" | "metaKey" | "repeat" | "isComposing" => EventExtra::Boolean(false),
        "key" | "code" | "data" => EventExtra::String(vec![]),
        _ => EventExtra::Number(0.),
    }
}

fn event_class(op: O) -> Option<EventClass> {
    Some(match op {
        O::UIEventConstructor => EventClass::Ui,
        O::FocusEventConstructor => EventClass::Focus,
        O::MouseEventConstructor => EventClass::Mouse,
        O::WheelEventConstructor => EventClass::Wheel,
        O::KeyboardEventConstructor => EventClass::Keyboard,
        O::CompositionEventConstructor => EventClass::Composition,
        _ => return None,
    })
}

// Inherited dictionaries precede derived dictionaries; each dictionary's
// supported members are visited in lexicographic order (WebIDL conversion).
fn event_fields(class: EventClass) -> &'static [&'static str] {
    const UI: &[&str] = &["detail", "view"];
    const FOCUS: &[&str] = &["detail", "view", "relatedTarget"];
    const MOUSE: &[&str] = &["detail", "view", "altKey", "ctrlKey", "metaKey", "shiftKey",
        "button", "buttons", "clientX", "clientY", "relatedTarget", "screenX", "screenY"];
    const WHEEL: &[&str] = &["detail", "view", "altKey", "ctrlKey", "metaKey", "shiftKey",
        "button", "buttons", "clientX", "clientY", "relatedTarget", "screenX", "screenY",
        "deltaMode", "deltaX", "deltaY", "deltaZ"];
    const KEYBOARD: &[&str] = &["detail", "view", "altKey", "ctrlKey", "metaKey", "shiftKey",
        "charCode", "code", "isComposing", "key", "keyCode", "location", "repeat", "which"];
    const COMPOSITION: &[&str] = &["detail", "view", "data"];
    match class {
        EventClass::Ui => UI, EventClass::Focus => FOCUS, EventClass::Mouse => MOUSE,
        EventClass::Wheel => WHEEL, EventClass::Keyboard => KEYBOARD,
        EventClass::Composition => COMPOSITION, _ => &[],
    }
}

fn ui_detail<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<f64, H::Error> {
    let number = host.to_number(value)?;
    if !number.is_finite() || number == 0. { return Ok(0.); }
    Ok(number.trunc().rem_euclid(4294967296.) as u32 as i32 as f64)
}

enum ConvertedExtra<V> { Native(EventExtra), Root(Option<V>) }
fn convert_extras<H: BindingHost>(host: &mut H, dictionary: &H::Value, class: EventClass)
    -> Result<Vec<(&'static str, ConvertedExtra<H::Value>)>, H::Error> {
    let mut extras = Vec::new();
    for &name in event_fields(class) {
        let value = host.get_property(dictionary, name)?;
        if host.is_undefined(&value) { continue; }
        let extra = match extra_spec(name) {
            EventExtra::Boolean(_) => ConvertedExtra::Native(EventExtra::Boolean(host.to_boolean(&value))),
            EventExtra::Number(_) => ConvertedExtra::Native(EventExtra::Number(
                if name == "detail" { ui_detail(host, &value)? } else { host.to_number(&value)? })),
            EventExtra::String(_) => ConvertedExtra::Native(EventExtra::String(host.to_dom_string(&value)?)),
            EventExtra::Root(_) => {
                if host.is_nullish(&value) { ConvertedExtra::Root(None) }
                else {
                    if name == "view" && host.window_event_target_slot(&value).is_none() {
                        return Err(host.type_error("UIEvent.view must be a Window"));
                    }
                    if name == "relatedTarget" { target(host, &value)?; }
                    ConvertedExtra::Root(Some(value))
                }
            },
        };
        extras.push((name, extra));
    }
    Ok(extras)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if matches!(op, O::MessageEventConstructor | O::MessageEventGet(_) | O::MessageEventInit) {
        return crate::message_event_bindings::invoke(host, op, receiver, args);
    }
    if op == O::EventTargetConstructor {
        let slot = host.with_dom(|dom| Ok(dom.create_event_target()))?;
        return host.wrap_platform_object(key(host, slot), InterfaceBrand::EVENT_TARGET);
    }
    if matches!(op, O::EventConstructor | O::CustomEventConstructor | O::CreateEvent)
        || event_class(op).is_some() {
        if op == O::CreateEvent {
            let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Document"))?;
            host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) { Ok(()) }
                else { Err(DomError::TypeError) })?;
        }
        let raw_type = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let event_type = if op == O::CreateEvent {
            // Interface names are ASCII aliases. An unmatched DOMString, including
            // lone surrogates, takes the NotSupportedError path rather than TypeError.
            String::from_utf16_lossy(&host.to_dom_string(raw_type)?)
        } else { utf16_string(host, raw_type)? };
        if op == O::CreateEvent && event_type.eq_ignore_ascii_case("MessageEvent") {
            return crate::message_event_bindings::create_legacy(host);
        }
        let init = if op != O::CreateEvent {
            let dictionary = args.get(1);
            if let Some(value) = dictionary.filter(|value| !host.is_nullish(value)) {
                host.require_object(value)?;
            }
            EventInit {
                bubbles: option(host, dictionary, "bubbles")?,
                cancelable: option(host, dictionary, "cancelable")?,
                composed: option(host, dictionary, "composed")?,
            }
        } else {
            if !["Event", "Events", "CustomEvent", "BeforeUnloadEvent", "CompositionEvent",
                "DeviceMotionEvent", "DeviceOrientationEvent", "DragEvent", "FocusEvent",
                "HashChangeEvent", "HTMLEvents", "KeyboardEvent", "MessageEvent",
                "MouseEvent", "MouseEvents", "StorageEvent", "SVGEvents", "TextEvent",
                "UIEvent", "UIEvents"].iter().any(|name| name.eq_ignore_ascii_case(&event_type)) {
                return host.with_dom(|_| Err(DomError::NotSupportedError));
            }
            EventInit::default()
        };
        let legacy_custom = op == O::CreateEvent && event_type.eq_ignore_ascii_case("CustomEvent");
        let mut event = if op == O::CustomEventConstructor {
            let dictionary = args.get(1);
            let detail = match dictionary {
                Some(value) if !host.is_nullish(value) => host.get_property(value, "detail")?,
                _ => host.undefined(),
            };
            let root = if host.is_nullish(&detail) { None } else { Some(host.retain_value(detail)) };
            Event::new_custom(event_type.clone(), init, Duration::ZERO, root)
        } else if legacy_custom {
            Event::new_custom("", init, Duration::ZERO, None)
        } else {
            Event::new(if op == O::CreateEvent { "" } else { &event_type }, init, Duration::ZERO)
        };
        if let Some(class) = event_class(op) {
            event.set_class(class);
            if let Some(dictionary) = args.get(1).filter(|value| !host.is_nullish(value)) {
                // Conversion can run user code and fail. Retain roots only once all
                // fields have converted so later exceptions cannot leak earlier roots.
                for (name, extra) in convert_extras(host, dictionary, class)? {
                    let extra = match extra {
                        ConvertedExtra::Native(value) => value,
                        ConvertedExtra::Root(value) => EventExtra::Root(value.map(|value| host.retain_value(value))),
                    };
                    event.set_extra(name, extra);
                }
            }
        }
        if op == O::CreateEvent {
            let class = match event_type.to_ascii_lowercase().as_str() {
                "compositionevent" => EventClass::Composition,
                "focusevent" => EventClass::Focus,
                "keyboardevent" => EventClass::Keyboard,
                "mouseevent" | "mouseevents" => EventClass::Mouse,
                "uievent" | "uievents" => EventClass::Ui,
                _ => event.class(),
            };
            event.set_class(class);
            event.mark_uninitialized();
        }
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
        if args.len() < 2 { return Err(host.type_error("Not enough arguments")); }
        let event_type = utf16_string(host, &args[0])?;
        let callback = &args[1];
        let null_callback = host.is_nullish(callback);
        if !null_callback && !host.is_object_value(callback)? {
            return Err(host.type_error("EventListener must be an object"));
        }
        // WebIDL converts the complete dictionary before the DOM algorithm can
        // return for null callbacks, duplicates, or already-aborted signals.
        let mut listener_options = ListenerOptions::default();
        let mut signal = None;
        if let Some(options) = args.get(2) {
            if host.is_object_value(options)? {
                listener_options.capture = option(host, Some(options), "capture")?;
                if op == O::AddEventListener {
                    listener_options.once = option(host, Some(options), "once")?;
                    listener_options.passive = passive_option(host, Some(options))?;
                    let value = host.get_property(options, "signal")?;
                    if !host.is_undefined(&value) {
                        signal = Some(crate::abort_bindings::signal(host, &value, InterfaceBrand::ABORT_SIGNAL)?);
                    }
                }
            } else {
                listener_options.capture = host.to_boolean(options);
            }
        }
        if null_callback { return Ok(host.undefined()); }
        let capture = listener_options.capture;
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
        let id = host.retain_value(callback.clone());
        let result = host.with_dom(|dom| dom.add_abortable_event_listener(
            target, &event_type, Some(id), listener_options, signal));
        if !matches!(result, Ok(Some(_))) { host.release_value(id); }
        result?;
        return Ok(host.undefined());
    }
    let id = event_id(host, &receiver)?;
    if let O::EventExtra(name) = op {
        let extra = host.with_dom(|dom| Ok(dom.events().event(id).extra(name).cloned()))?
            .unwrap_or_else(|| extra_spec(name));
        return Ok(match extra {
            EventExtra::Boolean(value) => host.boolean(value),
            EventExtra::Number(value) => host.number(value),
            EventExtra::String(value) => host.string(&value)?,
            EventExtra::Root(Some(root)) => host.root_value(root).unwrap_or_else(|| host.null()),
            EventExtra::Root(None) => host.null(),
        });
    }
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
