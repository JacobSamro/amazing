//! ARIAMixin reflection (DOM-B-REFLECT): `role` plus the `aria-*` string
//! attribute set defined by <https://w3c.github.io/aria/#ARIAMixin>. Every one
//! of these IDL attributes is a plain nullable-string (`DOMString?`)
//! reflection of the identically-named content attribute, and -- unlike
//! `element_reflection.rs`'s `LegacyReflectGet`/`Set`, which gate on a
//! specific tag or interface -- ARIAMixin applies to every `Element`
//! regardless of namespace or tag. That is different enough from the
//! existing tag-gated machinery to warrant its own small dispatch here
//! rather than bending that table's type check.
//!
//! Scope cut: the element-reference attributes (`ariaLabelledByElements` and
//! friends, `FrozenArray<Element>?`/`Element?`) are not implemented. Per
//! spec they need an explicitly-set-element override that the accessible
//! name/description computation (`amazing-a11y`, owned by the a11y lane)
//! would have to consult ahead of any ID-ref fallback -- an ID-ref-only
//! getter would silently diverge from that algorithm and from what
//! `Element.getAttribute("aria-labelledby")` already exposes. Routed to
//! a11y coordination rather than half-implemented.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, NodeKind};

pub(crate) fn handles(op: O) -> bool { matches!(op, O::AriaGet(_) | O::AriaSet(_)) }

pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value])
    -> Result<H::Value, H::Error> {
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected an Element"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    match op {
        O::AriaSet(attribute) => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            if host.is_nullish(value) {
                host.with_dom(|dom| dom.remove_attribute(node, attribute))?;
            } else {
                let units = host.to_dom_string(value)?;
                host.with_dom(|dom| crate::html_reflection::set(dom, node, attribute, units))?;
            }
            Ok(host.undefined())
        }
        O::AriaGet(attribute) => {
            let value = host.with_dom(|dom| Ok(dom.attribute_utf16(node, attribute)))?;
            match value { Some(units) => host.string(&units), None => Ok(host.null()) }
        }
        _ => unreachable!("routed by handles()"),
    }
}
