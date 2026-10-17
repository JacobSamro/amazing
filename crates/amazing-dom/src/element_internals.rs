//! HTML §4.13.7 `ElementInternals` and form-associated custom elements.
//! Native state only: values that are JS objects (File, FormData) are held as
//! opaque shared payloads that the bindings layer downcasts.
use crate::custom_elements::{CustomElementReaction, CustomElementState};
use crate::events::CallbackId;
use crate::forms::ValidityState;
use crate::render_mutation::NodeGeneration;
use crate::{Dom, DomError, NodeIdx};
use std::any::Any;
use std::sync::Arc;

/// A submission value or restore state set by `setFormValue()`.
#[derive(Clone)]
pub enum SubmissionValue {
    Text(Vec<u16>),
    Opaque(Arc<dyn Any + Send + Sync>),
}
impl std::fmt::Debug for SubmissionValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => write!(f, "Text({} units)", text.len()),
            Self::Opaque(_) => f.write_str("Opaque"),
        }
    }
}

#[derive(Debug, Default)]
pub struct InternalsState {
    pub(crate) submission: Option<SubmissionValue>,
    pub(crate) restore_state: Option<SubmissionValue>,
    pub(crate) validity: ValidityState,
    pub(crate) message: Vec<u16>,
    pub(crate) anchor: Option<NodeGeneration>,
    pub(crate) states: Vec<Vec<u16>>,
    pub(crate) aria: Vec<(&'static str, Vec<u16>)>,
    /// Explicitly set ARIA element references: (IDL name, nodes, cached JS array root).
    pub(crate) aria_elements: Vec<(&'static str, Vec<NodeGeneration>, Option<CallbackId>)>,
}

impl Dom {
    fn internals_index(&self, element: NodeIdx) -> Option<usize> {
        let generation = self.node_generation(element)?;
        self.custom_elements.internals.iter()
            .position(|(owner, _)| owner.node == element && owner.generation == generation)
    }
    fn internals_state(&self, element: NodeIdx) -> Option<&InternalsState> {
        self.internals_index(element).map(|index| &self.custom_elements.internals[index].1)
    }
    fn internals_state_mut(&mut self, element: NodeIdx) -> Result<&mut InternalsState, DomError> {
        let index = self.internals_index(element).ok_or(DomError::InvalidStateError)?;
        Ok(&mut self.custom_elements.internals[index].1)
    }
    pub fn internals_attached(&self, element: NodeIdx) -> bool {
        !self.custom_elements.internals.is_empty() && self.internals_index(element).is_some()
    }

    /// A custom element (or one mid-upgrade) whose definition is form-associated.
    pub fn is_form_associated_custom_element(&self, element: NodeIdx) -> bool {
        if self.custom_elements.face_definitions == 0 { return false; }
        if !matches!(self.custom_element_state(element),
            CustomElementState::Custom | CustomElementState::Precustomized) { return false; }
        self.custom_element_definition_of(element)
            .and_then(|id| self.custom_element_definition(id))
            .is_some_and(|definition| definition.form_associated)
    }

    /// `HTMLElement.attachInternals()`.
    pub fn attach_internals(&mut self, element: NodeIdx) -> Result<(), DomError> {
        self.check_node(element)?;
        if self.custom_element_is_value(element).ok().flatten().is_some() {
            return Err(DomError::NotSupportedError);
        }
        if !matches!(self.custom_element_state(element),
            CustomElementState::Custom | CustomElementState::Precustomized) {
            return Err(DomError::NotSupportedError);
        }
        let definition = self.custom_element_definition_of(element)
            .and_then(|id| self.custom_element_definition(id))
            .ok_or(DomError::NotSupportedError)?;
        if definition.disable_internals || self.internals_attached(element) {
            return Err(DomError::NotSupportedError);
        }
        let owner = self.render_handle(element);
        self.custom_elements.internals.push((owner, InternalsState::default()));
        Ok(())
    }

    /// `ElementInternals.shadowRoot`.
    pub fn internals_shadow_root(&self, element: NodeIdx) -> Option<NodeIdx> {
        let root = self.shadow_root_of_host(element)?;
        self.shadow_data(root).filter(|data| data.available_to_element_internals).map(|_| root)
    }

    fn require_face(&self, element: NodeIdx) -> Result<(), DomError> {
        if self.is_form_associated_custom_element(element) { Ok(()) } else { Err(DomError::NotSupportedError) }
    }
    pub fn internals_set_form_value(&mut self, element: NodeIdx, value: Option<SubmissionValue>,
        state: Option<SubmissionValue>) -> Result<(), DomError> {
        self.require_face(element)?;
        let internals = self.internals_state_mut(element)?;
        internals.submission = value;
        internals.restore_state = state;
        Ok(())
    }
    pub fn internals_submission_value(&self, element: NodeIdx) -> Option<SubmissionValue> {
        self.internals_state(element).and_then(|state| state.submission.clone())
    }
    pub fn internals_set_validity(&mut self, element: NodeIdx, flags: ValidityState, message: Vec<u16>,
        anchor: Option<NodeIdx>) -> Result<(), DomError> {
        self.require_face(element)?;
        if let Some(anchor) = anchor {
            self.check_node(anchor)?;
            if !self.shadow_including_descendant(element, anchor) { return Err(DomError::NotFoundError); }
        }
        let anchor = anchor.map(|anchor| self.render_handle(anchor));
        let valid = flags.valid();
        let internals = self.internals_state_mut(element)?;
        internals.validity = flags;
        internals.message = if valid { Vec::new() } else { message };
        internals.anchor = if valid { None } else { anchor };
        Ok(())
    }
    pub fn internals_anchor(&self, element: NodeIdx) -> Option<NodeIdx> {
        let anchor = self.internals_state(element)?.anchor?;
        (self.node_generation(anchor.node) == Some(anchor.generation)).then_some(anchor.node)
    }
    fn shadow_including_descendant(&self, ancestor: NodeIdx, node: NodeIdx) -> bool {
        // Inclusive: an element may anchor its own validity message.
        let mut current = node;
        loop {
            if current == ancestor { return true; }
            match self.get(current).parent {
                Some(parent) => current = parent,
                None => match self.shadow_roots.iter().find(|data| data.root == current) {
                    Some(data) => current = data.host,
                    None => return false,
                },
            }
        }
    }
    /// Constraint validation flags of an attached form-associated element.
    pub fn face_validity(&self, element: NodeIdx) -> ValidityState {
        self.internals_state(element).map(|state| state.validity.clone()).unwrap_or_default()
    }
    pub fn face_validation_message(&self, element: NodeIdx) -> Vec<u16> {
        self.internals_state(element).map(|state| state.message.clone()).unwrap_or_default()
    }
    /// Actually disabled: own `disabled` attribute, or inside a disabled fieldset
    /// outside that fieldset's first legend.
    pub fn is_actually_disabled(&self, node: NodeIdx) -> bool {
        if self.get_attribute_ns(node, None, "disabled").is_some() { return true; }
        let mut child = node;
        let mut ancestor = self.get(node).parent;
        while let Some(parent) = ancestor {
            if self.html_tag(parent, "fieldset") && self.get_attribute_ns(parent, None, "disabled").is_some() {
                let legend = self.children(parent).find(|candidate| self.html_tag(*candidate, "legend"));
                if legend != Some(child) { return true; }
            }
            child = parent;
            ancestor = self.get(parent).parent;
        }
        false
    }
    pub fn face_will_validate(&self, node: NodeIdx) -> bool {
        if self.is_actually_disabled(node) || self.get_attribute_ns(node, None, "readonly").is_some() {
            return false;
        }
        let mut ancestor = self.get(node).parent;
        while let Some(parent) = ancestor {
            if self.html_tag(parent, "datalist") { return false; }
            ancestor = self.get(parent).parent;
        }
        true
    }

    // CustomStateSet (setlike<DOMString>), insertion ordered.
    pub fn internals_states(&self, element: NodeIdx) -> Vec<Vec<u16>> {
        self.internals_state(element).map(|state| state.states.clone()).unwrap_or_default()
    }
    pub fn internals_state_add(&mut self, element: NodeIdx, name: Vec<u16>) -> Result<(), DomError> {
        let states = &mut self.internals_state_mut(element)?.states;
        if !states.contains(&name) { states.push(name); }
        self.record_render_full(element, "custom state set");
        Ok(())
    }
    pub fn internals_state_delete(&mut self, element: NodeIdx, name: &[u16]) -> Result<bool, DomError> {
        let states = &mut self.internals_state_mut(element)?.states;
        let before = states.len();
        states.retain(|candidate| candidate.as_slice() != name);
        let removed = states.len() != before;
        if removed { self.record_render_full(element, "custom state set"); }
        Ok(removed)
    }
    pub fn internals_state_clear(&mut self, element: NodeIdx) -> Result<(), DomError> {
        let states = &mut self.internals_state_mut(element)?.states;
        let changed = !states.is_empty();
        states.clear();
        if changed { self.record_render_full(element, "custom state set"); }
        Ok(())
    }
    /// Whether `:defined` matches: custom element state is "uncustomized" or "custom".
    pub fn is_defined_element(&self, element: NodeIdx) -> bool {
        use crate::custom_elements::CustomElementState as State;
        let Some(name) = self.name(element) else { return false };
        match self.custom_element_state(element) {
            State::Custom => true,
            State::Failed | State::Precustomized => false,
            State::Undefined => {
                if self.namespace(element) != Some(crate::Namespace::Html) { return true; }
                let is_candidate = Self::valid_custom_element_name(name)
                    || self.custom_element_is_value(element).ok().flatten().is_some();
                !is_candidate
            }
        }
    }
    /// Whether `:state(name)` matches the element.
    pub fn custom_state_matches(&self, element: NodeIdx, name: &[u16]) -> bool {
        self.internals_state(element).is_some_and(|state| state.states.iter().any(|s| s.as_slice() == name))
    }

    // ElementInternals ARIA reflection: plain default-semantics storage.
    pub fn internals_aria_get(&self, element: NodeIdx, name: &'static str) -> Option<Vec<u16>> {
        self.internals_state(element)?.aria.iter().find(|(key, _)| *key == name).map(|(_, value)| value.clone())
    }
    pub fn internals_aria_set(&mut self, element: NodeIdx, name: &'static str, value: Option<Vec<u16>>)
        -> Result<(), DomError> {
        let aria = &mut self.internals_state_mut(element)?.aria;
        aria.retain(|(key, _)| *key != name);
        if let Some(value) = value { aria.push((name, value)); }
        Ok(())
    }

    /// Explicitly set element references; stale (recycled) nodes are dropped.
    pub fn internals_aria_elements(&self, element: NodeIdx, name: &'static str)
        -> Option<(Vec<NodeIdx>, Option<CallbackId>)> {
        let (_, nodes, cache) = self.internals_state(element)?.aria_elements.iter().find(|(key, _, _)| *key == name)?;
        let live = nodes.iter().filter(|owner| self.node_generation(owner.node) == Some(owner.generation))
            .map(|owner| owner.node).collect();
        Some((live, *cache))
    }
    /// Replace (or with `None` clear) the references; returns the cache root to release.
    pub fn internals_aria_elements_set(&mut self, element: NodeIdx, name: &'static str,
        nodes: Option<Vec<NodeIdx>>) -> Result<Option<CallbackId>, DomError> {
        let handles = nodes.map(|nodes| nodes.into_iter().map(|node| self.render_handle(node)).collect::<Vec<_>>());
        let state = self.internals_state_mut(element)?;
        let old = state.aria_elements.iter().position(|(key, _, _)| *key == name)
            .map(|index| state.aria_elements.remove(index).2).flatten();
        if let Some(handles) = handles { state.aria_elements.push((name, handles, None)); }
        Ok(old)
    }
    pub fn internals_aria_elements_cache(&mut self, element: NodeIdx, name: &'static str, root: CallbackId)
        -> Result<(), DomError> {
        let state = self.internals_state_mut(element)?;
        if let Some(entry) = state.aria_elements.iter_mut().find(|(key, _, _)| *key == name) { entry.2 = Some(root); }
        Ok(())
    }

    /// Form reset algorithm for a form-associated custom element.
    pub(crate) fn face_reset(&mut self, element: NodeIdx) {
        if let Some(callback) = self.custom_element_definition_of(element)
            .and_then(|id| self.custom_element_definition(id)).and_then(|def| def.callbacks.form_reset) {
            self.enqueue_custom_element_reaction(CustomElementReaction::FormReset { element, callback });
        }
    }

    /// Reset form owners and disabled state of every form-associated custom
    /// element, enqueueing `formAssociatedCallback` / `formDisabledCallback` for
    /// each change since the last notification.
    pub(crate) fn face_sync_all(&mut self) {
        if self.custom_elements.face_definitions == 0 { return; }
        let candidates: Vec<NodeIdx> = self.custom_elements.element_states.iter()
            .filter(|(owner, _, state)| *state == CustomElementState::Custom
                && self.node_generation(owner.node) == Some(owner.generation))
            .map(|(owner, _, _)| owner.node).collect();
        for element in candidates {
            if !self.is_form_associated_custom_element(element) { continue; }
            let Some(handle) = self.node_generation(element).map(|generation| NodeGeneration { node: element, generation })
                else { continue };
            let owner = self.form_owner(element);
            let owner_handle = owner.and_then(|form| self.node_generation(form)
                .map(|generation| NodeGeneration { node: form, generation }));
            let disabled = self.is_actually_disabled(element);
            let position = self.custom_elements.face_last.iter().position(|(node, _, _)| *node == handle);
            let (last_owner, last_disabled) = position.map_or((None, false),
                |index| (self.custom_elements.face_last[index].1, self.custom_elements.face_last[index].2));
            if last_owner == owner_handle && last_disabled == disabled { continue; }
            match position {
                Some(index) => self.custom_elements.face_last[index] = (handle, owner_handle, disabled),
                None => self.custom_elements.face_last.push((handle, owner_handle, disabled)),
            }
            let Some(definition) = self.custom_element_definition_of(element)
                .and_then(|id| self.custom_element_definition(id)) else { continue };
            let callbacks = definition.callbacks;
            if last_owner != owner_handle {
                if let Some(callback) = callbacks.form_associated {
                    self.enqueue_custom_element_reaction(CustomElementReaction::FormAssociated {
                        element, callback, form: owner });
                }
            }
            if last_disabled != disabled {
                if let Some(callback) = callbacks.form_disabled {
                    self.enqueue_custom_element_reaction(CustomElementReaction::FormDisabled {
                        element, callback, disabled });
                }
            }
        }
        let live = |dom: &Dom, node: &NodeGeneration| dom.node_generation(node.node) == Some(node.generation);
        if self.custom_elements.face_last.len() > 64 {
            let kept: Vec<_> = self.custom_elements.face_last.iter().filter(|(node, _, _)| live(self, node)).cloned().collect();
            self.custom_elements.face_last = kept;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_elements::{CustomElementDefinition, CustomElementReaction as R};
    use crate::events::CallbackId;

    fn face() -> (Dom, NodeIdx, NodeIdx, usize) {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let html = dom.create_element(document, "html");
        let form = dom.create_element(html, "form");
        let element = dom.create_element(form, "x-face");
        let mut definition = CustomElementDefinition::new(document, "x-face".into(), "x-face".into(), CallbackId(1));
        definition.form_associated = true;
        definition.callbacks.form_associated = Some(CallbackId(2));
        definition.callbacks.form_disabled = Some(CallbackId(3));
        definition.callbacks.form_reset = Some(CallbackId(4));
        let id = dom.define_custom_element(definition).unwrap();
        dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(element, id).unwrap();
        dom.custom_element_super(id).unwrap();
        dom.finish_custom_element_upgrade(element, id, true).unwrap();
        (dom, form, element, id)
    }

    #[test]
    fn upgrade_resets_form_owner_and_notifies_callbacks() {
        let (mut dom, form, element, _) = face();
        assert_eq!(dom.take_custom_element_reactions(), vec![
            R::FormAssociated { element, callback: CallbackId(2), form: Some(form) }]);
        assert_eq!(dom.form_controls(form).unwrap(), vec![element]);
        dom.set_attribute(element, "disabled", "").unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![
            R::FormDisabled { element, callback: CallbackId(3), disabled: true }]);
        dom.remove_attribute(element, "disabled").unwrap();
        dom.take_custom_element_reactions();
        dom.reset_form(form).unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![R::FormReset { element, callback: CallbackId(4) }]);
        dom.remove(element).unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![
            R::FormAssociated { element, callback: CallbackId(2), form: None }]);
    }

    #[test]
    fn attach_internals_validity_states_and_errors() {
        let (mut dom, _, element, _) = face();
        dom.attach_internals(element).unwrap();
        assert_eq!(dom.attach_internals(element), Err(DomError::NotSupportedError));
        assert!(dom.will_validate(element).unwrap());
        let flags = ValidityState { value_missing: true, ..Default::default() };
        dom.internals_set_validity(element, flags, "bad".encode_utf16().collect(), None).unwrap();
        assert!(!dom.validity(element, |_, _| None).unwrap().valid());
        assert_eq!(dom.validation_message(element).unwrap(), "bad".encode_utf16().collect::<Vec<_>>());
        let outside = dom.create_element(NodeIdx(0), "div");
        assert_eq!(dom.internals_set_validity(element, ValidityState::default(), Vec::new(), Some(outside)),
            Err(DomError::NotFoundError));
        dom.internals_set_validity(element, ValidityState::default(), Vec::new(), None).unwrap();
        assert!(dom.validity(element, |_, _| None).unwrap().valid());
        dom.internals_state_add(element, vec![65]).unwrap();
        dom.internals_state_add(element, vec![65]).unwrap();
        assert_eq!(dom.internals_states(element), vec![vec![65]]);
        assert!(dom.custom_state_matches(element, &[65]));
        assert!(dom.internals_state_delete(element, &[65]).unwrap());
        let plain = dom.create_element(NodeIdx(0), "div");
        assert_eq!(dom.attach_internals(plain), Err(DomError::NotSupportedError));
    }
}
