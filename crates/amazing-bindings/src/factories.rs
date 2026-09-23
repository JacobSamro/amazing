use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{DomError, NodeIdx, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::DocumentImplementation | O::ImplementationCreateDocumentType
        | O::ImplementationCreateDocument | O::ImplementationCreateHTMLDocument
        | O::ImplementationHasFeature | O::CreateElement | O::CreateElementNS
        | O::CreateProcessingInstruction | O::DocumentAdoptNode | O::DocumentImportNode)
}

fn invalid_character<H: BindingHost>(host: &mut H) -> H::Error {
    match host.with_dom::<()>(|_| Err(DomError::InvalidCharacterError)) {
        Err(error) => error,
        Ok(()) => unreachable!(),
    }
}
fn string<H: BindingHost>(host: &mut H, value: &H::Value) -> Result<String, H::Error> {
    let units = host.to_dom_string(value)?;
    String::from_utf16(&units).map_err(|_| invalid_character(host))
}
fn argument<H: BindingHost>(host: &mut H, args: &[H::Value], index: usize) -> Result<String, H::Error> {
    let value = args.get(index).ok_or_else(|| host.type_error("Not enough arguments"))?;
    string(host, value)
}
fn document<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    let node = host.node_brand(receiver).ok_or_else(|| host.type_error("Expected a Document"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) {
        Ok(())
    } else { Err(DomError::TypeError) })?;
    Ok(node)
}
fn implementation_document<H: BindingHost>(host: &mut H, receiver: &H::Value) -> Result<NodeIdx, H::Error> {
    let key = host.platform_object(receiver, InterfaceBrand::DOM_IMPLEMENTATION)
        .ok_or_else(|| host.type_error("Expected a DOMImplementation"))?;
    let node = host.node_from_platform_key(key)
        .ok_or_else(|| host.type_error("Invalid DOMImplementation"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) {
        Ok(())
    } else { Err(DomError::TypeError) })?;
    Ok(node)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if op == O::DocumentImplementation {
        let node = document(host, &receiver)?;
        return host.wrap_platform_object(host.node_platform_key(node), InterfaceBrand::DOM_IMPLEMENTATION);
    }
    if op == O::ImplementationHasFeature {
        implementation_document(host, &receiver)?;
        return Ok(host.boolean(true));
    }
    if matches!(op, O::CreateElement | O::CreateElementNS) {
        let doc = document(host, &receiver)?;
        let html_document = host.is_html_document(doc);
        let (uri, name) = if op == O::CreateElement {
            let mut name = argument(host, args, 0)?;
            if html_document { name.make_ascii_lowercase(); }
            (html_document.then(|| "http://www.w3.org/1999/xhtml".to_owned()), name)
        } else {
            let uri_value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let uri = if host.is_nullish(uri_value) { None } else { Some(string(host, uri_value)?) };
            (uri, argument(host, args, 1)?)
        };
        let options_index = if op == O::CreateElement { 1 } else { 2 };
        let is_name = {
            if let Some(options) = args.get(options_index) {
                if host.is_nullish(options) { None } else {
                    let value = host.get_property(options, "is")?;
                    if host.is_undefined(&value) { None }
                    else { Some(String::from_utf16_lossy(&host.to_dom_string(&value)?)) }
                }
            } else { None }
        };
        let node = host.with_dom(|dom| {
            let node = if op == O::CreateElement {
                dom.create_element_local_in(doc, uri.as_deref(), &name)?
            } else {
                dom.create_element_ns_in(doc, uri.as_deref(), &name)?
            };
            if let Some(is_name) = &is_name { dom.set_attribute(node, "is", is_name)?; }
            Ok(node)
        })?;
        return host.wrap_node(node);
    }
    if op == O::CreateProcessingInstruction {
        let doc = document(host, &receiver)?;
        let target = argument(host, args, 0)?;
        let data_arg = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
        let data = host.to_dom_string(data_arg)?;
        let node = host.with_dom(|dom| dom.create_processing_instruction_in(doc, &target, &data))?;
        return host.wrap_node(node);
    }
    if matches!(op, O::DocumentAdoptNode | O::DocumentImportNode) {
        let doc = document(host, &receiver)?;
        let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let node = host.node_brand(value).ok_or_else(|| host.type_error("Expected a Node"))?;
        if op == O::DocumentAdoptNode {
            let adopted = host.with_dom(|dom| dom.adopt(node, doc))?;
            return host.wrap_node(adopted);
        }
        let deep = if let Some(value) = args.get(1).filter(|value| !host.is_undefined(value)) {
            if host.same_value(value, &host.boolean(true)) { true }
            else if host.same_value(value, &host.boolean(false)) { false }
            else if host.is_nullish(value) { true }
            else if host.string_units(value).is_some() { host.to_boolean(value) }
            else {
                let self_only = host.get_property(value, "selfOnly")?;
                let registry = host.get_property(value, "customElementRegistry")?;
                if !host.is_nullish(&registry) {
                    return host.with_dom::<H::Value>(|_| Err(DomError::NotSupportedError));
                }
                !host.to_boolean(&self_only)
            }
        } else { false };
        let imported = host.with_dom(|dom| dom.import_node(doc, node, deep))?;
        return host.wrap_node(imported);
    }
    let associated = implementation_document(host, &receiver)?;
    match op {
        O::ImplementationCreateDocumentType => {
            let name = argument(host, args, 0)?;
            let public = if args.len() > 1 { argument(host, args, 1)? } else { String::new() };
            let system = if args.len() > 2 { argument(host, args, 2)? } else { String::new() };
            let node = host.with_dom(|dom| dom.create_document_type_in(associated, &name, &public, &system))?;
            host.wrap_node(node)
        }
        O::ImplementationCreateDocument => {
            let uri_arg = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
            let uri = if host.is_nullish(uri_arg) { None } else { Some(string(host, uri_arg)?) };
            let qualified = argument(host, args, 1)?;
            let doctype = match args.get(2) {
                None => None,
                Some(value) if host.is_nullish(value) => None,
                Some(value) => Some(host.node_brand(value).ok_or_else(|| host.type_error("Expected a DocumentType"))?),
            };
            let new_doc = host.with_dom(|dom| {
                if let Some(node) = doctype {
                    if !matches!(dom.get(node).kind, NodeKind::DocumentType { .. }) { return Err(DomError::TypeError); }
                }
                let doc = dom.create_document();
                if let Some(node) = doctype { dom.pre_insert(node, doc, None)?; }
                if !qualified.is_empty() {
                    let root = dom.create_element_ns_in(doc, uri.as_deref(), &qualified)?;
                    dom.pre_insert(root, doc, None)?;
                }
                Ok(doc)
            })?;
            host.wrap_node(new_doc)
        }
        O::ImplementationCreateHTMLDocument => {
            let title = if let Some(value) = args.first() { Some(host.to_dom_string(value)?) } else { None };
            let new_doc = host.with_dom(|dom| {
                let doc = dom.create_document();
                let doctype = dom.create_document_type_in(doc, "html", "", "")?;
                dom.pre_insert(doctype, doc, None)?;
                let html = dom.create_element_ns_in(doc, Some("http://www.w3.org/1999/xhtml"), "html")?;
                dom.pre_insert(html, doc, None)?;
                let head = dom.create_element_ns_in(doc, Some("http://www.w3.org/1999/xhtml"), "head")?;
                dom.pre_insert(head, html, None)?;
                if let Some(ref title) = title {
                    let title_element = dom.create_element_ns_in(doc, Some("http://www.w3.org/1999/xhtml"), "title")?;
                    dom.pre_insert(title_element, head, None)?;
                    let text = dom.create_node_in(doc, NodeKind::Text { data: String::new() })?;
                    dom.replace_character_data(text, 0, 0, title)?;
                    dom.pre_insert(text, title_element, None)?;
                }
                let body = dom.create_element_ns_in(doc, Some("http://www.w3.org/1999/xhtml"), "body")?;
                dom.pre_insert(body, html, None)?;
                Ok(doc)
            })?;
            host.wrap_node(new_doc)
        }
        _ => unreachable!(),
    }
}
