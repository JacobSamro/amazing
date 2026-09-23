use crate::host::{DocumentField, DocumentHost, InterfaceBrand, Operation as O, UrlReflection};
use amazing_dom::{collection::CollectionFilter, DomError, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::NodeBaseURI | O::DocumentTitle | O::DocumentSetTitle | O::DocumentHead | O::DocumentBody
        | O::DocumentSetBody | O::DocumentElement | O::DocumentDoctype | O::DocumentReadyState | O::DocumentURL
        | O::DocumentURI | O::DocumentCharacterSet | O::DocumentContentType
        | O::DocumentCompatMode | O::DocumentReferrer | O::DocumentGetElementById
        | O::FragmentGetElementById | O::DocumentGetElementsByName | O::DocumentForms
        | O::DocumentImages | O::DocumentLinks | O::DocumentScripts
        | O::DocumentActiveElement | O::DocumentHasFocus)
}

pub(crate) fn invoke<H: DocumentHost + UrlReflection>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::NodeBaseURI {
        let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Node"))?;
        let (document, href) = host.with_dom(|dom| {
            let document = dom.owner_document(node).unwrap_or(node);
            Ok((document, dom.document_base_href(document)?))
        })?;
        let units = match href {
            Some(href) => host.resolve_url(&href)?,
            None => host.document_field(document, DocumentField::URL)?,
        };
        return host.string(&units);
    }
    let node = host.node_brand(&receiver).ok_or_else(|| host.type_error("Expected a Document"))?;
    let fragment = op == O::FragmentGetElementById;
    host.with_dom(|dom| match &dom.get(node).kind {
        NodeKind::Document if !fragment => Ok(()),
        NodeKind::DocumentFragment if fragment => Ok(()),
        _ => Err(DomError::TypeError),
    })?;
    match op {
        O::DocumentTitle => { let units = host.with_dom(|dom| dom.document_title(node))?; host.string(&units) }
        O::DocumentSetTitle => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let units = host.to_dom_string(value)?;
            host.with_dom(|dom| dom.set_document_title(node, &units))?;
            Ok(host.undefined())
        }
        O::DocumentHead | O::DocumentBody | O::DocumentElement | O::DocumentDoctype => {
            let target = host.with_dom(|dom| match op {
                O::DocumentHead => dom.document_head(node),
                O::DocumentBody => dom.document_body(node),
                O::DocumentDoctype => dom.document_doctype(node),
                _ => dom.document_element(node),
            })?;
            match target { Some(target) => host.wrap_node(target), None => Ok(host.null()) }
        }
        O::DocumentSetBody => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let body = host.node_brand(value).ok_or_else(|| host.type_error("Expected an Element"))?;
            host.with_dom(|dom| dom.set_document_body(node, body))?;
            Ok(host.undefined())
        }
        O::DocumentReadyState => {
            let state = host.with_dom(|dom| dom.document_ready_state(node))?;
            host.string(&state.as_str().encode_utf16().collect::<Vec<_>>())
        }
        O::DocumentURL | O::DocumentURI | O::DocumentCharacterSet | O::DocumentContentType
            | O::DocumentCompatMode | O::DocumentReferrer => {
            let field = match op {
                O::DocumentURL => DocumentField::URL,
                O::DocumentURI => DocumentField::DocumentURI,
                O::DocumentCharacterSet => DocumentField::CharacterSet,
                O::DocumentContentType => DocumentField::ContentType,
                O::DocumentCompatMode => DocumentField::CompatMode,
                _ => DocumentField::Referrer,
            };
            let units = host.document_field(node, field)?;
            host.string(&units)
        }
        O::DocumentGetElementById | O::FragmentGetElementById => {
            let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let id = host.to_dom_string(value)?;
            let element = host.with_dom(|dom| dom.get_element_by_id(node, &id))?;
            match element { Some(element) => host.wrap_node(element), None => Ok(host.null()) }
        }
        O::DocumentGetElementsByName | O::DocumentForms | O::DocumentImages
            | O::DocumentLinks | O::DocumentScripts => {
            let filter = match op {
                O::DocumentGetElementsByName => {
                    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
                    CollectionFilter::Name(host.to_dom_string(value)?)
                }
                O::DocumentForms => CollectionFilter::HtmlTag("form".encode_utf16().collect()),
                O::DocumentImages => CollectionFilter::HtmlTag("img".encode_utf16().collect()),
                O::DocumentLinks => CollectionFilter::Links,
                _ => CollectionFilter::HtmlTag("script".encode_utf16().collect()),
            };
            let slot = host.with_dom(|dom| dom.collection(node, filter))?;
            let mut key = host.node_platform_key(node);
            key.slot = slot as u64;
            host.wrap_platform_object(key, if op == O::DocumentGetElementsByName {
                InterfaceBrand::FILTERED_NODE_LIST
            } else { InterfaceBrand::COLLECTION })
        }
        O::DocumentActiveElement => {
            let active = host.active_element(node)?;
            let active = if active.is_some() { active } else {
                host.with_dom(|dom| Ok(dom.document_body(node)?.or(dom.document_element(node)?)))?
            };
            match active { Some(active) => host.wrap_node(active), None => Ok(host.null()) }
        }
        O::DocumentHasFocus => { let focused = host.has_focus(node)?; Ok(host.boolean(focused)) },
        _ => unreachable!(),
    }
}
