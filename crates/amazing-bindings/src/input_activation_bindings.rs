//! HTMLElement.click and checkbox/radio activation completion.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{events::{Dispatch, Event, EventClass, EventInit, EventTarget}};
use std::time::Duration;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::HTMLElementOnClick | O::HTMLElementSetOnClick | O::HTMLElementClick | O::InputIndeterminate | O::InputSetIndeterminate)
}
pub(crate) fn complete<H: BindingHost>(host: &mut H, dispatch: &mut Dispatch) -> Result<(), H::Error> {
    let node = host.with_dom(|dom| Ok(dom.finish_input_activation(dispatch)))?;
    if let Some(node) = node {
        crate::event_bindings::dispatch_user_agent_event(host, EventTarget::Node(node), "input",
            EventInit { bubbles: true, cancelable: false, composed: true })?;
        crate::event_bindings::dispatch_user_agent_event(host, EventTarget::Node(node), "change",
            EventInit { bubbles: true, cancelable: false, composed: false })?;
    }
    Ok(())
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an HTMLElement"))?;
    if matches!(op, O::HTMLElementOnClick | O::HTMLElementSetOnClick) {
        host.with_dom(|dom| if matches!(dom.get(node).kind,
            amazing_dom::NodeKind::Element { namespace: amazing_dom::Namespace::Html, .. }) { Ok(()) }
            else { Err(amazing_dom::DomError::TypeError) })?;
        let target = EventTarget::Node(node);
        if op == O::HTMLElementOnClick {
            let root = host.with_dom(|dom| Ok(dom.events().event_handler(target, "click")))?;
            return Ok(root.and_then(|root| host.root_value(root)).unwrap_or_else(|| host.null()));
        }
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        // EventHandler is `EventHandlerNonNull?` where EventHandlerNonNull carries
        // [LegacyTreatNonObjectAsNull]: a non-object value (including null/undefined)
        // clears the handler without throwing; an object is stored as-is even when it
        // is not callable (callable-ness is only checked when the handler is invoked).
        let object = host.is_object_value(value)?;
        let callback = object.then(|| host.retain_value(value.clone()));
        let old = host.with_dom(|dom| Ok(dom.events_mut().set_event_handler(target, "click", callback)))?;
        if let Some(old) = old { host.release_value(old); }
        return Ok(host.undefined());
    }
    if op != O::HTMLElementClick {
        let old = host.with_dom(|dom| dom.input_indeterminate(node))?;
        if op == O::InputIndeterminate { return Ok(host.boolean(old)); }
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let value = host.to_boolean(value);
        host.with_dom(|dom| dom.set_input_indeterminate(node, value))?;
        return Ok(host.undefined());
    }
    if !host.with_dom(|dom| dom.begin_html_click(node))? { return Ok(host.undefined()); }
    let result = (|| {
        let event = host.with_dom(|dom| {
            let mut event = Event::new("click", EventInit { bubbles: true, cancelable: true, composed: true }, Duration::ZERO);
            event.set_class(EventClass::Mouse);
            Ok(dom.events_mut().create_event(event))
        })?;
        let value = host.wrap_platform_object(crate::event_bindings::key(host, event.slot() as u64), InterfaceBrand::EVENT)?;
        crate::event_bindings::invoke(host, O::DispatchEvent, receiver, &[value])
    })();
    host.with_dom(|dom| { dom.end_html_click(node); Ok(()) })?;
    result.map(|_| host.undefined())
}
