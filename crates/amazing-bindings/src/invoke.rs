use crate::host::{AbortHost, BindingHost, DocumentHost, FragmentParser, Operation as O, SelectorEngine, UrlReflection};
use amazing_dom::{Dom, DomError, Namespace, NodeIdx, NodeKind, NodeOrString};

enum ResultValue {
    Undefined,
    Null,
    Boolean(bool),
    Number(f64),
    String(Vec<u16>),
    Node(NodeIdx),
    Children(NodeIdx),
}
fn node_result(node: Option<NodeIdx>) -> ResultValue {
    node.map_or(ResultValue::Null, ResultValue::Node)
}
fn valid(dom: &Dom, node: NodeIdx) -> Result<(), DomError> {
    if dom.node_generation(node).is_some() {
        Ok(())
    } else {
        Err(DomError::InvalidNode)
    }
}
fn receiver_valid(dom: &Dom, node: NodeIdx, op: O) -> Result<(), DomError> {
    valid(dom, node)?;
    let kind = &dom.get(node).kind;
    let good = match op {
        O::CreateText | O::CreateCDATASection | O::CreateComment | O::CreateFragment => matches!(kind, NodeKind::Document),
        O::Append | O::Prepend | O::ReplaceChildren => matches!(
            kind,
            NodeKind::Element { .. } | NodeKind::Document | NodeKind::DocumentFragment
        ),
        O::FirstElementChild | O::LastElementChild | O::ChildElementCount => matches!(
            kind, NodeKind::Element { .. } | NodeKind::Document | NodeKind::DocumentFragment
        ),
        O::PreviousElementSibling | O::NextElementSibling => matches!(
            kind, NodeKind::Element { .. } | NodeKind::Text { .. } | NodeKind::CDATASection { .. }
                | NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
        ),
        O::Before | O::After | O::ReplaceWith | O::Remove => {
            !matches!(kind, NodeKind::Document | NodeKind::DocumentFragment)
        }
        O::Data | O::SetData | O::Length => dom.character_data(node).is_some(),
        O::ElementNamespaceURI | O::ElementPrefix | O::ElementLocalName
        | O::ElementTagName => matches!(kind, NodeKind::Element { .. }),
        O::DocumentTypeName | O::DocumentTypePublicId | O::DocumentTypeSystemId =>
            matches!(kind, NodeKind::DocumentType { .. }),
        O::ProcessingInstructionTarget => matches!(kind, NodeKind::ProcessingInstruction { .. }),
        _ => true,
    };
    if good {
        Ok(())
    } else {
        Err(DomError::TypeError)
    }
}
fn text_content(dom: &Dom, node: NodeIdx) -> Option<Vec<u16>> {
    match dom.get(node).kind {
        NodeKind::Document | NodeKind::DocumentType { .. } => None,
        NodeKind::Element { .. } | NodeKind::DocumentFragment => Some(
            dom.tree_order(node)
                .into_iter()
                .filter(|id| matches!(dom.get(*id).kind, NodeKind::Text { .. } | NodeKind::CDATASection { .. }))
                .flat_map(|id| dom.character_data_utf16(id).unwrap_or_default())
                .collect(),
        ),
        NodeKind::Attr { .. } => dom.attr_snapshot(node).ok().map(|attr| attr.value),
        _ => dom.character_data_utf16(node),
    }
}
fn character_node(
    dom: &mut Dom,
    document: NodeIdx,
    comment: bool,
    data: &[u16],
) -> Result<NodeIdx, DomError> {
    let kind = if comment {
        NodeKind::Comment {
            data: String::new(),
        }
    } else {
        NodeKind::Text {
            data: String::new(),
        }
    };
    let node = dom.create_node_in(document, kind)?;
    if !data.is_empty() {
        dom.replace_character_data(node, 0, 0, data)?;
    }
    Ok(node)
}
fn set_data(dom: &mut Dom, node: NodeIdx, data: &[u16]) -> Result<(), DomError> {
    let length = dom
        .character_data_utf16(node)
        .ok_or(DomError::TypeError)?
        .len();
    dom.replace_character_data(node, 0, length, data)
}
fn required_node<H: BindingHost>(
    host: &mut H,
    value: Option<&H::Value>,
) -> Result<NodeIdx, H::Error> {
    value
        .and_then(|v| host.node_brand(v))
        .ok_or_else(|| host.type_error("Expected a Node"))
}

/// Native key metadata for a legacy platform object. The runtime calls this
/// outside a DOM borrow, then uses the count as the index range 0..count.
pub fn supported_index_count<H: BindingHost>(
    host: &mut H, operation: O, receiver: &H::Value,
) -> Result<u32, H::Error> {
    match operation {
        O::FormCollectionSupportedIndexCount | O::OptionsSupportedIndexCount | O::RadioNodeListSupportedIndexCount =>
            crate::form_bindings::supported_index_count(host, operation, receiver),
        O::HTMLCollectionSupportedIndexCount | O::NodeListSupportedIndexCount =>
            crate::collections::supported_index_count(host, operation, receiver),
        O::DatasetIndexCount => crate::dataset_bindings::supported_index_count(host, receiver),
        O::TokenListSupportedIndexCount => crate::tokenlist::supported_index_count(host, receiver),
        O::NamedNodeMapSupportedIndexCount => crate::attr_bindings::supported_index_count(host, receiver),
        _ => Err(host.type_error("Expected a supported index count hook")),
    }
}

/// Native UTF-16 names in WebIDL supported-property-name order.
pub fn supported_names<H: BindingHost>(
    host: &mut H, operation: O, receiver: &H::Value,
) -> Result<Vec<Vec<u16>>, H::Error> {
    if operation == O::DatasetSupportedNames {
        crate::dataset_bindings::supported_names(host, receiver)
    } else if operation == O::NamedNodeMapSupportedNames {
        crate::attr_bindings::supported_names(host, receiver)
    } else if crate::form_bindings::handles_supported_names(operation) {
        crate::form_bindings::supported_names(host, operation, receiver)
    } else {
        crate::collections::supported_names(host, operation, receiver)
    }
}
fn nullable_node<H: BindingHost>(
    host: &mut H,
    value: &H::Value,
) -> Result<Option<NodeIdx>, H::Error> {
    if host.is_nullish(value) {
        Ok(None)
    } else {
        required_node(host, Some(value)).map(Some)
    }
}

/// Invoke one installed synchronous DOM operation. WebIDL coercions run before
/// arena mutation, in argument order, and may re-enter the runtime or throw.
/// This entry point never drains a private job queue or drives an event loop.
pub fn invoke<H: BindingHost + FragmentParser + SelectorEngine + AbortHost + DocumentHost + UrlReflection>(
    host: &mut H,
    operation: O,
    receiver: H::Value,
    args: &[H::Value],
) -> Result<H::Value, H::Error> {
    host.with_dom(|dom| { dom.custom_element_push_reaction_frame(); Ok(()) })?;
    let result = invoke_inner(host, operation, receiver, args);
    let created = if matches!(operation, O::CreateElement | O::CreateElementNS) {
        result.as_ref().ok().and_then(|value| host.node_brand(value))
    } else { None };
    if let Some(node) = created {
        host.with_dom(|dom| { dom.queue_custom_element_upgrade(node); Ok(()) })?;
    }
    host.with_dom(|dom| { dom.custom_element_pop_reaction_frame(); Ok(()) })?;
    let delivered = crate::custom_element_bindings::deliver_custom_element_reactions_for_created(host, created);
    match result { Err(error) => Err(error), Ok(value) => { delivered?; Ok(value) } }
}

fn invoke_inner<H: BindingHost + FragmentParser + SelectorEngine + AbortHost + DocumentHost + UrlReflection>(
    host: &mut H, operation: O, receiver: H::Value, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    if crate::form_reflection::handles(operation) {
        return crate::form_reflection::invoke(host, operation, receiver, args);
    }
    if crate::web_bindings::handles(operation) {
        return crate::web_bindings::invoke(host, operation, receiver, args);
    }
    if crate::location_bindings::handles(operation) {
        return crate::location_bindings::invoke(host, operation, receiver, args);
    }
    if crate::parser_bindings::handles(operation) {
        return crate::parser_bindings::invoke(host, operation, receiver, args);
    }
    if crate::character_data_bindings::handles(operation) {
        return crate::character_data_bindings::invoke(host, operation, receiver, args);
    }
    if crate::custom_element_bindings::handles(operation) {
        return crate::custom_element_bindings::invoke(host, operation, receiver, args);
    }
    if crate::form_bindings::handles(operation) {
        return crate::form_bindings::invoke(host, operation, receiver, args);
    }
    if crate::shadow_bindings::handles(operation) {
        return crate::shadow_bindings::invoke(host, operation, receiver, args);
    }
    if crate::abort_bindings::handles(operation) {
        return crate::abort_bindings::invoke(host, operation, receiver, args);
    }
    if crate::legacy_document_bindings::handles(operation) {
        return crate::legacy_document_bindings::invoke(host, operation, receiver, args);
    }
    if crate::document_bindings::handles(operation) {
        return crate::document_bindings::invoke(host, operation, receiver, args);
    }
    if crate::element_reflection::handles(operation) {
        return crate::element_reflection::invoke(host, operation, receiver, args);
    }
    if crate::image_bindings::handles(operation) {
        return crate::image_bindings::invoke(host, operation, receiver);
    }
    if crate::markup_bindings::handles(operation) {
        return crate::markup_bindings::invoke(host, operation, receiver, args);
    }
    if crate::range_bindings::handles(operation) {
        return crate::range_bindings::invoke(host, operation, receiver, args);
    }
    if crate::selection_bindings::handles(operation) {
        return crate::selection_bindings::invoke(host, operation, receiver, args);
    }
    if crate::factories::handles(operation) {
        return crate::factories::invoke(host, operation, receiver, args);
    }
    if crate::collections::handles(operation) {
        return crate::collections::invoke(host, operation, receiver, args);
    }
    if crate::traversal_bindings::handles(operation) {
        return crate::traversal_bindings::invoke(host, operation, receiver, args);
    }
    if crate::event_bindings::handles(operation) {
        return crate::event_bindings::invoke(host, operation, receiver, args);
    }
    if crate::focus_bindings::handles(operation) {
        return crate::focus_bindings::invoke(host, operation, receiver, args);
    }
    if crate::mutation_bindings::handles(operation) {
        return crate::mutation_bindings::invoke(host, operation, receiver, args);
    }
    if crate::dom_exception::handles(operation) {
        return crate::dom_exception::invoke(host, operation, receiver, args);
    }
    if crate::dataset_bindings::handles(operation) {
        return crate::dataset_bindings::invoke(host, operation, receiver, args);
    }
    if crate::attr_bindings::handles(operation) {
        return crate::attr_bindings::invoke(host, operation, receiver, args);
    }
    if crate::tokenlist::handles(operation) {
        return crate::tokenlist::invoke(host, operation, receiver, args);
    }
    if crate::html_reflection::handles(operation) {
        return crate::html_reflection::invoke(host, operation, receiver, args);
    }
    let constructor = matches!(
        operation,
        O::TextConstructor
            | O::CommentConstructor
            | O::FragmentConstructor
            | O::DocumentConstructor
    );
    let target = if constructor {
        host.current_document()
    } else {
        required_node(host, Some(&receiver))?
    };
    if !constructor {
        host.with_dom(|dom| receiver_valid(dom, target, operation))?;
    }
    let required = match operation {
        O::CreateText | O::CreateCDATASection | O::CreateComment | O::AppendChild | O::RemoveChild
        | O::Contains | O::NodeLookupNamespaceURI | O::NodeIsDefaultNamespace | O::NodeIsSameNode
        | O::NodeIsEqualNode | O::NodeCompareDocumentPosition => 1,
        O::InsertBefore | O::ReplaceChild | O::MoveBefore => 2,
        _ => 0,
    };
    if args.len() < required {
        return Err(host.type_error("Not enough arguments"));
    }
    let mut values = Vec::new();
    if matches!(
        operation,
        O::Append | O::Prepend | O::ReplaceChildren | O::Before | O::After | O::ReplaceWith
    ) {
        for value in args {
            values.push(if let Some(node) = host.node_brand(value) {
                NodeOrString::Node(node)
            } else {
                NodeOrString::Utf16(host.to_dom_string(value)?)
            });
        }
    }
    let mut data = Vec::new();
    if matches!(
        operation,
        O::TextConstructor
            | O::CommentConstructor
            | O::CreateText
            | O::CreateCDATASection
            | O::CreateComment
            | O::SetData
            | O::SetNodeValue
            | O::SetTextContent
    ) {
        let value = args.first().cloned().unwrap_or_else(|| host.undefined());
        let default_empty = matches!(operation, O::TextConstructor | O::CommentConstructor)
            && host.is_undefined(&value);
        let nullable = (matches!(operation, O::SetNodeValue | O::SetTextContent)
            && host.is_nullish(&value))
            || (operation == O::SetData && host.is_nullish(&value) && !host.is_undefined(&value));
        if !default_empty && !nullable {
            data = host.to_dom_string(&value)?;
        }
    }
    let argument_node = if matches!(
        operation,
        O::AppendChild | O::InsertBefore | O::ReplaceChild | O::RemoveChild | O::MoveBefore
            | O::NodeCompareDocumentPosition
    ) {
        Some(required_node(host, args.first())?)
    } else if operation == O::Contains {
        nullable_node(host, &args[0])?
    } else {
        None
    };
    let reference = if matches!(operation, O::InsertBefore | O::MoveBefore) {
        nullable_node(host, &args[1])?
    } else if operation == O::ReplaceChild {
        Some(required_node(host, args.get(1))?)
    } else {
        None
    };
    let deep = operation == O::CloneNode && args.first().is_some_and(|v| host.to_boolean(v));
    let namespace_argument = if matches!(operation, O::NodeLookupNamespaceURI | O::NodeIsDefaultNamespace) {
        if host.is_nullish(&args[0]) { None } else {
            let units = host.to_dom_string(&args[0])?;
            Some(String::from_utf16(&units).map_err(|_| host.type_error("Invalid namespace string"))?)
        }
    } else { None };
    let same_node = if matches!(operation, O::NodeIsSameNode | O::NodeIsEqualNode) {
        if host.is_nullish(&args[0]) { None } else { Some(required_node(host, args.first())?) }
    } else { None };
    let html = if matches!(operation, O::NodeName | O::ElementTagName | O::CreateCDATASection) {
        let document = host.with_dom(|dom| Ok(dom.owner_document(target).unwrap_or(target)))?;
        host.is_html_document(document)
    } else {
        false
    };
    let result = host.with_dom(|dom| {
        use ResultValue as R;
        let result = match operation {
            O::DocumentConstructor => {
                let document = dom.create_document();
                dom.set_document_metadata(document, amazing_dom::document_surface::DocumentMetadata {
                    url: "about:blank".into(), content_type: "application/xml".into(),
                    character_set: "UTF-8".into(),
                })?;
                dom.set_document_ready_state(document, amazing_dom::document_surface::ReadyState::Complete)?;
                R::Node(document)
            }
            O::TextConstructor | O::CommentConstructor | O::CreateText | O::CreateComment => {
                R::Node(character_node(
                    dom,
                    target,
                    matches!(operation, O::CommentConstructor | O::CreateComment),
                    &data,
                )?)
            }
            O::CreateCDATASection => {
                if html { return Err(DomError::NotSupportedError); }
                let node = dom.create_node_in(target, NodeKind::CDATASection { data: String::new() })?;
                dom.replace_character_data(node, 0, 0, &data)?;
                R::Node(node)
            }
            O::FragmentConstructor | O::CreateFragment => {
                R::Node(dom.create_document_fragment(target)?)
            }
            O::Append => {
                dom.append_nodes(target, &values)?;
                R::Undefined
            }
            O::Prepend => {
                dom.prepend_nodes(target, &values)?;
                R::Undefined
            }
            O::ReplaceChildren => {
                dom.replace_children(target, &values)?;
                R::Undefined
            }
            O::Before => {
                dom.before_nodes(target, &values)?;
                R::Undefined
            }
            O::After => {
                dom.after_nodes(target, &values)?;
                R::Undefined
            }
            O::ReplaceWith => {
                dom.replace_with(target, &values)?;
                R::Undefined
            }
            O::Remove => {
                dom.remove(target)?;
                R::Undefined
            }
            O::AppendChild | O::InsertBefore => {
                R::Node(dom.pre_insert(argument_node.unwrap(), target, reference)?)
            }
            O::MoveBefore => {
                dom.move_before(argument_node.unwrap(), target, reference)?;
                R::Undefined
            }
            O::RemoveChild => R::Node(dom.pre_remove(argument_node.unwrap(), target)?),
            O::ReplaceChild => {
                R::Node(dom.replace(reference.unwrap(), argument_node.unwrap(), target)?)
            }
            O::CloneNode => R::Node(dom.clone_node(target, deep)?),
            O::NodeType => R::Number(match dom.get(target).kind {
                NodeKind::Element { .. } => 1.,
                NodeKind::Attr { .. } => 2.,
                NodeKind::Text { .. } => 3.,
                NodeKind::CDATASection { .. } => 4.,
                NodeKind::ProcessingInstruction { .. } => 7.,
                NodeKind::Comment { .. } => 8.,
                NodeKind::Document => 9.,
                NodeKind::DocumentType { .. } => 10.,
                NodeKind::DocumentFragment => 11.,
            }),
            O::NodeName | O::ElementTagName => {
                let name = match &dom.get(target).kind {
                    NodeKind::Element { namespace, .. } => {
                        let name = dom.qualified_name(target).unwrap();
                        if html && *namespace == Namespace::Html {
                            name.to_ascii_uppercase()
                        } else {
                            name
                        }
                    }
                    NodeKind::Attr { local_name, prefix, .. } => match prefix {
                        Some(prefix) => format!("{prefix}:{local_name}"),
                        None => local_name.clone(),
                    },
                    NodeKind::Text { .. } => "#text".into(),
                    NodeKind::CDATASection { .. } => "#cdata-section".into(),
                    NodeKind::Comment { .. } => "#comment".into(),
                    NodeKind::Document => "#document".into(),
                    NodeKind::DocumentFragment => "#document-fragment".into(),
                    NodeKind::DocumentType { name, .. } => name.clone(),
                    NodeKind::ProcessingInstruction { target, .. } => target.clone(),
                };
                R::String(name.encode_utf16().collect())
            }
            O::ElementNamespaceURI => dom.namespace_uri(target)
                .map_or(R::Null, |uri| R::String(uri.encode_utf16().collect())),
            O::ElementPrefix => dom.prefix(target)
                .map_or(R::Null, |prefix| R::String(prefix.encode_utf16().collect())),
            O::ElementLocalName => R::String(dom.local_name(target).unwrap().encode_utf16().collect()),
            O::DocumentTypeName | O::DocumentTypePublicId | O::DocumentTypeSystemId => {
                let NodeKind::DocumentType { name, public_id, system_id } = &dom.get(target).kind else {
                    unreachable!("validated DocumentType receiver")
                };
                let value = match operation {
                    O::DocumentTypeName => name,
                    O::DocumentTypePublicId => public_id,
                    _ => system_id,
                };
                R::String(value.encode_utf16().collect())
            }
            O::ProcessingInstructionTarget => {
                let NodeKind::ProcessingInstruction { target, .. } = &dom.get(target).kind else {
                    unreachable!("validated ProcessingInstruction receiver")
                };
                R::String(target.encode_utf16().collect())
            }
            O::NodeValue => if matches!(dom.get(target).kind, NodeKind::Attr { .. }) {
                R::String(dom.attr_snapshot(target)?.value)
            } else { dom.character_data_utf16(target).map_or(R::Null, R::String) },
            O::TextContent => text_content(dom, target).map_or(R::Null, R::String),
            O::SetNodeValue => {
                if matches!(dom.get(target).kind, NodeKind::Attr { .. }) {
                    dom.set_attr_value(target, data.clone())?;
                } else if dom.character_data(target).is_some() {
                    set_data(dom, target, &data)?;
                }
                R::Undefined
            }
            O::SetTextContent => {
                match dom.get(target).kind {
                    NodeKind::Document | NodeKind::DocumentType { .. } => (),
                    NodeKind::Element { .. } | NodeKind::DocumentFragment => {
                        let node = if data.is_empty() {
                            None
                        } else {
                            Some(character_node(
                                dom,
                                dom.owner_document(target).unwrap(),
                                false,
                                &data,
                            )?)
                        };
                        dom.replace_all(target, node)?;
                    }
                    NodeKind::Attr { .. } => dom.set_attr_value(target, data.clone())?,
                    _ => set_data(dom, target, &data)?,
                }
                R::Undefined
            }
            O::ParentNode => node_result(dom.get(target).parent),
            O::ParentElement => node_result(
                dom.get(target)
                    .parent
                    .filter(|p| matches!(dom.get(*p).kind, NodeKind::Element { .. })),
            ),
            O::OwnerDocument => node_result(dom.owner_document(target)),
            O::FirstChild => node_result(dom.get(target).first_child),
            O::LastChild => node_result(dom.get(target).last_child),
            O::PreviousSibling => node_result(dom.get(target).prev_sibling),
            O::NextSibling => node_result(dom.get(target).next_sibling),
            O::ChildNodes => R::Children(target),
            O::FirstElementChild => node_result(dom.first_element_child(target)),
            O::LastElementChild => node_result(dom.last_element_child(target)),
            O::ChildElementCount => R::Number(dom.child_element_count(target) as f64),
            O::PreviousElementSibling => node_result(dom.previous_element_sibling(target)),
            O::NextElementSibling => node_result(dom.next_element_sibling(target)),
            O::TemplateContent => R::Node(dom.ensure_template_contents(target)?),
            O::HasChildNodes => R::Boolean(dom.get(target).first_child.is_some()),
            O::IsConnected => R::Boolean(dom.is_connected(target)),
            O::Contains => {
                if let Some(node) = argument_node {
                    valid(dom, node)?;
                }
                R::Boolean(dom.contains(target, argument_node))
            }
            O::NodeLookupNamespaceURI => dom.lookup_namespace_uri(target, namespace_argument.as_deref())?
                .map_or(R::Null, |uri| R::String(uri.encode_utf16().collect())),
            O::NodeIsDefaultNamespace => R::Boolean(dom.is_default_namespace(target, namespace_argument.as_deref())?),
            O::NodeNormalize => { dom.normalize(target)?; R::Undefined },
            O::NodeIsSameNode => R::Boolean(same_node == Some(target)),
            O::NodeIsEqualNode => R::Boolean(same_node.is_some_and(|other| dom.is_equal_node(target, other))),
            O::NodeCompareDocumentPosition => R::Number(dom.compare_document_position(target, argument_node.unwrap()) as f64),
            O::Data => R::String(dom.character_data_utf16(target).unwrap()),
            O::SetData => {
                set_data(dom, target, &data)?;
                R::Undefined
            }
            O::Length => R::Number(dom.character_data_utf16(target).unwrap().len() as f64),
            _ => unreachable!("operation routed before node dispatch"),
        };
        Ok(result)
    })?;
    Ok(match result {
        ResultValue::Undefined => host.undefined(),
        ResultValue::Null => host.null(),
        ResultValue::Boolean(v) => host.boolean(v),
        ResultValue::Number(v) => host.number(v),
        ResultValue::String(v) => host.string(&v)?,
        ResultValue::Node(v) => {
            if operation == O::DocumentConstructor { host.mark_constructed_document(v); }
            host.wrap_node(v)?
        },
        ResultValue::Children(v) => host.child_nodes(v)?,
    })
}
