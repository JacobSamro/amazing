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
    observed_attribute_units: Option<Vec<Vec<u16>>>,
    pub disable_internals: bool,
    pub disable_shadow: bool,
    pub form_associated: bool,
    construction_stack: Vec<ConstructionEntry>,
    construction_revision: u64,
    document_generation: u64,
}
impl CustomElementDefinition {
    /// Exact WebIDL DOMString names; scalar native callers retain their existing API.
    pub fn set_observed_attribute_units(&mut self, names: Vec<Vec<u16>>) {
        self.observed_attribute_units = Some(names);
        self.observed_attributes.clear();
    }
    fn observes_attribute(&self, name: &str) -> bool {
        if let Some(names) = &self.observed_attribute_units {
            names.iter().any(|units| units.iter().copied().eq(name.encode_utf16()))
        } else { self.observed_attributes.iter().any(|observed| observed == name) }
    }
    pub fn new(document: NodeIdx, name: String, local_name: String, constructor: CallbackId) -> Self {
        Self { document, name, local_name, constructor, callbacks: LifecycleCallbacks::default(),
            observed_attributes: Vec::new(), observed_attribute_units: None, disable_internals: false, disable_shadow: false,
            form_associated: false, construction_stack: Vec::new(), construction_revision: 0, document_generation: 0 }
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
        old_value: Option<Vec<u16>>, new_value: Option<Vec<u16>>, namespace: Option<String> },
    FormAssociated { element: NodeIdx, callback: CallbackId, form: Option<NodeIdx> },
    FormDisabled { element: NodeIdx, callback: CallbackId, disabled: bool },
    FormReset { element: NodeIdx, callback: CallbackId },
}

/// Move-only popped queue from this exact native arena. Its immutable node
/// generations are captured when reactions are queued, before author code.
#[derive(Debug)]
pub struct CustomElementReactionFrame {
    arena: std::sync::Arc<()>,
    entries: std::collections::VecDeque<QueuedCustomElementReaction>,
}
impl CustomElementReactionFrame {
    /// No queued reaction: delivery can be skipped without touching the DOM.
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}
#[derive(Debug)]
struct QueuedCustomElementReaction {
    reaction: CustomElementReaction,
    nodes: Vec<crate::render_mutation::NodeGeneration>,
    valid: bool,
}

#[derive(Debug, Default)]
pub struct CustomElements {
    pub(crate) definitions: Vec<CustomElementDefinition>,
    pub(crate) element_states: Vec<(crate::render_mutation::NodeGeneration, usize, CustomElementState)>,
    /// ElementInternals native state (HTML §4.13.7), one entry per attached element.
    pub(crate) internals: Vec<(crate::render_mutation::NodeGeneration, crate::element_internals::InternalsState)>,
    /// Last form owner and actually-disabled state notified to each form-associated element.
    pub(crate) face_last: Vec<(crate::render_mutation::NodeGeneration, Option<crate::render_mutation::NodeGeneration>, bool)>,
    pub(crate) face_definitions: usize,
    pub(crate) label_clicks: Vec<NodeIdx>,
    upgrade_reactions: Vec<(crate::render_mutation::NodeGeneration, usize, Vec<CustomElementReaction>)>,
    reaction_frames: Vec<Vec<QueuedCustomElementReaction>>,
    backup_queue: Vec<CustomElementReaction>,
    when_defined: Vec<(crate::render_mutation::NodeGeneration, String, CallbackId)>,
    is_values: Vec<Option<(u64, Option<Vec<u16>>)>>,
    failed_creation_nodes: Vec<Option<u64>>,
    definition_running: Vec<crate::render_mutation::NodeGeneration>,
    suppress_tree_reactions: bool,
}

impl Dom {
    pub fn valid_custom_element_name(name: &str) -> bool {
        crate::shadow::custom_element_name(name)
    }
    pub fn custom_element_definitions(&self, document: NodeIdx) -> Vec<(usize, &CustomElementDefinition)> {
        self.custom_elements.definitions.iter().enumerate()
            .filter(|(_, definition)| definition.document == document
                && self.node_generation(document) == Some(definition.document_generation)).collect()
    }
    pub fn custom_element_definition(&self, id: usize) -> Option<&CustomElementDefinition> {
        self.custom_elements.definitions.get(id).filter(|definition|
            self.node_generation(definition.document) == Some(definition.document_generation))
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
        definition.construction_revision = 0;
        definition.document_generation = self.node_generation(definition.document).ok_or(DomError::InvalidNode)?;
        let id = self.custom_elements.definitions.len();
        let document = definition.document;
        let local_name = definition.local_name.clone();
        if definition.form_associated { self.custom_elements.face_definitions += 1; }
        self.custom_elements.definitions.push(definition);
        for element in self.shadow_including_tree_order(document) {
            if self.local_name(element) == Some(local_name.as_str()) {
                self.queue_custom_element_upgrade(element);
            }
        }
        Ok(id)
    }
    /// O(1) common-path guard: with no definition and no tracked element, no
    /// insertion, removal, adoption, attribute change or creation can queue a
    /// reaction, so the per-node walks are skipped entirely.
    #[inline]
    pub(crate) fn custom_elements_idle(&self) -> bool {
        self.custom_elements.definitions.is_empty() && self.custom_elements.element_states.is_empty()
    }
    pub fn custom_element_state(&self, element: NodeIdx) -> CustomElementState {
        if self.is_failed_custom_element_creation(element) { return CustomElementState::Failed; }
        self.custom_elements.element_states.iter().find(|(node, _, _)| node.node == element && self.node_generation(element) == Some(node.generation))
            .map_or(CustomElementState::Undefined, |(_, _, state)| *state)
    }
    pub fn custom_element_definition_of(&self, element: NodeIdx) -> Option<usize> {
        self.custom_elements.element_states.iter().find(|(node, _, _)| node.node == element && self.node_generation(element) == Some(node.generation))
            .map(|(_, definition, _)| *definition)
    }
    pub fn queue_custom_element_upgrade(&mut self, element: NodeIdx) {
        if self.custom_elements.definitions.is_empty() { return; }
        if self.namespace(element) != Some(Namespace::Html)
            || self.custom_element_state(element) != CustomElementState::Undefined { return; }
        let Some(local_name) = self.local_name(element) else { return };
        let document = self.get(element).owner_document;
        let is_name = self.custom_element_is_value(element).ok().flatten();
        let definition = self.custom_element_definitions(document).into_iter()
            .find(|(_, def)| def.local_name == local_name
                && (def.local_name == def.name || is_name == Some(def.name.encode_utf16().collect::<Vec<_>>().as_slice())))
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
        let def = self.custom_element_definition(definition).ok_or(DomError::InvalidNode)?;
        if def.disable_shadow && self.shadow_root_of_host(element).is_some() {
            return Err(DomError::NotSupportedError);
        }
        let def = def.clone();
        let constructor = def.constructor;
        // HTML upgrade captures existing callback arguments before invoking
        // the author constructor. Later attribute changes cannot reconstruct
        // those admission-time values, including unpaired UTF16 surrogates.
        let mut reactions = Vec::new();
        if let Some(callback) = def.callbacks.attribute_changed {
            for attribute in self.attributes_expanded(element).unwrap_or_default() {
                if def.observes_attribute(attribute.local_name) {
                    reactions.push(CustomElementReaction::AttributeChanged {
                        element, callback, name: attribute.local_name.to_owned(), old_value: None,
                        new_value: Some(self.attribute_ns_utf16(element, attribute.namespace_uri, attribute.local_name)
                            .expect("Existing native attribute has exact units")),
                        namespace: attribute.namespace_uri.map(str::to_owned),
                    });
                }
            }
        }
        if self.is_connected(element) {
            if let Some(callback) = def.callbacks.connected {
                reactions.push(CustomElementReaction::Connected { element, callback });
            }
        }
        let owner = self.render_handle(element);
        self.custom_elements.upgrade_reactions.push((owner, definition, reactions));
        self.custom_elements.element_states.push((owner, definition, CustomElementState::Precustomized));
        self.record_render_full(element, "custom element upgrade state");
        self.custom_elements.definitions[definition].construction_revision += 1;
        self.custom_elements.definitions[definition].construction_stack.push(ConstructionEntry::Element(element));
        Ok(constructor)
    }
    /// Called by the embedding's HTMLElement constructor when `super()` runs.
    pub fn custom_element_super(&mut self, definition: usize) -> Result<NodeIdx, DomError> {
        self.custom_element_definition(definition).ok_or(DomError::InvalidStateError)?;
        let def = self.custom_elements.definitions.get_mut(definition).ok_or(DomError::InvalidNode)?;
        def.construction_revision += 1;
        match def.construction_stack.last_mut() {
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
        let position = self.custom_elements.upgrade_reactions.iter()
            .rposition(|(owner, id, _)| owner.node == element && *id == definition)
            .ok_or(DomError::InvalidStateError)?;
        let (owner, _, reactions) = self.custom_elements.upgrade_reactions.remove(position);
        if self.node_generation(element) != Some(owner.generation) { return Err(DomError::InvalidStateError); }
        let def = self.custom_elements.definitions.get_mut(definition).ok_or(DomError::InvalidNode)?;
        def.construction_revision += 1;
        let marker = def.construction_stack.pop();
        let entry = self.custom_elements.element_states.iter_mut()
            .find(|(node, id, _)| *node == owner && *id == definition).ok_or(DomError::InvalidNode)?;
        if !constructed || marker != Some(ConstructionEntry::AlreadyConstructed) {
            entry.2 = CustomElementState::Failed;
            self.record_render_full(element, "custom element upgrade state");
            return Err(DomError::TypeError);
        }
        entry.2 = CustomElementState::Custom;
        self.record_render_full(element, "custom element upgrade state");
        for reaction in reactions { self.enqueue_custom_element_reaction(reaction); }
        if self.custom_elements.definitions.get(definition).is_some_and(|def| def.form_associated) {
            // Listed-element membership changed without a tree mutation.
            self.mutation_generation = self.mutation_generation.wrapping_add(1);
        }
        self.face_sync_all();
        Ok(())
    }
    pub fn custom_element_push_reaction_frame(&mut self) {
        self.custom_elements.reaction_frames.push(Vec::new());
    }
    pub fn custom_element_pop_reaction_frame(&mut self) {
        if let Some(mut frame) = self.custom_elements.reaction_frames.pop() {
            self.custom_elements.backup_queue.extend(frame.drain(..).map(|entry| entry.reaction));
        }
    }
    pub fn take_custom_element_reactions(&mut self) -> Vec<CustomElementReaction> {
        std::mem::take(&mut self.custom_elements.backup_queue)
    }
    /// Pop only the top frame; never merge or consume the backup queue.
    pub fn custom_element_pop_isolated_reaction_frame(&mut self) -> Result<CustomElementReactionFrame, DomError> {
        let entries = self.custom_elements.reaction_frames.pop().ok_or(DomError::InvalidStateError)?;
        Ok(CustomElementReactionFrame { arena: self.token_list_arena.clone(), entries: entries.into() })
    }
    /// A non-script embedding may defer this exact popped queue to the normal
    /// backup checkpoint. Ownership is checked before any queue mutation.
    pub fn custom_element_defer_reaction_frame(&mut self, mut frame: CustomElementReactionFrame) -> Result<(), DomError> {
        if !std::sync::Arc::ptr_eq(&frame.arena, &self.token_list_arena) { return Err(DomError::InvalidStateError); }
        while let Some(reaction) = self.custom_element_next_isolated_reaction(&mut frame)? {
            self.custom_elements.backup_queue.push(reaction);
        }
        Ok(())
    }
    pub fn custom_element_next_isolated_reaction(&self, frame: &mut CustomElementReactionFrame)
        -> Result<Option<CustomElementReaction>, DomError> {
        if !std::sync::Arc::ptr_eq(&frame.arena, &self.token_list_arena) { return Err(DomError::InvalidStateError); }
        while let Some(entry) = frame.entries.pop_front() {
            if entry.valid && entry.nodes.iter().all(|owner| self.node_generation(owner.node) == Some(owner.generation)) {
                return Ok(Some(entry.reaction));
            }
        }
        Ok(None)
    }
    pub(crate) fn enqueue_custom_element_reaction(&mut self, reaction: CustomElementReaction) {
        let nodes = if self.custom_elements.reaction_frames.is_empty() { Vec::new() } else {
            let mut nodes = vec![match &reaction {
                CustomElementReaction::Upgrade { element, .. }
                | CustomElementReaction::Connected { element, .. }
                | CustomElementReaction::Disconnected { element, .. }
                | CustomElementReaction::ConnectedMove { element, .. }
                | CustomElementReaction::Adopted { element, .. }
                | CustomElementReaction::AttributeChanged { element, .. }
                | CustomElementReaction::FormAssociated { element, .. }
                | CustomElementReaction::FormDisabled { element, .. }
                | CustomElementReaction::FormReset { element, .. } => *element,
            }];
            match &reaction {
                CustomElementReaction::Upgrade { definition, .. } => {
                    if let Some(document) = self.custom_element_definition_document(*definition) { nodes.push(document.node); }
                }
                CustomElementReaction::Adopted { old_document, new_document, .. } => {
                    nodes.push(*old_document); nodes.push(*new_document);
                }
                _ => {}
            }
            nodes.into_iter().filter_map(|node| self.node_generation(node).map(|generation|
                crate::render_mutation::NodeGeneration { node, generation })).collect()
        };
        let expected = match &reaction { CustomElementReaction::Upgrade { .. } => 2,
            CustomElementReaction::Adopted { .. } => 3, _ => 1 };
        let valid = nodes.len() == expected;
        match self.custom_elements.reaction_frames.last_mut() {
            Some(frame) => frame.push(QueuedCustomElementReaction { reaction, nodes, valid }),
            None => self.custom_elements.backup_queue.push(reaction),
        }
    }
    fn has_custom_element_reaction(&self, reaction: &CustomElementReaction) -> bool {
        self.custom_elements.backup_queue.contains(reaction)
            || self.custom_elements.reaction_frames.iter().any(|frame| frame.iter().any(|entry| &entry.reaction == reaction))
    }
    pub fn custom_element_when_defined(&mut self, document: NodeIdx, name: String, promise: CallbackId) {
        if self.node_generation(document).is_some() {
            self.custom_elements.when_defined.push((self.render_handle(document), name, promise));
        }
    }
    pub fn custom_element_pending_promise(&self, document: NodeIdx, name: &str) -> Option<CallbackId> {
        self.custom_elements.when_defined.iter().find(|(doc, candidate, _)|
            doc.node == document && self.node_generation(document) == Some(doc.generation) && candidate == name).map(|(_, _, promise)| *promise)
    }
    pub fn custom_element_begin_definition(&mut self, document: NodeIdx) -> Result<(), DomError> {
        self.check_node(document)?;
        let document = self.render_handle(document);
        if self.custom_elements.definition_running.contains(&document) {
            return Err(DomError::NotSupportedError);
        }
        self.custom_elements.definition_running.push(document);
        Ok(())
    }
    pub fn custom_element_end_definition(&mut self, document: NodeIdx) {
        let generation = self.node_generation(document);
        self.custom_elements.definition_running.retain(|candidate| candidate.node != document || Some(candidate.generation) != generation);
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
        let generation = self.node_generation(document);
        self.custom_elements.when_defined.retain(|(doc, candidate, promise)| {
            if doc.node == document && Some(doc.generation) == generation && candidate == name { result.push(*promise); false } else { true }
        });
        result
    }
    pub(crate) fn custom_element_tree_inserted(&mut self, root: NodeIdx) {
        if self.custom_elements.suppress_tree_reactions || self.custom_elements_idle() { return; }
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
                // Insertion tries to upgrade only a connected element.
                CustomElementState::Undefined if self.is_connected(element) => self.queue_custom_element_upgrade(element),
                _ => {}
            }
        }
    }
    pub(crate) fn custom_element_tree_removed(&mut self, root: NodeIdx) {
        if self.custom_elements.suppress_tree_reactions || self.custom_elements_idle() { return; }
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
        if moving || self.custom_elements_idle() || !self.is_connected(root) { return; }
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
        if self.custom_elements_idle() { return; }
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
        namespace: Option<&str>, name: &str, old_value: Option<Vec<u16>>, new_value: Option<Vec<u16>>) {
        if self.custom_elements_idle() { return; }
        if namespace.is_none() && matches!(name, "form" | "id" | "disabled") { self.face_sync_all(); }
        if self.custom_element_state(element) != CustomElementState::Custom { return; }
        let Some(id) = self.custom_element_definition_of(element) else { return };
        let def = &self.custom_elements.definitions[id];
        if !def.observes_attribute(name) { return; }
        if let Some(callback) = def.callbacks.attribute_changed {
            self.enqueue_custom_element_reaction(CustomElementReaction::AttributeChanged {
                element, callback, name: name.into(), old_value, new_value,
                namespace: namespace.map(str::to_owned),
            });
        }
    }
}

#[cfg(test)]
mod exact_value_tests {
    use super::*;
    fn custom() -> (Dom, NodeIdx) {
        let mut dom = Dom::new();
        let node = dom.create_element(NodeIdx(0), "x-token");
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-token".into(), "x-token".into(), CallbackId(11));
        definition.observed_attributes = vec!["class".into()];
        definition.callbacks.attribute_changed = Some(CallbackId(12));
        let id = dom.define_custom_element(definition).unwrap();
        dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(node, id).unwrap();
        dom.custom_element_super(id).unwrap();
        dom.finish_custom_element_upgrade(node, id, true).unwrap();
        dom.take_custom_element_reactions();
        (dom, node)
    }
    #[test]
    fn exact_custom_element_queued_set_remove_values_survive_later_mutation() {
        let (mut dom, node) = custom();
        dom.set_attribute_ns_units(node, None, "class", vec![0xd800, 0]).unwrap();
        dom.set_attribute_ns_units(node, None, "class", vec![0xdc00, 0xfffd]).unwrap();
        dom.remove_attribute(node, "class").unwrap();
        dom.set_attribute(node, "class", "later").unwrap();
        let reactions = dom.take_custom_element_reactions();
        assert_eq!(reactions.len(), 4);
        let values: Vec<_> = reactions.into_iter().map(|reaction| {
            let CustomElementReaction::AttributeChanged { old_value, new_value, .. } = reaction else { panic!("attribute reaction"); };
            (old_value, new_value)
        }).collect();
        assert_eq!(values, vec![(None, Some(vec![0xd800, 0])),
            (Some(vec![0xd800, 0]), Some(vec![0xdc00, 0xfffd])),
            (Some(vec![0xdc00, 0xfffd]), None), (None, Some("later".encode_utf16().collect()))]);
    }
    #[test]
    fn exact_custom_element_upgrade_captures_existing_attribute_units() {
        let mut dom = Dom::new();let node = dom.create_element(NodeIdx(0), "x-token");
        dom.set_attribute_ns_units(node, None, "class", vec![0xd800, 32, 0xdc00]).unwrap();
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-token".into(), "x-token".into(), CallbackId(11));
        definition.observed_attributes = vec!["class".into()];definition.callbacks.attribute_changed = Some(CallbackId(12));
        let id = dom.define_custom_element(definition).unwrap();dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(node, id).unwrap();dom.custom_element_super(id).unwrap();
        dom.finish_custom_element_upgrade(node, id, true).unwrap();
        let reactions = dom.take_custom_element_reactions();
        assert_eq!(reactions, vec![CustomElementReaction::AttributeChanged { element: node,
            callback: CallbackId(12), name: "class".into(), old_value: None,
            new_value: Some(vec![0xd800, 32, 0xdc00]), namespace: None }]);
    }
    #[test]
    fn exact_custom_element_upgrade_snapshot_precedes_constructor_mutation() {
        let mut dom = Dom::new();let node = dom.create_element(NodeIdx(0), "x-token");
        dom.set_attribute_ns_units(node, None, "class", vec![0xd800]).unwrap();
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-token".into(), "x-token".into(), CallbackId(11));
        definition.observed_attributes = vec!["class".into()];definition.callbacks.attribute_changed = Some(CallbackId(12));
        let id = dom.define_custom_element(definition).unwrap();dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(node, id).unwrap();dom.custom_element_super(id).unwrap();
        dom.set_attribute_ns_units(node, None, "class", vec![0xdc00]).unwrap();
        dom.finish_custom_element_upgrade(node, id, true).unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![CustomElementReaction::AttributeChanged {
            element: node, callback: CallbackId(12), name: "class".into(), old_value: None,
            new_value: Some(vec![0xd800]), namespace: None }]);
        assert_eq!(dom.attribute_utf16(node, "class"), Some(vec![0xdc00]));
    }
    #[test]
    fn exact_custom_element_failed_upgrade_discards_admitted_value_snapshots() {
        let mut dom = Dom::new();let node = dom.create_element(NodeIdx(0), "x-token");
        dom.set_attribute_ns_units(node, None, "class", vec![0xd800]).unwrap();
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-token".into(), "x-token".into(), CallbackId(11));
        definition.observed_attributes = vec!["class".into()];definition.callbacks.attribute_changed = Some(CallbackId(12));
        let id = dom.define_custom_element(definition).unwrap();dom.take_custom_element_reactions();
        dom.begin_custom_element_upgrade(node, id).unwrap();dom.custom_element_super(id).unwrap();
        assert_eq!(dom.finish_custom_element_upgrade(node, id, false), Err(DomError::TypeError));
        assert!(dom.custom_elements.upgrade_reactions.is_empty());
        assert!(dom.take_custom_element_reactions().is_empty());
        dom.set_attribute_ns_units(node, None, "class", vec![0xdc00]).unwrap();
        assert!(dom.take_custom_element_reactions().is_empty());
    }
    #[test]
    fn exact_custom_element_namespaced_attribute_values_and_observed_filter() {
        let (mut dom, node) = custom();
        dom.set_attribute_ns_units(node, Some("urn:tokens"), "class", vec![0xd800]).unwrap();
        dom.set_attribute_ns_units(node, None, "ignored", vec![0xdc00]).unwrap();
        dom.remove_attribute_ns(node, Some("urn:tokens"), "class").unwrap();
        let reactions = dom.take_custom_element_reactions();
        assert_eq!(reactions.len(), 2);
        assert!(matches!(&reactions[0], CustomElementReaction::AttributeChanged { old_value: None,
            new_value: Some(value), namespace: Some(namespace), .. } if *value == vec![0xd800] && namespace == "urn:tokens"));
        assert!(matches!(&reactions[1], CustomElementReaction::AttributeChanged { old_value: Some(value),
            new_value: None, namespace: Some(namespace), .. } if *value == vec![0xd800] && namespace == "urn:tokens"));
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
                name: "title".into(), old_value: None, new_value: Some("ready".encode_utf16().collect()), namespace: None },
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
            old_value: None, new_value: Some(new), .. } if *new == "foo".encode_utf16().collect::<Vec<_>>()));
        assert!(matches!(&reactions[1], CustomElementReaction::AttributeChanged {
            old_value: Some(old), new_value: None, .. } if *old == "foo".encode_utf16().collect::<Vec<_>>()));
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

#[cfg(test)]
mod fresh_construction_tests {
    use super::*;
    fn fixture() -> (Dom, usize) {
        let mut dom = Dom::new();
        let mut definition = CustomElementDefinition::new(NodeIdx(0), "x-fresh".into(), "x-fresh".into(), CallbackId(71));
        definition.callbacks.attribute_changed = Some(CallbackId(72));
        definition.observed_attributes.push("class".into());
        let definition = dom.define_custom_element(definition).unwrap();
        (dom, definition)
    }
    fn context(dom: &mut Dom) -> AutonomousCreationContext {
        let ElementCreationPlan::Autonomous(context) = dom.prepare_custom_element_creation(dom.render_handle(NodeIdx(0)),
            Some("http://www.w3.org/1999/xhtml"), None, "x-fresh", None, true).unwrap() else { panic!("Autonomous plan"); };
        context
    }
    fn construct(dom: &mut Dom, definition: usize) -> crate::render_mutation::NodeGeneration {
        let cap = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), definition, CallbackId(71)).unwrap();
        dom.consume_html_constructor(cap).unwrap()
    }
    #[test]
    fn fresh_construction_finishes_before_exact_parser_attribute_admission() {
        let (mut dom, definition) = fixture(); let count = dom.node_count();
        let context = context(&mut dom); assert_eq!(dom.node_count(), count);
        let element = construct(&mut dom, definition);
        let CustomElementCreationCompletion::Created(selected) = dom.complete_custom_element_creation(context, Some(element)).unwrap()
            else { panic!("Actual returned node is valid"); };
        assert_eq!(selected, element);
        dom.set_attribute_ns_units(element.node, None, "class", vec![0xd800]).unwrap();
        assert_eq!(dom.take_custom_element_reactions(), vec![CustomElementReaction::AttributeChanged {
            element: element.node, callback: CallbackId(72), name: "class".into(), old_value: None,
            new_value: Some(vec![0xd800]), namespace: None,
        }]);
    }
    #[test]
    fn fresh_and_upgrade_completion_are_distinct_generation_fenced_receipts() {
        let (mut dom, definition) = fixture(); let context = context(&mut dom);
        assert_eq!(dom.custom_element_super(definition), Err(DomError::InvalidStateError));
        let first = construct(&mut dom, definition); let second = construct(&mut dom, definition);
        assert_ne!(first, second);
        assert!(matches!(dom.complete_custom_element_creation(context, Some(second)).unwrap(), CustomElementCreationCompletion::Created(owner) if owner == second));
        let node = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "x-fresh").unwrap();
        dom.begin_custom_element_upgrade(node, definition).unwrap();
        assert_eq!(dom.custom_element_super(definition), Ok(node));
        assert_eq!(dom.custom_element_super(definition), Err(DomError::InvalidStateError));
        dom.finish_custom_element_upgrade(node, definition, true).unwrap();
    }
    #[test]
    fn fresh_constructor_attribute_side_effects_fail_without_initial_value_reconstruction() {
        let (mut dom, definition) = fixture(); let context = context(&mut dom);
        let held = construct(&mut dom, definition);
        dom.set_attribute_ns_units(held.node, None, "class", vec![0xdc00]).unwrap();
        let CustomElementCreationCompletion::Failed(failure) = dom.complete_custom_element_creation(context, Some(held)).unwrap()
            else { panic!("Invalid attrs require real fallback"); };
        assert_eq!(failure.reason(), DomError::NotSupportedError);
        assert_eq!(dom.custom_element_state(held.node), CustomElementState::Custom);
        let fallback = dom.finish_custom_element_creation_failure(failure).unwrap();
        assert_ne!(held, fallback);
        assert!(dom.is_failed_custom_element_creation(fallback.node));
        assert_eq!(dom.take_custom_element_reactions(), vec![CustomElementReaction::AttributeChanged {
            element: held.node, callback: CallbackId(72), name: "class".into(), old_value: None,
            new_value: Some(vec![0xdc00]), namespace: None,
        }]);
    }
}

/// Move-only authority issued from one actual native registry and arena.
/// The host sets the canonical wrapper prototype before consuming this token.
#[derive(Debug)]
pub struct HtmlConstructionCapability {
    arena: std::sync::Arc<()>,
    document: crate::render_mutation::NodeGeneration,
    creation_document: crate::render_mutation::NodeGeneration,
    definition: usize,
    constructor: CallbackId,
    revision: u64,
    element: crate::render_mutation::NodeGeneration,
    stacked: bool,
}
impl HtmlConstructionCapability {
    pub fn element(&self) -> crate::render_mutation::NodeGeneration { self.element }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlConstructorDefinition { pub name: String, pub local_name: String }
impl Dom {
    /// Validates the callee realm's actual document and registered constructor.
    pub fn html_constructor_definition(&self, document: crate::render_mutation::NodeGeneration,
        definition: usize, constructor: CallbackId) -> Result<HtmlConstructorDefinition, DomError> {
        if self.node_generation(document.node) != Some(document.generation)
            || !matches!(self.get(document.node).kind, NodeKind::Document) {
            return Err(DomError::InvalidStateError);
        }
        let def = self.custom_elements.definitions.get(definition).ok_or(DomError::TypeError)?;
        if def.document != document.node || def.document_generation != document.generation || def.constructor != constructor { return Err(DomError::TypeError); }
        Ok(HtmlConstructorDefinition { name: def.name.clone(), local_name: def.local_name.clone() })
    }
    /// Called after the author prototype getter, with the original captured arena.
    pub fn prepare_html_constructor(&mut self, document: crate::render_mutation::NodeGeneration,
        definition: usize, constructor: CallbackId) -> Result<HtmlConstructionCapability, DomError> {
        self.prepare_html_constructor_in_document(document, document, definition, constructor)
    }
    pub fn prepare_html_constructor_in_document(&mut self, document: crate::render_mutation::NodeGeneration,
        creation_document: crate::render_mutation::NodeGeneration, definition: usize, constructor: CallbackId)
        -> Result<HtmlConstructionCapability, DomError> {
        let description = self.html_constructor_definition(document, definition, constructor)?;
        if self.node_generation(creation_document.node) != Some(creation_document.generation)
            || !matches!(self.get(creation_document.node).kind, NodeKind::Document) {
            return Err(DomError::InvalidStateError);
        }
        let def = &self.custom_elements.definitions[definition];
        let revision = def.construction_revision;
        let top = def.construction_stack.last().copied();
        let (element, stacked) = match top {
            Some(ConstructionEntry::AlreadyConstructed) => return Err(DomError::TypeError),
            Some(ConstructionEntry::Element(element)) => {
                if !self.custom_elements.upgrade_reactions.iter().any(|(owner, id, _)|
                    owner.node == element && *id == definition
                        && self.node_generation(element) == Some(owner.generation))
                    || self.custom_element_definition_of(element) != Some(definition)
                    || self.custom_element_state(element) != CustomElementState::Precustomized {
                    return Err(DomError::InvalidStateError);
                }
                (element, true)
            }
            None => {
                let element = self.create_element_ns_in(creation_document.node,
                    Some("http://www.w3.org/1999/xhtml"), &description.local_name)?;
                if description.name != description.local_name {
                    self.initialize_custom_element_is_value(element, self.node_generation(element).unwrap(),
                        Some(&description.name.encode_utf16().collect::<Vec<_>>()))?;
                }
                self.custom_elements.element_states.push((self.render_handle(element), definition, CustomElementState::Custom));
                self.record_render_full(element, "direct custom element construction");
                (element, false)
            }
        };
        Ok(HtmlConstructionCapability { arena: self.token_list_arena.clone(), document, creation_document,
            definition, constructor, revision, element: self.render_handle(element), stacked })
    }
    /// Marks real super construction once, after successful wrapper prototype setup.
    pub fn consume_html_constructor(&mut self, capability: HtmlConstructionCapability)
        -> Result<crate::render_mutation::NodeGeneration, DomError> {
        if !std::sync::Arc::ptr_eq(&self.token_list_arena, &capability.arena)
            || self.node_generation(capability.element.node) != Some(capability.element.generation)
            || self.node_generation(capability.creation_document.node) != Some(capability.creation_document.generation) {
            return Err(DomError::InvalidStateError);
        }
        self.html_constructor_definition(capability.document, capability.definition, capability.constructor)?;
        let def = &self.custom_elements.definitions[capability.definition];
        if def.construction_revision != capability.revision { return Err(DomError::InvalidStateError); }
        if capability.stacked {
            if def.construction_stack.last() != Some(&ConstructionEntry::Element(capability.element.node)) {
                return Err(DomError::InvalidStateError);
            }
            self.custom_element_super(capability.definition)?;
        } else {
            if !def.construction_stack.is_empty() { return Err(DomError::InvalidStateError); }
            self.custom_elements.definitions[capability.definition].construction_revision += 1;
        }
        Ok(capability.element)
    }
}

#[cfg(test)]
mod html_constructor_tests {
    use super::*;
    fn registry(dom: &mut Dom, local: &str) -> usize {
        dom.define_custom_element(CustomElementDefinition::new(NodeIdx(0),
            "x-native-constructor".into(), local.into(), CallbackId(91))).unwrap()
    }
    #[test]
    fn direct_new_allocates_native_custom_node_in_actual_document() {
        for local in ["x-native-constructor", "button"] {
            let mut dom = Dom::new(); let id = registry(&mut dom, local);
            let document = dom.render_handle(NodeIdx(0));
            let capability = dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
            let element = capability.element();
            assert_eq!(dom.local_name(element.node), Some(local));
            assert_eq!(dom.owner_document(element.node), Some(document.node));
            assert_eq!(dom.custom_element_state(element.node), CustomElementState::Custom);
            assert_eq!(dom.custom_element_definition_of(element.node), Some(id));
            assert_eq!(dom.get(element.node).parent, None);
            assert_eq!(dom.consume_html_constructor(capability), Ok(element));
        }
    }
    #[test]
    fn upgrade_super_returns_same_node_and_rejects_already_constructed() {
        let mut dom = Dom::new(); let id = registry(&mut dom, "x-native-constructor");
        let node = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "x-native-constructor").unwrap();
        dom.begin_custom_element_upgrade(node, id).unwrap();
        let document = dom.render_handle(NodeIdx(0));
        let capability = dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
        assert_eq!(capability.element().node, node);
        assert_eq!(dom.consume_html_constructor(capability).unwrap().node, node);
        assert!(matches!(dom.prepare_html_constructor(document, id, CallbackId(91)), Err(DomError::TypeError)));
        dom.finish_custom_element_upgrade(node, id, true).unwrap();
    }
    #[test]
    fn foreign_arena_cannot_consume_identical_numeric_identity() {
        let mut a = Dom::new(); let mut b = Dom::new();
        let id = registry(&mut a, "x-native-constructor"); registry(&mut b, "x-native-constructor");
        let document = a.render_handle(NodeIdx(0));
        let capability = a.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
        assert_eq!(b.consume_html_constructor(capability), Err(DomError::InvalidStateError));
    }
    #[test]
    fn wrong_constructor_document_generation_and_document_kind_reject() {
        let mut dom = Dom::new(); let id = registry(&mut dom, "x-native-constructor");
        let document = dom.render_handle(NodeIdx(0));
        assert!(matches!(dom.prepare_html_constructor(document, id, CallbackId(92)), Err(DomError::TypeError)));
        let stale = crate::render_mutation::NodeGeneration { generation: document.generation + 1, ..document };
        assert!(matches!(dom.prepare_html_constructor(stale, id, CallbackId(91)), Err(DomError::InvalidStateError)));
        let node = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "div").unwrap();
        assert!(matches!(dom.prepare_html_constructor(dom.render_handle(node), id, CallbackId(91)), Err(DomError::InvalidStateError)));
    }
    #[test]
    fn stack_revision_fences_reentrant_or_competing_capabilities() {
        let mut dom = Dom::new(); let id = registry(&mut dom, "x-native-constructor");
        let document = dom.render_handle(NodeIdx(0));
        let first = dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
        let second = dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
        dom.consume_html_constructor(second).unwrap();
        assert_eq!(dom.consume_html_constructor(first), Err(DomError::InvalidStateError));
    }
    #[test]
    fn failed_prototype_setup_does_not_mark_pending_super_constructed() {
        let mut dom = Dom::new(); let id = registry(&mut dom, "x-native-constructor");
        let node = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "x-native-constructor").unwrap();
        dom.begin_custom_element_upgrade(node, id).unwrap();
        let document = dom.render_handle(NodeIdx(0));
        drop(dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap());
        let retry = dom.prepare_html_constructor(document, id, CallbackId(91)).unwrap();
        dom.consume_html_constructor(retry).unwrap();
        dom.finish_custom_element_upgrade(node, id, true).unwrap();
    }
}

impl Dom {
    /// Immutable registry custody captured when the definition was registered.
    pub fn custom_element_definition_document(&self, definition: usize)
        -> Option<crate::render_mutation::NodeGeneration> {
        self.custom_element_definition(definition).map(|def| crate::render_mutation::NodeGeneration {
            node: def.document, generation: def.document_generation })
    }
    /// Private creation-time is value, independent of the mutable content attribute.
    pub fn custom_element_is_value(&self, element: NodeIdx) -> Result<Option<&[u16]>, DomError> {
        self.check_node(element)?;
        if !matches!(self.get(element).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        let generation = self.node_generation(element);
        Ok(self.custom_elements.is_values.get(element.0 as usize).and_then(Option::as_ref)
            .filter(|(owner_generation, _)| Some(*owner_generation) == generation)
            .and_then(|(_, value)| value.as_deref()))
    }
    /// Native creation/parsed-transfer hook, never an authored attribute setter.
    pub fn initialize_custom_element_is_value(&mut self, element: NodeIdx, expected_generation: u64,
        value: Option<&[u16]>) -> Result<(), DomError> {
        self.check_node(element)?;
        if self.node_generation(element) != Some(expected_generation)
            || !matches!(self.get(element).kind, NodeKind::Element { .. }) { return Err(DomError::InvalidStateError); }
        let slot = element.0 as usize;
        if let Some((generation, current)) = self.custom_elements.is_values.get(slot).and_then(Option::as_ref) {
            if *generation == expected_generation {
                return if current.as_deref() == value { Ok(()) } else { Err(DomError::InvalidStateError) };
            }
        }
        self.custom_elements.is_values.resize_with(self.custom_elements.is_values.len().max(slot + 1), || None);
        self.custom_elements.is_values[slot] = Some((expected_generation, value.map(<[u16]>::to_vec)));
        Ok(())
    }
    pub(crate) fn copy_custom_element_is_value(&mut self, source: &Dom, source_node: NodeIdx,
        target: NodeIdx) -> Result<(), DomError> {
        if matches!(self.get(target).kind, NodeKind::Element { .. }) {
            self.initialize_custom_element_is_value(target, self.node_generation(target).ok_or(DomError::InvalidNode)?,
                source.custom_element_is_value(source_node)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod constructor_custody_tests {
    use super::*;
    #[test]
    fn recycled_document_never_revives_registry_constructor_name_or_waiters() {
        let mut dom = Dom::new();
        let doc = dom.create_document();
        let old = dom.render_handle(doc);
        let id = dom.define_custom_element(CustomElementDefinition::new(doc,
            "x-registry".into(), "x-registry".into(), CallbackId(10))).unwrap();
        dom.custom_element_when_defined(doc, "x-later".into(), CallbackId(11));
        dom.custom_element_begin_definition(doc).unwrap();
        dom.retire_unreachable_node(doc).unwrap();
        let new_doc = dom.create_document();
        assert_eq!(new_doc, doc);
        assert_ne!(dom.node_generation(doc), Some(old.generation));
        assert!(dom.custom_element_definitions(doc).is_empty());
        assert!(dom.custom_element_definition(id).is_none());
        assert!(dom.custom_element_definition_document(id).is_none());
        assert!(dom.custom_element_definition_by_name(doc, "x-registry").is_none());
        assert!(dom.custom_element_pending_promise(doc, "x-later").is_none());
        assert!(dom.take_custom_element_when_defined(doc, "x-later").is_empty());
        assert!(dom.html_constructor_definition(dom.render_handle(doc), id, CallbackId(10)).is_err());
        dom.custom_element_begin_definition(doc).unwrap();
        dom.custom_element_end_definition(doc);
        let new_id = dom.define_custom_element(CustomElementDefinition::new(doc,
            "x-registry".into(), "x-registry".into(), CallbackId(10))).unwrap();
        assert_ne!(new_id, id);
        assert_eq!(dom.custom_element_definition_document(new_id), Some(dom.render_handle(doc)));
    }
    #[test]
    fn private_is_value_survives_content_mutation_clone_import_and_serialization() {
        let mut dom = Dom::new();
        let id = dom.define_custom_element(CustomElementDefinition::new(NodeIdx(0),
            "x-button".into(), "button".into(), CallbackId(12))).unwrap();
        let capability = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), id, CallbackId(12)).unwrap();
        let node = dom.consume_html_constructor(capability).unwrap().node;
        let expected: Vec<_> = "x-button".encode_utf16().collect();
        assert_eq!(dom.custom_element_is_value(node).unwrap(), Some(expected.as_slice()));
        assert_eq!(dom.get_attribute_ns(node, None, "is"), None);
        dom.set_attribute(node, "id", "real").unwrap();
        assert_eq!(String::from_utf16(&dom.serialize_node(node).unwrap()).unwrap(),
            "<button is=\"x-button\" id=\"real\"></button>");
        dom.set_attribute(node, "is", "author").unwrap();
        assert_eq!(dom.custom_element_is_value(node).unwrap(), Some(expected.as_slice()));
        assert_eq!(String::from_utf16(&dom.serialize_node(node).unwrap()).unwrap(),
            "<button id=\"real\" is=\"author\"></button>");
        dom.remove_attribute(node, "is").unwrap();
        let clone = dom.clone_node(node, false).unwrap();
        let imported = dom.import_node(NodeIdx(0), node, false).unwrap();
        for copy in [clone, imported] {
            assert_eq!(dom.custom_element_is_value(copy).unwrap(), Some(expected.as_slice()));
            assert_eq!(dom.get_attribute_ns(copy, None, "is"), None);
            assert_eq!(dom.custom_element_state(copy), CustomElementState::Undefined);
        }
    }
    #[test]
    fn parsed_is_value_preserves_exact_units_without_author_attribute_inference() {
        let mut source = Dom::new();
        let node = source.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "button").unwrap();
        let units = [0xd800, 0x22, 0x26, 0xdc00];
        source.initialize_custom_element_is_value(node, source.node_generation(node).unwrap(), Some(&units)).unwrap();
        assert!(source.initialize_custom_element_is_value(node, source.node_generation(node).unwrap(), Some(&[1])).is_err());
        let mut target = Dom::new();
        let copy = target.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "button").unwrap();
        target.copy_custom_element_is_value(&source, node, copy).unwrap();
        assert_eq!(target.custom_element_is_value(copy).unwrap(), Some(units.as_slice()));
        target.set_attribute(copy, "is", "changed").unwrap();
        assert_eq!(target.custom_element_is_value(copy).unwrap(), Some(units.as_slice()));
        let ordinary = target.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "button").unwrap();
        target.set_attribute(ordinary, "is", "x-fake").unwrap();
        assert_eq!(target.custom_element_is_value(ordinary).unwrap(), None);
    }
    #[test]
    fn recycled_element_does_not_inherit_private_is_or_custom_state() {
        let mut dom = Dom::new();
        let id = dom.define_custom_element(CustomElementDefinition::new(NodeIdx(0),
            "x-button".into(), "button".into(), CallbackId(12))).unwrap();
        let capability = dom.prepare_html_constructor(dom.render_handle(NodeIdx(0)), id, CallbackId(12)).unwrap();
        let old = dom.consume_html_constructor(capability).unwrap();
        dom.retire_unreachable_node(old.node).unwrap();
        let fresh = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "button").unwrap();
        assert_eq!(old.node, fresh);
        assert_eq!(dom.custom_element_is_value(fresh).unwrap(), None);
        assert_eq!(dom.custom_element_state(fresh), CustomElementState::Undefined);
        assert_eq!(dom.custom_element_definition_of(fresh), None);
        assert!(dom.initialize_custom_element_is_value(fresh, old.generation, Some(&[1])).is_err());
    }
}

#[cfg(test)]
mod is_value_namespace_tests {
    use super::*;
    #[test]
    fn non_html_creation_and_clone_keep_private_is_without_customization() {
        let mut dom = Dom::new();
        let svg = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "circle").unwrap();
        dom.initialize_custom_element_is_value(svg, dom.node_generation(svg).unwrap(), Some(&[0xd800])).unwrap();
        let clone = dom.clone_node(svg, false).unwrap();
        assert_eq!(dom.custom_element_is_value(clone).unwrap(), Some([0xd800].as_slice()));
        assert_eq!(dom.custom_element_state(clone), CustomElementState::Undefined);
        assert_eq!(dom.get_attribute_ns(clone, None, "is"), None);
    }
    #[test]
    fn fresh_autonomous_completion_clears_creation_options_is() {
        let mut dom = Dom::new();
        let id = dom.define_custom_element(CustomElementDefinition::new(NodeIdx(0),
            "x-autonomous".into(), "x-autonomous".into(), CallbackId(13))).unwrap();
        let node = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "x-autonomous").unwrap();
        let generation = dom.node_generation(node).unwrap();
        dom.initialize_custom_element_is_value(node, generation, Some(&[1])).unwrap();
        let ElementCreationPlan::Autonomous(context) = dom.prepare_custom_element_creation(dom.render_handle(NodeIdx(0)),
            Some("http://www.w3.org/1999/xhtml"), None, "x-autonomous", Some(&[1]), true).unwrap()
            else { panic!("Actual autonomous creation context"); };
        assert!(matches!(dom.complete_custom_element_creation(context, Some(dom.render_handle(node))).unwrap(),
            CustomElementCreationCompletion::Created(owner) if owner.node == node));
        assert_eq!(dom.custom_element_is_value(node).unwrap(), None);
    }
}


#[path = "custom_element_creation.rs"]
mod creation;
pub use creation::{AutonomousCreationContext, ElementCreationPlan, CustomElementCreationCompletion,
    CustomElementCreationFailure};
impl Dom {
    pub fn is_failed_custom_element_creation(&self, element: NodeIdx) -> bool {
        self.custom_elements.failed_creation_nodes.get(element.0 as usize).copied().flatten()
            .is_some_and(|generation| self.node_generation(element) == Some(generation))
    }
}

#[cfg(test)]
mod isolated_reaction_frame_tests {
    use super::*;
    fn custom(dom: &mut Dom, name: &str, callback: u64) -> NodeIdx {
        let mut definition = CustomElementDefinition::new(NodeIdx(0), name.into(), name.into(), CallbackId(callback));
        definition.observed_attributes.push("data-a".into());
        definition.callbacks.attribute_changed = Some(CallbackId(callback + 10));
        let definition = dom.define_custom_element(definition).unwrap();
        let document = dom.render_handle(NodeIdx(0));
        let cap = dom.prepare_html_constructor_in_document(document, document, definition, CallbackId(callback)).unwrap();
        let element = cap.element().node; dom.consume_html_constructor(cap).unwrap(); element
    }
    #[test]
    fn isolated_nested_frames_preserve_backup_and_exact_attribute_units() {
        let mut dom = Dom::new(); let a = custom(&mut dom, "x-frame-a", 1);
        dom.set_attribute(a, "data-a", "backup").unwrap();
        dom.custom_element_push_reaction_frame();
        dom.set_attribute(a, "data-a", "outer").unwrap();
        dom.custom_element_push_reaction_frame();
        dom.set_attribute_ns_units(a, None, "data-a", vec![0xD800]).unwrap();
        let mut inner = dom.custom_element_pop_isolated_reaction_frame().unwrap();
        let mut outer = dom.custom_element_pop_isolated_reaction_frame().unwrap();
        assert!(matches!(dom.custom_element_next_isolated_reaction(&mut inner).unwrap(),
            Some(CustomElementReaction::AttributeChanged { old_value: Some(old), new_value: Some(new), .. })
            if old == "outer".encode_utf16().collect::<Vec<_>>() && new == vec![0xD800]));
        assert!(dom.custom_element_next_isolated_reaction(&mut inner).unwrap().is_none());
        assert!(matches!(dom.custom_element_next_isolated_reaction(&mut outer).unwrap(),
            Some(CustomElementReaction::AttributeChanged { new_value: Some(new), .. }) if new == "outer".encode_utf16().collect::<Vec<_>>()));
        assert!(matches!(dom.take_custom_element_reactions().as_slice(),
            [CustomElementReaction::AttributeChanged { old_value: None, new_value: Some(new), .. }] if new == &"backup".encode_utf16().collect::<Vec<_>>()));
    }
    #[test]
    fn isolated_frame_rejects_foreign_arena_without_consuming_payload() {
        let mut original = Dom::new(); let a = custom(&mut original, "x-frame-owner", 2);
        original.custom_element_push_reaction_frame(); original.set_attribute(a, "data-a", "original").unwrap();
        let mut frame = original.custom_element_pop_isolated_reaction_frame().unwrap();
        let foreign = Dom::new();
        assert_eq!(foreign.custom_element_next_isolated_reaction(&mut frame), Err(DomError::InvalidStateError));
        assert!(matches!(original.custom_element_next_isolated_reaction(&mut frame).unwrap(), Some(CustomElementReaction::AttributeChanged { element, .. }) if element == a));
    }
    #[test]
    fn isolated_frame_skips_recycled_target_and_keeps_later_live_reaction() {
        let mut dom = Dom::new(); let dead = custom(&mut dom, "x-frame-dead", 3); let live = custom(&mut dom, "x-frame-live", 4);
        dom.custom_element_push_reaction_frame(); dom.set_attribute(dead, "data-a", "dead").unwrap(); dom.set_attribute(live, "data-a", "live").unwrap();
        let mut frame = dom.custom_element_pop_isolated_reaction_frame().unwrap();
        let old = dom.render_handle(dead); dom.retire_unreachable_node(dead).unwrap();
        let replacement = dom.create_element(NodeIdx(0), "div");
        assert_eq!(replacement, dead); assert_ne!(dom.node_generation(replacement), Some(old.generation));
        assert!(matches!(dom.custom_element_next_isolated_reaction(&mut frame).unwrap(), Some(CustomElementReaction::AttributeChanged { element, .. }) if element == live));
        assert!(dom.custom_element_next_isolated_reaction(&mut frame).unwrap().is_none());
    }
}
