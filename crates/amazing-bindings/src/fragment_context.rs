//! Live DOM snapshots consumed only after releasing the host arena borrow.
use crate::host::{FragmentAttribute, FragmentContext, XmlFragmentContext};
use amazing_dom::{Dom, DomError, NodeIdx};
use std::collections::BTreeSet;

const HTML: &str = "http://www.w3.org/1999/xhtml";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

pub(crate) fn html(dom: &Dom, element: Option<NodeIdx>, document: NodeIdx) -> Result<FragmentContext, DomError> {
    let (local_name, namespace_uri, attributes, has_form_ancestor) = if let Some(element) = element {
        let attributes = dom.attributes_expanded(element).unwrap_or_default().into_iter().map(|attr| {
            FragmentAttribute {
                namespace_uri: attr.namespace_uri.map(str::to_owned),
                prefix: attr.prefix.map(str::to_owned), local_name: attr.local_name.into(),
                value: dom.attribute_ns_utf16(element, attr.namespace_uri, attr.local_name)
                    .unwrap_or_else(|| attr.value.encode_utf16().collect()),
            }
        }).collect();
        let form = dom.inclusive_ancestors(element).into_iter().any(|ancestor|
            dom.namespace_uri(ancestor) == Some(HTML) && dom.local_name(ancestor) == Some("form"));
        (dom.local_name(element).ok_or(DomError::TypeError)?.into(),
            dom.namespace_uri(element).map(str::to_owned), attributes, form)
    } else {
        // The parsing APIs prescribe an unattached HTML body context here.
        ("body".into(), Some(HTML.into()), Vec::new(), false)
    };
    Ok(FragmentContext {
        local_name, namespace_uri, attributes, has_form_ancestor,
        scripting_enabled: dom.document_scripting_enabled(document)?,
        quirks_mode: dom.document_quirks_mode(document)?,
    })
}

pub(crate) fn xml(dom: &Dom, element: Option<NodeIdx>) -> Result<XmlFragmentContext, DomError> {
    let Some(element) = element else {
        return Ok(XmlFragmentContext {
            namespace_uri: Some(HTML.into()), prefix: None,
            namespaces: vec![(None, HTML.into()), (Some("xml".into()), XML.into())],
        });
    };
    let mut prefixes = BTreeSet::from([None, Some("xml".into())]);
    let mut declared = BTreeSet::new();
    for ancestor in dom.inclusive_ancestors(element) {
        if let Some(prefix) = dom.prefix(ancestor) { prefixes.insert(Some(prefix.into())); }
        for attr in dom.attributes_expanded(ancestor).unwrap_or_default() {
            if attr.namespace_uri != Some(XMLNS) { continue; }
            let prefix = if attr.prefix == Some("xmlns") { Some(attr.local_name.into()) }
                else if attr.prefix.is_none() && attr.local_name == "xmlns" { None }
                else { continue };
            prefixes.insert(prefix.clone()); declared.insert(prefix);
        }
    }
    let mut namespaces = Vec::new();
    for prefix in prefixes {
        match dom.lookup_namespace_uri(element, prefix.as_deref())? {
            Some(uri) => namespaces.push((prefix, uri)),
            None if prefix.is_none() || declared.contains(&prefix) => namespaces.push((prefix, String::new())),
            None => (),
        }
    }
    Ok(XmlFragmentContext {
        namespace_uri: dom.namespace_uri(element).map(str::to_owned),
        prefix: dom.prefix(element).map(str::to_owned), namespaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::document_surface::QuirksMode;

    #[test]
    fn prefixed_xml_context_preserves_separate_default_and_undeclaration() {
        let mut dom = Dom::new();
        let parent = dom.create_element_ns_in(NodeIdx(0), Some("urn:parent"), "p:parent").unwrap();
        dom.set_attribute_qualified_ns(parent, Some(XMLNS), "xmlns", "urn:default").unwrap();
        dom.set_attribute_qualified_ns(parent, Some(XMLNS), "xmlns:x", "urn:outer").unwrap();
        let child = dom.create_element_ns(parent, Some("urn:child"), "c:child").unwrap();
        dom.set_attribute_qualified_ns(child, Some(XMLNS), "xmlns:x", "").unwrap();
        let context = xml(&dom, Some(child)).unwrap();
        assert_eq!(context.namespace_uri.as_deref(), Some("urn:child"));
        assert_eq!(context.prefix.as_deref(), Some("c"));
        assert!(context.namespaces.contains(&(None, "urn:default".into())));
        assert!(context.namespaces.contains(&(Some("c".into()), "urn:child".into())));
        assert!(context.namespaces.contains(&(Some("x".into()), String::new())));
        assert!(context.namespaces.contains(&(Some("xml".into()), XML.into())));
        dom.set_attribute_qualified_ns(child, Some(XMLNS), "xmlns", "").unwrap();
        assert!(xml(&dom, Some(child)).unwrap().namespaces.contains(&(None, String::new())));
    }

    #[test]
    fn html_context_captures_namespace_attribute_units_form_and_document_mode() {
        let mut dom = Dom::new();
        let form = dom.create_element(NodeIdx(0), "form");
        let context = dom.create_element_ns(form, Some("urn:context"), "C:MixedCase").unwrap();
        dom.set_attribute_qualified_ns(context, Some("urn:attribute"), "a:value", "display").unwrap();
        let attr = dom.get_attribute_node(context, Some("urn:attribute"), "value").unwrap();
        dom.set_attr_value(attr, vec![0xd800, 65, 0xdc00]).unwrap();
        dom.set_document_scripting_enabled(NodeIdx(0), true).unwrap();
        dom.set_document_quirks_mode(NodeIdx(0), QuirksMode::Quirks).unwrap();
        let snapshot = html(&dom, Some(context), NodeIdx(0)).unwrap();
        assert_eq!(snapshot.local_name, "MixedCase");
        assert_eq!(snapshot.namespace_uri.as_deref(), Some("urn:context"));
        assert_eq!(snapshot.attributes[0].value, vec![0xd800, 65, 0xdc00]);
        assert_eq!(snapshot.attributes[0].prefix.as_deref(), Some("a"));
        assert!(snapshot.has_form_ancestor && snapshot.scripting_enabled);
        assert_eq!(snapshot.quirks_mode, QuirksMode::Quirks);
    }
}
