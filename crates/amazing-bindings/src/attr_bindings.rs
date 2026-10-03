use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{attr::AttrSnapshot, DomError, NodeIdx, NodeKind};

pub(crate) fn supported_index_count<H: BindingHost>(host: &mut H, receiver: &H::Value)
    -> Result<u32, H::Error> {
    let element = map_element(host, receiver)?;
    let count = host.with_dom(|dom| Ok(dom.attribute_nodes(element)?.len()))?;
    u32::try_from(count).map_err(|_| host.type_error("Attribute count exceeds u32"))
}

pub(crate) fn supported_names<H: BindingHost>(host: &mut H, receiver: &H::Value)
    -> Result<Vec<Vec<u16>>, H::Error> {
    let element = map_element(host, receiver)?;
    let (names, owner, html_namespace) = host.with_dom(|dom| {
        let attrs = dom.attribute_nodes(element)?;
        let names = attrs.into_iter().map(|attr| dom.attr_snapshot(attr).map(|attr| attr.name()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((names, dom.owner_document(element).unwrap(),
            dom.namespace_uri(element) == Some("http://www.w3.org/1999/xhtml")))
    })?;
    let html = html_namespace && host.is_html_document(owner);
    let mut seen = std::collections::HashSet::new();
    Ok(names.into_iter().filter(|name| !html || name.to_ascii_lowercase() == *name)
        .map(|name| name.encode_utf16().collect::<Vec<_>>())
        .filter(|name| seen.insert(name.clone())).collect())
}

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::CreateAttribute | O::CreateAttributeNS | O::ElementAttributes
        | O::ElementHasAttributes | O::ElementGetAttributeNames | O::ElementHasAttribute
        | O::ElementHasAttributeNS | O::ElementToggleAttribute
        | O::AttrNamespaceURI | O::AttrPrefix | O::AttrLocalName | O::AttrName
        | O::AttrValue | O::AttrSetValue | O::AttrOwnerElement | O::AttrSpecified
        | O::ElementGetAttribute | O::ElementGetAttributeNS | O::ElementSetAttribute
        | O::ElementSetAttributeNS | O::ElementRemoveAttribute | O::ElementRemoveAttributeNS
        | O::ElementGetAttributeNode | O::ElementGetAttributeNodeNS
        | O::ElementSetAttributeNode | O::ElementSetAttributeNodeNS
        | O::ElementRemoveAttributeNode | O::NamedNodeMapLength | O::NamedNodeMapItem
        | O::NamedNodeMapGetNamedItem | O::NamedNodeMapGetNamedItemNS
        | O::NamedNodeMapSetNamedItem | O::NamedNodeMapSetNamedItemNS
        | O::NamedNodeMapRemoveNamedItem | O::NamedNodeMapRemoveNamedItemNS)
}
fn string<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    let units = host.to_dom_string(value)?;
    String::from_utf16(&units).map_err(|_| host.type_error("Invalid attribute name"))
}
fn argument<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<String, H::Error> {
    let arg = args.get(index).ok_or_else(|| host.type_error("Not enough arguments"))?;
    string(host, arg)
}
fn nullable_uri<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<Option<String>, H::Error> {
    let value = args.get(index).ok_or_else(|| host.type_error("Not enough arguments"))?;
    Ok(if host.is_nullish(value) { None } else { Some(string(host, value)?) }
        .filter(|uri| !uri.is_empty()))
}
fn node<H: BindingHost>(host: &mut H, value: Option<&H::Value>) -> Result<NodeIdx, H::Error> {
    value.and_then(|value| host.node_brand(value)).ok_or_else(|| host.type_error("Expected a Node"))
}
fn element<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    let node = node(host, Some(receiver))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn document<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    let node = node(host, Some(receiver))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn map_element<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    let key = host.platform_object(receiver, InterfaceBrand::NAMED_NODE_MAP)
        .ok_or_else(|| host.type_error("Expected a NamedNodeMap"))?;
    let node = host.node_from_platform_key(key).ok_or_else(|| host.type_error("Invalid NamedNodeMap"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Element { .. }) { Ok(()) }
        else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn html_element_in_html_document<H: BindingHost>(host: &mut H, element: NodeIdx)
    -> Result<bool, H::Error> {
    let (document, html_namespace) = host.with_dom(|dom| Ok((dom.owner_document(element).unwrap(),
        dom.namespace_uri(element) == Some("http://www.w3.org/1999/xhtml"))))?;
    Ok(html_namespace && host.is_html_document(document))
}
fn wrap_optional<H: BindingHost>(host: &mut H, node: Option<NodeIdx>) -> Result<H::Value, H::Error> {
    match node { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
}
fn attr_value<H: BindingHost>(host: &mut H, attr: NodeIdx) -> Result<AttrSnapshot, H::Error> {
    host.with_dom(|dom| dom.attr_snapshot(attr))
}
fn u32_arg<H: BindingHost>(host: &mut H, args: &[H::Value]) -> Result<u32, H::Error> {
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    let number = host.to_number(value)?;
    Ok(if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as u32 } else { 0 })
}
fn qualified_match(snapshot: &AttrSnapshot, name: &str) -> bool { snapshot.name() == name }
fn named<H: BindingHost>(host: &mut H, element: NodeIdx, uri: Option<&str>, name: &str, qualified: bool) -> Result<Option<NodeIdx>, H::Error> {
    host.with_dom(|dom| {
        let nodes = dom.attribute_nodes(element)?;
        Ok(nodes.into_iter().find(|attr| dom.attr_snapshot(*attr).is_ok_and(|snapshot|
            if qualified { qualified_match(&snapshot, name) }
            else { snapshot.namespace_uri.as_deref() == uri && snapshot.local_name == name })))
    })
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if matches!(op, O::CreateAttribute | O::CreateAttributeNS) {
        host.current_document_for_nodes()?;
        let doc = document(host, &receiver)?;
        let attr = if op == O::CreateAttribute {
            let mut name = argument(host, args, 0)?;
            if host.is_html_document(doc) { name.make_ascii_lowercase(); }
            host.with_dom(|dom| dom.create_attribute_in(doc, &name))?
        } else {
            let uri = nullable_uri(host, args, 0)?;
            let qualified = argument(host, args, 1)?;
            host.with_dom(|dom| dom.create_attribute_ns_in(doc, uri.as_deref(), &qualified))?
        };
        return host.wrap_node(attr);
    }
    if matches!(op, O::AttrNamespaceURI | O::AttrPrefix | O::AttrLocalName | O::AttrName
        | O::AttrValue | O::AttrSetValue | O::AttrOwnerElement | O::AttrSpecified) {
        let attr = node(host, Some(&receiver))?;
        let snapshot = attr_value(host, attr)?;
        return Ok(match op {
            O::AttrNamespaceURI => match snapshot.namespace_uri { Some(s) => host.string(&s.encode_utf16().collect::<Vec<_>>())?, None => host.null() },
            O::AttrPrefix => match snapshot.prefix { Some(s) => host.string(&s.encode_utf16().collect::<Vec<_>>())?, None => host.null() },
            O::AttrLocalName => host.string(&snapshot.local_name.encode_utf16().collect::<Vec<_>>())?,
            O::AttrName => host.string(&snapshot.name().encode_utf16().collect::<Vec<_>>())?,
            O::AttrValue => host.string(&snapshot.value)?,
            O::AttrOwnerElement => wrap_optional(host, snapshot.owner_element)?,
            O::AttrSpecified => host.boolean(true),
            O::AttrSetValue => {
                let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
                let units = host.to_dom_string(value)?;
                host.with_dom(|dom| dom.set_attr_value(attr, units))?;
                host.undefined()
            }
            _ => unreachable!(),
        });
    }
    let element = if matches!(op, O::NamedNodeMapLength | O::NamedNodeMapItem
        | O::NamedNodeMapGetNamedItem | O::NamedNodeMapGetNamedItemNS
        | O::NamedNodeMapSetNamedItem | O::NamedNodeMapSetNamedItemNS
        | O::NamedNodeMapRemoveNamedItem | O::NamedNodeMapRemoveNamedItemNS) {
        map_element(host, &receiver)?
    } else { element(host, &receiver)? };
    if op == O::ElementAttributes {
        return host.wrap_platform_object(host.node_platform_key(element), InterfaceBrand::NAMED_NODE_MAP);
    }
    if op == O::ElementHasAttributes {
        let present = host.with_dom(|dom| Ok(dom.attributes_expanded(element)
            .is_some_and(|attrs| !attrs.is_empty())))?;
        return Ok(host.boolean(present));
    }
    if op == O::ElementGetAttributeNames {
        let names = host.with_dom(|dom| Ok(dom.attributes_expanded(element).unwrap().into_iter()
            .map(|attr| attr.qualified_name()).collect::<Vec<_>>()))?;
        let values = names.iter().map(|name| host.string(&name.encode_utf16().collect::<Vec<_>>()))
            .collect::<Result<Vec<_>, _>>()?;
        return host.create_array(values);
    }
    if matches!(op, O::ElementHasAttribute | O::ElementHasAttributeNS | O::ElementToggleAttribute) {
        let uri = if op == O::ElementHasAttributeNS { nullable_uri(host, args, 0)? } else { None };
        let mut name = argument(host, args, usize::from(op == O::ElementHasAttributeNS))?;
        let force = if op == O::ElementToggleAttribute { args.get(1).map(|value| host.to_boolean(value)) }
            else { None };
        if op == O::ElementToggleAttribute && !amazing_dom::Dom::is_valid_attribute_name(&name) {
            return host.with_dom::<H::Value>(|_| Err(DomError::InvalidCharacterError));
        }
        let html = html_element_in_html_document(host, element)?;
        if uri.is_none() && op != O::ElementHasAttributeNS && html { name.make_ascii_lowercase(); }
        let present = host.with_dom(|dom| Ok(if op == O::ElementHasAttributeNS {
            dom.attribute_ns(element, uri.as_deref(), &name).is_some()
        } else {
            dom.attributes_expanded(element).unwrap().into_iter()
                .any(|attr| attr.qualified_name() == name)
        }))?;
        if op != O::ElementToggleAttribute { return Ok(host.boolean(present)); }
        if !present && force != Some(false) {
            host.with_dom(|dom| dom.set_attribute(element, &name, ""))?;
            return Ok(host.boolean(true));
        }
        if present && force != Some(true) {
            if let Some(attr) = named(host, element, None, &name, true)? {
                host.with_dom(|dom| dom.remove_attribute_node(element, attr))?;
            }
            return Ok(host.boolean(false));
        }
        return Ok(host.boolean(present));
    }
    if matches!(op, O::NamedNodeMapLength | O::NamedNodeMapItem) {
        let index = if op == O::NamedNodeMapItem { Some(u32_arg(host, args)?) } else { None };
        let attrs = host.with_dom(|dom| dom.attribute_nodes(element))?;
        return if let Some(index) = index { wrap_optional(host, attrs.get(index as usize).copied()) }
            else { Ok(host.number(attrs.len() as f64)) };
    }
    if matches!(op, O::ElementSetAttributeNode | O::ElementSetAttributeNodeNS
        | O::NamedNodeMapSetNamedItem | O::NamedNodeMapSetNamedItemNS
        | O::ElementRemoveAttributeNode) {
        let attr = node(host, args.first())?;
        attr_value(host, attr)?;
        let old = host.with_dom(|dom| if op == O::ElementRemoveAttributeNode {
            dom.remove_attribute_node(element, attr).map(Some)
        } else { dom.set_attribute_node(element, attr) })?;
        return wrap_optional(host, old);
    }
    let qualified = matches!(op, O::ElementGetAttribute | O::ElementSetAttribute
        | O::ElementRemoveAttribute | O::ElementGetAttributeNode
        | O::NamedNodeMapGetNamedItem | O::NamedNodeMapRemoveNamedItem);
    let (uri, mut name, value_index) = if qualified {
        (None, argument(host, args, 0)?, 1)
    } else {
        (nullable_uri(host, args, 0)?, argument(host, args, 1)?, 2)
    };
    if qualified && html_element_in_html_document(host, element)? {
        name.make_ascii_lowercase();
    }
    if matches!(op, O::ElementGetAttribute | O::ElementGetAttributeNS
        | O::ElementGetAttributeNode | O::ElementGetAttributeNodeNS
        | O::NamedNodeMapGetNamedItem | O::NamedNodeMapGetNamedItemNS) {
        let attr = named(host, element, uri.as_deref(), &name, qualified)?;
        return if matches!(op, O::ElementGetAttribute | O::ElementGetAttributeNS) {
            match attr { Some(attr) => {
                let units = attr_value(host, attr)?.value;
                host.string(&units)
            }, None => Ok(host.null()) }
        } else { wrap_optional(host, attr) };
    }
    if matches!(op, O::ElementSetAttribute | O::ElementSetAttributeNS) {
        let value = args.get(value_index).ok_or_else(|| host.type_error("Not enough arguments"))?;
        let units = host.to_dom_string(value)?;
        if qualified && !amazing_dom::Dom::is_valid_attribute_name(&name) {
            return host.with_dom::<H::Value>(|_| Err(DomError::InvalidCharacterError));
        }
        let display = String::from_utf16_lossy(&units);
        let existing = if qualified { named(host, element, None, &name, true)? } else { None };
        let attr = host.with_dom(|dom| {
            if let Some(attr) = existing {
                let snapshot = dom.attr_snapshot(attr)?;
                dom.set_attribute_ns(element, snapshot.namespace_uri.as_deref(), &snapshot.local_name, &display)?;
                return Ok(attr);
            }
            if qualified { dom.set_attribute(element, &name, &display)?; }
            else { dom.set_attribute_qualified_ns(element, uri.as_deref(), &name, &display)?; }
            let (local, uri) = if qualified { (name.as_str(), None) }
                else { (name.rsplit(':').next().unwrap(), uri.as_deref()) };
            dom.get_attribute_node(element, uri, local).ok_or(DomError::InvalidStateError)
        })?;
        host.with_dom(|dom| dom.store_attr_value_units(attr, units))?;
        return Ok(host.undefined());
    }
    if matches!(op, O::ElementRemoveAttribute | O::ElementRemoveAttributeNS
        | O::NamedNodeMapRemoveNamedItem | O::NamedNodeMapRemoveNamedItemNS) {
        let attr = named(host, element, uri.as_deref(), &name, qualified)?;
        if matches!(op, O::NamedNodeMapRemoveNamedItem | O::NamedNodeMapRemoveNamedItemNS) {
            let attr = attr.ok_or_else(|| host.with_dom::<()>(|_| Err(DomError::NotFoundError)).err().unwrap())?;
            host.with_dom(|dom| dom.remove_attribute_node(element, attr))?;
            return host.wrap_node(attr);
        }
        if let Some(attr) = attr { host.with_dom(|dom| dom.remove_attribute_node(element, attr))?; }
        return Ok(host.undefined());
    }
    unreachable!()
}
