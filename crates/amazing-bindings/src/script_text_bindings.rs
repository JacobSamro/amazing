//! HTMLScriptElement.text: native child text and string replace-all.
use crate::host::{BindingHost, Operation as O};
use amazing_dom::{DomError, Namespace, NodeKind};

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::ScriptText | O::ScriptSetText)
}

pub(crate) fn invoke<H: BindingHost>(
    host: &mut H,
    op: O,
    receiver: H::Value,
    args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let node = host
        .node_brand(&receiver)
        .ok_or_else(|| host.type_error("Expected an HTMLScriptElement"))?;
    let document = host.with_dom(|dom| {
        if dom.node_generation(node).is_none()
            || dom.namespace(node) != Some(Namespace::Html)
            || dom.local_name(node) != Some("script")
        {
            return Err(DomError::TypeError);
        }
        dom.owner_document(node).ok_or(DomError::InvalidNode)
    })?;
    // Capture actual arena/node/document capabilities before author conversion.
    // Never resolve an allocation document from the possibly changed current realm.
    let node_key = host.node_object_key(node);
    let document_key = host.node_object_key(document);
    if op == O::ScriptText {
        let units = host.with_dom(|dom| {
            Ok(dom
                .children(node)
                .filter(|&child| {
                    matches!(
                        dom.get(child).kind,
                        NodeKind::Text { .. } | NodeKind::CDATASection { .. }
                    )
                })
                .flat_map(|child| dom.character_data_utf16(child).unwrap_or_default())
                .collect::<Vec<_>>())
        })?;
        return host.string(&units);
    }
    let value = args.first().cloned().unwrap_or_else(|| host.undefined());
    let units = host.to_dom_string(&value)?;
    if host.node_from_object_key(node_key) != Some(node)
        || host.node_from_object_key(document_key) != Some(document)
    {
        return Err(host.type_error("Script text receiver or allocation Document is stale"));
    }
    host.with_dom(|dom| {
        let text = if units.is_empty() {
            None
        } else {
            let text = dom.create_node_in(
                document,
                NodeKind::Text {
                    data: String::new(),
                },
            )?;
            dom.replace_character_data(text, 0, 0, &units)?;
            Some(text)
        };
        dom.replace_all(node, text)
    })?;
    Ok(host.undefined())
}
