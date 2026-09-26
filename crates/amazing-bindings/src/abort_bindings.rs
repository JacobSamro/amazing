use crate::host::{AbortHost, BindingHost, InterfaceBrand, Operation as O, PlatformKey};
use amazing_dom::{abort::{AbortAlgorithm, AbortSignalId}, events::{EventTarget, ListenerOptions}, DomError};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::AbortControllerConstructor | O::AbortControllerSignal | O::AbortControllerAbort
        | O::AbortSignalAborted | O::AbortSignalReason | O::AbortSignalThrowIfAborted
        | O::AbortSignalOnAbort | O::AbortSignalSetOnAbort
        | O::AbortSignalAbort | O::AbortSignalAny | O::AbortSignalTimeout)
}
fn key<H: BindingHost>(host: &H, id: AbortSignalId) -> PlatformKey {
    PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: id.0 as u64, generation: 1 }
}
pub(crate) fn signal<H: BindingHost>(host: &mut H, value: &H::Value, brand: InterfaceBrand)
    -> Result<AbortSignalId, H::Error> {
    let key = host.platform_object(value, brand).ok_or_else(|| host.type_error("Expected an AbortSignal"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid AbortSignal"));
    }
    let id = AbortSignalId(key.slot as usize);
    host.with_dom(|dom| dom.abort_signal_checked(id))?;
    Ok(id)
}
pub(crate) fn default_reason<H: BindingHost>(host: &mut H, name: &str) -> Result<H::Value, H::Error> {
    let slot = host.with_dom(|dom| Ok(dom.create_dom_exception(
        name.encode_utf16().collect(), Vec::new())))?;
    let key = PlatformKey { arena: host.node_platform_key(host.current_document()).arena,
        slot: slot as u64, generation: 1 };
    host.wrap_platform_object(key, InterfaceBrand::DOM_EXCEPTION)
}
fn reason<H: BindingHost>(host: &mut H, arg: Option<&H::Value>, name: &str) -> Result<H::Value, H::Error> {
    if let Some(value) = arg.filter(|value| !host.is_undefined(value)) { Ok(value.clone()) }
    else { default_reason(host, name) }
}
pub(crate) fn propagate<H: AbortHost>(host: &mut H, id: AbortSignalId, value: H::Value)
    -> Result<(), H::Error> {
    let root = host.retain_value(value.clone());
    let deliveries = host.with_dom(|dom| dom.abort_signal(id, root))?;
    if deliveries.is_empty() { host.release_value(root); return Ok(()); }
    for delivery in deliveries {
        for algorithm in delivery.algorithms {
            match algorithm {
                AbortAlgorithm::Host(id) => host.run_abort_algorithm(id, &value)?,
                AbortAlgorithm::RemoveEventListener(id) => {
                    let removed = host.with_dom(|dom| Ok(dom.events_mut().remove_listener_id(id)))?;
                    if let Some(callback) = removed { host.release_value(callback); }
                }
            }
        }
        crate::event_bindings::dispatch_abort_event(host, delivery.signal)?;
    }
    Ok(())
}
pub fn fire_abort_timeout<H: AbortHost>(host: &mut H, id: AbortSignalId) -> Result<(), H::Error> {
    let existing = host.with_dom(|dom| dom.abort_signal_reason(id))?;
    if existing.is_some() { return Ok(()); }
    let value = default_reason(host, "TimeoutError")?;
    propagate(host, id, value)
}
pub(crate) fn invoke<H: AbortHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::AbortControllerConstructor {
        let id = host.with_dom(|dom| Ok(dom.create_abort_signal()))?;
        return host.wrap_platform_object(key(host, id), InterfaceBrand::ABORT_CONTROLLER);
    }
    if op == O::AbortSignalAbort {
        let value = reason(host, args.first(), "AbortError")?;
        let root = host.retain_value(value);
        let id = host.with_dom(|dom| {
            let id = dom.create_abort_signal();
            dom.abort_signal(id, root).map(|_| id)
        })?;
        return host.wrap_platform_object(key(host, id), InterfaceBrand::ABORT_SIGNAL);
    }
    if op == O::AbortSignalAny {
        let source = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let length = host.get_property(source, "length")?;
        let number = host.to_number(&length)?;
        if !number.is_finite() || number < 0. || number > 1_000_000. {
            return Err(host.type_error("Invalid AbortSignal sequence length"));
        }
        let count = number.trunc() as usize;
        let mut sources = Vec::with_capacity(count);
        for index in 0..count {
            let value = host.get_property(source, &index.to_string())?;
            sources.push(signal(host, &value, InterfaceBrand::ABORT_SIGNAL)?);
        }
        let id = host.with_dom(|dom| dom.create_dependent_abort_signal(&sources))?;
        return host.wrap_platform_object(key(host, id), InterfaceBrand::ABORT_SIGNAL);
    }
    if op == O::AbortSignalTimeout {
        let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let number = host.to_number(arg)?;
        if !number.is_finite() || number < 0. || number >= 18_446_744_073_709_551_616. {
            return Err(host.type_error("AbortSignal.timeout requires an unsigned long long"));
        }
        let milliseconds = number.trunc() as u64;
        let id = host.with_dom(|dom| Ok(dom.create_abort_signal()))?;
        host.schedule_abort_timeout(id, milliseconds)?;
        return host.wrap_platform_object(key(host, id), InterfaceBrand::ABORT_SIGNAL);
    }
    let is_controller = matches!(op, O::AbortControllerSignal | O::AbortControllerAbort);
    let id = signal(host, &receiver, if is_controller { InterfaceBrand::ABORT_CONTROLLER }
        else { InterfaceBrand::ABORT_SIGNAL })?;
    if op == O::AbortSignalOnAbort {
        let handler = host.with_dom(|dom| dom.abort_event_handler(id))?;
        return Ok(handler.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()));
    }
    if op == O::AbortSignalSetOnAbort {
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let handler = if host.is_callable(value) { Some(host.retain_value(value.clone())) } else { None };
        let old = host.with_dom(|dom| {
            let target = EventTarget::External(dom.abort_signal_target(id)?);
            let old = dom.set_abort_event_handler(id, handler)?;
            if let Some(old) = old { dom.events_mut().remove_listener(target, "abort", Some(old), false); }
            if let Some(handler) = handler {
                dom.add_event_listener(target, "abort", Some(handler), ListenerOptions::default());
            }
            Ok(old)
        })?;
        if let Some(old) = old { host.release_value(old); }
        return Ok(host.undefined());
    }
    match op {
        O::AbortControllerSignal => host.wrap_platform_object(key(host, id), InterfaceBrand::ABORT_SIGNAL),
        O::AbortControllerAbort => {
            if host.with_dom(|dom| dom.abort_signal_reason(id))?.is_none() {
                let value = reason(host, args.first(), "AbortError")?;
                propagate(host, id, value)?;
            }
            Ok(host.undefined())
        }
        O::AbortSignalAborted => {
            let aborted = host.with_dom(|dom| dom.abort_signal_reason(id))?.is_some();
            Ok(host.boolean(aborted))
        }
        O::AbortSignalReason | O::AbortSignalThrowIfAborted => {
            let root = host.with_dom(|dom| dom.abort_signal_reason(id))?;
            let value = root.and_then(|root| host.root_value(root));
            if op == O::AbortSignalThrowIfAborted {
                if let Some(value) = value { return Err(host.throw_value(value)); }
                return Ok(host.undefined());
            }
            Ok(value.unwrap_or_else(|| host.undefined()))
        }
        _ => Err(host.with_dom::<()>(|_| Err(DomError::InvalidStateError)).unwrap_err()),
    }
}
