use crate::host::{BindingHost, InterfaceBrand, Operation as O};
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind};

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
    let node = host.node_from_object_key(key)
        .ok_or_else(|| host.type_error("Invalid DOMImplementation"))?;
    host.with_dom(|dom| if matches!(dom.get(node).kind, NodeKind::Document) {
        Ok(())
    } else { Err(DomError::TypeError) })?;
    Ok(node)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H, op: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    host.current_document_for_nodes()?;
    if op == O::DocumentImplementation {
        let node = document(host, &receiver)?;
        return host.wrap_platform_object(host.node_object_key(node), InterfaceBrand::DOM_IMPLEMENTATION);
    }
    if op == O::ImplementationHasFeature {
        implementation_document(host, &receiver)?;
        return Ok(host.boolean(true));
    }
    if matches!(op, O::CreateElement | O::CreateElementNS) {
        let doc = document(host, &receiver)?;
        let document_key = host.node_object_key(doc);
        let capture = host.capture_dom(document_key.arena)?;
        let html_document = host.is_html_document(doc);
        let xhtml_document = host.with_dom(|dom| Ok(dom.document_metadata(doc)?
            .is_some_and(|metadata| metadata.content_type == "application/xhtml+xml")))?;
        let (uri, name) = if op == O::CreateElement {
            let mut name = argument(host, args, 0)?;
            if html_document { name.make_ascii_lowercase(); }
            ((html_document || xhtml_document).then(|| "http://www.w3.org/1999/xhtml".to_owned()), name)
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
                    else { Some(host.to_dom_string(&value)?) }
                }
            } else { None }
        };
        let (prefix, local_name) = host.with_captured_dom(&capture, |_| Dom::validate_element_creation_name(
            uri.as_deref(), &name, op == O::CreateElementNS))?;
        let plan = host.with_captured_dom(&capture, |dom| dom.prepare_dom_custom_element_creation(
            amazing_dom::render_mutation::NodeGeneration { node: doc, generation: document_key.generation },
            uri.as_deref(), prefix.as_deref(), &local_name, is_name.as_deref()))?;
        return crate::custom_element_bindings::create_element_from_plan(host, &capture, document_key.arena, plan);
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
            if args.len() < 2 { return Err(host.type_error("Not enough arguments")); }
            let uri = if host.is_nullish(uri_arg) { None } else { Some(string(host, uri_arg)?) };
            let qualified = match args.get(1) {
                Some(value) if host.is_nullish(value) && !host.is_undefined(value) => String::new(),
                Some(value) => string(host, value)?,
                None => unreachable!("qualified name is required"),
            };
            let doctype = match args.get(2) {
                None => None,
                Some(value) if host.is_nullish(value) => None,
                Some(value) => Some(host.node_brand(value).ok_or_else(|| host.type_error("Expected a DocumentType"))?),
            };
            let origin = host.capture_document_origin(associated)?;
            let new_doc = host.with_dom(|dom| {
                if let Some(node) = doctype {
                    if !matches!(dom.get(node).kind, NodeKind::DocumentType { .. }) { return Err(DomError::TypeError); }
                }
                let doc = dom.create_document_in_realm_of(associated)?;
                let content_type = match uri.as_deref() {
                    Some("http://www.w3.org/1999/xhtml") => "application/xhtml+xml",
                    Some("http://www.w3.org/2000/svg") => "image/svg+xml",
                    _ => "application/xml",
                };
                dom.set_document_metadata(doc, amazing_dom::document_surface::DocumentMetadata {
                    url: "about:blank".into(), content_type: content_type.into(),
                    character_set: "UTF-8".into(),
                })?;
                dom.set_document_ready_state(doc, amazing_dom::document_surface::ReadyState::Complete)?;
                if let Some(node) = doctype { dom.pre_insert(node, doc, None)?; }
                if !qualified.is_empty() {
                    let root = dom.create_element_ns_in(doc, uri.as_deref(), &qualified)?;
                    dom.pre_insert(root, doc, None)?;
                }
                Ok(doc)
            })?;
            host.document_created(associated, new_doc, origin)?;
            host.wrap_node(new_doc)
        }
        O::ImplementationCreateHTMLDocument => {
            let title = match args.first() {
                Some(value) if !host.is_undefined(value) => Some(host.to_dom_string(value)?),
                _ => None,
            };
            let origin = host.capture_document_origin(associated)?;
            let new_doc = host.with_dom(|dom| {
                let doc = dom.create_document_in_realm_of(associated)?;
                dom.set_document_metadata(doc, amazing_dom::document_surface::DocumentMetadata {
                    url: "about:blank".into(), content_type: "text/html".into(),
                    character_set: "UTF-8".into(),
                })?;
                dom.set_document_ready_state(doc, amazing_dom::document_surface::ReadyState::Complete)?;
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
            host.document_created(associated, new_doc, origin)?;
            host.wrap_node(new_doc)
        }
        _ => unreachable!(),
    }
}
