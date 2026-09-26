//! Navigation event payloads. Navigation scheduling remains with the embedding.
use crate::{event_bindings::{event_id, key}, host::{BindingHost, InterfaceBrand, Operation as O}};
use amazing_dom::events::{Event, EventClass, EventExtra, EventInit};
use std::time::Duration;
pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::HashChangeEventConstructor | O::PopStateEventConstructor | O::PageTransitionEventConstructor
        | O::HashChangeOldURL | O::HashChangeNewURL | O::PopStateState | O::PopStateHasUAVisualTransition
        | O::PageTransitionPersisted | O::BeforeUnloadReturnValue | O::BeforeUnloadSetReturnValue)
}
fn member<H: BindingHost>(host: &mut H, dictionary: Option<&H::Value>, name: &str) -> Result<H::Value, H::Error> {
    match dictionary.filter(|value| !host.is_nullish(value)) {
        Some(value) => host.get_property(value, name), None => Ok(host.undefined()),
    }
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let class = match op {
        O::HashChangeEventConstructor | O::HashChangeOldURL | O::HashChangeNewURL => EventClass::HashChange,
        O::PopStateEventConstructor | O::PopStateState | O::PopStateHasUAVisualTransition => EventClass::PopState,
        O::PageTransitionEventConstructor | O::PageTransitionPersisted => EventClass::PageTransition,
        _ => EventClass::BeforeUnload,
    };
    if matches!(op, O::HashChangeEventConstructor | O::PopStateEventConstructor | O::PageTransitionEventConstructor) {
        let raw = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let event_type = host.to_dom_string(raw)?;
        let dictionary = args.get(1);
        if let Some(value) = dictionary.filter(|value| !host.is_nullish(value)) { host.require_object(value)?; }
        let bubbles = member(host, dictionary, "bubbles")?; let bubbles = host.to_boolean(&bubbles);
        let cancelable = member(host, dictionary, "cancelable")?; let cancelable = host.to_boolean(&cancelable);
        let composed = member(host, dictionary, "composed")?; let composed = host.to_boolean(&composed);
        let mut event = Event::new_utf16(&event_type, EventInit { bubbles, cancelable, composed }, Duration::ZERO);
        event.set_class(class);
        match class {
            EventClass::HashChange => {
                for name in ["newURL", "oldURL"] {
                    let value = member(host, dictionary, name)?;
                    let units = if host.is_undefined(&value) { Vec::new() } else { host.to_dom_string(&value)? };
                    event.set_extra(name, EventExtra::String(String::from_utf16_lossy(&units).encode_utf16().collect()));
                }
            }
            EventClass::PopState => {
                let transition = member(host, dictionary, "hasUAVisualTransition")?;
                event.set_extra("hasUAVisualTransition", EventExtra::Boolean(host.to_boolean(&transition)));
                let value = member(host, dictionary, "state")?;
                let root = if host.is_nullish(&value) { None } else { Some(host.retain_value(value)) };
                event.set_extra("state", EventExtra::Root(root));
            }
            EventClass::PageTransition => {
                let value = member(host, dictionary, "persisted")?;
                event.set_extra("persisted", EventExtra::Boolean(host.to_boolean(&value)));
            }
            _ => unreachable!(),
        }
        let id = host.with_dom(|dom| Ok(dom.events_mut().create_event(event)))?;
        return host.wrap_platform_object(key(host, id.slot() as u64), InterfaceBrand::EVENT);
    }
    let id = event_id(host, &receiver)?;
    if host.with_dom(|dom| Ok(dom.events().event(id).class()))? != class {
        return Err(host.type_error("Incompatible navigation event receiver"));
    }
    if op == O::BeforeUnloadSetReturnValue {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let value = host.to_dom_string(value)?;
        host.with_dom(|dom| { dom.events_mut().event_mut(id).set_extra("returnValue", EventExtra::String(value)); Ok(()) })?;
        return Ok(host.undefined());
    }
    let name = match op {
        O::HashChangeOldURL => "oldURL", O::HashChangeNewURL => "newURL", O::PopStateState => "state",
        O::PopStateHasUAVisualTransition => "hasUAVisualTransition", O::PageTransitionPersisted => "persisted",
        _ => "returnValue",
    };
    // Legacy createEvent instances read the specified defaults for absent payload entries.
    let extra = host.with_dom(|dom| Ok(dom.events().event(id).extra(name).cloned()))?;
    match extra {
        Some(EventExtra::String(value)) => host.string(&value),
        Some(EventExtra::Boolean(value)) => Ok(host.boolean(value)),
        Some(EventExtra::Root(Some(root))) => Ok(host.root_value(root).unwrap_or_else(|| host.null())),
        _ if name == "state" => Ok(host.null()),
        _ if matches!(name, "hasUAVisualTransition" | "persisted") => Ok(host.boolean(false)),
        _ => host.string(&[]),
    }
}
