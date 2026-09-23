//! WebIDL entry points for native shadow trees and HTML slots.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{
    shadow::{ShadowMode, ShadowOptions, SlotAssignment},
    DomError, NodeIdx, NodeKind,
};

pub(crate) fn handles(op: O) -> bool {
    matches!(
        op,
        O::GetRootNode
            | O::ElementAttachShadow
            | O::ElementShadowRoot
            | O::ElementAssignedSlot
            | O::ElementSlot
            | O::ElementSetSlot
            | O::TextAssignedSlot
            | O::ShadowHost
            | O::ShadowMode
            | O::ShadowDelegatesFocus
            | O::ShadowSlotAssignment
            | O::ShadowClonable
            | O::ShadowSerializable
            | O::SlotName
            | O::SlotSetName
            | O::SlotAssign
            | O::SlotAssignedNodes
            | O::SlotAssignedElements
    )
}
fn required_node<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<NodeIdx, H::Error> {
    host.node_brand(value)
        .ok_or_else(|| host.type_error("Expected a Node"))
}
fn optional_bool<H: BindingHost>(
    host: &mut H,
    object: &H::Value,
    name: &str,
) -> Result<bool, H::Error> {
    let value = host.get_property(object, name)?;
    Ok(host.to_boolean(&value))
}
fn optional_string<H: BindingHost>(
    host: &mut H,
    object: &H::Value,
    name: &str,
    default: &str,
) -> Result<String, H::Error> {
    let value = host.get_property(object, name)?;
    if host.is_undefined(&value) {
        return Ok(default.into());
    }
    let units = host.to_dom_string(&value)?;
    String::from_utf16(&units).map_err(|_| host.type_error("Invalid shadow option"))
}
fn optional_node<H: BindingHost>(
    host: &mut H,
    node: Option<NodeIdx>,
) -> Result<H::Value, H::Error> {
    match node {
        Some(node) => host.wrap_node(node),
        None => Ok(host.null()),
    }
}
fn check_element<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        if matches!(dom.get(node).kind, NodeKind::Element { .. }) {
            Ok(())
        } else {
            Err(DomError::TypeError)
        }
    })
}
fn check_slot<H: BindingHost>(host: &mut H, node: NodeIdx) -> Result<(), H::Error> {
    host.with_dom(|dom| {
        if dom.is_html_slot(node) {
            Ok(())
        } else {
            Err(DomError::TypeError)
        }
    })
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: H::Value,
    args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let node = required_node(host, &receiver)?;
    match op {
        O::GetRootNode => {
            let composed = match args.first() {
                None => false,
                Some(value) if host.is_nullish(value) => false,
                Some(value) => optional_bool(host, value, "composed")?,
            };
            let root = host.with_dom(|dom| dom.root_node(node, composed))?;
            host.wrap_node(root)
        }
        O::ElementAttachShadow => {
            check_element(host, node)?;
            let init = args
                .first()
                .ok_or_else(|| host.type_error("ShadowRootInit is required"))?;
            if host.is_nullish(init) {
                return Err(host.type_error("ShadowRootInit.mode is required"));
            }
            let mode = optional_string(host, init, "mode", "")?;
            let mode = match mode.as_str() {
                "open" => ShadowMode::Open,
                "closed" => ShadowMode::Closed,
                _ => return Err(host.type_error("Invalid ShadowRootMode")),
            };
            let delegates_focus = optional_bool(host, init, "delegatesFocus")?;
            let serializable = optional_bool(host, init, "serializable")?;
            let assignment = optional_string(host, init, "slotAssignment", "named")?;
            let slot_assignment = match assignment.as_str() {
                "named" => SlotAssignment::Named,
                "manual" => SlotAssignment::Manual,
                _ => return Err(host.type_error("Invalid SlotAssignmentMode")),
            };
            let clonable = optional_bool(host, init, "clonable")?;
            let options = ShadowOptions {
                mode,
                delegates_focus,
                slot_assignment,
                clonable,
                serializable,
            };
            let root = host.with_dom(|dom| dom.attach_shadow(node, options))?;
            host.wrap_node(root)
        }
        O::ElementShadowRoot | O::ElementAssignedSlot | O::TextAssignedSlot => {
            let result = host.with_dom(|dom| match op {
                O::ElementShadowRoot if matches!(dom.get(node).kind, NodeKind::Element { .. }) => {
                    Ok(dom.visible_shadow_root(node))
                }
                O::ElementAssignedSlot
                    if matches!(dom.get(node).kind, NodeKind::Element { .. }) =>
                {
                    Ok(dom.assigned_slot(node))
                }
                O::TextAssignedSlot if matches!(dom.get(node).kind, NodeKind::Text { .. }) => {
                    Ok(dom.assigned_slot(node))
                }
                _ => Err(DomError::TypeError),
            })?;
            optional_node(host, result)
        }
        O::ShadowHost
        | O::ShadowMode
        | O::ShadowDelegatesFocus
        | O::ShadowSlotAssignment
        | O::ShadowClonable
        | O::ShadowSerializable => {
            let data =
                host.with_dom(|dom| dom.shadow_data(node).cloned().ok_or(DomError::TypeError))?;
            match op {
                O::ShadowHost => host.wrap_node(data.host),
                O::ShadowMode => host.string(match data.options.mode {
                    ShadowMode::Open => &['o' as u16, 'p' as u16, 'e' as u16, 'n' as u16],
                    ShadowMode::Closed => {
                        &['c' as u16, 'l' as u16, 'o' as u16, 's' as u16, 'e' as u16]
                    }
                }),
                O::ShadowDelegatesFocus => Ok(host.boolean(data.options.delegates_focus)),
                O::ShadowSlotAssignment => host.string(match data.options.slot_assignment {
                    SlotAssignment::Named => {
                        &['n' as u16, 'a' as u16, 'm' as u16, 'e' as u16, 'd' as u16]
                    }
                    SlotAssignment::Manual => &[
                        'm' as u16, 'a' as u16, 'n' as u16, 'u' as u16, 'a' as u16, 'l' as u16,
                    ],
                }),
                O::ShadowClonable => Ok(host.boolean(data.options.clonable)),
                _ => Ok(host.boolean(data.options.serializable)),
            }
        }
        O::ElementSlot | O::SlotName | O::ElementSetSlot | O::SlotSetName => {
            if matches!(op, O::SlotName | O::SlotSetName) {
                check_slot(host, node)?;
            } else {
                check_element(host, node)?;
            }
            let name = if matches!(op, O::SlotName | O::SlotSetName) {
                "name"
            } else {
                "slot"
            };
            if matches!(op, O::ElementSlot | O::SlotName) {
                let value = host.with_dom(|dom| {
                    Ok(dom
                        .get_attribute_ns(node, None, name)
                        .unwrap_or("")
                        .encode_utf16()
                        .collect::<Vec<_>>())
                })?;
                host.string(&value)
            } else {
                let value = args.first().cloned().unwrap_or_else(|| host.undefined());
                let units = host.to_dom_string(&value)?;
                let value = String::from_utf16(&units)
                    .map_err(|_| host.type_error("Invalid attribute value"))?;
                host.with_dom(|dom| dom.set_attribute(node, name, &value))?;
                Ok(host.undefined())
            }
        }
        O::SlotAssign => {
            check_slot(host, node)?;
            let nodes = args
                .iter()
                .map(|value| required_node(host, value))
                .collect::<Result<Vec<_>, _>>()?;
            host.with_dom(|dom| dom.assign_slot(node, &nodes))?;
            Ok(host.undefined())
        }
        O::SlotAssignedNodes | O::SlotAssignedElements => {
            check_slot(host, node)?;
            let flatten = match args.first() {
                None => false,
                Some(value) if host.is_nullish(value) => false,
                Some(value) => optional_bool(host, value, "flatten")?,
            };
            let nodes = host
                .with_dom(|dom| dom.assigned_nodes(node, flatten, op == O::SlotAssignedElements))?;
            let mut values = Vec::with_capacity(nodes.len());
            for node in nodes {
                values.push(host.wrap_node(node)?);
            }
            host.create_array(values)
        }
        _ => unreachable!("shadow operation routed first"),
    }
}
