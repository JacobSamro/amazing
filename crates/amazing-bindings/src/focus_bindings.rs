use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, NodeIdx};

pub(crate) fn handles(op: O) -> bool { matches!(op, O::ElementFocus | O::ElementBlur) }

fn default_focus_visible<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<bool, H::Error> {
    host.with_dom(|dom| {
        let name = dom.local_name(node).unwrap_or("");
        Ok(matches!(name, "textarea" | "select") || name == "input"
            && dom.attribute_utf16(node, "type").is_none_or(|value| {
                !matches!(String::from_utf16_lossy(&value).to_ascii_lowercase().as_str(),
                    "button" | "checkbox" | "color" | "file" | "hidden" | "image" | "radio" | "range" | "reset" | "submit")
            }) || dom.attribute_utf16(node, "contenteditable").is_some())
    })
}

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
    let (document, current, focusable) = host.with_dom(|dom| {
        if dom.node_generation(node).is_none() { return Err(DomError::InvalidNode); }
        let document = dom.owner_document(node).ok_or(DomError::TypeError)?;
        Ok((document, dom.active_element(document), dom.is_focusable_area(node)))
    })?;
    if op == O::ElementBlur {
        if current != Some(node) { return Ok(host.undefined()); }
        host.with_dom(|dom| { dom.set_active_element(document, None, false); Ok(()) })?;
        super::event_bindings::dispatch_focus_event(host, node, "blur", None)?;
        super::event_bindings::dispatch_focus_event(host, node, "focusout", None)?;
        return Ok(host.undefined());
    }
    if !focusable || current == Some(node) { return Ok(host.undefined()); }
    let mut visible = default_focus_visible(host, node)?;
    if let Some(options) = args.first().filter(|value| !host.is_nullish(value)) {
        let option = host.get_property(options, "focusVisible")?;
        if !host.is_undefined(&option) { visible = host.to_boolean(&option); }
        // Reading preventScroll performs WebIDL dictionary access even though
        // scrolling is handled by the future GUI focus hook.
        let _prevent_scroll = host.get_property(options, "preventScroll")?;
    }
    host.with_dom(|dom| { dom.set_active_element(document, Some(node), visible); Ok(()) })?;
    if let Some(old) = current {
        super::event_bindings::dispatch_focus_event(host, old, "blur", Some(node))?;
        super::event_bindings::dispatch_focus_event(host, old, "focusout", Some(node))?;
    }
    if host.with_dom(|dom| Ok(dom.active_element(document)))? != Some(node) {
        return Ok(host.undefined());
    }
    super::event_bindings::dispatch_focus_event(host, node, "focus", current)?;
    super::event_bindings::dispatch_focus_event(host, node, "focusin", current)?;
    Ok(host.undefined())
}
