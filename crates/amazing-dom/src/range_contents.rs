use super::{AbstractRange, BoundaryPoint as Point, Range};
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::cmp::Ordering;
impl Dom {
    fn range_contains_node(&self, r: AbstractRange, node: NodeIdx) -> bool {
        self.compare_boundary_points(r.start, Point::new(node, 0))
            .is_ok_and(|o| o == Ordering::Less)
            && self
                .compare_boundary_points(Point::new(node, self.node_length(node)), r.end)
                .is_ok_and(|o| o == Ordering::Less)
    }
    fn range_partial_node(&self, r: AbstractRange, node: NodeIdx) -> bool {
        self.contains(node, Some(r.start.node)) != self.contains(node, Some(r.end.node))
    }
    fn extraction_collapse_point(&self, r: AbstractRange) -> Point {
        if self.contains(r.start.node, Some(r.end.node)) {
            return r.start;
        }
        let mut reference = r.start.node;
        while let Some(parent) = self.get(reference).parent {
            if self.contains(parent, Some(r.end.node)) {
                return Point::new(parent, self.node_index(reference) + 1);
            }
            reference = parent;
        }
        unreachable!("range has one root")
    }
    /// Core node clone, including namespaces and exact CharacterData code units.
    pub fn clone_node(&mut self, node: NodeIdx, deep: bool) -> Result<NodeIdx, DomError> {
        self.check_node(node)?;
        if self.is_shadow_root(node) { return Err(DomError::NotSupportedError); }
        let copy = self.alloc_unattached(self.get(node).kind.clone());
        if let NodeKind::Attr { owner_element, .. } = &mut self.nodes[copy.0 as usize].kind {
            *owner_element = None;
        }
        if !matches!(self.get(node).kind, NodeKind::Document) {
            self.nodes[copy.0 as usize].owner_document = self.get(node).owner_document;
        }
        self.nodes[copy.0 as usize].namespaced_attributes =
            self.get(node).namespaced_attributes.clone();
        self.nodes[copy.0 as usize].character_data_units =
            self.get(node).character_data_units.clone();
        self.nodes[copy.0 as usize].prefix = self.get(node).prefix.clone();
        self.clone_form_state(node, copy);
        if deep {
            let children: Vec<_> = self.children(node).collect();
            for child in children {
                let child = self.clone_node(child, true)?;
                self.pre_insert(child, copy, None)?;
            }
        }
        if let Some(root) = self.shadow_root_of_host(node) {
            let options = self.shadow_data(root).unwrap().options;
            if options.clonable {
                let copy_root = self.attach_shadow(copy, options)?;
                let children: Vec<_> = self.children(root).collect();
                for child in children {
                    let child = self.clone_node(child, true)?;
                    self.pre_insert(child, copy_root, None)?;
                }
            }
        }
        Ok(copy)
    }
    pub fn range_delete_contents(&mut self, id: Range) -> Result<(), DomError> {
        let r = self.range(id)?;
        if r.collapsed() {
            return Ok(());
        }
        if r.start.node == r.end.node && self.character_data(r.start.node).is_some() {
            return self.replace_character_data(
                r.start.node,
                r.start.offset,
                r.end.offset - r.start.offset,
                &[],
            );
        }
        let nodes: Vec<_> = self
            .tree_order(self.range_common_ancestor(id)?)
            .into_iter()
            .filter(|n| {
                self.range_contains_node(r, *n)
                    && !self
                        .get(*n)
                        .parent
                        .is_some_and(|p| self.range_contains_node(r, p))
            })
            .collect();
        let point = self.extraction_collapse_point(r);
        self.put_range(
            id,
            AbstractRange {
                start: point,
                end: point,
            },
        );
        if self.character_data(r.start.node).is_some() {
            self.replace_character_data(
                r.start.node,
                r.start.offset,
                self.node_length(r.start.node) - r.start.offset,
                &[],
            )?;
        }
        for node in nodes {
            self.remove_internal(node, false);
        }
        if self.character_data(r.end.node).is_some() {
            self.replace_character_data(r.end.node, 0, r.end.offset, &[])?;
        }
        Ok(())
    }
    pub fn range_extract_contents(&mut self, id: Range) -> Result<NodeIdx, DomError> {
        self.range_copy_contents(id, true)
    }
    pub fn range_clone_contents(&mut self, id: Range) -> Result<NodeIdx, DomError> {
        self.range_copy_contents(id, false)
    }
    fn range_copy_contents(&mut self, id: Range, extract: bool) -> Result<NodeIdx, DomError> {
        let r = self.range(id)?;
        let fragment = self.create_document_fragment(self.get(r.start.node).owner_document)?;
        if r.collapsed() {
            return Ok(fragment);
        }
        if r.start.node == r.end.node && self.character_data(r.start.node).is_some() {
            let copy = self.clone_node(r.start.node, false)?;
            let data = self.character_data_utf16(r.start.node).unwrap();
            self.store_character_data(copy, data[r.start.offset..r.end.offset].to_vec());
            self.pre_insert(copy, fragment, None)?;
            if extract {
                self.replace_character_data(
                    r.start.node,
                    r.start.offset,
                    r.end.offset - r.start.offset,
                    &[],
                )?;
            }
            return Ok(fragment);
        }
        let common = self.range_common_ancestor(id)?;
        let children: Vec<_> = self.children(common).collect();
        let first = if !self.contains(r.start.node, Some(r.end.node)) {
            children
                .iter()
                .copied()
                .find(|n| self.range_partial_node(r, *n))
        } else {
            None
        };
        let last = if !self.contains(r.end.node, Some(r.start.node)) {
            children
                .iter()
                .copied()
                .rev()
                .find(|n| self.range_partial_node(r, *n))
        } else {
            None
        };
        let contained: Vec<_> = children
            .into_iter()
            .filter(|n| self.range_contains_node(r, *n))
            .collect();
        if contained
            .iter()
            .any(|n| matches!(self.get(*n).kind, NodeKind::DocumentType { .. }))
        {
            return Err(DomError::HierarchyRequestError);
        }
        if extract {
            let p = self.extraction_collapse_point(r);
            self.put_range(id, AbstractRange { start: p, end: p });
        }
        if let Some(first) = first {
            self.copy_partial_contents(
                first,
                AbstractRange {
                    start: r.start,
                    end: Point::new(first, self.node_length(first)),
                },
                fragment,
                extract,
            )?;
        }
        for node in contained {
            let node = if extract {
                node
            } else {
                self.clone_node(node, true)?
            };
            self.pre_insert(node, fragment, None)?;
        }
        if let Some(last) = last {
            self.copy_partial_contents(
                last,
                AbstractRange {
                    start: Point::new(last, 0),
                    end: r.end,
                },
                fragment,
                extract,
            )?;
        }
        Ok(fragment)
    }
    fn copy_partial_contents(
        &mut self,
        node: NodeIdx,
        r: AbstractRange,
        fragment: NodeIdx,
        extract: bool,
    ) -> Result<(), DomError> {
        let copy = self.clone_node(node, false)?;
        if let Some(data) = self.character_data_utf16(node) {
            self.store_character_data(copy, data[r.start.offset..r.end.offset].to_vec());
            self.pre_insert(copy, fragment, None)?;
            if extract {
                self.replace_character_data(
                    node,
                    r.start.offset,
                    r.end.offset - r.start.offset,
                    &[],
                )?;
            }
        } else {
            self.pre_insert(copy, fragment, None)?;
            let subrange = self.register_range(r);
            let result = self.range_copy_contents(subrange, extract);
            self.release_range(subrange);
            self.pre_insert(result?, copy, None)?;
        }
        Ok(())
    }
    pub fn range_insert_node(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        let r = self.range(id)?;
        self.check_node(node)?;
        let start = r.start.node;
        if matches!(
            self.get(start).kind,
            NodeKind::Comment { .. } | NodeKind::ProcessingInstruction { .. }
        ) || (self.is_text(start) && self.get(start).parent.is_none())
            || start == node
        {
            return Err(DomError::HierarchyRequestError);
        }
        let mut reference = if self.is_text(start) {
            Some(start)
        } else {
            self.children(start).nth(r.start.offset)
        };
        let parent = reference.and_then(|n| self.get(n).parent).unwrap_or(start);
        self.ensure_pre_insert_validity(node, parent, reference)?;
        if self.is_text(start) {
            reference = Some(self.split_text(start, r.start.offset)?);
        }
        if reference == Some(node) {
            reference = self.get(node).next_sibling;
        }
        if self.get(node).parent.is_some() {
            self.remove_internal(node, false);
        }
        let offset = reference
            .map(|n| self.node_index(n))
            .unwrap_or_else(|| self.node_length(parent))
            + if matches!(self.get(node).kind, NodeKind::DocumentFragment) {
                self.node_length(node)
            } else {
                1
            };
        self.pre_insert(node, parent, reference)?;
        if self.range(id)?.collapsed() {
            let mut r = self.range(id)?;
            r.end = Point::new(parent, offset);
            self.put_range(id, r);
        }
        Ok(())
    }
    pub fn range_surround_contents(&mut self, id: Range, parent: NodeIdx) -> Result<(), DomError> {
        let r = self.range(id)?;
        self.check_node(parent)?;
        if self
            .tree_order(self.range_common_ancestor(id)?)
            .into_iter()
            .any(|n| !self.is_text(n) && self.range_partial_node(r, n))
        {
            return Err(DomError::InvalidStateError);
        }
        if matches!(
            self.get(parent).kind,
            NodeKind::Document | NodeKind::DocumentType { .. } | NodeKind::DocumentFragment
        ) {
            return Err(DomError::InvalidNodeTypeError);
        }
        let fragment = self.range_extract_contents(id)?;
        if self.get(parent).first_child.is_some() {
            self.replace_all(parent, None)?;
        }
        self.range_insert_node(id, parent)?;
        self.pre_insert(fragment, parent, None)?;
        self.range_select_node(id, parent)
    }
}
