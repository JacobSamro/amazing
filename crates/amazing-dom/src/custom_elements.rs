//! HTML §4.13 native custom element definitions, upgrade state, and reactions.
//! JavaScript constructors/callbacks remain opaque strong roots in the host.
use crate::{events::CallbackId, Dom, DomError, Namespace, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LifecycleCallbacks {
    pub connected: Option<CallbackId>,
    pub disconnected: Option<CallbackId>,
    pub connected_move: Option<CallbackId>,
    pub adopted: Option<CallbackId>,
    pub attribute_changed: Option<CallbackId>,
    pub form_associated: Option<CallbackId>,
    pub form_reset: Option<CallbackId>,
    pub form_disabled: Option<CallbackId>,
    pub form_state_restore: Option<CallbackId>,
}

#[derive(Clone, Debug)]
pub struct CustomElementDefinition {
    pub document: NodeIdx,
    pub name: String,
    pub local_name: String,
    pub constructor: CallbackId,
    pub callbacks: LifecycleCallbacks,
    pub observed_attributes: Vec<String>,
    pub disable_internals: bool,
    pub disable_shadow: bool,
    pub form_associated: bool,
    construction_stack: Vec<ConstructionEntry>,
}
impl CustomElementDefinition {
    pub fn new(document: NodeIdx, name: String, local_name: String, constructor: CallbackId) -> Self {
        Self { document, name, local_name, constructor, callbacks: LifecycleCallbacks::default(),
            observed_attributes: Vec::new(), disable_internals: false, disable_shadow: false,
            form_associated: false, construction_stack: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstructionEntry { Element(NodeIdx), AlreadyConstructed }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomElementState { Undefined, Precustomized, Custom, Failed }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomElementReaction {
    Upgrade { element: NodeIdx, definition: usize },
    Connected { element: NodeIdx, callback: CallbackId },
    Disconnected { element: NodeIdx, callback: CallbackId },
    ConnectedMove { element: NodeIdx, callback: CallbackId },
    Adopted { element: NodeIdx, callback: CallbackId, old_document: NodeIdx, new_document: NodeIdx },
    AttributeChanged { element: NodeIdx, callback: CallbackId, name: String,
        old_value: Option<String>, new_value: Option<String>, namespace: Option<String> },
}

#[derive(Debug, Default)]
pub struct CustomElements {
    definitions: Vec<CustomElementDefinition>,
    element_states: Vec<(NodeIdx, usize, CustomElementState)>,
    reaction_frames: Vec<Vec<CustomElementReaction>>,
    backup_queue: Vec<CustomElementReaction>,
    when_defined: Vec<(NodeIdx, String, CallbackId)>,
    definition_running: Vec<NodeIdx>,
    suppress_tree_reactions: bool,
}

impl Dom {
    pub fn valid_custom_element_name(name: &str) -> bool {
        crate::shadow::custom_element_name(name)
    }
    pub fn custom_element_definitions(&self, document: NodeIdx) -> Vec<(usize, &CustomElementDefinition)> {
        self.custom_elements.definitions.iter().enumerate()
            .filter(|(_, definition)| definition.document == document).collect()
    }
    pub fn custom_element_definition(&self, id: usize) -> Option<&CustomElementDefinition> {
        self.custom_elements.definitions.get(id)
    }
    pub fn custom_element_definition_by_name(&self, document: NodeIdx, name: &str) -> Option<(usize, &CustomElementDefinition)> {
        self.custom_element_definitions(document).into_iter().find(|(_, definition)| definition.name == name)
    }
    pub fn define_custom_element(&mut self, mut definition: CustomElementDefinition) -> Result<usize, DomError> {
        self.check_node(definition.document)?;
        if !matches!(self.get(definition.document).kind, NodeKind::Document)
            || !Self::valid_custom_element_name(&definition.name)
            || self.custom_element_definition_by_name(definition.document, &definition.name).is_some() {
            return Err(DomError::NotSupportedError);
        }
        definition.construction_stack.clear();
        let id = self.custom_elements.definitions.len();
        let document = definition.document;
        let local_name = definition.local_name.clone();
        self.custom_elements.definitions.push(definition);
        for element in self.shadow_including_tree_order(document) {
            if self.local_name(element) == Some(local_name.as_str()) {
                self.queue_custom_element_upgrade(element);
            }
        }
        Ok(id)
    }
    pub fn custom_element_state(&self, element: NodeIdx) -> CustomElementState {
        self.custom_elements.element_states.iter().find(|(node, _, _)| *node == element)
            .map_or(CustomElementState::Undefined, |(_, _, state)| *state)
    }
    pub fn custom_element_definition_of(&self, element: NodeIdx) -> Option<usize> {
        self.custom_elements.element_states.iter().find(|(node, _, _)| *node == element)
            .map(|(_, definition, _)| *definition)
    }
    pub fn queue_custom_element_upgrade(&mut self, element: NodeIdx) {
        if self.namespace(element) != Some(Namespace::Html)
            || self.custom_element_state(element) != CustomElementState::Undefined { return; }
        let Some(local_name) = self.local_name(element) else { return };
        let document = self.get(element).owner_document;
        let is_name = self.get_attribute_ns(element, None, "is");
        let definition = self.custom_element_definitions(document).into_iter()
            .find(|(_, def)| def.local_name == local_name
                && (def.local_name == def.name || is_name == Some(def.name.as_str())))
            .map(|(id, _)| id);
        if let Some(definition) = definition {
            let reaction = CustomElementReaction::Upgrade { element, definition };
            if !self.has_custom_element_reaction(&reaction) { self.enqueue_custom_element_reaction(reaction); }
        }
    }
    pub fn begin_custom_element_upgrade(&mut self, element: NodeIdx, definition: usize) -> Result<CallbackId, DomError> {
        self.check_node(element)?;
        if self.custom_element_state(element) != CustomElementState::Undefined {
            return Err(DomError::NotSupportedError);
        }
        let def = self.custom_elements.definitions.get(definition).ok_or(DomError::InvalidNode)?;
        if def.disable_shadow && self.shadow_root_of_host(element).is_some() {
            return Err(DomError::NotSupportedError);
        }
        let constructor = def.constructor;
        self.custom_elements.element_states.push((element, definition, CustomElementState::Precustomized));
        self.record_render_full(element, "custom element upgrade state");
        self.custom_elements.definitions[definition].construction_stack.push(ConstructionEntry::Element(element));
        Ok(constructor)
    }
    /// Called by the embedding's HTMLElement constructor when `super()` runs.
    pub fn custom_element_super(&mut self, definition: usize) -> Result<NodeIdx, DomError> {
        let stack = &mut self.custom_elements.definitions.get_mut(definition)
            .ok_or(DomError::InvalidNode)?.construction_stack;
        match stack.last_mut() {
            Some(entry @ ConstructionEntry::Element(_)) => {
                let ConstructionEntry::Element(element) = *entry else { unreachable!() };
                *entry = ConstructionEntry::AlreadyConstructed;
                Ok(element)
            }
            _ => Err(DomError::InvalidStateError),
        }
    }
    pub fn finish_custom_element_upgrade(&mut self, element: NodeIdx, definition: usize,
        constructed: bool) -> Result<(), DomError> {
        let marker = self.custom_elements.definitions.get_mut(definition)
            .ok_or(DomError::InvalidNode)?.construction_stack.pop();
        let entry = self.custom_elements.element_states.iter_mut()
            .find(|(node, id, _)| *node == element && *id == definition).ok_or(DomError::InvalidNode)?;
        if !constructed || marker != Some(ConstructionEntry::AlreadyConstructed) {
            entry.2 = CustomElementState::Failed;
            self.record_render_full(element, "custom element upgrade state");
            return Err(DomError::TypeError);
        }
        entry.2 = CustomElementState::Custom;
        self.record_render_full(element, "custom element upgrade state");
        let def = self.custom_elements.definitions[definition].clone();
        if let Some(callback) = def.callbacks.attribute_changed {
            let attributes: Vec<_> = self.attributes_expanded(element).unwrap_or_default()
                .into_iter().filter(|attr| def.observed_attributes.iter().any(|name| name == attr.local_name))
                .map(|attr| (attr.local_name.to_owned(), attr.value.to_owned(),
                    attr.namespace_uri.map(str::to_owned))).collect();
            for (name, value, namespace) in attributes {
                self.enqueue_custom_element_reaction(CustomElementReaction::AttributeChanged {
                    element, callback, name, old_value: None,
                    new_value: Some(value), namespace,
                });
            }
        }
        if self.is_connected(element) {
            if let Some(callback) = def.callbacks.connected {
                self.enqueue_custom_element_reaction(CustomElementReaction::Connected { element, callback });
            }
        }
        Ok(())
    }
    pub fn custom_element_push_reaction_frame(&mut self) {
        self.custom_elements.reaction_frames.push(Vec::new());
    }
    pub fn custom_element_pop_reaction_frame(&mut self) {
        if let Some(mut frame) = self.custom_elements.reaction_frames.pop() {
            self.custom_elements.backup_queue.append(&mut frame);
        }
    }
    pub fn take_custom_element_reactions(&mut self) -> Vec<CustomElementReaction> {
        std::mem::take(&mut self.custom_elements.backup_queue)
    }
    fn enqueue_custom_element_reaction(&mut self, reaction: CustomElementReaction) {
        match self.custom_elements.reaction_frames.last_mut() {
            Some(frame) => frame.push(reaction),
            None => self.custom_elements.backup_queue.push(reaction),
        }
    }
    fn has_custom_element_reaction(&self, reaction: &CustomElementReaction) -> bool {
        self.custom_elements.backup_queue.contains(reaction)
            || self.custom_elements.reaction_frames.iter().any(|frame| frame.contains(reaction))
    }
    pub fn custom_element_when_defined(&mut self, document: NodeIdx, name: String, promise: CallbackId) {
        self.custom_elements.when_defined.push((document, name, promise));
    }
    pub fn custom_element_pending_promise(&self, document: NodeIdx, name: &str) -> Option<CallbackId> {
        self.custom_elements.when_defined.iter().find(|(doc, candidate, _)|
            *doc == document && candidate == name).map(|(_, _, promise)| *promise)
    }
    pub fn custom_element_begin_definition(&mut self, document: NodeIdx) -> Result<(), DomError> {
        if self.custom_elements.definition_running.contains(&document) {
            return Err(DomError::NotSupportedError);
        }
        self.custom_elements.definition_running.push(document);
        Ok(())
    }
    pub fn custom_element_end_definition(&mut self, document: NodeIdx) {
        self.custom_elements.definition_running.retain(|candidate| *candidate != document);
    }
    pub fn queue_custom_element_upgrades_for_tree(&mut self, root: NodeIdx) -> Result<(), DomError> {
        self.check_node(root)?;
        for element in self.shadow_including_tree_order(root) {
            self.queue_custom_element_upgrade(element);
        }
        Ok(())
    }
    pub fn take_custom_element_when_defined(&mut self, document: NodeIdx, name: &str) -> Vec<CallbackId> {
        let mut result = Vec::new();
        self.custom_elements.when_defined.retain(|(doc, candidate, promise)| {
            if *doc == document && candidate == name { result.push(*promise); false } else { true }
        });
        result
    }
    pub(crate) fn custom_element_tree_inserted(&mut self, root: NodeIdx) {
        if self.custom_elements.suppress_tree_reactions { return; }
        for element in self.shadow_including_tree_order(root) {
            match self.custom_element_state(element) {
                CustomElementState::Custom => {
                    if self.is_connected(element) {
                        if let Some(id) = self.custom_element_definition_of(element) {
                            if let Some(callback) = self.custom_elements.definitions[id].callbacks.connected {
                                self.enqueue_custom_element_reaction(CustomElementReaction::Connected { element, callback });
                            }
                        }
                    }
                }
                CustomElementState::Undefined => self.queue_custom_element_upgrade(element),
                _ => {}
            }
        }
    }
    pub(crate) fn custom_element_tree_removed(&mut self, root: NodeIdx) {
        if self.custom_elements.suppress_tree_reactions { return; }
        if !self.is_connected(root) { return; }
        for element in self.shadow_including_tree_order(root) {
            if self.custom_element_state(element) == CustomElementState::Custom {
                if let Some(id) = self.custom_element_definition_of(element) {
                    if let Some(callback) = self.custom_elements.definitions[id].callbacks.disconnected {
                        self.enqueue_custom_element_reaction(CustomElementReaction::Disconnected { element, callback });
                    }
                }
            }
        }
    }
    pub(crate) fn custom_element_preserve_move(&mut self, root: NodeIdx, moving: bool) {
        self.custom_elements.suppress_tree_reactions = moving;
        if moving || !self.is_connected(root) { return; }
        for element in self.shadow_including_tree_order(root) {
            if self.custom_element_state(element) != CustomElementState::Custom { continue; }
            let Some(id) = self.custom_element_definition_of(element) else { continue };
            let callbacks = self.custom_elements.definitions[id].callbacks;
            if let Some(callback) = callbacks.connected_move {
                self.enqueue_custom_element_reaction(CustomElementReaction::ConnectedMove { element, callback });
            } else {
                if let Some(callback) = callbacks.disconnected {
                    self.enqueue_custom_element_reaction(CustomElementReaction::Disconnected { element, callback });
                }
                if let Some(callback) = callbacks.connected {
                    self.enqueue_custom_element_reaction(CustomElementReaction::Connected { element, callback });
                }
            }
        }
    }
    pub(crate) fn custom_element_adopted(&mut self, root: NodeIdx, old_document: NodeIdx, new_document: NodeIdx) {
        for element in self.shadow_including_tree_order(root) {
            if self.custom_element_state(element) == CustomElementState::Custom {
                if let Some(id) = self.custom_element_definition_of(element) {
                    if let Some(callback) = self.custom_elements.definitions[id].callbacks.adopted {
                        self.enqueue_custom_element_reaction(CustomElementReaction::Adopted {
                            element, callback, old_document, new_document,
                        });
                    }
                }
            }
        }
    }
    pub(crate) fn custom_element_attribute_changed(&mut self, element: NodeIdx,
        namespace: Option<&str>, name: &str, old_value: Option<String>, new_value: Option<String>) {
        if self.custom_element_state(element) != CustomElementState::Custom { return; }
        let Some(id) = self.custom_element_definition_of(element) else { return };
        let def = &self.custom_elements.definitions[id];
        if !def.observed_attributes.iter().any(|observed| observed == name) { return; }
        if let Some(callback) = def.callbacks.attribute_changed {
            self.enqueue_custom_element_reaction(CustomElementReaction::AttributeChanged {
                element, callback, name: name.into(), old_value, new_value,
                namespace: namespace.map(str::to_owned),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pinned WPT custom-elements/CustomElementRegistry.html and
    // custom-element-reaction-queue.html assertion ports.
    #[test]
    fn definition_validation_upgrade_stack_and_initial_reactions() {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let html = dom.create_element(document, "html");
        let element = dom.create_element(html, "x-card");
        dom.set_attribute(element, "title", "ready").unwrap();
        let mut definition = CustomElementDefinition::new(document, "x-card".into(), "x-card".into(), CallbackId(1));
        definition.callbacks.connected = Some(CallbackId(2));
        definition.callbacks.attribute_changed = Some(CallbackId(3));
        definition.observed_attributes.push("title".into());
        let id = dom.define_custom_element(definition).unwrap();
        assert_eq!(dom.custom_element_definition_by_name(document, "x-card").unwrap().0, id);
        assert_eq!(dom.define_custom_element(CustomElementDefinition::new(document,
            "x-card".into(), "x-card".into(), CallbackId(4))), Err(DomError::NotSupportedError));
        assert!(!Dom::valid_custom_element_name("xCard"));
        assert_eq!(dom.take_custom_element_reactions(), vec![CustomElementReaction::Upgrade { element, definition: id }]);
        assert_eq!(dom.begin_custom_element_upgrade(element, id), Ok(CallbackId(1)));
        assert_eq!(dom.custom_element_super(id), Ok(element));
        assert_eq!(dom.custom_element_super(id), Err(DomError::InvalidStateError));
        dom.finish_custom_element_upgrade(element, id, true).unwrap();
        assert_eq!(dom.custom_element_state(element), CustomElementState::Custom);
        assert_eq!(dom.take_custom_element_reactions(), vec![
            CustomElementReaction::AttributeChanged { element, callback: CallbackId(3),
                name: "title".into(), old_value: None, new_value: Some("ready".into()), namespace: None },
            CustomElementReaction::Connected { element, callback: CallbackId(2) },
        ]);
    }

    // Pinned WPT custom-elements/attribute-changed-callback.html,
    // connected-callbacks.html, disconnected-callbacks.html, adopted-callback.html.
    #[test]
    fn live_mutations_enqueue_observed_lifecycle_reactions() {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let html = dom.create_element(document, "html");
        let element = dom.create_element(html, "x-thing");
        let mut definition = CustomElementDefinition::new(document, "x-thing".into(), "x-thing".into(), CallbackId(1));
        definition.callbacks = LifecycleCallbacks { connected: Some(CallbackId(2)),
            disconnected: Some(CallbackId(3)), adopted: Some(CallbackId(4)),
            attribute_changed: Some(CallbackId(5)), ..Default::default() };
        definition.observed_attributes.push("title".into());
        let id = dom.define_custom_element(definition).unwrap();
        dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(element, id).unwrap();
        dom.custom_element_super(id).unwrap();
        dom.finish_custom_element_upgrade(element, id, true).unwrap();
        dom.take_custom_element_reactions();
        dom.custom_element_push_reaction_frame();
        dom.set_attribute(element, "title", "foo").unwrap();
        dom.set_attribute(element, "ignored", "x").unwrap();
        dom.remove_attribute(element, "title").unwrap();
        dom.detach(element);
        dom.attach_last(html, element);
        let other = dom.create_document();
        dom.adopt(element, other).unwrap();
        dom.custom_element_pop_reaction_frame();
        let reactions = dom.take_custom_element_reactions();
        assert_eq!(reactions.len(), 6);
        assert!(matches!(&reactions[0], CustomElementReaction::AttributeChanged {
            old_value: None, new_value: Some(new), .. } if new == "foo"));
        assert!(matches!(&reactions[1], CustomElementReaction::AttributeChanged {
            old_value: Some(old), new_value: None, .. } if old == "foo"));
        assert!(matches!(reactions[2], CustomElementReaction::Disconnected { .. }));
        assert!(matches!(reactions[3], CustomElementReaction::Connected { .. }));
        assert!(matches!(reactions[4], CustomElementReaction::Disconnected { .. }));
        assert!(matches!(reactions[5], CustomElementReaction::Adopted { .. }));
    }

    #[test]
    fn move_before_uses_connected_move_callback_or_lifecycle_fallback() {
        let mut dom = Dom::new();
        let document = NodeIdx(0);
        let html = dom.create_element(document, "html");
        let left = dom.create_element(html, "div");
        let right = dom.create_element(html, "section");
        let moved = dom.create_element(left, "x-moved");
        let mut definition = CustomElementDefinition::new(document,
            "x-moved".into(), "x-moved".into(), CallbackId(1));
        definition.callbacks.connected_move = Some(CallbackId(2));
        definition.callbacks.connected = Some(CallbackId(3));
        definition.callbacks.disconnected = Some(CallbackId(4));
        let id = dom.define_custom_element(definition).unwrap();
        dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(moved, id).unwrap();
        dom.custom_element_super(id).unwrap();
        dom.finish_custom_element_upgrade(moved, id, true).unwrap();
        dom.take_custom_element_reactions();
        dom.move_before(moved, right, None).unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![
            CustomElementReaction::ConnectedMove { element: moved, callback: CallbackId(2) },
        ]);
        assert_eq!(dom.get(moved).parent, Some(right));
    }
}
