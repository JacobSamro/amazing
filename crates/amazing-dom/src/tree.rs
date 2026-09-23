//! DOM Standard §4.2.3 and §4.4. Checked DOM operations share the parser arena.
use crate::{Dom, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomError {
    HierarchyRequestError,
    NotFoundError,
    NotSupportedError,
    InvalidStateError,
    TypeError,
    InvalidNode,
    InvalidNodeTypeError,
    IndexSizeError,
    WrongDocumentError,
    InvalidCharacterError,
    NamespaceError,
    InUseAttributeError,
    SyntaxError,
}
impl std::fmt::Display for DomError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DomError {}

/// compareDocumentPosition bit flags. Disconnected ordering uses stable arena ids.
pub struct DocumentPosition;
impl DocumentPosition {
    pub const DISCONNECTED: u16 = 1;
    pub const PRECEDING: u16 = 2;
    pub const FOLLOWING: u16 = 4;
    pub const CONTAINS: u16 = 8;
    pub const CONTAINED_BY: u16 = 16;
    pub const IMPLEMENTATION_SPECIFIC: u16 = 32;
}
impl Dom {
    pub(crate) fn check_node(&self, node: NodeIdx) -> Result<(), DomError> {
        if (node.0 as usize) < self.nodes.len() {
            Ok(())
        } else {
            Err(DomError::InvalidNode)
        }
    }
    /// Multiple documents share one arena so adoption preserves every handle.
    pub fn create_document(&mut self) -> NodeIdx {
        self.alloc_unattached(NodeKind::Document)
    }
    pub fn create_document_fragment(&mut self, document: NodeIdx) -> Result<NodeIdx, DomError> {
        self.create_node_in(document, NodeKind::DocumentFragment)
    }
    pub fn create_node_in(
        &mut self,
        document: NodeIdx,
        kind: NodeKind,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document)
            || matches!(kind, NodeKind::Document)
        {
            return Err(DomError::NotSupportedError);
        }
        let id = self.alloc_unattached(kind);
        self.nodes[id.0 as usize].owner_document = document;
        Ok(id)
    }
    pub fn owner_document(&self, node: NodeIdx) -> Option<NodeIdx> {
        if matches!(self.get(node).kind, NodeKind::Document) {
            None
        } else {
            Some(self.get(node).owner_document)
        }
    }
    pub fn root(&self, mut node: NodeIdx) -> NodeIdx {
        while let Some(parent) = self.get(node).parent {
            node = parent;
        }
        node
    }
    pub fn is_connected(&self, node: NodeIdx) -> bool {
        matches!(self.get(self.root(node)).kind, NodeKind::Document)
    }
    pub fn contains(&self, ancestor: NodeIdx, other: Option<NodeIdx>) -> bool {
        let mut cursor = other;
        while let Some(node) = cursor {
            if node == ancestor {
                return true;
            }
            cursor = self.get(node).parent;
        }
        false
    }
    pub fn is_ancestor(&self, ancestor: NodeIdx, node: NodeIdx) -> bool {
        ancestor != node && self.contains(ancestor, Some(node))
    }
    pub fn is_descendant(&self, node: NodeIdx, ancestor: NodeIdx) -> bool {
        self.is_ancestor(ancestor, node)
    }
    pub fn is_sibling(&self, a: NodeIdx, b: NodeIdx) -> bool {
        a != b && self.get(a).parent.is_some() && self.get(a).parent == self.get(b).parent
    }
    pub fn inclusive_ancestors(&self, mut node: NodeIdx) -> Vec<NodeIdx> {
        let mut result = vec![node];
        while let Some(parent) = self.get(node).parent {
            result.push(parent);
            node = parent;
        }
        result
    }
    /// Owned preorder snapshot, safe to retain while mutating the arena.
    pub fn tree_order(&self, root: NodeIdx) -> Vec<NodeIdx> {
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            result.push(node);
            let mut child = self.get(node).last_child;
            while let Some(id) = child {
                stack.push(id);
                child = self.get(id).prev_sibling;
            }
        }
        result
    }
    pub fn compare_document_position(&self, node: NodeIdx, other: NodeIdx) -> u16 {
        use DocumentPosition as P;
        if node == other {
            return 0;
        }
        if self.root(node) != self.root(other) {
            return P::DISCONNECTED
                | P::IMPLEMENTATION_SPECIFIC
                | if node < other {
                    P::FOLLOWING
                } else {
                    P::PRECEDING
                };
        }
        if self.is_ancestor(other, node) {
            return P::CONTAINS | P::PRECEDING;
        }
        if self.is_ancestor(node, other) {
            return P::CONTAINED_BY | P::FOLLOWING;
        }
        for id in self.tree_order(self.root(node)) {
            if id == node {
                return P::FOLLOWING;
            }
            if id == other {
                return P::PRECEDING;
            }
        }
        unreachable!("both nodes in same tree")
    }
    /// No mutation occurs on any validation failure. Exclusions are used only
    /// by replace/replace-all, exactly as in ensure pre-insert validity.
    fn insertion_validity(
        &self,
        node: NodeIdx,
        parent: NodeIdx,
        child: Option<NodeIdx>,
        exclude: &[NodeIdx],
    ) -> Result<(), DomError> {
        self.check_node(parent)?;
        self.check_node(node)?;
        if !matches!(
            self.get(parent).kind,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Element { .. }
        ) {
            return Err(DomError::HierarchyRequestError);
        }
        if self.contains(node, Some(parent)) {
            return Err(DomError::HierarchyRequestError);
        }
        if let Some(child) = child {
            self.check_node(child)?;
            if self.get(child).parent != Some(parent) {
                return Err(DomError::NotFoundError);
            }
        }
        let kind = &self.get(node).kind;
        if matches!(kind, NodeKind::Document | NodeKind::Attr { .. }) {
            return Err(DomError::HierarchyRequestError);
        }
        if !matches!(self.get(parent).kind, NodeKind::Document) {
            return if matches!(kind, NodeKind::DocumentType { .. }) {
                Err(DomError::HierarchyRequestError)
            } else {
                Ok(())
            };
        }
        if matches!(kind, NodeKind::Text { .. } | NodeKind::CDATASection { .. }) {
            return Err(DomError::HierarchyRequestError);
        }
        if matches!(
            kind,
            NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
        ) {
            return Ok(());
        }
        if matches!(kind, NodeKind::DocumentFragment) {
            let elements = self
                .children(node)
                .filter(|n| matches!(self.get(*n).kind, NodeKind::Element { .. }))
                .count();
            if elements > 1
                || self
                    .children(node)
                    .any(|n| matches!(self.get(n).kind, NodeKind::Text { .. } | NodeKind::CDATASection { .. }))
            {
                return Err(DomError::HierarchyRequestError);
            }
            if elements == 0 {
                return Ok(());
            }
        }
        let siblings: Vec<_> = self.children(parent).collect();
        let index = child
            .map(|n| siblings.iter().position(|id| *id == n).unwrap())
            .unwrap_or(siblings.len());
        let element = |n: &NodeIdx| matches!(self.get(*n).kind, NodeKind::Element { .. });
        let doctype = |n: &NodeIdx| matches!(self.get(*n).kind, NodeKind::DocumentType { .. });
        let invalid = if matches!(kind, NodeKind::Element { .. } | NodeKind::DocumentFragment) {
            siblings.iter().any(|n| element(n) && !exclude.contains(n))
                || siblings.iter().skip(index + 1).any(doctype)
                || child.is_some_and(|n| doctype(&n) && !exclude.contains(&n))
        } else {
            siblings.iter().any(|n| doctype(n) && !exclude.contains(n))
                || siblings
                    .iter()
                    .take(index)
                    .any(|n| element(n) && (child.is_some() || !exclude.contains(n)))
        };
        if invalid {
            Err(DomError::HierarchyRequestError)
        } else {
            Ok(())
        }
    }
    pub fn ensure_pre_insert_validity(
        &self,
        node: NodeIdx,
        parent: NodeIdx,
        child: Option<NodeIdx>,
    ) -> Result<(), DomError> {
        self.insertion_validity(node, parent, child, &[])
    }
    pub fn pre_insert(
        &mut self,
        node: NodeIdx,
        parent: NodeIdx,
        child: Option<NodeIdx>,
    ) -> Result<NodeIdx, DomError> {
        self.ensure_pre_insert_validity(node, parent, child)?;
        let reference = if child == Some(node) {
            self.get(node).next_sibling
        } else {
            child
        };
        self.insert_internal(node, parent, reference, false);
        Ok(node)
    }
    pub fn pre_remove(&mut self, child: NodeIdx, parent: NodeIdx) -> Result<NodeIdx, DomError> {
        self.check_node(child)?;
        self.check_node(parent)?;
        if self.get(child).parent != Some(parent) {
            return Err(DomError::NotFoundError);
        }
        self.remove_internal(child, false);
        Ok(child)
    }
    pub fn replace(
        &mut self,
        child: NodeIdx,
        node: NodeIdx,
        parent: NodeIdx,
    ) -> Result<NodeIdx, DomError> {
        self.insertion_validity(node, parent, Some(child), &[child])?;
        let mut reference = self.get(child).next_sibling;
        if reference == Some(node) {
            reference = self.get(node).next_sibling;
        }
        let previous = self.get(child).prev_sibling;
        self.adopt_internal(node, self.get(parent).owner_document);
        let removed = if self.get(child).parent.is_some() {
            self.remove_internal(child, true);
            vec![child]
        } else {
            vec![]
        };
        let added = self.insertion_nodes(node);
        self.insert_internal(node, parent, reference, true);
        self.queue_tree_record(parent, added, removed, previous, reference);
        Ok(child)
    }
    /// Checked entry point for replacing all children (e.g. a parsed fragment).
    pub fn replace_all(&mut self, parent: NodeIdx, node: Option<NodeIdx>) -> Result<(), DomError> {
        self.check_node(parent)?;
        let removed: Vec<_> = self.children(parent).collect();
        if let Some(node) = node {
            self.insertion_validity(node, parent, None, &removed)?;
        } else if !matches!(
            self.get(parent).kind,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Element { .. }
        ) {
            return Err(DomError::HierarchyRequestError);
        }
        let added = node.map(|n| self.insertion_nodes(n)).unwrap_or_default();
        for child in &removed {
            self.remove_internal(*child, true);
        }
        if let Some(node) = node {
            self.insert_internal(node, parent, None, true);
        }
        self.queue_tree_record(parent, added, removed, None, None);
        Ok(())
    }
    pub fn adopt(&mut self, node: NodeIdx, document: NodeIdx) -> Result<NodeIdx, DomError> {
        self.check_node(node)?;
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document)
            || matches!(self.get(node).kind, NodeKind::Document)
        {
            return Err(DomError::NotSupportedError);
        }
        self.adopt_internal(node, document);
        Ok(node)
    }
    fn adopt_internal(&mut self, node: NodeIdx, document: NodeIdx) {
        if self.get(node).parent.is_some() {
            self.remove_internal(node, false);
        }
        self.set_subtree_document(node, document);
    }
    pub(crate) fn set_subtree_document(&mut self, node: NodeIdx, document: NodeIdx) {
        if self.get(node).owner_document == document {
            return;
        }
        for id in self.tree_order(node) {
            self.nodes[id.0 as usize].owner_document = document;
        }
    }
    fn insertion_nodes(&self, node: NodeIdx) -> Vec<NodeIdx> {
        if matches!(self.get(node).kind, NodeKind::DocumentFragment) {
            self.children(node).collect()
        } else {
            vec![node]
        }
    }
    fn insert_internal(
        &mut self,
        node: NodeIdx,
        parent: NodeIdx,
        child: Option<NodeIdx>,
        suppress: bool,
    ) {
        let nodes = self.insertion_nodes(node);
        if nodes.is_empty() {
            return;
        }
        if matches!(self.get(node).kind, NodeKind::DocumentFragment) {
            for id in &nodes {
                self.remove_internal(*id, true);
            }
            self.queue_tree_record(node, vec![], nodes.clone(), None, None);
        }
        self.ranges_insert(parent, child, nodes.len());
        let previous = if let Some(child) = child {
            self.get(child).prev_sibling
        } else {
            self.get(parent).last_child
        };
        for id in &nodes {
            self.adopt_internal(*id, self.get(parent).owner_document);
            self.link_before(parent, child, *id);
        }
        if !suppress {
            self.queue_tree_record(parent, nodes, vec![], previous, child);
        }
    }
    pub(crate) fn link_before(&mut self, parent: NodeIdx, before: Option<NodeIdx>, node: NodeIdx) {
        self.mutation_generation = self.mutation_generation.wrapping_add(1);
        let prev = if let Some(before) = before {
            self.get(before).prev_sibling
        } else {
            self.get(parent).last_child
        };
        let n = &mut self.nodes[node.0 as usize];
        n.parent = Some(parent);
        n.prev_sibling = prev;
        n.next_sibling = before;
        if let Some(prev) = prev {
            self.nodes[prev.0 as usize].next_sibling = Some(node);
        } else {
            self.nodes[parent.0 as usize].first_child = Some(node);
        }
        if let Some(next) = before {
            self.nodes[next.0 as usize].prev_sibling = Some(node);
        } else {
            self.nodes[parent.0 as usize].last_child = Some(node);
        }
    }
    pub(crate) fn remove_internal(&mut self, node: NodeIdx, suppress: bool) {
        self.mutation_generation = self.mutation_generation.wrapping_add(1);
        let n = self.get(node);
        let parent = n.parent.expect("remove requires parent");
        let prev = n.prev_sibling;
        let next = n.next_sibling;
        self.ranges_remove(node, parent);
        self.iterators_remove(node);
        let ancestors = self.inclusive_ancestors(parent);
        self.mutations.register_transients(node, &ancestors);
        if let Some(prev) = prev {
            self.nodes[prev.0 as usize].next_sibling = next;
        } else {
            self.nodes[parent.0 as usize].first_child = next;
        }
        if let Some(next) = next {
            self.nodes[next.0 as usize].prev_sibling = prev;
        } else {
            self.nodes[parent.0 as usize].last_child = prev;
        }
        let n = &mut self.nodes[node.0 as usize];
        n.parent = None;
        n.prev_sibling = None;
        n.next_sibling = None;
        if !suppress {
            self.queue_tree_record(parent, vec![], vec![node], prev, next);
        }
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
