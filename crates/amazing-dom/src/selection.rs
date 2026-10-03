// Portions ported from jsdom/jsdom (lib/jsdom/living/selection/Selection-impl.js)
// @ 9547fbf5b60af45d54c9e34d0891c7808f6ee2ce, MIT.
//! Document-owned Selection retains one live Range and immutable document identity.
use crate::{Dom, DomError, NodeIdx, NodeKind, range::{AbstractRange, BoundaryPoint, Range, StaticRangeId}};
use std::cmp::Ordering;

/// Append-only arena handle. Document slot reuse cannot retarget a held Selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectionId(usize);
impl SelectionId {
    pub fn slot(self) -> usize { self.0 }
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionDirection { Forwards, Backwards, Directionless }
#[derive(Clone, Copy, Debug)]
pub struct SelectionState {
    pub document: NodeIdx,
    pub document_generation: u64,
    pub range: Option<Range>,
    pub direction: SelectionDirection,
    observed: Option<AbstractRange>,
}
/// A pending document event, independent of Selection or Range wrapper identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionChangeTask {
    pub document: NodeIdx,
    pub document_generation: u64,
    token: u64,
}
#[derive(Debug, Default)]
pub(crate) struct Selections {
    entries: Vec<SelectionState>,
    scheduled: Vec<SelectionChangeTask>,
    outbox: Vec<SelectionChangeTask>,
    next_token: u64,
}

impl Dom {
    fn selection_document_generation(&self, document: NodeIdx) -> Result<u64, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) { return Err(DomError::TypeError); }
        self.node_generation(document).ok_or(DomError::InvalidStateError)
    }
    pub fn document_selection(&mut self, document: NodeIdx) -> Result<SelectionId, DomError> {
        let generation = self.selection_document_generation(document)?;
        if let Some(slot) = self.selections.entries.iter().position(|s| s.document == document && s.document_generation == generation) {
            return Ok(SelectionId(slot));
        }
        let id = SelectionId(self.selections.entries.len());
        self.selections.entries.push(SelectionState { document, document_generation: generation,
            range: None, direction: SelectionDirection::Directionless, observed: None });
        Ok(id)
    }
    pub fn selection(&self, id: SelectionId) -> Result<SelectionState, DomError> {
        let state = self.selections.entries.get(id.0).copied().ok_or(DomError::InvalidStateError)?;
        if self.selection_document_generation(state.document)? != state.document_generation { return Err(DomError::InvalidStateError); }
        Ok(state)
    }
    pub fn selection_document(&self, id: SelectionId) -> Result<NodeIdx, DomError> { Ok(self.selection(id)?.document) }
    pub fn selection_range(&self, document: NodeIdx) -> Result<Option<Range>, DomError> {
        let generation = self.selection_document_generation(document)?;
        Ok(self.selections.entries.iter().find(|s| s.document == document && s.document_generation == generation).and_then(|s| s.range))
    }
    fn schedule_selection_change(&mut self, document: NodeIdx, generation: u64) {
        if self.selections.scheduled.iter().any(|task| task.document == document && task.document_generation == generation) { return; }
        self.selections.next_token = self.selections.next_token.checked_add(1).expect("Selection task token exhausted");
        let task = SelectionChangeTask { document, document_generation: generation, token: self.selections.next_token };
        self.selections.scheduled.push(task);
        self.selections.outbox.push(task);
    }
    pub fn drain_selection_change_tasks(&mut self) -> Vec<SelectionChangeTask> { std::mem::take(&mut self.selections.outbox) }
    /// Clear the document-owned flag BEFORE event listeners run. A reentrant change queues a fresh task.
    pub fn begin_selection_change(&mut self, task: SelectionChangeTask) -> bool {
        let Some(index) = self.selections.scheduled.iter().position(|pending| *pending == task) else { return false; };
        self.selections.scheduled.remove(index);
        self.selection_document_generation(task.document).is_ok_and(|generation| generation == task.document_generation)
    }
    pub(crate) fn selection_holds_range(&self, range: Range) -> bool {
        self.selections.entries.iter().any(|state| state.range == Some(range)
            && self.node_generation(state.document) == Some(state.document_generation))
    }
    /// Called after each live-range boundary mutation, including intermediate mutations in a bulk DOM operation.
    pub(crate) fn selection_ranges_changed(&mut self) {
        let mut changed = Vec::new();
        for state in &mut self.selections.entries {
            let current = state.range.and_then(|range| self.ranges.get(range.slot()).copied().flatten());
            if current != state.observed {
                state.observed = current;
                changed.push((state.document, state.document_generation));
            }
        }
        for (document, generation) in changed { self.schedule_selection_change(document, generation); }
    }
    fn associate_selection_range(&mut self, id: SelectionId, range: Option<Range>, direction: SelectionDirection) -> Result<(), DomError> {
        let old = self.selection(id)?;
        let observed = range.map(|range| self.range(range)).transpose()?;
        let state = &mut self.selections.entries[id.0];
        state.range = range; state.direction = direction; state.observed = observed;
        if old.range != range { self.schedule_selection_change(old.document, old.document_generation); }
        Ok(())
    }
    pub fn selection_points(&self, id: SelectionId) -> Result<Option<(BoundaryPoint, BoundaryPoint)>, DomError> {
        let state = self.selection(id)?;
        let Some(range) = state.range else { return Ok(None); };
        let range = self.range(range)?;
        Ok(Some(if state.direction == SelectionDirection::Forwards { (range.start, range.end) } else { (range.end, range.start) }))
    }
    /// Public Selection boundary visibility follows the document tree, not mutable ownerDocument.
    pub fn selection_visible_points(&self, id: SelectionId) -> Result<Option<(BoundaryPoint, BoundaryPoint)>, DomError> {
        let document = self.selection_document(id)?;
        Ok(self.selection_points(id)?.filter(|(anchor, focus)| self.root(anchor.node) == document && self.root(focus.node) == document))
    }
    pub fn selection_add(&mut self, id: SelectionId, range: Range) -> Result<(), DomError> {
        let document = self.selection_document(id)?;
        let boundaries = self.range(range)?;
        if self.root(boundaries.start.node) != document || self.root(boundaries.end.node) != document { return Ok(()); }
        if self.selection_visible_points(id)?.is_some() { return Ok(()); }
        self.associate_selection_range(id, Some(range), SelectionDirection::Forwards)
    }
    pub fn selection_clear(&mut self, id: SelectionId) -> Result<(), DomError> {
        self.associate_selection_range(id, None, SelectionDirection::Directionless)
    }
    pub fn selection_remove(&mut self, id: SelectionId, range: Range) -> Result<(), DomError> {
        if self.selection(id)?.range != Some(range) { return Err(DomError::NotFoundError); }
        self.selection_clear(id)
    }
    fn selection_check_point(&self, point: BoundaryPoint) -> Result<(), DomError> {
        self.check_node(point.node)?;
        if matches!(self.get(point.node).kind, NodeKind::DocumentType { .. }) { return Err(DomError::InvalidNodeTypeError); }
        if point.offset > self.node_length(point.node) { return Err(DomError::IndexSizeError); }
        Ok(())
    }
    pub fn selection_collapse_at(&mut self, id: SelectionId, point: Option<BoundaryPoint>) -> Result<(), DomError> {
        let document = self.selection_document(id)?;
        let Some(point) = point else { return self.selection_clear(id); };
        self.selection_check_point(point)?;
        if self.shadow_including_root(point.node) != document { return Ok(()); }
        let range = self.register_range(AbstractRange { start: point, end: point });
        self.associate_selection_range(id, Some(range), SelectionDirection::Directionless)
    }
    pub fn selection_collapse_to(&mut self, id: SelectionId, start: bool) -> Result<(), DomError> {
        let old = self.selection(id)?.range.ok_or(DomError::InvalidStateError)?;
        let old = self.range(old)?;
        let point = if start { old.start } else { old.end };
        let range = self.register_range(AbstractRange { start: point, end: point });
        self.associate_selection_range(id, Some(range), SelectionDirection::Directionless)
    }
    pub fn selection_extend(&mut self, id: SelectionId, point: BoundaryPoint) -> Result<(), DomError> {
        let document = self.selection_document(id)?;
        self.check_node(point.node)?;
        if self.shadow_including_root(point.node) != document { return Ok(()); }
        let (anchor, _) = self.selection_points(id)?.ok_or(DomError::InvalidStateError)?;
        self.selection_check_point(point)?;
        let ordering = self.compare_boundary_points(anchor, point);
        let boundaries = match ordering {
            Ok(Ordering::Greater) => AbstractRange { start: point, end: anchor },
            Ok(_) => AbstractRange { start: anchor, end: point },
            Err(DomError::WrongDocumentError) => AbstractRange { start: point, end: point },
            Err(error) => return Err(error),
        };
        let direction = if ordering == Ok(Ordering::Greater) { SelectionDirection::Backwards } else { SelectionDirection::Forwards };
        let range = self.register_range(boundaries);
        self.associate_selection_range(id, Some(range), direction)
    }
    pub fn selection_set_base_and_extent(&mut self, id: SelectionId, anchor: BoundaryPoint, focus: BoundaryPoint) -> Result<(), DomError> {
        let document = self.selection_document(id)?;
        self.check_node(anchor.node)?; self.check_node(focus.node)?;
        if anchor.offset > self.node_length(anchor.node) || focus.offset > self.node_length(focus.node) { return Err(DomError::IndexSizeError); }
        if self.shadow_including_root(anchor.node) != document || self.shadow_including_root(focus.node) != document { return Ok(()); }
        self.selection_check_point(anchor)?; self.selection_check_point(focus)?;
        // The normal Range setters handle distinct roots without inventing their order.
        let before = self.compare_boundary_points(anchor, focus).is_ok_and(|ordering| ordering == Ordering::Less);
        let range = self.create_range(document)?;
        let (start, end) = if before { (anchor, focus) } else { (focus, anchor) };
        self.range_set_start(range, start)?; self.range_set_end(range, end)?;
        let direction = if self.compare_boundary_points(focus, anchor) == Ok(Ordering::Less) { SelectionDirection::Backwards } else { SelectionDirection::Forwards };
        self.associate_selection_range(id, Some(range), direction)
    }
    pub fn selection_select_children(&mut self, id: SelectionId, node: NodeIdx) -> Result<(), DomError> {
        let document = self.selection_document(id)?;
        self.check_node(node)?;
        if matches!(self.get(node).kind, NodeKind::DocumentType { .. }) { return Err(DomError::InvalidNodeTypeError); }
        if self.root(node) != document { return Ok(()); }
        let range = self.register_range(AbstractRange { start: BoundaryPoint::new(node, 0), end: BoundaryPoint::new(node, self.children(node).count()) });
        self.associate_selection_range(id, Some(range), SelectionDirection::Forwards)
    }
    pub fn selection_delete_from_document(&mut self, id: SelectionId) -> Result<(), DomError> {
        if self.selection_visible_points(id)?.is_some() {
            let range = self.selection(id)?.range.unwrap();
            self.range_delete_contents(range)?;
        }
        Ok(())
    }
    pub fn selection_composed_ranges(&mut self, id: SelectionId, shadow_roots: &[NodeIdx]) -> Result<Vec<StaticRangeId>, DomError> {
        let Some(range) = self.selection(id)?.range else { return Ok(Vec::new()); };
        for &root in shadow_roots { self.check_node(root)?; if self.shadow_data(root).is_none() { return Err(DomError::TypeError); } }
        let mut range = self.range(range)?;
        for (point, end) in [(&mut range.start, false), (&mut range.end, true)] {
            loop {
                let root = self.root(point.node);
                let Some(shadow) = self.shadow_data(root) else { break; };
                if shadow_roots.iter().any(|&allowed| self.shadow_including_ancestors(allowed).contains(&root)) { break; }
                let host = shadow.host;
                let parent = self.get(host).parent.ok_or(DomError::InvalidStateError)?;
                *point = BoundaryPoint::new(parent, self.node_index(host) + usize::from(end));
            }
        }
        // Rescoping may cross roots; StaticRange only validates actual boundary nodes/offsets.
        Ok(vec![self.register_static_range(range.start, range.end)?])
    }
    // Existing native embedding API delegates to the document singleton.
    pub fn selection_add_range(&mut self, document: NodeIdx, range: Range) -> Result<(), DomError> { let id = self.document_selection(document)?; self.selection_add(id, range) }
    pub fn selection_remove_all_ranges(&mut self, document: NodeIdx) -> Result<(), DomError> { let id = self.document_selection(document)?; self.selection_clear(id) }
    pub fn selection_remove_range(&mut self, document: NodeIdx, range: Range) -> Result<(), DomError> { let id = self.document_selection(document)?; self.selection_remove(id, range) }
    pub fn selection_collapse(&mut self, document: NodeIdx, point: Option<BoundaryPoint>) -> Result<(), DomError> { let id = self.document_selection(document)?; self.selection_collapse_at(id, point) }
    pub fn selection_select_all_children(&mut self, document: NodeIdx, node: NodeIdx) -> Result<(), DomError> { let id = self.document_selection(document)?; self.selection_select_children(id, node) }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
