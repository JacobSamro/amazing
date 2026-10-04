//! Genuine Window target kind and current Document/browsing-context associations.
//! Window identity survives retirement; a Document event path does not.
use crate::{Dom, DomError, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug)]
struct Association { document: NodeIdx, generation: u64, slot: u64 }

#[derive(Default, Debug)]
pub(crate) struct WindowEventTargets {
    slots: Vec<u64>,
    associations: Vec<Association>,
    generation: u64,
}

impl Dom {
    /// Allocate one immutable native target for an actual Window global.
    /// A retargetable WindowProxy is not itself this native identity.
    pub fn create_window_event_target(&mut self) -> u64 {
        let slot = self.create_event_target();
        self.window_event_targets.slots.push(slot);
        slot
    }
    pub fn is_window_event_target(&self, slot: u64) -> bool {
        self.has_event_target(slot) && self.window_event_targets.slots.contains(&slot)
    }
    fn window_document_generation(&self, document: NodeIdx) -> Result<u64, DomError> {
        self.check_node(document)?;
        if !matches!(self.get(document).kind, NodeKind::Document) {
            return Err(DomError::TypeError);
        }
        self.node_generation(document).ok_or(DomError::InvalidStateError)
    }
    fn check_window_association(&self, document: NodeIdx, generation: u64, slot: u64) -> Result<(), DomError> {
        if self.window_document_generation(document)? != generation || !self.has_event_target(slot) {
            return Err(DomError::InvalidStateError);
        }
        for association in &self.window_event_targets.associations {
            if self.node_generation(association.document) != Some(association.generation) { continue; }
            if (association.document == document && association.slot != slot)
                || (association.slot == slot && association.document != document) {
                return Err(DomError::InvalidStateError);
            }
        }
        Ok(())
    }
    /// Register an actual current browsing-context association before author callbacks.
    /// Repeated identical registration is inert; a Window cannot belong to two Documents.
    pub fn associate_document_window_event_target(&mut self, document: NodeIdx, generation: u64, slot: u64) -> Result<(), DomError> {
        self.check_window_association(document, generation, slot)?;
        if !self.is_window_event_target(slot) { return Err(DomError::InvalidStateError); }
        if self.window_event_targets.associations.iter().any(|a|
            a.document == document && a.generation == generation && a.slot == slot) { return Ok(()); }
        let association = Association { document, generation, slot };
        if let Some(old) = self.window_event_targets.associations.iter_mut().find(|a| a.document == document) {
            *old = association;
        } else { self.window_event_targets.associations.push(association); }
        self.window_event_targets.generation = self.window_event_targets.generation.wrapping_add(1);
        Ok(())
    }
    pub fn document_window_event_target(&self, document: NodeIdx) -> Option<u64> {
        let generation = self.window_document_generation(document).ok()?;
        self.window_event_targets.associations.iter().find(|a|
            a.document == document && a.generation == generation).map(|a| a.slot)
    }
    /// Exact retirement CAS: a delayed callback cannot clear a replacement Document.
    /// This leaves the genuine old Window target and its listeners intact.
    pub fn dissociate_document_window_event_target(&mut self, document: NodeIdx, generation: u64, expected_slot: u64) -> bool {
        let Some(index) = self.window_event_targets.associations.iter().position(|a|
            a.document == document && a.generation == generation && a.slot == expected_slot) else { return false; };
        self.window_event_targets.associations.remove(index);
        self.window_event_targets.generation = self.window_event_targets.generation.wrapping_add(1);
        true
    }
    /// Determine the actual HTML event-handler target for an event type
    /// (without the `on` prefix). IDL and content handlers share this rule.
    pub fn event_handler_target(&self, node: NodeIdx, event_type: &str) -> Result<Option<crate::events::EventTarget>, DomError> {
        self.check_node(node)?;
        let reflects_window = matches!(event_type,
            "blur" | "error" | "focus" | "load" | "resize" | "scroll"
            | "afterprint" | "beforeprint" | "beforeunload" | "hashchange"
            | "languagechange" | "message" | "messageerror" | "offline" | "online"
            | "pageswap" | "pagehide" | "pagereveal" | "pageshow" | "popstate"
            | "rejectionhandled" | "storage" | "unhandledrejection" | "unload");
        let body = self.namespace(node) == Some(crate::Namespace::Html)
            && matches!(self.local_name(node), Some("body" | "frameset"));
        if !body || !reflects_window { return Ok(Some(crate::events::EventTarget::Node(node))); }
        // HTML asks whether the node Document is active, not whether this is
        // its current body element or whether this particular body is connected.
        Ok(self.document_window_event_target(self.get(node).owner_document)
            .map(crate::events::EventTarget::External))
    }
    /// Path-sensitive caches must include this alongside listener/tree generations.
    pub fn window_event_association_generation(&self) -> u64 { self.window_event_targets.generation }

    /// Compatibility entry point for the explicitly installed root Window.
    pub fn set_window_event_target(&mut self, slot: u64) -> Result<(), DomError> {
        let document = NodeIdx(0);
        let generation = self.window_document_generation(document)?;
        self.check_window_association(document, generation, slot)?;
        if !self.is_window_event_target(slot) { self.window_event_targets.slots.push(slot); }
        self.associate_document_window_event_target(document, generation, slot)
    }
    pub fn window_event_target(&self) -> Option<u64> { self.document_window_event_target(NodeIdx(0)) }
}

#[cfg(test)]
#[path = "window_event_targets_tests.rs"]
mod tests;
