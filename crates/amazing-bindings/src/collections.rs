use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{collection::CollectionFilter, DomError, NodeKind};

pub(crate) fn handles(op: O) -> bool { matches!(op, O::NodeListLength | O::NodeListItem | O::NodeListIndexedGetter
    | O::ElementChildren | O::DocumentChildren | O::FragmentChildren | O::ElementGetElementsByTagName | O::ElementGetElementsByTagNameNS
    | O::ElementGetElementsByClassName | O::DocumentGetElementsByTagName
    | O::DocumentGetElementsByTagNameNS | O::DocumentGetElementsByClassName
    | O::HTMLCollectionLength | O::HTMLCollectionItem | O::HTMLCollectionNamedItem
    | O::HTMLCollectionIndexedGetter | O::HTMLCollectionNamedGetter) }

fn collection_slot<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<usize, H::Error> {
    let key = host.platform_object(receiver, InterfaceBrand::COLLECTION)
        .ok_or_else(|| host.type_error("Expected an HTMLCollection"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid HTMLCollection"));
    }
    Ok(key.slot as usize)
}

fn node_list_length<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<usize, H::Error> {
    let live = host.platform_object(receiver, InterfaceBrand::NODE_LIST);
    let static_list = host.platform_object(receiver, InterfaceBrand::STATIC_NODE_LIST);
    let filtered = host.platform_object(receiver, InterfaceBrand::FILTERED_NODE_LIST);
    let key = live.or(static_list).or(filtered).ok_or_else(|| host.type_error("Expected a NodeList"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid NodeList"));
    }
    let parent = if live.is_some() { Some(host.node_from_platform_key(key)
        .ok_or_else(|| host.type_error("Invalid NodeList"))?) } else { None };
    host.with_dom(|dom| match parent {
        Some(parent) => Ok(dom.children(parent).count()),
        None if filtered.is_some() => dom.collection_length(key.slot as usize),
        None => dom.static_node_list(key.slot as usize).map(|nodes| nodes.len())
            .ok_or(amazing_dom::DomError::InvalidStateError),
    })
}

pub(crate) fn supported_index_count<H: BindingHost>(
    host: &mut H, op: O, receiver: &H::Value,
) -> Result<u32, H::Error> {
    let count = match op {
        O::HTMLCollectionSupportedIndexCount => {
            let slot = collection_slot(host, receiver)?;
            host.with_dom(|dom| dom.collection_length(slot))?
        }
        O::NodeListSupportedIndexCount => node_list_length(host, receiver)?,
        _ => return Err(host.type_error("Expected a supported index count hook")),
    };
    Ok(u32::try_from(count).expect("DOM collection length exceeds u32"))
}

pub(crate) fn supported_names<H: BindingHost>(
    host: &mut H, op: O, receiver: &H::Value,
) -> Result<Vec<Vec<u16>>, H::Error> {
    if op != O::HTMLCollectionSupportedNames {
        return Err(host.type_error("Expected a supported names hook"));
    }
    let slot = collection_slot(host, receiver)?;
    host.with_dom(|dom| dom.collection_names(slot))
}

fn index<H: BindingHost>(host: &mut H, arg: Option<&H::Value>) -> Result<usize, H::Error> {
    let arg = arg.ok_or_else(|| host.type_error("Not enough arguments"))?;
    let number = host.to_number(arg)?;
    Ok(if number.is_finite() { number.trunc().rem_euclid(4_294_967_296.0) as usize } else { 0 })
}

fn string<H: BindingHost>(host: &mut H, arg: Option<&H::Value>) -> Result<Vec<u16>, H::Error> {
    let arg = arg.ok_or_else(|| host.type_error("Not enough arguments"))?;
    host.to_dom_string(arg)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if matches!(op, O::ElementChildren | O::DocumentChildren | O::FragmentChildren | O::ElementGetElementsByTagName | O::ElementGetElementsByTagNameNS
        | O::ElementGetElementsByClassName | O::DocumentGetElementsByTagName
        | O::DocumentGetElementsByTagNameNS | O::DocumentGetElementsByClassName) {
        let root = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Node"))?;
        let document = matches!(op, O::DocumentChildren | O::DocumentGetElementsByTagName | O::DocumentGetElementsByTagNameNS | O::DocumentGetElementsByClassName);
        host.with_dom(|dom| match &dom.get(root).kind {
            NodeKind::Document if document => Ok(()),
            NodeKind::DocumentFragment if op == O::FragmentChildren => Ok(()),
            NodeKind::Element { .. } if !document && op != O::FragmentChildren => Ok(()),
            _ => Err(DomError::TypeError),
        })?;
        let filter = match op {
            O::ElementChildren | O::DocumentChildren | O::FragmentChildren => CollectionFilter::Children,
            O::ElementGetElementsByTagName | O::DocumentGetElementsByTagName => CollectionFilter::TagName(string(host, args.first())?),
            O::ElementGetElementsByTagNameNS | O::DocumentGetElementsByTagNameNS => {
                let uri_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
                let uri = if host.is_nullish(uri_arg) { None } else { Some(string(host, Some(uri_arg))?).filter(|value| !value.is_empty()) };
                CollectionFilter::TagNameNs(uri, string(host, args.get(1))?)
            }
            _ => {
                let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
                let units = host.to_dom_string(arg)?;
                let classes = units.split(|unit| matches!(*unit, 9 | 10 | 12 | 13 | 32))
                    .filter(|token| !token.is_empty()).map(<[u16]>::to_vec).collect();
                CollectionFilter::ClassNames(classes)
            }
        };
        let slot = host.with_dom(|dom| dom.collection(root, filter))?;
        let mut key = host.node_platform_key(root);
        key.slot = slot as u64;
        return host.wrap_platform_object(key, InterfaceBrand::COLLECTION);
    }
    if matches!(op, O::HTMLCollectionLength | O::HTMLCollectionItem | O::HTMLCollectionNamedItem
        | O::HTMLCollectionIndexedGetter | O::HTMLCollectionNamedGetter) {
        let slot = collection_slot(host, &receiver)?;
        if op == O::HTMLCollectionLength {
            let len = host.with_dom(|dom| dom.collection_length(slot))?;
            return Ok(host.number(len as f64));
        }
        let node = if matches!(op, O::HTMLCollectionItem | O::HTMLCollectionIndexedGetter) {
            let position = index(host, args.first())?;
            host.with_dom(|dom| dom.collection_item(slot, position))?
        } else {
            let arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let name = host.to_dom_string(arg)?;
            host.with_dom(|dom| dom.collection_named_item(slot, &name))?
        };
        return match node { Some(node) => host.wrap_node(node), None => Ok(host.null()) };
    }
    let live = host.platform_object(&receiver, InterfaceBrand::NODE_LIST);
    let static_list = host.platform_object(&receiver, InterfaceBrand::STATIC_NODE_LIST);
    let filtered = host.platform_object(&receiver, InterfaceBrand::FILTERED_NODE_LIST);
    let key = live.or(static_list).or(filtered).ok_or_else(|| host.type_error("Expected a NodeList"))?;
    if key.arena != host.node_platform_key(host.current_document()).arena || key.generation != 1 {
        return Err(host.type_error("Invalid NodeList"));
    }
    let parent = if live.is_some() { Some(host.node_from_platform_key(key)
        .ok_or_else(|| host.type_error("Invalid NodeList"))?) } else { None };
    if op == O::NodeListLength {
        let length = node_list_length(host, &receiver)?;
        return Ok(host.number(length as f64));
    }
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    let number = host.to_number(value)?;
    let index = if number.is_finite() {
        number.trunc().rem_euclid(4_294_967_296.0) as u32
    } else { 0 };
    let node = host.with_dom(|dom| match parent {
        Some(parent) => Ok(dom.children(parent).nth(index as usize)),
        None if filtered.is_some() => dom.collection_item(key.slot as usize, index as usize),
        None => dom.static_node_list(key.slot as usize).map(|nodes| nodes.get(index as usize).copied())
            .ok_or(amazing_dom::DomError::InvalidStateError),
    })?;
    match node { Some(node) => host.wrap_node(node), None => Ok(host.null()) }
}
