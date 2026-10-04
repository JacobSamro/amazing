//! DOM lookupNamespaceURI and isDefaultNamespace over the live tree.
use crate::{Dom, DomError, NodeIdx, NodeKind};

const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

impl Dom {
    fn namespace_context_element(&self, node: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        self.check_node(node)?;
        Ok(match &self.get(node).kind {
            NodeKind::Document => self.document_element(node)?,
            NodeKind::Element { .. } => Some(node),
            NodeKind::Attr { .. } => self.attr_snapshot(node)?.owner_element,
            NodeKind::DocumentType { .. } | NodeKind::DocumentFragment => None,
            _ => self.get(node).parent.filter(|parent|
                matches!(self.get(*parent).kind, NodeKind::Element { .. })),
        })
    }

    pub fn lookup_namespace_uri(&self, node: NodeIdx, prefix: Option<&str>)
        -> Result<Option<String>, DomError> {
        self.lookup_namespace_uri_utf16(node, prefix)
            .map(|value| value.map(|units| String::from_utf16_lossy(&units)))
    }

    /// Compatibility entry for native callers whose prefix is already UTF-8.
    /// Returned namespace declarations retain their actual DOMString units.
    pub fn lookup_namespace_uri_utf16(&self, node: NodeIdx, prefix: Option<&str>)
        -> Result<Option<Vec<u16>>, DomError> {
        let prefix = prefix.map(|prefix| prefix.encode_utf16().collect::<Vec<_>>());
        self.lookup_namespace_uri_units(node, prefix.as_deref())
    }

    /// DOM locate-a-namespace using exact nullable DOMString input/output.
    pub fn lookup_namespace_uri_units(&self, node: NodeIdx, prefix: Option<&[u16]>)
        -> Result<Option<Vec<u16>>, DomError> {
        let prefix = prefix.filter(|prefix| !prefix.is_empty());
        let mut current = self.namespace_context_element(node)?;
        let mut remaining = self.nodes.len();
        while let Some(element) = current {
            if remaining == 0 { return Err(DomError::InvalidStateError); }
            remaining -= 1;
            if prefix.is_some_and(|prefix| prefix == [120, 109, 108]) {
                return Ok(Some(XML.encode_utf16().collect()));
            }
            if prefix.is_some_and(|prefix| prefix == [120, 109, 108, 110, 115]) {
                return Ok(Some(XMLNS.encode_utf16().collect()));
            }
            let own_prefix = self.prefix(element);
            let matches = match (own_prefix, prefix) {
                (None, None) => true,
                (Some(own), Some(prefix)) => own.encode_utf16().eq(prefix.iter().copied()),
                _ => false,
            };
            if matches {
                if let Some(uri) = self.namespace_uri(element) {
                    return Ok(Some(uri.encode_utf16().collect()));
                }
            }
            for attr in self.attributes_expanded(element).unwrap_or_default() {
                if attr.namespace_uri != Some(XMLNS) { continue; }
                let matches = match prefix {
                    None => attr.prefix.is_none() && attr.local_name == "xmlns",
                    Some(prefix) => attr.prefix == Some("xmlns")
                        && attr.local_name.encode_utf16().eq(prefix.iter().copied()),
                };
                if matches {
                    let value = self.attribute_ns_utf16(element, attr.namespace_uri, attr.local_name)
                        .unwrap_or_else(|| attr.value.encode_utf16().collect());
                    return Ok((!value.is_empty()).then_some(value));
                }
            }
            current = self.get(element).parent.filter(|parent|
                matches!(self.get(*parent).kind, NodeKind::Element { .. }));
        }
        Ok(None)
    }

    /// DOM locate-a-namespace-prefix: own non-null prefix, first matching
    /// prefixed declaration in attribute order, then the parent element.
    pub fn lookup_prefix_units(&self, node: NodeIdx, namespace: Option<&[u16]>)
        -> Result<Option<Vec<u16>>, DomError> {
        let mut current = self.namespace_context_element(node)?;
        let Some(namespace) = namespace.filter(|namespace| !namespace.is_empty()) else {
            return Ok(None);
        };
        let mut remaining = self.nodes.len();
        while let Some(element) = current {
            if remaining == 0 { return Err(DomError::InvalidStateError); }
            remaining -= 1;
            if self.namespace_uri(element).is_some_and(|uri|
                uri.encode_utf16().eq(namespace.iter().copied())) {
                if let Some(prefix) = self.prefix(element) {
                    return Ok(Some(prefix.encode_utf16().collect()));
                }
            }
            for attr in self.attributes_expanded(element).unwrap_or_default() {
                if attr.prefix != Some("xmlns") { continue; }
                let value = self.attribute_ns_utf16(element, attr.namespace_uri, attr.local_name)
                    .unwrap_or_else(|| attr.value.encode_utf16().collect());
                if value == namespace { return Ok(Some(attr.local_name.encode_utf16().collect())); }
            }
            current = self.get(element).parent.filter(|parent|
                matches!(self.get(*parent).kind, NodeKind::Element { .. }));
        }
        Ok(None)
    }

    pub fn is_default_namespace_units(&self, node: NodeIdx, namespace: Option<&[u16]>)
        -> Result<bool, DomError> {
        Ok(self.lookup_namespace_uri_units(node, None)?.as_deref()
            == namespace.filter(|namespace| !namespace.is_empty()))
    }

    pub fn is_default_namespace(&self, node: NodeIdx, namespace: Option<&str>)
        -> Result<bool, DomError> {
        let namespace = namespace.map(|namespace| namespace.encode_utf16().collect::<Vec<_>>());
        self.is_default_namespace_units(node, namespace.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wpt_lookup_uses_own_namespace_declarations_and_parent_element() {
        let mut dom = Dom::new();
        let root = dom.create_element_ns(NodeIdx(0), Some("urn:own"), "p:root").unwrap();
        assert_eq!(dom.lookup_namespace_uri(root, Some("p")).unwrap().as_deref(), Some("urn:own"));
        assert_eq!(dom.lookup_namespace_uri(root, Some("xml")).unwrap().as_deref(), Some(XML));
        assert_eq!(dom.lookup_namespace_uri(root, None).unwrap(), None);
        dom.set_attribute_qualified_ns(root, Some(XMLNS), "xmlns", "urn:default").unwrap();
        dom.set_attribute_qualified_ns(root, Some(XMLNS), "xmlns:bar", "urn:bar").unwrap();
        assert_eq!(dom.lookup_namespace_uri(root, None).unwrap().as_deref(), Some("urn:default"));
        assert_eq!(dom.lookup_namespace_uri(root, Some("bar")).unwrap().as_deref(), Some("urn:bar"));
        let child = dom.create_element_ns(root, Some("urn:child"), "child").unwrap();
        let comment = dom.create_node_in(NodeIdx(0), NodeKind::Comment { data: String::new() }).unwrap();
        dom.pre_insert(comment, child, None).unwrap();
        assert_eq!(dom.lookup_namespace_uri(child, None).unwrap().as_deref(), Some("urn:child"));
        assert_eq!(dom.lookup_namespace_uri(comment, Some("bar")).unwrap().as_deref(), Some("urn:bar"));
        assert!(dom.is_default_namespace(child, Some("urn:child")).unwrap());
        assert!(!dom.is_default_namespace(child, Some("urn:default")).unwrap());
        let fragment = dom.create_document_fragment(NodeIdx(0)).unwrap();
        assert_eq!(dom.lookup_namespace_uri(fragment, Some("xml")).unwrap(), None);
        assert!(dom.is_default_namespace(fragment, None).unwrap());
        let empty = dom.create_document();
        assert_eq!(dom.lookup_namespace_uri(empty, Some("xml")).unwrap(), None);
    }
}

#[cfg(test)]
mod fragment_namespace_tests {
    use crate::{Dom, NodeIdx};
    #[test]
    fn namespace_undeclaration_stops_ancestor_lookup() {
        let mut dom = Dom::new();
        let root = dom.create_element_ns_in(NodeIdx(0), Some("urn:root"), "root").unwrap();
        dom.set_attribute_qualified_ns(root, Some("http://www.w3.org/2000/xmlns/"), "xmlns:p", "urn:parent").unwrap();
        let child = dom.create_element_ns(root, None, "child").unwrap();
        dom.set_attribute_qualified_ns(child, Some("http://www.w3.org/2000/xmlns/"), "xmlns", "").unwrap();
        dom.set_attribute_qualified_ns(child, Some("http://www.w3.org/2000/xmlns/"), "xmlns:p", "").unwrap();
        assert_eq!(dom.lookup_namespace_uri(child, None), Ok(None));
        assert_eq!(dom.lookup_namespace_uri(child, Some("p")), Ok(None));
        assert_eq!(dom.lookup_namespace_uri(root, Some("p")), Ok(Some("urn:parent".into())));
        assert_eq!(dom.lookup_namespace_uri(child, Some("xml")), Ok(Some("http://www.w3.org/XML/1998/namespace".into())));
    }
}

#[cfg(test)]
mod complete_lookup_tests {
    use super::*;
    fn units(s: &str) -> Vec<u16> { s.encode_utf16().collect() }
    #[test]
    fn prefix_selection_uses_own_identity_first_then_ordered_declarations_and_ancestors() {
        let mut dom=Dom::new();
        let root=dom.create_element_ns(NodeIdx(0),Some("urn:own"),"p:root").unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns:z","urn:own").unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns:a","urn:other").unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns:b","urn:other").unwrap();
        assert_eq!(dom.lookup_prefix_units(root,Some(&units("urn:own"))).unwrap(),Some(units("p")));
        assert_eq!(dom.lookup_prefix_units(root,Some(&units("urn:other"))).unwrap(),Some(units("a")));
        let child=dom.create_element_ns(root,Some("urn:child"),"q:child").unwrap();
        dom.set_attribute_qualified_ns(child,Some(XMLNS),"xmlns:a","urn:shadow").unwrap();
        // The current DOM prefix algorithm returns the ancestor's first
        // declaration directly; it does not validate it against this child.
        assert_eq!(dom.lookup_prefix_units(child,Some(&units("urn:other"))).unwrap(),Some(units("a")));
        assert_eq!(dom.lookup_prefix_units(child,Some(&units("urn:shadow"))).unwrap(),Some(units("a")));
        dom.remove_attribute_ns(root,Some(XMLNS),"a").unwrap();
        assert_eq!(dom.lookup_prefix_units(child,Some(&units("urn:other"))).unwrap(),Some(units("b")));
    }
    #[test]
    fn exact_units_survive_namespace_declarations_without_replacement_character_aliasing() {
        let mut dom=Dom::new();let root=dom.create_element_ns(NodeIdx(0),None,"r").unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns:p","initial").unwrap();
        let attr=dom.get_attribute_node(root,Some(XMLNS),"p").unwrap();
        dom.set_attr_value(attr,vec![0xd800]).unwrap();
        dom.set_attribute_qualified_ns(root,Some(XMLNS),"xmlns","initial").unwrap();
        let attr=dom.get_attribute_node(root,Some(XMLNS),"xmlns").unwrap();
        dom.set_attr_value(attr,vec![0xdc00]).unwrap();
        assert_eq!(dom.lookup_namespace_uri_units(root,Some(&units("p"))).unwrap(),Some(vec![0xd800]));
        assert_eq!(dom.lookup_prefix_units(root,Some(&[0xd800])).unwrap(),Some(units("p")));
        assert_eq!(dom.lookup_prefix_units(root,Some(&[0xfffd])).unwrap(),None);
        assert!(dom.is_default_namespace_units(root,Some(&[0xdc00])).unwrap());
        assert!(!dom.is_default_namespace_units(root,Some(&[0xfffd])).unwrap());
        assert_eq!(dom.lookup_namespace_uri_units(root,Some(&[0xd800])).unwrap(),None);
    }
    #[test]
    fn native_context_follows_actual_parent_and_attr_owner_without_crossing_fragment_hosts() {
        let mut dom=Dom::new();let root=dom.create_element_ns(NodeIdx(0),Some("urn:r"),"p:r").unwrap();
        let comment=dom.create_node_in(NodeIdx(0),NodeKind::Comment{data:"c".into()}).unwrap();
        dom.pre_insert(comment,root,None).unwrap();
        assert_eq!(dom.lookup_prefix_units(comment,Some(&units("urn:r"))).unwrap(),Some(units("p")));
        let fragment=dom.create_document_fragment(NodeIdx(0)).unwrap();
        dom.pre_insert(comment,fragment,None).unwrap();
        assert_eq!(dom.lookup_prefix_units(comment,Some(&units("urn:r"))).unwrap(),None);
        assert_eq!(dom.lookup_prefix_units(fragment,Some(&units("urn:r"))).unwrap(),None);
        assert_eq!(dom.lookup_prefix_units(NodeIdx(0),Some(&units("urn:r"))).unwrap(),Some(units("p")));
        assert_eq!(dom.lookup_prefix_units(root,None).unwrap(),None);
        assert_eq!(dom.lookup_prefix_units(root,Some(&[])).unwrap(),None);
        assert!(dom.lookup_prefix_units(NodeIdx(u32::MAX),Some(&units("urn:r"))).is_err());
    }
}
