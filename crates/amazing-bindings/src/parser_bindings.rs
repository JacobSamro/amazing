use crate::host::{BindingHost, DocumentField, DocumentHost, InterfaceBrand, Operation as O};
use amazing_dom::DomError;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::DOMParserConstructor | O::DOMParserParseFromString | O::DocumentParseHTMLUnsafe)
}

pub(crate) fn invoke<H: BindingHost + DocumentHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::DOMParserConstructor {
        host.current_document_for_nodes()?;
        return host.dom_parser_new();
    }
    if op == O::DocumentParseHTMLUnsafe {
        // The actual parser opts in before consuming source. The resulting
        // Document remains inert; scripting and shadow-root policy differ.
        let html = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let html = String::from_utf16_lossy(&host.to_dom_string(html)?);
        let relevant = host.current_document_for_nodes()?;
        let origin = host.capture_document_origin(relevant)?;
        let url = host.document_field(relevant, DocumentField::URL)?;
        let url = String::from_utf16_lossy(&url);
        let document = match host.create_inert_document_with_declarative_shadow_roots(
            html.as_bytes(), "text/html", &url, Some("UTF-8"), true) {
            Ok(document) => document,
            Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
        };
        host.document_created(relevant, document, origin)?;
        return host.wrap_node(document);
    }
    if host.platform_object(&receiver, InterfaceBrand::DOM_PARSER).is_none() {
        return Err(host.type_error("Expected a DOMParser"));
    }
    let source = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
    let source = String::from_utf16_lossy(&host.to_dom_string(source)?);
    let kind = args.get(1).ok_or_else(|| host.type_error("Not enough arguments"))?;
    let kind = String::from_utf16_lossy(&host.to_dom_string(kind)?);
    if !matches!(kind.as_str(), "text/html" | "text/xml" | "application/xml"
        | "application/xhtml+xml" | "image/svg+xml") {
        return Err(host.type_error("Unsupported DOMParser content type"));
    }
    let relevant = host.dom_parser_relevant_document(&receiver)?;
    let url = host.document_field(relevant, DocumentField::URL)?;
    let url = String::from_utf16_lossy(&url);
    let result = host.create_inert_document_for_parser(&receiver, source.as_bytes(), &kind, &url, Some("UTF-8"));
    let document = match result {
        Ok(document) => document,
        Err(DomError::SyntaxError) if kind != "text/html" => {
            let error = br#"<parsererror xmlns="http://www.mozilla.org/newlayout/xml/parsererror.xml">XML parsing error</parsererror>"#;
            match host.create_inert_document_for_parser(&receiver, error, &kind, &url, Some("UTF-8")) {
                Ok(document) => document,
                Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
            }
        }
        Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
    };
    host.wrap_node(document)
}
