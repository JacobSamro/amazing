//! ProcessingInstruction constructor and pseudo-attribute methods.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, NodeKind};
pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ProcessingInstructionConstructor | O::PiHasAttributes | O::PiGetAttributeNames
        | O::PiGetAttribute | O::PiSetAttribute | O::PiRemoveAttribute | O::PiToggleAttribute | O::PiHasAttribute)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::ProcessingInstructionConstructor {
        if args.is_empty() { return Err(host.type_error("Not enough arguments")); }
        let target = host.to_dom_string(&args[0])?;
        let data = if let Some(value) = args.get(1).filter(|value| !host.is_undefined(value)) {
            host.to_dom_string(value)?
        } else { Vec::new() };
        let document = host.current_document();
        let node = host.with_dom(|dom| {
            let target = String::from_utf16(&target).map_err(|_| DomError::InvalidCharacterError)?;
            dom.create_processing_instruction_in(document, &target, &data)
        })?;
        return host.wrap_node(node);
    }
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a ProcessingInstruction"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::ProcessingInstruction { .. }) { Ok(()) } else { Err(DomError::TypeError) })?;
    let required = match op { O::PiHasAttributes | O::PiGetAttributeNames => 0, O::PiSetAttribute => 2, _ => 1 };
    if args.len() < required { return Err(host.type_error("Not enough arguments")); }
    let name = if required > 0 { host.to_dom_string(&args[0])? } else { Vec::new() };
    match op {
        O::PiSetAttribute => {
            let value = host.to_dom_string(&args[1])?;
            host.with_dom(|dom| dom.set_processing_instruction_attribute(node, &name, &value))?;
            Ok(host.undefined())
        }
        O::PiRemoveAttribute => {
            host.with_dom(|dom| dom.remove_processing_instruction_attribute(node, &name))?;
            Ok(host.undefined())
        }
        O::PiToggleAttribute => {
            let force = args.get(1).filter(|value| !host.is_undefined(value)).map(|value| host.to_boolean(value));
            let value = host.with_dom(|dom| dom.toggle_processing_instruction_attribute(node, &name, force))?;
            Ok(host.boolean(value))
        }
        _ => {
            let attributes = host.with_dom(|dom| dom.processing_instruction_attributes(node))?;
            match op {
                O::PiHasAttributes => Ok(host.boolean(!attributes.is_empty())),
                O::PiHasAttribute => Ok(host.boolean(attributes.iter().any(|(key, _)| key == &name))),
                O::PiGetAttribute => match attributes.iter().find(|(key, _)| key == &name) {
                    Some((_, value)) => host.string(value), None => Ok(host.null()),
                },
                O::PiGetAttributeNames => {
                    let values = attributes.iter().map(|(key, _)| host.string(key)).collect::<Result<Vec<_>, _>>()?;
                    host.create_array(values)
                }
                _ => unreachable!(),
            }
        }
    }
}
