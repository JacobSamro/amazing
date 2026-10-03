//! Stateless branded XMLSerializer interface; native DOM owns serialization.
use crate::host::{BindingHost, InterfaceBrand, Operation as O};
pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::XMLSerializerConstructor | O::XMLSerializerSerializeToString)
}
pub(crate) fn invoke<H: BindingHost>(host: &mut H, op: O, receiver: H::Value, args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::XMLSerializerConstructor { return host.xml_serializer_new(); }
    if host.platform_object(&receiver, InterfaceBrand::XML_SERIALIZER).is_none() {
        return Err(host.type_error("Expected an XMLSerializer"));
    }
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    let node = host.node_brand(value).ok_or_else(|| host.type_error("Expected a Node"))?;
    let serialized = host.with_dom(|dom| dom.serialize_xml(node, false))?;
    host.string(&serialized)
}
