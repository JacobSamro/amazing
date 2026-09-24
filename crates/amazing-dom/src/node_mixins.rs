//! ParentNode/ChildNode mutation algorithms (DOM Standard §4.2.6 and §4.2.8).
use crate::{Dom, DomError, NodeIdx, NodeKind};

/// Binding-converted `(Node or DOMString)` argument. JavaScript coercions happen
/// before entering the arena; no user code runs while these algorithms borrow it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeOrString {
    Node(NodeIdx),
    String(String),
    /// Exact DOMString, including unpaired surrogates.
    Utf16(Vec<u16>),
}

impl Dom {
    pub fn first_element_child(&self, parent: NodeIdx) -> Option<NodeIdx> {
        self.children(parent).find(|child| matches!(self.get(*child).kind, NodeKind::Element { .. }))
    }

    pub fn last_element_child(&self, parent: NodeIdx) -> Option<NodeIdx> {
        self.children(parent).filter(|child| matches!(self.get(*child).kind, NodeKind::Element { .. })).last()
    }

    pub fn child_element_count(&self, parent: NodeIdx) -> usize {
        self.children(parent).filter(|child| matches!(self.get(*child).kind, NodeKind::Element { .. })).count()
    }

    pub fn previous_element_sibling(&self, node: NodeIdx) -> Option<NodeIdx> {
        let mut sibling = self.get(node).prev_sibling;
        while let Some(current) = sibling {
            if matches!(self.get(current).kind, NodeKind::Element { .. }) { return sibling; }
            sibling = self.get(current).prev_sibling;
        }
        None
    }

    pub fn next_element_sibling(&self, node: NodeIdx) -> Option<NodeIdx> {
        let mut sibling = self.get(node).next_sibling;
        while let Some(current) = sibling {
            if matches!(self.get(current).kind, NodeKind::Element { .. }) { return sibling; }
            sibling = self.get(current).next_sibling;
        }
        None
    }

    fn convert_nodes(
        &mut self,
        document: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<NodeIdx, DomError> {
        let mut nodes = Vec::with_capacity(values.len());
        for value in values {
            nodes.push(match value {
                NodeOrString::Node(node) => {
                    self.check_node(*node)?;
                    *node
                }
                NodeOrString::Utf16(data) => {
                    let node = self.create_node_in(
                        document,
                        NodeKind::Text {
                            data: String::new(),
                        },
                    )?;
                    self.store_character_data(node, data.clone());
                    node
                }
                NodeOrString::String(data) => {
                    self.create_node_in(document, NodeKind::Text { data: data.clone() })?
                }
            });
        }
        if nodes.len() == 1 {
            return Ok(nodes[0]);
        }
        let fragment = self.create_document_fragment(document)?;
        for node in nodes {
            // Conversion itself moves/adopts nodes, even if final insertion
            // subsequently fails. Repeated node arguments move, not clone.
            self.pre_insert(node, fragment, None)?;
        }
        Ok(fragment)
    }

    fn parent_mixin_document(&self, parent: NodeIdx) -> Result<NodeIdx, DomError> {
        self.check_node(parent)?;
        if !matches!(
            self.get(parent).kind,
            NodeKind::Document | NodeKind::DocumentFragment | NodeKind::Element { .. }
        ) {
            return Err(DomError::TypeError);
        }
        Ok(self.get(parent).owner_document)
    }

    fn check_child_mixin(&self, node: NodeIdx) -> Result<(), DomError> {
        self.check_node(node)?;
        if matches!(
            self.get(node).kind,
            NodeKind::Document | NodeKind::DocumentFragment
        ) {
            Err(DomError::TypeError)
        } else {
            Ok(())
        }
    }

    /// ParentNode.append, after WebIDL argument conversion.
    pub fn append_nodes(
        &mut self,
        parent: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        let document = self.parent_mixin_document(parent)?;
        let node = self.convert_nodes(document, values)?;
        self.pre_insert(node, parent, None)?;
        Ok(())
    }

    /// ParentNode.prepend. Determine the first child *after* node conversion.
    pub fn prepend_nodes(
        &mut self,
        parent: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        let document = self.parent_mixin_document(parent)?;
        let node = self.convert_nodes(document, values)?;
        self.pre_insert(node, parent, self.get(parent).first_child)?;
        Ok(())
    }

    pub fn replace_children(
        &mut self,
        parent: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        let document = self.parent_mixin_document(parent)?;
        let node = self.convert_nodes(document, values)?;
        self.replace_all(parent, Some(node))
    }

    fn viable_sibling(
        &self,
        node: NodeIdx,
        values: &[NodeOrString],
        previous: bool,
    ) -> Option<NodeIdx> {
        let mut sibling = if previous {
            self.get(node).prev_sibling
        } else {
            self.get(node).next_sibling
        };
        while let Some(id) = sibling {
            if !values.contains(&NodeOrString::Node(id)) {
                return Some(id);
            }
            sibling = if previous {
                self.get(id).prev_sibling
            } else {
                self.get(id).next_sibling
            };
        }
        None
    }

    pub fn before_nodes(
        &mut self,
        target: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        self.check_child_mixin(target)?;
        let Some(parent) = self.get(target).parent else {
            return Ok(());
        };
        let previous = self.viable_sibling(target, values, true);
        let node = self.convert_nodes(self.get(target).owner_document, values)?;
        let reference = previous.map_or(self.get(parent).first_child, |p| self.get(p).next_sibling);
        self.pre_insert(node, parent, reference)?;
        Ok(())
    }

    pub fn after_nodes(
        &mut self,
        target: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        self.check_child_mixin(target)?;
        let Some(parent) = self.get(target).parent else {
            return Ok(());
        };
        let next = self.viable_sibling(target, values, false);
        let node = self.convert_nodes(self.get(target).owner_document, values)?;
        self.pre_insert(node, parent, next)?;
        Ok(())
    }

    pub fn replace_with(
        &mut self,
        target: NodeIdx,
        values: &[NodeOrString],
    ) -> Result<(), DomError> {
        self.check_child_mixin(target)?;
        let Some(parent) = self.get(target).parent else {
            return Ok(());
        };
        let next = self.viable_sibling(target, values, false);
        let node = self.convert_nodes(self.get(target).owner_document, values)?;
        if self.get(target).parent == Some(parent) {
            self.replace(target, node, parent)?;
        } else {
            // The target can have moved into the conversion fragment.
            self.pre_insert(node, parent, next)?;
        }
        Ok(())
    }

    pub fn remove(&mut self, target: NodeIdx) -> Result<(), DomError> {
        self.check_child_mixin(target)?;
        if let Some(parent) = self.get(target).parent {
            self.pre_remove(target, parent)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "node_mixins_tests.rs"]
mod tests;
