//! DOM Standard §6. Owned filter requests release the arena borrow while script runs.
use crate::{Dom, DomError, NodeIdx, NodeKind};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeIterator(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeWalker(usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodePointer {
    pub node: NodeIdx,
    pub before: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraversalId(usize);
impl From<NodeIterator> for TraversalId {
    fn from(id: NodeIterator) -> Self {
        Self(id.0)
    }
}
impl From<TreeWalker> for TraversalId {
    fn from(id: TreeWalker) -> Self {
        Self(id.0)
    }
}
pub struct NodeFilter;
impl NodeFilter {
    pub const ACCEPT: u16 = 1;
    pub const REJECT: u16 = 2;
    pub const SKIP: u16 = 3;
    pub const SHOW_ALL: u32 = u32::MAX;
    pub const SHOW_ELEMENT: u32 = 1;
    pub const SHOW_TEXT: u32 = 4;
    pub const SHOW_PROCESSING_INSTRUCTION: u32 = 64;
    pub const SHOW_COMMENT: u32 = 128;
    pub const SHOW_DOCUMENT: u32 = 256;
    pub const SHOW_DOCUMENT_TYPE: u32 = 512;
    pub const SHOW_DOCUMENT_FRAGMENT: u32 = 1024;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Traverse {
    Next,
    Previous,
    Parent,
    FirstChild,
    LastChild,
    NextSibling,
    PreviousSibling,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraversalStep {
    Filter {
        traversal: TraversalId,
        callback: u64,
        node: NodeIdx,
    },
    Done(Option<NodeIdx>),
}
#[derive(Debug, Clone, Copy)]
enum Kind {
    Iterator {
        reference: NodePointer,
        candidate: Option<NodePointer>,
    },
    Walker {
        current: NodeIdx,
    },
}
#[derive(Debug, Clone, Copy)]
enum State {
    Iterator(bool),
    Parent,
    Child(bool),
    Sibling(bool),
    SiblingParent(bool),
    Next,
    PreviousDescend,
    PreviousParent,
}
#[derive(Debug, Clone, Copy)]
struct Pending {
    state: State,
    node: NodeIdx,
}
#[derive(Debug)]
struct Traverser {
    root: NodeIdx,
    show: u32,
    filter: Option<u64>,
    kind: Kind,
    pending: Option<Pending>,
}
#[derive(Debug, Default)]
pub(crate) struct Traversals {
    entries: Vec<Option<Traverser>>,
}
impl Dom {
    fn traverser(&self, id: TraversalId) -> Result<&Traverser, DomError> {
        self.traversals
            .entries
            .get(id.0)
            .and_then(Option::as_ref)
            .ok_or(DomError::InvalidStateError)
    }
    fn traverser_mut(&mut self, id: TraversalId) -> Result<&mut Traverser, DomError> {
        self.traversals
            .entries
            .get_mut(id.0)
            .and_then(Option::as_mut)
            .ok_or(DomError::InvalidStateError)
    }
    fn make_traverser(
        &mut self,
        root: NodeIdx,
        show: u32,
        filter: Option<u64>,
        kind: Kind,
    ) -> Result<usize, DomError> {
        self.check_node(root)?;
        let id = self.traversals.entries.len();
        self.traversals.entries.push(Some(Traverser {
            root,
            show,
            filter,
            kind,
            pending: None,
        }));
        Ok(id)
    }
    pub fn create_node_iterator(
        &mut self,
        root: NodeIdx,
        show: u32,
        filter: Option<u64>,
    ) -> Result<NodeIterator, DomError> {
        Ok(NodeIterator(self.make_traverser(
            root,
            show,
            filter,
            Kind::Iterator {
                reference: NodePointer {
                    node: root,
                    before: true,
                },
                candidate: None,
            },
        )?))
    }
    pub fn create_tree_walker(
        &mut self,
        root: NodeIdx,
        show: u32,
        filter: Option<u64>,
    ) -> Result<TreeWalker, DomError> {
        Ok(TreeWalker(self.make_traverser(
            root,
            show,
            filter,
            Kind::Walker { current: root },
        )?))
    }
    pub fn traversal_root(&self, id: impl Into<TraversalId>) -> Result<NodeIdx, DomError> {
        Ok(self.traverser(id.into())?.root)
    }
    pub fn traversal_what_to_show(&self, id: impl Into<TraversalId>) -> Result<u32, DomError> {
        Ok(self.traverser(id.into())?.show)
    }
    pub fn traversal_filter(&self, id: impl Into<TraversalId>) -> Result<Option<u64>, DomError> {
        Ok(self.traverser(id.into())?.filter)
    }
    pub fn iterator_reference(&self, id: NodeIterator) -> Result<NodePointer, DomError> {
        match self.traverser(id.into())?.kind {
            Kind::Iterator { reference, .. } => Ok(reference),
            _ => Err(DomError::InvalidStateError),
        }
    }
    pub fn walker_current(&self, id: TreeWalker) -> Result<NodeIdx, DomError> {
        match self.traverser(id.into())?.kind {
            Kind::Walker { current } => Ok(current),
            _ => Err(DomError::InvalidStateError),
        }
    }
    pub fn walker_set_current(&mut self, id: TreeWalker, node: NodeIdx) -> Result<(), DomError> {
        self.check_node(node)?;
        self.traverser_mut(id.into())?.kind = Kind::Walker { current: node };
        Ok(())
    }
    pub fn iterator_detach(&self, id: NodeIterator) -> Result<(), DomError> {
        self.traverser(id.into()).map(|_| ())
    }
    /// Binding GC seam. Unlike legacy detach(), this invalidates the handle.
    pub fn release_traversal(&mut self, id: impl Into<TraversalId>) {
        if let Some(slot) = self.traversals.entries.get_mut(id.into().0) {
            *slot = None;
        }
    }
    fn following(&self, node: NodeIdx, root: NodeIdx, descend: bool) -> Option<NodeIdx> {
        if descend {
            if let Some(child) = self.get(node).first_child {
                return Some(child);
            }
        }
        let mut n = node;
        loop {
            if n == root {
                return None;
            }
            if let Some(next) = self.get(n).next_sibling {
                return Some(next);
            }
            n = self.get(n).parent?;
        }
    }
    fn preceding(&self, node: NodeIdx, root: NodeIdx) -> Option<NodeIdx> {
        if node == root {
            return None;
        }
        if let Some(mut n) = self.get(node).prev_sibling {
            while let Some(last) = self.get(n).last_child {
                n = last;
            }
            Some(n)
        } else {
            self.get(node).parent
        }
    }
    fn directional_child(&self, node: NodeIdx, forward: bool) -> Option<NodeIdx> {
        if forward {
            self.get(node).first_child
        } else {
            self.get(node).last_child
        }
    }
    fn directional_sibling(&self, node: NodeIdx, forward: bool) -> Option<NodeIdx> {
        if forward {
            self.get(node).next_sibling
        } else {
            self.get(node).prev_sibling
        }
    }
    fn finish_traversal(
        &mut self,
        id: TraversalId,
        result: Option<NodeIdx>,
    ) -> Result<TraversalStep, DomError> {
        let t = self.traverser_mut(id)?;
        t.pending = None;
        match &mut t.kind {
            Kind::Iterator {
                reference,
                candidate,
            } => {
                if result.is_some() {
                    *reference = candidate.unwrap();
                }
                *candidate = None;
            }
            Kind::Walker { current } => {
                if let Some(node) = result {
                    *current = node;
                }
            }
        }
        Ok(TraversalStep::Done(result))
    }
    /// Start one synchronous DOM traversal. Drive each Filter request with resume_traversal.
    /// The binding may mutate Dom during a callback; no tree snapshot is retained.
    pub fn begin_traversal(
        &mut self,
        id: impl Into<TraversalId>,
        direction: Traverse,
    ) -> Result<TraversalStep, DomError> {
        let id = id.into();
        let t = self.traverser(id)?;
        if t.pending.is_some() {
            // The active check belongs to filtering, not the traversal entry.
            // A nested operation with no candidate can return null without
            // disturbing the suspended outer callback.
            let empty = match t.kind {
                Kind::Walker { current } => match direction {
                    Traverse::Parent => current == t.root || self.get(current).parent.is_none(),
                    Traverse::FirstChild => self.get(current).first_child.is_none(),
                    Traverse::LastChild => self.get(current).last_child.is_none(),
                    Traverse::NextSibling | Traverse::PreviousSibling => self
                        .walker_sibling_candidate(id, current, direction == Traverse::NextSibling)?
                        .is_none(),
                    Traverse::Next => self.following(current, t.root, true).is_none(),
                    Traverse::Previous => self.walker_previous_candidate(id, current)?.is_none(),
                },
                Kind::Iterator { reference, .. } => match direction {
                    Traverse::Next => {
                        !reference.before && self.following(reference.node, t.root, true).is_none()
                    }
                    Traverse::Previous => {
                        reference.before && self.preceding(reference.node, t.root).is_none()
                    }
                    _ => return Err(DomError::NotSupportedError),
                },
            };
            return if empty {
                Ok(TraversalStep::Done(None))
            } else {
                Err(DomError::InvalidStateError)
            };
        }
        let root = t.root;
        let next = match t.kind {
            Kind::Iterator { reference, .. } => {
                if !matches!(direction, Traverse::Next | Traverse::Previous) {
                    return Err(DomError::NotSupportedError);
                }
                if let Kind::Iterator { candidate, .. } = &mut self.traverser_mut(id)?.kind {
                    *candidate = Some(reference);
                }
                self.iterator_candidate(id, direction == Traverse::Next)?
            }
            Kind::Walker { current } => match direction {
                Traverse::Parent => {
                    if current == root {
                        None
                    } else {
                        self.get(current).parent.map(|node| Pending {
                            node,
                            state: State::Parent,
                        })
                    }
                }
                Traverse::FirstChild | Traverse::LastChild => {
                    let forward = direction == Traverse::FirstChild;
                    self.directional_child(current, forward)
                        .map(|node| Pending {
                            node,
                            state: State::Child(forward),
                        })
                }
                Traverse::NextSibling | Traverse::PreviousSibling => {
                    self.walker_sibling_candidate(id, current, direction == Traverse::NextSibling)?
                }
                Traverse::Next => self.following(current, root, true).map(|node| Pending {
                    node,
                    state: State::Next,
                }),
                Traverse::Previous => self.walker_previous_candidate(id, current)?,
            },
        };
        self.drive_traversal(id, next, None)
    }
    pub fn resume_traversal(
        &mut self,
        id: TraversalId,
        result: Result<u16, DomError>,
    ) -> Result<TraversalStep, DomError> {
        let pending = self
            .traverser_mut(id)?
            .pending
            .take()
            .ok_or(DomError::InvalidStateError)?;
        match result {
            Ok(value) => self.drive_traversal(id, Some(pending), Some(value)),
            Err(error) => {
                self.finish_traversal(id, None)?;
                Err(error)
            }
        }
    }
    /// Exception payloads stay in the binding; cancellation clears native active state.
    pub fn cancel_traversal(&mut self, id: TraversalId) -> Result<(), DomError> {
        self.finish_traversal(id, None).map(|_| ())
    }
    fn node_show_bit(&self, node: NodeIdx) -> u32 {
        match self.get(node).kind {
            NodeKind::Element { .. } => 1,
            NodeKind::Text { .. } => 4,
            NodeKind::ProcessingInstruction { .. } => 64,
            NodeKind::Comment { .. } => 128,
            NodeKind::Document => 256,
            NodeKind::DocumentType { .. } => 512,
            NodeKind::DocumentFragment => 1024,
        }
    }
    fn drive_traversal(
        &mut self,
        id: TraversalId,
        mut next: Option<Pending>,
        mut response: Option<u16>,
    ) -> Result<TraversalStep, DomError> {
        loop {
            let Some(p) = next else {
                return self.finish_traversal(id, None);
            };
            let t = self.traverser(id)?;
            let result = if let Some(value) = response.take() {
                value
            } else if t.show & self.node_show_bit(p.node) == 0 {
                NodeFilter::SKIP
            } else if let Some(callback) = t.filter {
                self.traverser_mut(id)?.pending = Some(p);
                return Ok(TraversalStep::Filter {
                    traversal: id,
                    callback,
                    node: p.node,
                });
            } else {
                NodeFilter::ACCEPT
            };
            let root = self.traverser(id)?.root;
            let accept = result == NodeFilter::ACCEPT;
            next = match p.state {
                State::Iterator(forward) => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    self.iterator_candidate(id, forward)?
                }
                State::Parent => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    if p.node == root {
                        None
                    } else {
                        self.get(p.node).parent.map(|node| Pending {
                            node,
                            state: State::Parent,
                        })
                    }
                }
                State::Child(forward) => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    if result == NodeFilter::SKIP
                        && self.directional_child(p.node, forward).is_some()
                    {
                        Some(Pending {
                            node: self.directional_child(p.node, forward).unwrap(),
                            state: p.state,
                        })
                    } else {
                        self.walker_child_side(id, p.node, forward)?
                    }
                }
                State::Sibling(forward) => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    if result != NodeFilter::REJECT
                        && self.directional_child(p.node, forward).is_some()
                    {
                        Some(Pending {
                            node: self.directional_child(p.node, forward).unwrap(),
                            state: p.state,
                        })
                    } else {
                        self.walker_sibling_candidate(id, p.node, forward)?
                    }
                }
                State::SiblingParent(forward) => {
                    if accept {
                        return self.finish_traversal(id, None);
                    }
                    self.walker_sibling_candidate(id, p.node, forward)?
                }
                State::Next => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    self.following(p.node, root, result != NodeFilter::REJECT)
                        .map(|node| Pending {
                            node,
                            state: State::Next,
                        })
                }
                State::PreviousDescend => {
                    if result != NodeFilter::REJECT && self.get(p.node).last_child.is_some() {
                        Some(Pending {
                            node: self.get(p.node).last_child.unwrap(),
                            state: State::PreviousDescend,
                        })
                    } else {
                        if accept {
                            return self.finish_traversal(id, Some(p.node));
                        }
                        self.walker_previous_candidate(id, p.node)?
                    }
                }
                State::PreviousParent => {
                    if accept {
                        return self.finish_traversal(id, Some(p.node));
                    }
                    self.walker_previous_candidate(id, p.node)?
                }
            };
        }
    }
    fn iterator_candidate(
        &mut self,
        id: TraversalId,
        forward: bool,
    ) -> Result<Option<Pending>, DomError> {
        let t = self.traverser(id)?;
        let Kind::Iterator {
            candidate: Some(mut p),
            ..
        } = t.kind
        else {
            return Err(DomError::InvalidStateError);
        };
        if forward {
            if !p.before {
                let Some(next) = self.following(p.node, t.root, true) else {
                    return Ok(None);
                };
                p.node = next;
            }
            p.before = false;
        } else {
            if p.before {
                let Some(prev) = self.preceding(p.node, t.root) else {
                    return Ok(None);
                };
                p.node = prev;
            }
            p.before = true;
        }
        if let Kind::Iterator { candidate, .. } = &mut self.traverser_mut(id)?.kind {
            *candidate = Some(p);
        }
        Ok(Some(Pending {
            node: p.node,
            state: State::Iterator(forward),
        }))
    }
    fn walker_child_side(
        &self,
        id: TraversalId,
        mut node: NodeIdx,
        forward: bool,
    ) -> Result<Option<Pending>, DomError> {
        let t = self.traverser(id)?;
        let Kind::Walker { current } = t.kind else {
            return Err(DomError::InvalidStateError);
        };
        loop {
            if let Some(sibling) = self.directional_sibling(node, forward) {
                return Ok(Some(Pending {
                    node: sibling,
                    state: State::Child(forward),
                }));
            }
            let Some(parent) = self.get(node).parent else {
                return Ok(None);
            };
            if parent == current || parent == t.root {
                return Ok(None);
            }
            node = parent;
        }
    }
    fn walker_sibling_candidate(
        &self,
        id: TraversalId,
        node: NodeIdx,
        forward: bool,
    ) -> Result<Option<Pending>, DomError> {
        let root = self.traverser(id)?.root;
        if node == root {
            return Ok(None);
        }
        if let Some(sibling) = self.directional_sibling(node, forward) {
            return Ok(Some(Pending {
                node: sibling,
                state: State::Sibling(forward),
            }));
        }
        Ok(self
            .get(node)
            .parent
            .filter(|n| *n != root)
            .map(|node| Pending {
                node,
                state: State::SiblingParent(forward),
            }))
    }
    fn walker_previous_candidate(
        &self,
        id: TraversalId,
        node: NodeIdx,
    ) -> Result<Option<Pending>, DomError> {
        if node == self.traverser(id)?.root {
            return Ok(None);
        }
        if let Some(prev) = self.get(node).prev_sibling {
            return Ok(Some(Pending {
                node: prev,
                state: State::PreviousDescend,
            }));
        }
        Ok(self.get(node).parent.map(|node| Pending {
            node,
            state: State::PreviousParent,
        }))
    }
    fn adjust_pointer(&self, root: NodeIdx, removed: NodeIdx, p: NodePointer) -> NodePointer {
        if !self.contains(removed, Some(p.node)) || self.contains(removed, Some(root)) {
            return p;
        }
        if p.before {
            if let Some(next) = self.following(removed, root, false) {
                return NodePointer {
                    node: next,
                    before: true,
                };
            }
        }
        NodePointer {
            node: self
                .preceding(removed, root)
                .expect("removed descendant has predecessor"),
            before: false,
        }
    }
    pub(crate) fn iterators_remove(&mut self, removed: NodeIdx) {
        for index in 0..self.traversals.entries.len() {
            let Some(t) = self.traversals.entries[index].as_ref() else {
                continue;
            };
            if let Kind::Iterator {
                reference,
                candidate,
            } = t.kind
            {
                let reference = self.adjust_pointer(t.root, removed, reference);
                let candidate = candidate.map(|p| self.adjust_pointer(t.root, removed, p));
                self.traversals.entries[index].as_mut().unwrap().kind = Kind::Iterator {
                    reference,
                    candidate,
                };
            }
        }
    }
}
#[cfg(test)]
#[path = "traversal_tests.rs"]
mod tests;
