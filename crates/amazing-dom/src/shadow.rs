//! DOM Standard §4.2.2 and §4.8: shadow trees and slot assignment.
//! A ShadowRoot is an arena DocumentFragment with separate, private host data.
use crate::{custom_elements::CustomElementState, Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowMode {
    Open,
    Closed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotAssignment {
    Named,
    Manual,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowOptions {
    pub mode: ShadowMode,
    pub delegates_focus: bool,
    pub slot_assignment: SlotAssignment,
    pub clonable: bool,
    pub serializable: bool,
}
#[derive(Clone, Debug)]
pub struct ShadowRootData {
    pub root: NodeIdx,
    pub host: NodeIdx,
    pub options: ShadowOptions,
    pub declarative: bool,
    pub available_to_element_internals: bool,
    pub keep_custom_element_registry_null: bool,
}
#[derive(Clone, Debug)]
pub struct SlotState {
    pub slot: NodeIdx,
    pub manual: Vec<NodeIdx>,
    pub assigned: Vec<NodeIdx>,
}

pub(crate) fn custom_element_name(name: &str) -> bool {
    // HTML's valid custom element name uses the shared element-local-name
    // validator, then adds ASCII-case, hyphen, and reserved-name constraints.
    let reserved = [
        "annotation-xml",
        "color-profile",
        "font-face",
        "font-face-src",
        "font-face-uri",
        "font-face-format",
        "font-face-name",
        "missing-glyph",
    ];
    crate::namespace::valid_element_local_name(name)
        && name.contains('-')
        && !reserved.contains(&name)
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.chars().any(|c| c.is_ascii_uppercase())
}

impl Dom {
    pub fn is_shadow_root(&self, node: NodeIdx) -> bool {
        self.shadow_roots.iter().any(|data| data.root == node)
    }
    pub fn shadow_data(&self, root: NodeIdx) -> Option<&ShadowRootData> {
        self.shadow_roots.iter().find(|data| data.root == root)
    }
    pub fn shadow_root_of_host(&self, host: NodeIdx) -> Option<NodeIdx> {
        self.shadow_roots
            .iter()
            .find(|data| data.host == host)
            .map(|data| data.root)
    }
    /// The Element.shadowRoot getter hides closed roots; internal algorithms
    /// use `shadow_root_of_host` instead.
    pub fn visible_shadow_root(&self, host: NodeIdx) -> Option<NodeIdx> {
        self.shadow_roots
            .iter()
            .find(|data| data.host == host && data.options.mode == ShadowMode::Open)
            .map(|data| data.root)
    }
    pub fn attach_shadow(
        &mut self,
        host: NodeIdx,
        options: ShadowOptions,
    ) -> Result<NodeIdx, DomError> {
        self.check_node(host)?;
        let Some(name) = self.local_name(host) else {
            return Err(DomError::TypeError);
        };
        if self.namespace(host) != Some(Namespace::Html)
            || !(matches!(
                name,
                "article"
                    | "aside"
                    | "blockquote"
                    | "body"
                    | "div"
                    | "footer"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "header"
                    | "main"
                    | "nav"
                    | "p"
                    | "section"
                    | "span"
            ) || custom_element_name(name))
        {
            return Err(DomError::NotSupportedError);
        }
        let definition = self.custom_element_definition_of(host).or_else(|| {
            self.custom_element_definitions(self.get(host).owner_document).into_iter()
                .find(|(_, definition)| definition.local_name == name
                    && (definition.name == name
                        || self.get_attribute_ns(host, None, "is") == Some(definition.name.as_str())))
                .map(|(id, _)| id)
        });
        if definition.is_some_and(|id| self.custom_element_definition(id)
            .is_some_and(|definition| definition.disable_shadow)) {
            return Err(DomError::NotSupportedError);
        }
        if let Some(root) = self.shadow_root_of_host(host) {
            let data = self.shadow_data(root).expect("attached shadow root");
            if !data.declarative || data.options.mode != options.mode {
                return Err(DomError::NotSupportedError);
            }
            self.replace_all(root, None)?;
            self.shadow_roots.iter_mut().find(|data| data.root == root)
                .expect("attached shadow root").declarative = false;
            return Ok(root);
        }
        let root = self.create_document_fragment(self.get(host).owner_document)?;
        let available_to_element_internals = matches!(self.custom_element_state(host),
            CustomElementState::Precustomized | CustomElementState::Custom);
        self.shadow_roots.push(ShadowRootData {
            root,
            host,
            options,
            declarative: false,
            available_to_element_internals,
            keep_custom_element_registry_null: false,
        });
        self.record_render_full(host, "shadow root attached");
        self.reassign_all_slots();
        Ok(root)
    }
    /// Parser-owned declarative path. The parser moves its template contents
    /// into the returned root after this operation succeeds.
    pub fn attach_declarative_shadow(&mut self, host: NodeIdx, options: ShadowOptions,
        keep_custom_element_registry_null: bool) -> Result<NodeIdx, DomError> {
        if self.shadow_root_of_host(host).is_some() {
            return Err(DomError::NotSupportedError);
        }
        let root = self.attach_shadow(host, options)?;
        let data = self.shadow_roots.iter_mut().find(|data| data.root == root)
            .expect("attached shadow root");
        data.declarative = true;
        data.available_to_element_internals = true;
        data.keep_custom_element_registry_null = keep_custom_element_registry_null;
        Ok(root)
    }
    /// Ordinary tree root unless composed is requested; shadow roots cross to
    /// their host without changing parentNode or the light-tree child list.
    pub fn root_node(&self, node: NodeIdx, composed: bool) -> Result<NodeIdx, DomError> {
        self.check_node(node)?;
        let mut root = self.root(node);
        if composed {
            while let Some(data) = self.shadow_data(root) {
                root = self.root(data.host);
            }
        }
        Ok(root)
    }
    pub fn shadow_including_root(&self, node: NodeIdx) -> NodeIdx {
        self.root_node(node, true).expect("valid arena node")
    }
    pub fn shadow_including_ancestors(&self, node: NodeIdx) -> Vec<NodeIdx> {
        let mut result = Vec::new();
        let mut cursor = Some(node);
        while let Some(id) = cursor {
            result.push(id);
            cursor = self
                .get(id)
                .parent
                .or_else(|| self.shadow_data(id).map(|data| data.host));
        }
        result
    }
    /// DOM Standard "host-including inclusive ancestor": like
    /// `shadow_including_ancestors`, but a template-contents `DocumentFragment`
    /// also hops to its owning `<template>` element when its own tree is
    /// exhausted, alongside a shadow root hopping to its host. Used by the
    /// tree builder's adoption-agency algorithm to detect a would-be cycle
    /// crossing a shadow or template-content boundary before performing a
    /// move that a plain same-tree ancestor walk would miss.
    /// HTML-40 (1) follow-up: cycle-safe. An *ordinary* parent-child cycle
    /// can never exist in a live `Dom` (`attach_before`'s own hard assert),
    /// but the host-including hops this walks (shadow root -> host,
    /// template contents -> owning template) are not parent-child edges
    /// that assert ever sees -- a script can construct a live *host-
    /// including* cycle (`adoption-agency-reparenting-shadow-cycle.html`
    /// does exactly this: the shadow host ends up an ordinary descendant of
    /// a node inside its own shadow root's content). This is precisely the
    /// shape `is_host_including_inclusive_ancestor` exists to *detect*, so
    /// it must terminate on one rather than loop forever -- stopping and
    /// returning the (necessarily incomplete, but that's fine: the caller
    /// only asks "does this chain contain X", and a cycle not yet
    /// containing X by the time it repeats never will) chain so far the
    /// moment a node repeats.
    pub fn host_including_ancestors(&self, node: NodeIdx) -> Vec<NodeIdx> {
        let mut result = Vec::new();
        let mut cursor = Some(node);
        while let Some(id) = cursor {
            if result.contains(&id) { break; }
            result.push(id);
            cursor = self.host_including_parent(id);
        }
        result
    }

    /// One step of `host_including_ancestors`: `node`'s ordinary DOM
    /// parent, or (when `node` has none, i.e. it is itself a root of some
    /// kind) whichever host-including hop applies -- a shadow root to its
    /// host, or a template's contents fragment to its owning `<template>`.
    /// Exposed on its own for `markup.rs`'s `IncrementalDocumentImport::
    /// reverse_sync`, which needs to walk this same notion of "parent" one
    /// step at a time while mirroring not-yet-imported ancestors, not just
    /// test full-ancestor-chain membership.
    pub fn host_including_parent(&self, node: NodeIdx) -> Option<NodeIdx> {
        self.get(node).parent
            .or_else(|| self.shadow_data(node).map(|data| data.host))
            .or_else(|| self.template_of_contents(node))
    }
    /// Whether `candidate` is a host-including inclusive ancestor of `node`
    /// (DOM Standard): `candidate` is `node` itself, an ordinary ancestor, or
    /// reached by additionally crossing a shadow-root-to-host or
    /// template-contents-to-owning-template boundary.
    pub fn is_host_including_inclusive_ancestor(&self, candidate: NodeIdx, node: NodeIdx) -> bool {
        self.host_including_ancestors(node).contains(&candidate)
    }
    pub fn shadow_including_tree_order(&self, root: NodeIdx) -> Vec<NodeIdx> {
        let mut result = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            result.push(id);
            let mut children: Vec<_> = self.children(id).collect();
            if let Some(shadow) = self.shadow_root_of_host(id) {
                children.insert(0, shadow);
            }
            stack.extend(children.into_iter().rev());
        }
        result
    }
    pub fn is_html_slot(&self, node: NodeIdx) -> bool {
        self.namespace(node) == Some(Namespace::Html) && self.local_name(node) == Some("slot")
    }
    fn is_slottable(&self, node: NodeIdx) -> bool {
        matches!(
            self.get(node).kind,
            NodeKind::Element { .. } | NodeKind::Text { .. }
        )
    }
    fn slot_name(&self, node: NodeIdx) -> &str {
        self.get_attribute_ns(node, None, "name").unwrap_or("")
    }
    fn slottable_name(&self, node: NodeIdx) -> &str {
        if matches!(self.get(node).kind, NodeKind::Element { .. }) {
            self.get_attribute_ns(node, None, "slot").unwrap_or("")
        } else {
            ""
        }
    }
    fn slots_in_root(&self, root: NodeIdx) -> Vec<NodeIdx> {
        self.tree_order(root)
            .into_iter()
            .skip(1)
            .filter(|id| self.is_html_slot(*id))
            .collect()
    }
    fn slot_state(&self, slot: NodeIdx) -> Option<&SlotState> {
        self.slot_states.iter().find(|entry| entry.slot == slot)
    }
    pub fn find_slot(&self, slottable: NodeIdx, open_only: bool) -> Option<NodeIdx> {
        if !self.is_slottable(slottable) {
            return None;
        }
        let host = self.get(slottable).parent?;
        let root = self.shadow_root_of_host(host)?;
        let data = self.shadow_data(root)?;
        if open_only && data.options.mode != ShadowMode::Open {
            return None;
        }
        let slots = self.slots_in_root(root);
        match data.options.slot_assignment {
            SlotAssignment::Named => slots
                .into_iter()
                .find(|slot| self.slot_name(*slot) == self.slottable_name(slottable)),
            SlotAssignment::Manual => slots.into_iter().find(|slot| {
                self.slot_state(*slot)
                    .is_some_and(|state| state.manual.contains(&slottable))
            }),
        }
    }
    /// Element.assignedSlot and Text.assignedSlot hide a slot in a closed root.
    pub fn assigned_slot(&self, node: NodeIdx) -> Option<NodeIdx> {
        self.find_slot(node, true)
    }
    pub fn assign_slot(&mut self, slot: NodeIdx, nodes: &[NodeIdx]) -> Result<(), DomError> {
        self.check_node(slot)?;
        if !self.is_html_slot(slot) {
            return Err(DomError::TypeError);
        }
        for &node in nodes {
            self.check_node(node)?;
            if !self.is_slottable(node) {
                return Err(DomError::TypeError);
            }
        }
        for state in &mut self.slot_states {
            state.manual.retain(|id| !nodes.contains(id));
        }
        let mut unique = Vec::new();
        for &node in nodes {
            if !unique.contains(&node) {
                unique.push(node);
            }
        }
        if let Some(state) = self.slot_states.iter_mut().find(|state| state.slot == slot) {
            state.manual = unique;
        } else {
            self.slot_states.push(SlotState {
                slot,
                manual: unique,
                assigned: vec![],
            });
        }
        self.reassign_all_slots();
        Ok(())
    }
    pub fn assigned_nodes(
        &self,
        slot: NodeIdx,
        flatten: bool,
        elements_only: bool,
    ) -> Result<Vec<NodeIdx>, DomError> {
        self.check_node(slot)?;
        if !self.is_html_slot(slot) {
            return Err(DomError::TypeError);
        }
        let mut result = if flatten {
            self.flattened_slottables(slot, 0)
        } else {
            self.slot_state(slot)
                .map_or_else(Vec::new, |state| state.assigned.clone())
        };
        if elements_only {
            result.retain(|id| matches!(self.get(*id).kind, NodeKind::Element { .. }));
        }
        Ok(result)
    }
    fn flattened_slottables(&self, slot: NodeIdx, depth: usize) -> Vec<NodeIdx> {
        if depth > self.nodes.len() || !self.is_shadow_root(self.root(slot)) {
            return vec![];
        }
        let mut nodes = self
            .slot_state(slot)
            .map_or_else(Vec::new, |state| state.assigned.clone());
        if nodes.is_empty() {
            nodes = self
                .children(slot)
                .filter(|id| self.is_slottable(*id))
                .collect();
        }
        let mut result = Vec::new();
        for node in nodes {
            if self.is_html_slot(node) && self.is_shadow_root(self.root(node)) {
                result.extend(self.flattened_slottables(node, depth + 1));
            } else {
                result.push(node);
            }
        }
        result
    }
    /// Recomputes the affected slot lists from tree order. This bounded first
    /// implementation uses a full scan after native tree/attribute mutations.
    pub(crate) fn reassign_all_slots(&mut self) {
        let roots: Vec<_> = self.shadow_roots.iter().map(|data| data.root).collect();
        let mut active = Vec::new();
        for root in roots {
            for slot in self.slots_in_root(root) {
                active.push(slot);
                let host = self.shadow_data(root).unwrap().host;
                let desired: Vec<_> = if self.shadow_data(root).unwrap().options.slot_assignment
                    == SlotAssignment::Manual
                {
                    self.slot_state(slot).map_or_else(Vec::new, |state| {
                        state
                            .manual
                            .iter()
                            .copied()
                            .filter(|id| {
                                self.get(*id).parent == Some(host)
                                    && self.find_slot(*id, false) == Some(slot)
                            })
                            .collect()
                    })
                } else {
                    self.children(host)
                        .filter(|id| {
                            self.is_slottable(*id) && self.find_slot(*id, false) == Some(slot)
                        })
                        .collect()
                };
                let index = self.slot_states.iter().position(|state| state.slot == slot);
                let old = index.map_or_else(Vec::new, |i| self.slot_states[i].assigned.clone());
                if old != desired {
                    let old_handles = old.iter().map(|id| self.render_handle(*id)).collect();
                    let new_handles = desired.iter().map(|id| self.render_handle(*id)).collect();
                    self.record_render_mutation(slot,
                        crate::render_mutation::RenderMutationKind::SlotAssignment {
                            old: old_handles, new: new_handles,
                        });
                    self.queue_slotchange(slot);
                }
                if let Some(index) = index {
                    self.slot_states[index].assigned = desired;
                } else {
                    self.slot_states.push(SlotState {
                        slot,
                        manual: vec![],
                        assigned: desired,
                    });
                }
            }
        }
        let removed: Vec<_> = self
            .slot_states
            .iter()
            .filter(|state| !active.contains(&state.slot) && !state.assigned.is_empty())
            .map(|state| state.slot)
            .collect();
        for slot in removed {
            if let Some(state) = self.slot_state(slot) {
                let old = state.assigned.iter().map(|id| self.render_handle(*id)).collect();
                self.record_render_mutation(slot,
                    crate::render_mutation::RenderMutationKind::SlotAssignment {
                        old, new: Vec::new(),
                    });
            }
            self.queue_slotchange(slot);
        }
        for state in &mut self.slot_states {
            if !active.contains(&state.slot) {
                state.assigned.clear();
            }
        }
    }
}
#[cfg(test)]
#[path = "shadow_tests.rs"]
mod tests;
