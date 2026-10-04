//! HTML MessageEvent; JavaScript values stay rooted by the embedding.
use crate::event_bindings::{dispatch_created_event, event_id, key};
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::events::{Event, EventId, EventInit, EventTarget, MessageEventData};
use std::time::Duration;

/// Already-decoded delivery data. IPC owns structured clone and transfer.
/// `source` and every port must be canonical live wrappers in the receiving realm.
pub struct MessageEventPayload<V> {
    pub data: V,
    pub origin: Vec<u16>,
    pub last_event_id: Vec<u16>,
    pub source: Option<V>,
    pub ports: Vec<V>,
}

fn validate<H: BindingHost>(host: &mut H, payload: &MessageEventPayload<H::Value>) -> Result<(), H::Error> {
    if payload.source.as_ref().is_some_and(|source| !host.is_message_event_source(source)) {
        return Err(host.type_error("Expected a Window, MessagePort or ServiceWorker source"));
    }
    if payload.ports.iter().any(|port| !host.is_message_port(port)) {
        return Err(host.type_error("Expected a MessagePort"));
    }
    Ok(())
}
fn root<H: BindingHost>(host: &mut H, payload: MessageEventPayload<H::Value>) -> Result<MessageEventData, H::Error> {
    validate(host, &payload)?;
    // Complete fallible conversion before retaining any roots.
    let ports = host.create_frozen_array(payload.ports)?;
    Ok(MessageEventData {
        data: Some(host.retain_value(payload.data)),
        origin: String::from_utf16_lossy(&payload.origin).encode_utf16().collect(),
        last_event_id: payload.last_event_id,
        source: payload.source.map(|value| host.retain_value(value)),
        ports: Some(host.retain_value(ports)),
    })
}
fn release<H: BindingHost>(host: &mut H, data: MessageEventData) {
    for root in [data.data, data.source, data.ports].into_iter().flatten() {
        host.release_value(root);
    }
}
fn create<H: BindingHost>(host: &mut H, kind: Vec<u16>, init: EventInit,
    payload: MessageEventPayload<H::Value>) -> Result<EventId, H::Error> {
    let data = root(host, payload)?;
    let result = host.with_dom(|dom| Ok(dom.events_mut().create_event(
        Event::new_message_utf16(&kind, init, Duration::ZERO, data.clone()))));
    let result = result.and_then(|event| {
        host.message_event_created(event)?;
        Ok(event)
    });
    if result.is_err() { release(host, data); }
    result
}
/// Deliver a trusted message through the common DOM listener registry.
/// Returns false if the event was canceled (message events are non-cancelable).
pub fn dispatch_message_event<H: BindingHost>(host: &mut H, target: EventTarget,
    kind: &str, payload: MessageEventPayload<H::Value>) -> Result<bool, H::Error> {
    let event = create(host, kind.encode_utf16().collect(), EventInit::default(), payload)?;
    dispatch_created_event(host, target, event)
}
pub(crate) fn create_legacy<H: BindingHost>(host: &mut H) -> Result<H::Value, H::Error> {
    let id = host.with_dom(|dom| {
        let mut event = Event::new_message("", EventInit::default(), Duration::ZERO,
            MessageEventData::default());
        event.mark_uninitialized();
        Ok(dom.events_mut().create_event(event))
    })?;
    host.message_event_created(id)?;
    host.wrap_platform_object(key(host, id.slot() as u64), InterfaceBrand::EVENT)
}
fn member<H: BindingHost>(host: &mut H, dictionary: Option<&H::Value>, name: &str) -> Result<H::Value, H::Error> {
    match dictionary {
        Some(value) if !host.is_nullish(value) => host.get_property(value, name),
        _ => Ok(host.undefined()),
    }
}
fn string_or_empty<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Vec<u16>, H::Error> {
    if host.is_undefined(value) { Ok(vec![]) } else { host.to_dom_string(value) }
}
fn ports<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<Vec<H::Value>, H::Error> {
    if host.is_undefined(value) { return Ok(vec![]); }
    // Reject null and other non-objects with TypeError before an optional host
    // hook can report NotSupportedError for the iterable conversion capability.
    host.require_object(value)?;
    host.sequence_message_ports(value)
}
fn source<H: BindingHost>(host: &mut H, value: H::Value) -> Result<Option<H::Value>, H::Error> {
    if host.is_nullish(&value) { return Ok(None); }
    if !host.is_message_event_source(&value) {
        return Err(host.type_error("Expected a Window, MessagePort or ServiceWorker source"));
    }
    Ok(Some(value))
}
fn type_string<H: BindingHost>(host: &mut H, args: &[H::Value]) -> Result<Vec<u16>, H::Error> {
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    host.to_dom_string(value)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::MessageEventConstructor {
        let kind = type_string(host, args)?;
        let dictionary = args.get(1);
        if let Some(value) = dictionary.filter(|value| !host.is_nullish(value)) {
            host.require_object(value)?;
        }
        // WebIDL converts inherited dictionary members first, then this dictionary
        // in lexicographic order. Each getter is read once and converted immediately.
        let value = member(host, dictionary, "bubbles")?;
        let bubbles = host.to_boolean(&value);
        let value = member(host, dictionary, "cancelable")?;
        let cancelable = host.to_boolean(&value);
        let value = member(host, dictionary, "composed")?;
        let composed = host.to_boolean(&value);
        let data = member(host, dictionary, "data")?;
        let data = if host.is_undefined(&data) { host.null() } else { data };
        let value = member(host, dictionary, "lastEventId")?;
        let last_event_id = string_or_empty(host, &value)?;
        let value = member(host, dictionary, "origin")?;
        let origin = string_or_empty(host, &value)?;
        let value = member(host, dictionary, "ports")?;
        let ports = ports(host, &value)?;
        let value = member(host, dictionary, "source")?;
        let source = source(host, value)?;
        let id = create(host, kind, EventInit { bubbles, cancelable, composed },
            MessageEventPayload { data, origin, last_event_id, source, ports })?;
        return host.wrap_platform_object(key(host, id.slot() as u64), InterfaceBrand::EVENT);
    }
    let id = event_id(host, &receiver)?;
    let data = host.with_dom(|dom| Ok(dom.events().event(id).message_data().cloned()))?
        .ok_or_else(|| host.type_error("Expected a MessageEvent"))?;
    if let O::MessageEventGet(name) = op {
        return match name {
            "origin" => host.string(&data.origin),
            "lastEventId" => host.string(&data.last_event_id),
            "data" | "source" => {
                let root = if name == "data" { data.data } else { data.source };
                Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()))
            },
            "ports" => {
                if let Some(value) = data.ports.and_then(|root| host.root_value(root)) { return Ok(value); }
                let value = host.create_frozen_array(vec![])?;
                let root = host.retain_value(value.clone());
                let result = host.with_dom(|dom| { dom.events_mut().event_mut(id).set_message_ports_root(root); Ok(()) });
                if result.is_err() { host.release_value(root); }
                result?;
                Ok(value)
            },
            _ => Err(host.type_error("Unknown MessageEvent attribute")),
        };
    }
    let kind = type_string(host, args)?;
    let bubbles = args.get(1).is_some_and(|value| host.to_boolean(value));
    let cancelable = args.get(2).is_some_and(|value| host.to_boolean(value));
    let data = args.get(3).filter(|value| !host.is_undefined(value)).cloned().unwrap_or_else(|| host.null());
    let origin_value = args.get(4).cloned().unwrap_or_else(|| host.undefined());
    let origin = string_or_empty(host, &origin_value)?;
    let id_value = args.get(5).cloned().unwrap_or_else(|| host.undefined());
    let last_event_id = string_or_empty(host, &id_value)?;
    let source_value = args.get(6).cloned().unwrap_or_else(|| host.null());
    let source = source(host, source_value)?;
    let ports_value = args.get(7).cloned().unwrap_or_else(|| host.undefined());
    let ports = ports(host, &ports_value)?;
    let rooted = root(host, MessageEventPayload { data, origin, last_event_id, source, ports })?;
    let previous = host.with_dom(|dom| Ok(dom.events_mut().event_mut(id)
        .init_message_event_utf16(&kind, bubbles, cancelable, rooted.clone())));
    match previous {
        Ok(Some(previous)) => release(host, previous),
        Ok(None) => release(host, rooted),
        Err(error) => { release(host, rooted); return Err(error); },
    }
    Ok(host.undefined())
}
