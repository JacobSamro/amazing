//! Range contextual fragment parsing with the live start-node context.
use crate::host::{BindingHost, FragmentParser};
use amazing_dom::{range::Range, NodeKind};

const HTML: &str = "http://www.w3.org/1999/xhtml";

pub(crate) fn create<H: BindingHost + FragmentParser>(
    host: &mut H, range: Range, args: &[H::Value],
) -> Result<H::Value, H::Error> {
    let value = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    // String conversion may move the live Range: capture its context afterward.
    let markup = host.to_dom_string(value)?;
    host.current_document_for_nodes()?;
    let (document, element, html_hint) = host.with_dom(|dom| {
        let node = dom.range(range)?.start.node;
        let document = dom.owner_document(node).unwrap_or(node);
        let element = match &dom.get(node).kind {
            NodeKind::Element { .. } => Some(node),
            NodeKind::Text { .. } | NodeKind::CDATASection { .. } | NodeKind::Comment { .. } =>
                dom.get(node).parent.filter(|parent| matches!(dom.get(*parent).kind, NodeKind::Element { .. })),
            _ => None,
        };
        let html_hint = dom.document_metadata(document)?.map(|metadata| metadata.content_type == "text/html");
        Ok((document, element, html_hint))
    })?;
    let html_document = html_hint.unwrap_or_else(|| host.is_html_document(document));
    let element = host.with_dom(|dom| Ok(element.filter(|element| !(html_document
        && dom.namespace_uri(*element) == Some(HTML) && dom.local_name(*element) == Some("html")))))?;
    let parsed = if html_document {
        let context = host.with_dom(|dom| crate::fragment_context::html(dom, element, document))?;
        host.parse_fragment(context, &markup)
    } else {
        let context = host.with_dom(|dom| crate::fragment_context::xml(dom, element))?;
        host.parse_xml_fragment(context, &markup)
    };
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => return host.with_dom(|_| Err(error)),
    };
    let fragment = host.with_dom(|dom| dom.import_fragment(&parsed, document))?;
    host.wrap_node(fragment)
}
