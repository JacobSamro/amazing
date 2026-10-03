//! DOM lookupNamespaceURI and isDefaultNamespace over the live tree.
use crate::{Dom, DomError, NodeIdx, NodeKind};

const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

impl Dom {
    pub fn lookup_namespace_uri(
        &self, node: NodeIdx, prefix: Option<&str>,
    ) -> Result<Option<String>, DomError> {
        self.check_node(node)?;
        let prefix = prefix.filter(|prefix| !prefix.is_empty());
        let start = match &self.get(node).kind {
            NodeKind::Document => self.document_element(node)?,
            NodeKind::Element { .. } => Some(node),
            NodeKind::Attr { .. } => self.attr_snapshot(node)?.owner_element,
            NodeKind::DocumentType { .. } | NodeKind::DocumentFragment => None,
            _ => self.get(node).parent.filter(|parent|
                matches!(self.get(*parent).kind, NodeKind::Element { .. })),
        };
        let mut current = start;
        while let Some(element) = current {
            if prefix == Some("xml") { return Ok(Some(XML.into())); }
            if prefix == Some("xmlns") { return Ok(Some(XMLNS.into())); }
            let namespace = self.namespace_uri(element);
            if namespace.is_some() && self.prefix(element) == prefix {
                return Ok(namespace.map(str::to_owned));
            }
            for attr in self.attributes_expanded(element).unwrap_or_default() {
                if attr.namespace_uri != Some(XMLNS) { continue; }
                let matches = match prefix {
                    None => attr.prefix.is_none() && attr.local_name == "xmlns",
                    Some(prefix) => attr.prefix == Some("xmlns") && attr.local_name == prefix,
                };
                if matches { return Ok((!attr.value.is_empty()).then(|| attr.value.to_owned())); }
            }
            current = self.get(element).parent.filter(|parent|
                matches!(self.get(*parent).kind, NodeKind::Element { .. }));
        }
        Ok(None)
    }

    pub fn is_default_namespace(&self, node: NodeIdx, namespace: Option<&str>)
        -> Result<bool, DomError> {
        Ok(self.lookup_namespace_uri(node, None)?.as_deref()
            == namespace.filter(|namespace| !namespace.is_empty()))
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
