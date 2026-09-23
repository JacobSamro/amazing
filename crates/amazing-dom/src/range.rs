//! DOM Standard §5. Live handles belong to their Dom; snapshots never track mutations.
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundaryPoint {
    pub node: NodeIdx,
    pub offset: usize,
}
impl BoundaryPoint {
    pub fn new(node: NodeIdx, offset: usize) -> Self {
        Self { node, offset }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbstractRange {
    pub start: BoundaryPoint,
    pub end: BoundaryPoint,
}
impl AbstractRange {
    pub fn collapsed(self) -> bool {
        self.start == self.end
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticRange(AbstractRange);
impl StaticRange {
    pub fn boundaries(self) -> AbstractRange {
        self.0
    }
}
/// Immutable boundary snapshot, stored in an append-only arena for wrappers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticRangeId(usize);
impl StaticRangeId {
    pub fn slot(self) -> usize { self.0 }
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
/// Arena-local, non-reused live range handle. `release_range` is a GC seam, not detach().
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range(usize);
impl Range {
    pub const START_TO_START: u16 = 0;
    pub const START_TO_END: u16 = 1;
    pub const END_TO_END: u16 = 2;
    pub const END_TO_START: u16 = 3;
    /// Stable arena slot for the binding layer. Slots are never reused.
    pub fn slot(self) -> usize { self.0 }
    /// The binding layer validates the slot through `Dom::range` before use.
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
impl Dom {
    pub fn register_static_range(&mut self, start: BoundaryPoint, end: BoundaryPoint) -> Result<StaticRangeId, DomError> {
        let range = self.create_static_range(start, end)?;
        let id = StaticRangeId(self.static_ranges.len());
        self.static_ranges.push(Some(range));
        Ok(id)
    }
    pub fn static_range(&self, id: StaticRangeId) -> Result<StaticRange, DomError> {
        self.static_ranges.get(id.0).copied().flatten().ok_or(DomError::InvalidStateError)
    }
    pub fn release_static_range(&mut self, id: StaticRangeId) {
        if let Some(slot) = self.static_ranges.get_mut(id.0) { *slot = None; }
    }
    pub fn node_length(&self, node: NodeIdx) -> usize {
        if let Some(data) = self.character_data_utf16(node) {
            data.len()
        } else {
            self.children(node).count()
        }
    }
    pub fn node_index(&self, node: NodeIdx) -> usize {
        let mut n = node;
        let mut index = 0;
        while let Some(prev) = self.get(n).prev_sibling {
            index += 1;
            n = prev;
        }
        index
    }
    fn check_boundary(&self, p: BoundaryPoint) -> Result<(), DomError> {
        self.check_node(p.node)?;
        if matches!(self.get(p.node).kind, NodeKind::DocumentType { .. }) {
            return Err(DomError::InvalidNodeTypeError);
        }
        if p.offset > self.node_length(p.node) {
            return Err(DomError::IndexSizeError);
        }
        Ok(())
    }
    pub fn compare_boundary_points(
        &self,
        a: BoundaryPoint,
        b: BoundaryPoint,
    ) -> Result<Ordering, DomError> {
        self.check_node(a.node)?;
        self.check_node(b.node)?;
        if self.root(a.node) != self.root(b.node) {
            return Err(DomError::WrongDocumentError);
        }
        if a.node == b.node {
            return Ok(a.offset.cmp(&b.offset));
        }
        if self.contains(a.node, Some(b.node)) {
            let mut child = b.node;
            while self.get(child).parent != Some(a.node) {
                child = self.get(child).parent.unwrap();
            }
            return Ok(if self.node_index(child) < a.offset {
                Ordering::Greater
            } else {
                Ordering::Less
            });
        }
        if self.contains(b.node, Some(a.node)) {
            return Ok(self.compare_boundary_points(b, a)?.reverse());
        }
        Ok(
            if self.compare_document_position(a.node, b.node) & crate::DocumentPosition::FOLLOWING
                != 0
            {
                Ordering::Less
            } else {
                Ordering::Greater
            },
        )
    }
    pub fn create_static_range(
        &self,
        start: BoundaryPoint,
        end: BoundaryPoint,
    ) -> Result<StaticRange, DomError> {
        // Construction deliberately does not enforce offsets, order, or common root.
        for p in [start, end] {
            self.check_node(p.node)?;
            if matches!(self.get(p.node).kind, NodeKind::DocumentType { .. }) {
                return Err(DomError::InvalidNodeTypeError);
            }
        }
        Ok(StaticRange(AbstractRange { start, end }))
    }
    pub fn static_range_valid(&self, range: StaticRange) -> bool {
        let r = range.0;
        self.check_boundary(r.start).is_ok()
            && self.check_boundary(r.end).is_ok()
            && self
                .compare_boundary_points(r.start, r.end)
                .is_ok_and(|o| o != Ordering::Greater)
    }
    pub fn create_range(&mut self, document: NodeIdx) -> Result<Range, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return Err(DomError::InvalidNodeTypeError);
        }
        Ok(self.register_range(AbstractRange {
            start: BoundaryPoint::new(document, 0),
            end: BoundaryPoint::new(document, 0),
        }))
    }
    pub(crate) fn register_range(&mut self, r: AbstractRange) -> Range {
        let id = Range(self.ranges.len());
        self.ranges.push(Some(r));
        id
    }
    pub fn range(&self, id: Range) -> Result<AbstractRange, DomError> {
        self.ranges
            .get(id.0)
            .copied()
            .flatten()
            .ok_or(DomError::InvalidStateError)
    }
    pub fn release_range(&mut self, id: Range) {
        if let Some(r) = self.ranges.get_mut(id.0) {
            *r = None;
        }
    }
    pub fn detach_range(&self, id: Range) -> Result<(), DomError> {
        self.range(id).map(|_| ())
    }
    pub fn clone_range(&mut self, id: Range) -> Result<Range, DomError> {
        Ok(self.register_range(self.range(id)?))
    }
    pub(crate) fn put_range(&mut self, id: Range, r: AbstractRange) {
        self.ranges[id.0] = Some(r);
    }
    pub fn range_set_start(&mut self, id: Range, point: BoundaryPoint) -> Result<(), DomError> {
        self.range_set_boundary(id, point, true)
    }
    pub fn range_set_end(&mut self, id: Range, point: BoundaryPoint) -> Result<(), DomError> {
        self.range_set_boundary(id, point, false)
    }
    fn range_set_boundary(
        &mut self,
        id: Range,
        point: BoundaryPoint,
        start: bool,
    ) -> Result<(), DomError> {
        let mut r = self.range(id)?;
        self.check_boundary(point)?;
        if start {
            if self
                .compare_boundary_points(point, r.end)
                .map_or(true, |o| o == Ordering::Greater)
            {
                r.end = point;
            }
            r.start = point;
        } else {
            if self
                .compare_boundary_points(point, r.start)
                .map_or(true, |o| o == Ordering::Less)
            {
                r.start = point;
            }
            r.end = point;
        }
        self.put_range(id, r);
        Ok(())
    }
    fn adjacent_boundary(&self, node: NodeIdx, after: bool) -> Result<BoundaryPoint, DomError> {
        self.check_node(node)?;
        Ok(BoundaryPoint::new(
            self.get(node)
                .parent
                .ok_or(DomError::InvalidNodeTypeError)?,
            self.node_index(node) + usize::from(after),
        ))
    }
    pub fn range_set_start_before(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range_set_start(id, self.adjacent_boundary(node, false)?)
    }
    pub fn range_set_start_after(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range_set_start(id, self.adjacent_boundary(node, true)?)
    }
    pub fn range_set_end_before(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range_set_end(id, self.adjacent_boundary(node, false)?)
    }
    pub fn range_set_end_after(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range_set_end(id, self.adjacent_boundary(node, true)?)
    }
    pub fn range_collapse(&mut self, id: Range, to_start: bool) -> Result<(), DomError> {
        let r = self.range(id)?;
        let p = if to_start { r.start } else { r.end };
        self.put_range(id, AbstractRange { start: p, end: p });
        Ok(())
    }
    pub fn range_select_node(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range(id)?;
        let start = self.adjacent_boundary(node, false)?;
        self.put_range(
            id,
            AbstractRange {
                start,
                end: BoundaryPoint::new(start.node, start.offset + 1),
            },
        );
        Ok(())
    }
    pub fn range_select_node_contents(&mut self, id: Range, node: NodeIdx) -> Result<(), DomError> {
        self.range(id)?;
        let start = BoundaryPoint::new(node, 0);
        self.check_boundary(start)?;
        self.put_range(
            id,
            AbstractRange {
                start,
                end: BoundaryPoint::new(node, self.node_length(node)),
            },
        );
        Ok(())
    }
    pub fn range_compare_boundary_points(
        &self,
        id: Range,
        how: u16,
        source: Range,
    ) -> Result<Ordering, DomError> {
        if how > 3 {
            return Err(DomError::NotSupportedError);
        }
        let a = self.range(id)?;
        let b = self.range(source)?;
        let (a, b) = match how {
            0 => (a.start, b.start),
            1 => (a.end, b.start),
            2 => (a.end, b.end),
            _ => (a.start, b.end),
        };
        self.compare_boundary_points(a, b)
    }
    pub fn range_common_ancestor(&self, id: Range) -> Result<NodeIdx, DomError> {
        let r = self.range(id)?;
        Ok(self
            .inclusive_ancestors(r.start.node)
            .into_iter()
            .find(|n| self.contains(*n, Some(r.end.node)))
            .unwrap())
    }
    pub fn range_intersects_node(&self, id: Range, node: NodeIdx) -> Result<bool, DomError> {
        let r = self.range(id)?;
        self.check_node(node)?;
        if self.root(node) != self.root(r.start.node) {
            return Ok(false);
        }
        let Some(parent) = self.get(node).parent else {
            return Ok(true);
        };
        let i = self.node_index(node);
        Ok(
            self.compare_boundary_points(BoundaryPoint::new(parent, i), r.end)? == Ordering::Less
                && self.compare_boundary_points(BoundaryPoint::new(parent, i + 1), r.start)?
                    == Ordering::Greater,
        )
    }
    pub fn range_is_point_in_range(&self, id: Range, point: BoundaryPoint) -> Result<bool, DomError> {
        let range = self.range(id)?;
        self.check_node(point.node)?;
        if self.root(point.node) != self.root(range.start.node) { return Ok(false); }
        self.check_boundary(point)?;
        Ok(self.compare_boundary_points(range.start, point)? != Ordering::Greater
            && self.compare_boundary_points(point, range.end)? != Ordering::Greater)
    }
    pub fn range_compare_point(&self, id: Range, point: BoundaryPoint) -> Result<i16, DomError> {
        let range = self.range(id)?;
        self.check_node(point.node)?;
        if self.root(point.node) != self.root(range.start.node) { return Err(DomError::WrongDocumentError); }
        self.check_boundary(point)?;
        Ok(if self.compare_boundary_points(point, range.start)? == Ordering::Less { -1 }
            else if self.compare_boundary_points(point, range.end)? == Ordering::Greater { 1 }
            else { 0 })
    }
    pub fn range_to_utf16_string(&mut self, id: Range) -> Result<Vec<u16>, DomError> {
        let fragment = self.range_clone_contents(id)?;
        Ok(self.tree_order(fragment).into_iter()
            .filter(|node| self.is_text(*node))
            .flat_map(|node| self.character_data_utf16(node).unwrap_or_default())
            .collect())
    }
    pub(crate) fn ranges_insert(&mut self, parent: NodeIdx, before: Option<NodeIdx>, count: usize) {
        let Some(before) = before else {
            return;
        };
        let index = self.node_index(before);
        for r in self.ranges.iter_mut().flatten() {
            for p in [&mut r.start, &mut r.end] {
                if p.node == parent && p.offset > index {
                    p.offset += count;
                }
            }
        }
    }
    pub(crate) fn ranges_remove(&mut self, node: NodeIdx, parent: NodeIdx) {
        let index = self.node_index(node);
        let inside: Vec<_> = self
            .ranges
            .iter()
            .map(|r| {
                r.map(|r| {
                    (
                        self.contains(node, Some(r.start.node)),
                        self.contains(node, Some(r.end.node)),
                    )
                })
            })
            .collect();
        for (r, inside) in self.ranges.iter_mut().zip(inside) {
            if let (Some(r), Some((start, end))) = (r, inside) {
                for (p, inside) in [(&mut r.start, start), (&mut r.end, end)] {
                    if inside {
                        *p = BoundaryPoint::new(parent, index);
                    } else if p.node == parent && p.offset > index {
                        p.offset -= 1;
                    }
                }
            }
        }
    }
    pub(crate) fn ranges_replace_data(
        &mut self,
        node: NodeIdx,
        offset: usize,
        count: usize,
        length: usize,
    ) {
        for r in self.ranges.iter_mut().flatten() {
            for p in [&mut r.start, &mut r.end] {
                if p.node != node {
                    continue;
                }
                if p.offset > offset && p.offset <= offset + count {
                    p.offset = offset;
                } else if p.offset > offset + count {
                    p.offset = p.offset - count + length;
                }
            }
        }
    }
    /// Merge adjacent Text descendants, relocating boundaries before removals.
    pub fn normalize(&mut self, root: NodeIdx) -> Result<(), DomError> {
        self.check_node(root)?;
        let descendants = self.tree_order(root);
        for node in descendants.into_iter().skip(1) {
            if !self.is_text(node) || !self.contains(root, Some(node)) {
                continue;
            }
            let mut length = self.node_length(node);
            if length == 0 {
                self.remove_internal(node, false);
                continue;
            }
            let mut siblings = Vec::new();
            let mut next = self.get(node).next_sibling;
            let mut data = Vec::new();
            while let Some(n) = next {
                if !self.is_text(n) {
                    break;
                }
                siblings.push(n);
                data.extend(self.character_data_utf16(n).unwrap());
                next = self.get(n).next_sibling;
            }
            self.replace_character_data(node, length, 0, &data)?;
            for current in &siblings {
                let parent = self.get(*current).parent.unwrap();
                let index = self.node_index(*current);
                for r in self.ranges.iter_mut().flatten() {
                    for p in [&mut r.start, &mut r.end] {
                        if p.node == *current {
                            p.node = node;
                            p.offset += length;
                        } else if p.node == parent && p.offset == index {
                            p.node = node;
                            p.offset = length;
                        }
                    }
                }
                length += self.node_length(*current);
            }
            for current in siblings {
                self.remove_internal(current, false);
            }
        }
        Ok(())
    }
    pub fn split_text(&mut self, node: NodeIdx, offset: usize) -> Result<NodeIdx, DomError> {
        self.check_node(node)?;
        if !self.is_text(node) {
            return Err(DomError::InvalidNodeTypeError);
        }
        let data = self.character_data_utf16(node).unwrap();
        if offset > data.len() {
            return Err(DomError::IndexSizeError);
        }
        let new = self.create_node_in(
            self.get(node).owner_document,
            NodeKind::Text {
                data: String::new(),
            },
        )?;
        self.store_character_data(new, data[offset..].to_vec());
        if let Some(parent) = self.get(node).parent {
            self.pre_insert(new, parent, self.get(node).next_sibling)?;
            let after = self.node_index(node) + 1;
            for r in self.ranges.iter_mut().flatten() {
                for p in [&mut r.start, &mut r.end] {
                    if p.node == node && p.offset > offset {
                        p.node = new;
                        p.offset -= offset;
                    } else if p.node == parent && p.offset == after {
                        p.offset += 1;
                    }
                }
            }
        }
        self.replace_character_data(node, offset, data.len() - offset, &[])?;
        Ok(new)
    }
}
#[path = "range_contents.rs"]
mod contents;
#[cfg(test)]
#[path = "range_tests.rs"]
mod tests;
