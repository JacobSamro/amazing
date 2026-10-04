//! Native CSS Shadow §4.1 flat-tree ancestry, without exposing closed roots to script.
use crate::{Dom, DomError, NodeIdx, NodeKind};

enum ParentStep {
    Parent(NodeIdx),
    Root,
    Suppressed,
}

impl Dom {
    /// Actual flat-tree parent, including closed/manual slot assignment.
    ///
    /// An omitted light/fallback branch has no flat-tree ancestry, including
    /// its descendants. Slots remain parents; display:contents box flattening
    /// is a separate operation. Document and fragment roots are boundaries.
    /// Detached element subtrees retain their internal ancestry; consumers
    /// must separately establish connection/activity before installed styling.
    /// This is a native current-node query. Saved node handles must validate
    /// their generation before calling; no user code or mutation occurs here.
    pub fn flat_tree_parent(&self, node: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        self.check_node(node)?;
        let first = match self.flat_parent_step(node)? {
            ParentStep::Parent(parent) => parent,
            ParentStep::Root | ParentStep::Suppressed => return Ok(None),
        };
        let mut current = first;
        // Validate the entire projected ancestry: an ordinary parent below an
        // omitted fallback/light branch is not a rendered flat parent either.
        for _ in 0..self.node_count() {
            match self.flat_parent_step(current)? {
                ParentStep::Parent(parent) => current = parent,
                ParentStep::Root => return Ok(Some(first)),
                ParentStep::Suppressed => return Ok(None),
            }
        }
        Err(DomError::InvalidStateError)
    }

    fn flat_parent_step(&self, node: NodeIdx) -> Result<ParentStep, DomError> {
        self.check_node(node)?;
        if matches!(self.get(node).kind, NodeKind::Document | NodeKind::DocumentFragment) {
            return Ok(ParentStep::Root);
        }
        let Some(parent) = self.get(node).parent else { return Ok(ParentStep::Root); };
        self.check_node(parent)?;
        if let Some(shadow) = self.shadow_data(parent) {
            self.check_node(shadow.host)?;
            return Ok(ParentStep::Parent(shadow.host));
        }
        if self.shadow_root_of_host(parent).is_some() {
            return Ok(match self.find_slot(node, false) {
                Some(slot) => ParentStep::Parent(slot),
                None => ParentStep::Suppressed,
            });
        }
        if self.is_html_slot(parent) && !self.assigned_nodes(parent, false, false)?.is_empty() {
            return Ok(ParentStep::Suppressed);
        }
        if matches!(self.get(parent).kind, NodeKind::Document | NodeKind::DocumentFragment) {
            return Ok(ParentStep::Root);
        }
        Ok(ParentStep::Parent(parent))
    }
}

#[cfg(test)]
#[path = "flat_tree_tests.rs"]
mod tests;
