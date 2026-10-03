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
        // Static `Document.parseHTMLUnsafe(html, options?)` (HTML spec
        // "dynamic markup insertion"): a new inert `text/html` Document
        // parsed with declarative shadow roots enabled. Every full-document
        // parse in this engine already allows them (`TreeBuilder::new`'s
        // default, unconditional -- unlike the fragment-parsing entry
        // points `setHTMLUnsafe` needs, a full-document parse has real
        // ancestor elements genuinely on the open-elements stack from the
        // moment `<html>`/`<body>` open, so a top-level `<template
        // shadowrootmode>` under `<body>` already attaches correctly), so
        // this needs no new parser support -- just routing through the
        // same `create_inert_document` primitive `DOMParser.
        // parseFromString('text/html', ...)` already uses below. `options`
        // (a sanitizer config) is unused: this engine has no sanitizer, so
        // "unsafe" (skip sanitizing) and the sanitized static `parseHTML`
        // this engine doesn't yet implement would currently behave
        // identically anyway (mirrors `ElementSetHTMLUnsafe`'s own scope
        // note in `host.rs`).
        let html = args.first().ok_or_else(|| host.type_error("Not enough arguments"))?;
        let html = String::from_utf16_lossy(&host.to_dom_string(html)?);
        let relevant = host.current_document_for_nodes()?;
        let url = host.document_field(relevant, DocumentField::URL)?;
        let url = String::from_utf16_lossy(&url);
        let document = match host.create_inert_document(html.as_bytes(), "text/html", &url, Some("UTF-8")) {
            Ok(document) => document,
            Err(error) => return host.with_dom::<H::Value>(|_| Err(error)),
        };
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
