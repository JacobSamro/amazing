//! Live element collections backed by the DOM arena.
use crate::{Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionFilter {
    Children,
    TagName(Vec<u16>),
    TagNameNs(Option<Vec<u16>>, Vec<u16>),
    ClassNames(Vec<Vec<u16>>),
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
            let nodes: Vec<_> = match filter {
                CollectionFilter::Children => self.children(root).collect(),
                _ => self.tree_order(root).into_iter().skip(1).collect(),
            };
            let items = nodes.into_iter().filter(|node| {
                let NodeKind::Element { name, namespace, .. } = &self.get(*node).kind else { return false };
                match &filter {
                    CollectionFilter::Children => true,
                    CollectionFilter::TagName(query) => query == &vec![b'*' as u16] || {
                        let actual: Vec<_> = name.encode_utf16().collect();
                        if *namespace == Namespace::Html {
                            actual.iter().map(|unit| if (65..=90).contains(unit) { unit + 32 } else { *unit }).collect::<Vec<_>>() ==
                                query.iter().map(|unit| if (65..=90).contains(unit) { unit + 32 } else { *unit }).collect::<Vec<_>>()
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
