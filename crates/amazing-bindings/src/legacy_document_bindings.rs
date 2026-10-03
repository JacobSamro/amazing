//! Legacy document/body IDL attributes and specified no-op methods.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, Namespace, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::DocumentLegacyColorGet(_) | O::DocumentLegacyColorSet(_)
        | O::BodyLegacyColorGet(_) | O::BodyLegacyColorSet(_)
        | O::DocumentLegacyNoop | O::WindowLegacyNoop)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::WindowLegacyNoop {
        let global = host.global_object()?;
        if !host.is_nullish(&receiver) && !host.same_value(&receiver, &global)
            && host.window_event_target_slot(&receiver).is_none() {
            return Err(host.type_error("Expected a Window"));
        }
        return Ok(host.undefined());
    }
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Invalid legacy method receiver"))?;
    let body = matches!(op, O::BodyLegacyColorGet(_) | O::BodyLegacyColorSet(_));
    host.with_dom(|dom| {
        if if body { dom.namespace(node) == Some(Namespace::Html) && dom.local_name(node) == Some("body") }
            else { matches!(dom.get(node).kind, NodeKind::Document) } { Ok(()) }
        else { Err(DomError::TypeError) }
    })?;
    match op {
        O::DocumentLegacyNoop => Ok(host.undefined()),
        O::DocumentLegacyColorGet(color) | O::BodyLegacyColorGet(color) => {
            let value = host.with_dom(|dom| if body { dom.body_legacy_color(node, color) }
                else { dom.document_legacy_color(node, color) })?;
            host.string(&value)
        }
        O::DocumentLegacyColorSet(color) | O::BodyLegacyColorSet(color) => {
            let value = args.first().ok_or_else(|| host.type_error("Missing color value"))?;
            // LegacyNullToEmptyString applies to null, not undefined.
            let value = if host.is_nullish(value) && !host.is_undefined(value) { Vec::new() }
                else { host.to_dom_string(value)? };
            host.with_dom(|dom| if body { dom.set_body_legacy_color(node, color, value) }
                else { dom.set_document_legacy_color(node, color, value) })?;
            Ok(host.undefined())
        }
        _ => unreachable!(),
    }
}
