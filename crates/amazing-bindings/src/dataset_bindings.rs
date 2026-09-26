//! Live DOMStringMap bindings. The native DOM owns attribute mutation semantics.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{DomError, Namespace, NodeIdx};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ElementDataset | O::DatasetGet | O::DatasetSet | O::DatasetDelete | O::DatasetIndexedGet)
}
fn element<H: BindingHost>(host: &mut H, receiver: &H::Value, map: bool) -> Result<NodeIdx, H::Error> {
    let node = if map {
        let key = host.platform_object(receiver, InterfaceBrand::DOM_STRING_MAP)
            .ok_or_else(|| host.type_error("Expected a DOMStringMap"))?;
        host.node_from_platform_key(key).ok_or_else(|| host.type_error("Invalid DOMStringMap"))?
    } else {
        host.node_brand(receiver).ok_or_else(|| host.type_error("Expected an HTML, SVG or MathML element"))?
    };
    host.with_dom(|dom| {
        if matches!(dom.namespace(node), Some(Namespace::Html | Namespace::Svg | Namespace::MathMl)) { Ok(node) }
        else { Err(DomError::TypeError) }
    })
}
pub(crate) fn supported_index_count<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<u32, H::Error> {
    element(host, receiver, true)?;
    Ok(0)
}
pub(crate) fn supported_names<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<Vec<Vec<u16>>, H::Error> {
    let node = element(host, receiver, true)?;
    host.with_dom(|dom| dom.dataset_names(node))
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    let node = element(host, &receiver, op != O::ElementDataset)?;
    if op == O::ElementDataset {
        return host.wrap_platform_object(host.node_platform_key(node), InterfaceBrand::DOM_STRING_MAP);
    }
    if op == O::DatasetIndexedGet { return Ok(host.undefined()); }
    let name = args.first().ok_or_else(|| host.type_error("Missing dataset property name"))?;
    let name = host.to_dom_string(name)?;
    match op {
        O::DatasetGet => {
            let value = host.with_dom(|dom| dom.dataset_value(node, &name))?;
            match value { Some(value) => host.string(&value), None => Ok(host.undefined()) }
        }
        O::DatasetSet => {
            let value = args.get(1).ok_or_else(|| host.type_error("Missing dataset property value"))?;
            let value = host.to_dom_string(value)?;
            host.with_dom(|dom| dom.set_dataset_value(node, &name, value))?;
            Ok(host.undefined())
        }
        O::DatasetDelete => {
            host.with_dom(|dom| dom.delete_dataset_value(node, &name))?;
            Ok(host.boolean(true))
        }
        _ => unreachable!(),
    }
}
