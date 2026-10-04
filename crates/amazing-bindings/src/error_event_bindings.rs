//! Payload interfaces used by synchronous and asynchronous error reporting.
use crate::{event_bindings::{event_id, key}, host::{BindingHost, InterfaceBrand, Operation as O}};
use amazing_dom::events::{Event, EventClass, EventExtra, EventInit};
use std::time::Duration;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ErrorEventConstructor | O::PromiseRejectionEventConstructor | O::ErrorEventGet(_) | O::PromiseRejectionEventGet(_))
}
fn member<H: BindingHost>(host: &mut H, dictionary: Option<&H::Value>, name: &str) -> Result<H::Value, H::Error> {
    match dictionary.filter(|value| !host.is_nullish(value)) {
        Some(value) => host.get_property(value, name), None => Ok(host.undefined()),
    }
}
fn unsigned<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<f64, H::Error> {
    let value = host.to_number(value)?;
    Ok(if value.is_finite() { value.trunc().rem_euclid(4_294_967_296.) as u32 as f64 } else { 0. })
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let class = if matches!(op, O::ErrorEventConstructor | O::ErrorEventGet(_)) { EventClass::Error }
        else { EventClass::PromiseRejection };
    if matches!(op, O::ErrorEventConstructor | O::PromiseRejectionEventConstructor) {
        if class == EventClass::PromiseRejection && args.len() < 2 {
            return Err(host.type_error("Not enough arguments"));
        }
        let raw = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let kind = host.to_dom_string(raw)?;
        let dictionary = args.get(1);
        if let Some(value) = dictionary.filter(|value| !host.is_nullish(value)) { host.require_object(value)?; }
        let bubbles = member(host, dictionary, "bubbles")?; let bubbles = host.to_boolean(&bubbles);
        let cancelable = member(host, dictionary, "cancelable")?; let cancelable = host.to_boolean(&cancelable);
        let composed = member(host, dictionary, "composed")?; let composed = host.to_boolean(&composed);
        let mut event = Event::new_utf16(&kind, EventInit { bubbles, cancelable, composed }, Duration::ZERO);
        event.set_class(class);
        // Retain arbitrary JS payloads only after every dictionary conversion succeeds.
        let mut roots = Vec::new();
        if class == EventClass::Error {
            let value = member(host, dictionary, "colno")?;
            event.set_extra("colno", EventExtra::Number(unsigned(host, &value)?));
            roots.push(("error", member(host, dictionary, "error")?));
            let value = member(host, dictionary, "filename")?;
            let value = if host.is_undefined(&value) { Vec::new() } else { host.to_dom_string(&value)? };
            event.set_extra("filename", EventExtra::String(String::from_utf16_lossy(&value).encode_utf16().collect()));
            let value = member(host, dictionary, "lineno")?;
            event.set_extra("lineno", EventExtra::Number(unsigned(host, &value)?));
            let value = member(host, dictionary, "message")?;
            let value = if host.is_undefined(&value) { Vec::new() } else { host.to_dom_string(&value)? };
            event.set_extra("message", EventExtra::String(value));
        } else {
            let promise = member(host, dictionary, "promise")?;
            // The IDL deliberately uses required object, not Promise<any>.
            host.require_object(&promise)?;
            roots.push(("promise", promise));
            roots.push(("reason", member(host, dictionary, "reason")?));
        }
        for (name, value) in roots {
            let root = if host.is_undefined(&value) { None } else { Some(host.retain_value(value)) };
            event.set_extra(name, EventExtra::Root(root));
        }
        let id = host.with_dom(|dom| Ok(dom.events_mut().create_event(event)))?;
        host.event_created(id)?;
        return host.wrap_platform_object(key(host, id.slot() as u64), InterfaceBrand::EVENT);
    }
    let id = event_id(host, &receiver)?;
    if host.with_dom(|dom| Ok(dom.events().event(id).class()))? != class {
        return Err(host.type_error("Incompatible error event receiver"));
    }
    let name = match op { O::ErrorEventGet(name) | O::PromiseRejectionEventGet(name) => name, _ => unreachable!() };
    let value = host.with_dom(|dom| Ok(dom.events().event(id).extra(name).cloned()))?;
    match value {
        Some(EventExtra::String(value)) => host.string(&value),
        Some(EventExtra::Number(value)) => Ok(host.number(value)),
        Some(EventExtra::Root(Some(root))) => Ok(host.root_value(root).unwrap_or_else(|| host.undefined())),
        _ if matches!(name, "lineno" | "colno") => Ok(host.number(0.)),
        _ if matches!(name, "message" | "filename") => host.string(&[]),
        _ => Ok(host.undefined()),
    }
}
