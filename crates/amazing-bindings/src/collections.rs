use crate::host::{BindingHost, InterfaceBrand, Operation as O};

pub(crate) fn handles(op: O) -> bool { matches!(op, O::NodeListLength | O::NodeListItem) }

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let live = host.platform_object(&receiver, InterfaceBrand::NODE_LIST);
    let static_list = host.platform_object(&receiver, InterfaceBrand::STATIC_NODE_LIST);
    let key = live.or(static_list).ok_or_else(|| host.type_error("Expected a NodeList"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid NodeList"));
    }
    let parent = if live.is_some() { Some(host.node_from_platform_key(key)
        .ok_or_else(|| host.type_error("Invalid NodeList"))?) } else { None };
    if op == O::NodeListLength {
        let length = host.with_dom(|dom| match parent {
            Some(parent) => Ok(dom.children(parent).count()),
            None => dom.static_node_list(key.slot as usize).map(|nodes| nodes.len())
                .ok_or(amazing_dom::DomError::InvalidStateError),
        })?;
        return Ok(host.number(length as f64));
    }
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    let number = host.to_number(value)?;
    let index = if number.is_finite() {
        number.trunc().rem_euclid(4_294_967_296.0) as u32
    } else { 0 };
    let node = host.with_dom(|dom| match parent {
        Some(parent) => Ok(dom.children(parent).nth(index as usize)),
        None => dom.static_node_list(key.slot as usize).map(|nodes| nodes.get(index as usize).copied())
            .ok_or(amazing_dom::DomError::InvalidStateError),
    })?;
    match node { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
}
