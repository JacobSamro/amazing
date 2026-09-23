use crate::host::{BindingHost, DocumentField, DocumentHost, InterfaceBrand, Operation as O};
use amazing_dom::DomError;

pub(crate) fn handles(op: O) -> bool {
    matches!(op, O::DOMParserConstructor | O::DOMParserParseFromString)
}

pub(crate) fn invoke<H: BindingHost + DocumentHost>(host: &mut H, op: O, receiver: H::Value,
    args: &[H::Value]) -> Result<H::Value, H::Error> {
    if op == O::DOMParserConstructor { return host.dom_parser_new(); }
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
    let url = host.document_field(host.current_document(), DocumentField::URL)?;
    let url = String::from_utf16_lossy(&url);
    let result = host.create_inert_document(source.as_bytes(), &kind, &url, Some("UTF-8"));
    let document = match result {
        Ok(document) => document,
        Err(DomError::SyntaxError) if kind != "text/html" => {
            let error = br#"<parsererror xmlns="http://www.mozilla.org/newlayout/xml/parsererror.xml">XML parsing error</parsererror>"#;
            match host.create_inert_document(error, &kind, &url, Some("UTF-8")) {
                Ok(document) => document,
                Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
            }
        }
        Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
    };
    host.wrap_node(document)
}
