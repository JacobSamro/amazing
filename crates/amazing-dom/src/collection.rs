//! Live element collections backed by the DOM arena.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionFilter {
    Children,
    HtmlChildren(Vec<String>),
    TableRows,
    TagName(Vec<u16>),
    TagNameNs(Option<Vec<u16>>, Vec<u16>),
    ClassNames(Vec<Vec<u16>>),
    HtmlTag(Vec<u16>),
    Name(Vec<u16>),
    Links,
    Anchors,
    Empty,
}

#[derive(Debug)]
pub struct LiveCollection {
    root: NodeIdx,
    filter: CollectionFilter,
    generation: u64,
    items: Vec<NodeIdx>,
    rebuilds: u64,
}

impl Dom {
    /// Canonical live token-list identity for one element and content attribute.
    pub fn token_list(&mut self, node: NodeIdx, attribute: &str) -> Result<usize, DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        if let Some(slot) = self.token_lists.iter().position(|entry| entry.0 == node && entry.1 == attribute) {
            return Ok(slot);
        }
        let slot = self.token_lists.len();
        self.token_lists.push((node, attribute.to_owned()));
        Ok(slot)
    }
    pub fn token_list_info(&self, slot: usize) -> Option<(NodeIdx, &str)> {
        self.token_lists.get(slot).map(|entry| (entry.0, entry.1.as_str()))
    }
    /// Reuses the collection attached to an owner and filter. In particular,
    /// repeated `children` access returns the same native identity.
    pub fn collection(&mut self, root: NodeIdx, filter: CollectionFilter) -> Result<usize, DomError> {
        self.check_node(root)?;
        if !matches!(self.get(root).kind, NodeKind::Element { .. } | NodeKind::Document | NodeKind::DocumentFragment) {
            return Err(DomError::TypeError);
        }
        if let Some(index) = self.collections.iter().position(|c| c.root == root && c.filter == filter) {
            return Ok(index);
        }
        let index = self.collections.len();
        self.collections.push(LiveCollection { root, filter, generation: u64::MAX, items: Vec::new(), rebuilds: 0 });
        Ok(index)
    }

    fn refresh_collection(&mut self, index: usize) -> Result<&LiveCollection, DomError> {
        let (root, filter, generation) = {
            let c = self.collections.get(index).ok_or(DomError::InvalidStateError)?;
            (c.root, c.filter.clone(), c.generation)
        };
        if generation != self.mutation_generation {
            let document = self.owner_document(root).unwrap_or(root);
            let html_document = self.document_metadata(document)?
                .map_or(true, |metadata| metadata.content_type == "text/html");
            let nodes: Vec<_> = match filter {
                CollectionFilter::Children | CollectionFilter::HtmlChildren(_) => self.children(root).collect(),
                CollectionFilter::TableRows => self.table_rows(root)?,
                _ => self.tree_order(root).into_iter().skip(1).collect(),
            };
            let items = nodes.into_iter().filter(|node| {
                let NodeKind::Element { name, namespace, .. } = &self.get(*node).kind else { return false };
                match &filter {
                    CollectionFilter::Children | CollectionFilter::TableRows => true,
                    CollectionFilter::HtmlChildren(names) => *namespace == Namespace::Html && names.contains(name),
                    CollectionFilter::TagName(query) => query == &vec![b'*' as u16] || {
                        let actual: Vec<_> = self.qualified_name(*node).unwrap().encode_utf16().collect();
                        if html_document && *namespace == Namespace::Html {
                            actual == query.iter().map(|unit| if (65..=90).contains(unit) { unit + 32 } else { *unit }).collect::<Vec<_>>()
                        } else { &actual == query }
                    },
                    CollectionFilter::TagNameNs(uri, local) => {
                        (local == &vec![b'*' as u16] || name.encode_utf16().eq(local.iter().copied())) &&
                            (uri.as_deref() == Some(&[b'*' as u16][..]) ||
                                self.namespace_uri(*node).map(|uri| uri.encode_utf16().collect::<Vec<_>>()).as_ref() == uri.as_ref())
                    },
                    CollectionFilter::ClassNames(required) => {
                        if required.is_empty() { return false; }
                        let class = self.attribute_utf16(*node, "class").unwrap_or_default();
                        required.iter().all(|token| class.split(|unit| matches!(*unit, 9 | 10 | 12 | 13 | 32))
                            .any(|candidate| candidate == token))
                    },
                    CollectionFilter::HtmlTag(tag) => *namespace == Namespace::Html &&
                        name.encode_utf16().eq(tag.iter().copied()),
                    CollectionFilter::Name(query) => *namespace == Namespace::Html &&
                        self.attribute_utf16(*node, "name").as_deref() == Some(query),
                    CollectionFilter::Empty => false,
                    CollectionFilter::Anchors => *namespace == Namespace::Html && name == "a"
                        && self.get_attribute_ns(*node, None, "name").is_some(),
                    CollectionFilter::Links => *namespace == Namespace::Html &&
                        matches!(name.as_str(), "a" | "area") &&
                        self.get_attribute_ns(*node, None, "href").is_some(),
                }
            }).collect();
            let c = &mut self.collections[index];
            c.items = items;
            c.generation = self.mutation_generation;
            c.rebuilds += 1;
        }
        Ok(&self.collections[index])
    }

    pub fn collection_length(&mut self, index: usize) -> Result<usize, DomError> {
        Ok(self.refresh_collection(index)?.items.len())
    }
    pub fn collection_item(&mut self, index: usize, item: usize) -> Result<Option<NodeIdx>, DomError> {
        Ok(self.refresh_collection(index)?.items.get(item).copied())
    }
    pub fn collection_names(&mut self, index: usize) -> Result<Vec<Vec<u16>>, DomError> {
        let items = self.refresh_collection(index)?.items.clone();
        let mut result = Vec::new();
        for node in items {
            for value in [self.attribute_utf16(node, "id"),
                (self.namespace(node) == Some(Namespace::Html)).then(|| self.attribute_utf16(node, "name")).flatten()] {
                if let Some(value) = value.filter(|v| !v.is_empty()) {
                    if !result.contains(&value) { result.push(value); }
                }
            }
        }
        Ok(result)
    }
    pub fn collection_named_item(&mut self, index: usize, key: &[u16]) -> Result<Option<NodeIdx>, DomError> {
        if key.is_empty() { return Ok(None); }
        let items = self.refresh_collection(index)?.items.clone();
        Ok(items.into_iter().find(|node| {
            self.attribute_utf16(*node, "id").as_deref() == Some(key) ||
                (self.namespace(*node) == Some(Namespace::Html) && self.attribute_utf16(*node, "name").as_deref() == Some(key))
        }))
    }
    pub fn collection_rebuilds(&self, index: usize) -> Option<u64> {
        self.collections.get(index).map(|c| c.rebuilds)
    }
    pub fn attribute_utf16(&self, node: NodeIdx, name: &str) -> Option<Vec<u16>> {
        if let Some(value) = self.nodes.iter().find_map(|candidate| match &candidate.kind {
            NodeKind::Attr { local_name, namespace_uri: None, value, owner_element: Some(owner), .. }
                if *owner == node && local_name == name => Some(value.clone()),
            _ => None,
        }) { return Some(value); }
        self.get_attribute_ns(node, None, name).map(|value| value.encode_utf16().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_document_filters_use_attribute_presence_and_exclude_foreign_namespaces() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let anchor = dom.create_element(root, "a");
        let unnamed = dom.create_element(root, "a");
        let foreign = dom.create_element_ns_in(NodeIdx(0), Some("urn:foreign"), "a").unwrap();
        dom.pre_insert(foreign, root, None).unwrap();
        dom.set_attribute(anchor, "name", "").unwrap();
        dom.set_attribute(foreign, "name", "foreign").unwrap();
        dom.set_attribute(unnamed, "href", "").unwrap();
        let anchors = dom.collection(NodeIdx(0), CollectionFilter::Anchors).unwrap();
        let empty = dom.collection(NodeIdx(0), CollectionFilter::Empty).unwrap();
        assert_eq!(dom.collection_length(anchors).unwrap(), 1);
        assert_eq!(dom.collection_item(anchors, 0).unwrap(), Some(anchor));
        assert_eq!(dom.collection(NodeIdx(0), CollectionFilter::Anchors).unwrap(), anchors);
        dom.set_attribute(unnamed, "name", "second").unwrap();
        assert_eq!(dom.collection_length(anchors).unwrap(), 2);
        dom.remove_attribute(anchor, "name").unwrap();
        assert_eq!(dom.collection_item(anchors, 0).unwrap(), Some(unnamed));
        dom.create_element(root, "applet");
        dom.create_element(root, "object");
        assert_eq!(dom.collection_length(empty).unwrap(), 0);
    }
    #[test]
    fn tag_name_uses_qualified_name_and_html_document_case_rules() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        let upper = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "I").unwrap();
        let prefixed = dom.create_element_ns_in(NodeIdx(0), Some("urn:test"), "te:ST").unwrap();
        dom.pre_insert(upper, root, None).unwrap();
        dom.pre_insert(prefixed, root, None).unwrap();
        let query = |dom: &mut Dom, name: &str| {
            let slot = dom.collection(root, CollectionFilter::TagName(name.encode_utf16().collect())).unwrap();
            (0..dom.collection_length(slot).unwrap()).map(|index| dom.collection_item(slot, index).unwrap().unwrap()).collect::<Vec<_>>()
        };
        assert!(query(&mut dom, "I").is_empty());
        assert_eq!(query(&mut dom, "te:ST"), vec![prefixed]);
        assert!(query(&mut dom, "ST").is_empty());
    }
    #[test]
    fn live_collection_reuses_identity_and_invalidates_only_on_mutation() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        let first = dom.create_element(root, "span");
        let children = dom.collection(root, CollectionFilter::Children).unwrap();
        assert_eq!(children, dom.collection(root, CollectionFilter::Children).unwrap());
        assert_eq!(dom.collection_item(children, 0).unwrap(), Some(first));
        assert_eq!(dom.collection_length(children).unwrap(), 1);
        assert_eq!(dom.collection_rebuilds(children), Some(1));
        let second = dom.create_element(root, "span");
        assert_eq!(dom.collection_item(children, 1).unwrap(), Some(second));
        assert_eq!(dom.collection_rebuilds(children), Some(2));
        dom.detach(first);
        assert_eq!(dom.collection_item(children, 0).unwrap(), Some(second));
    }
    #[test]
    fn filters_and_names_track_tree_and_attributes() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        let outer = dom.create_element(root, "section");
        let inner = dom.create_element(outer, "span");
        dom.set_attribute(inner, "class", "a b").unwrap();
        dom.set_attribute(inner, "id", "target").unwrap();
        dom.set_attribute(outer, "name", "target").unwrap();
        let classes = dom.collection(root, CollectionFilter::ClassNames(vec![vec![b'a' as u16], vec![b'b' as u16]])).unwrap();
        assert_eq!(dom.collection_item(classes, 0).unwrap(), Some(inner));
        assert_eq!(dom.collection_named_item(classes, &"target".encode_utf16().collect::<Vec<_>>()).unwrap(), Some(inner));
        let tags = dom.collection(root, CollectionFilter::TagName("*".encode_utf16().collect())).unwrap();
        assert_eq!(dom.collection_names(tags).unwrap(), vec!["target".encode_utf16().collect::<Vec<_>>()]);
        dom.set_attribute(inner, "class", "a").unwrap();
        assert_eq!(dom.collection_length(classes).unwrap(), 0);
        let empty = dom.collection(root, CollectionFilter::ClassNames(Vec::new())).unwrap();
        assert_eq!(dom.collection_length(empty).unwrap(), 0);
    }
    #[test]
    fn namespace_filter_and_named_item_follow_tree_order() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "div");
        let first = dom.create_element(root, "span");
        dom.set_attribute(first, "name", "same").unwrap();
        let second = dom.create_element(root, "span");
        dom.set_attribute(second, "id", "same").unwrap();
        let html = dom.collection(root, CollectionFilter::TagNameNs(Some("http://www.w3.org/1999/xhtml".encode_utf16().collect()), "span".encode_utf16().collect())).unwrap();
        assert_eq!(dom.collection_length(html).unwrap(), 2);
        assert_eq!(dom.collection_named_item(html, &"same".encode_utf16().collect::<Vec<_>>()).unwrap(), Some(first));
        let null = dom.collection(root, CollectionFilter::TagNameNs(None, "span".encode_utf16().collect())).unwrap();
        assert_eq!(dom.collection_length(null).unwrap(), 0);
        let wildcard = dom.collection(root, CollectionFilter::TagNameNs(Some("*".encode_utf16().collect()), "*".encode_utf16().collect())).unwrap();
        assert_eq!(dom.collection_length(wildcard).unwrap(), 2);
    }
}
