//! Engine-internal mutation journal for incremental rendering.
//! Entries are independent of script-visible MutationObserver records.
use crate::{Dom, NodeIdx, NodeKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeGeneration {
    pub node: NodeIdx,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeToken {
    pub namespace_uri: Option<String>,
    pub local_name: String,
    pub value: Vec<u16>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectorTokens {
    pub id: Option<Vec<u16>>,
    pub classes: Vec<Vec<u16>>,
    pub attributes: Vec<AttributeToken>,
    pub states: Vec<&'static str>,
}

impl SelectorTokens {
    pub(crate) fn restore_attribute(&mut self, namespace_uri: Option<&str>,
        local_name: &str, value: Option<Vec<u16>>) {
        self.attributes.retain(|attribute| attribute.namespace_uri.as_deref() != namespace_uri
            || attribute.local_name != local_name);
        if let Some(value) = value {
            self.attributes.push(AttributeToken { namespace_uri: namespace_uri.map(str::to_owned),
                local_name: local_name.into(), value });
        }
        self.id = self.attributes.iter().find(|attribute| attribute.namespace_uri.is_none()
            && attribute.local_name == "id").map(|attribute| attribute.value.clone());
        self.classes = self.attributes.iter().find(|attribute| attribute.namespace_uri.is_none()
            && attribute.local_name == "class").map_or_else(Vec::new, |attribute|
                attribute.value.split(|unit| matches!(*unit, 9 | 10 | 12 | 13 | 32))
                    .filter(|token| !token.is_empty()).map(|token| token.to_vec()).collect());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructuralScope {
    Document(NodeGeneration),
    ShadowRoot(NodeGeneration),
    Detached(NodeGeneration),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderMutationKind {
    Tree {
        parent: NodeGeneration,
        previous_sibling: Option<NodeGeneration>,
        next_sibling: Option<NodeGeneration>,
        inserted: bool,
        scope: StructuralScope,
        old_tokens: SelectorTokens,
        new_tokens: SelectorTokens,
    },
    Attribute {
        namespace_uri: Option<String>,
        local_name: String,
        old_value: Option<Vec<u16>>,
        new_value: Option<Vec<u16>>,
        old_tokens: SelectorTokens,
        new_tokens: SelectorTokens,
    },
    CharacterData { old_value: Vec<u16>, new_value: Vec<u16> },
    State { name: &'static str, old_value: bool, new_value: bool },
    SlotAssignment { old: Vec<NodeGeneration>, new: Vec<NodeGeneration> },
    RequiresFullRebuild { document: NodeGeneration, reason: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderMutation {
    pub epoch: u64,
    pub target: NodeGeneration,
    pub kind: RenderMutationKind,
}

impl Dom {
    /// Returns the current generation of a live arena slot.
    pub fn node_generation(&self, node: NodeIdx) -> Option<u64> {
        self.node_slot_live.get(node.0 as usize).copied().filter(|live| *live)
            .map(|_| self.node_generations[node.0 as usize])
    }

    pub fn render_mutation_epoch(&self) -> u64 { self.render_epoch }

    /// Drain only mutations visible at the captured rendering-update epoch.
    /// Mutations made while style/layout runs remain queued for the next update.
    pub fn take_render_mutations(&mut self, epoch: u64) -> Vec<RenderMutation> {
        let boundary = self.render_mutations.partition_point(|entry| entry.epoch <= epoch);
        self.render_mutations.drain(..boundary).collect()
    }

    pub(crate) fn render_handle(&self, node: NodeIdx) -> NodeGeneration {
        NodeGeneration { node, generation: self.node_generations[node.0 as usize] }
    }

    pub(crate) fn record_render_mutation(&mut self, target: NodeIdx, kind: RenderMutationKind) {
        self.render_epoch = self.render_epoch.checked_add(1).expect("render mutation epoch exhausted");
        let target = self.render_handle(target);
        if self.render_mutations.len() >= 65_536 {
            let first_epoch = self.render_mutations.first().map_or(self.render_epoch, |entry| entry.epoch);
            self.render_mutations.clear();
            let document = self.owner_document(target.node).unwrap_or(target.node);
            let document = self.render_handle(document);
            self.render_mutations.push(RenderMutation { epoch: first_epoch, target,
                kind: RenderMutationKind::RequiresFullRebuild { document, reason: "journal capacity" } });
        } else {
            self.render_mutations.push(RenderMutation { epoch: self.render_epoch, target, kind });
        }
    }

    pub(crate) fn render_scope(&self, parent: NodeIdx) -> StructuralScope {
        let root = self.root(parent);
        let handle = self.render_handle(root);
        if self.is_shadow_root(root) { StructuralScope::ShadowRoot(handle) }
        else if matches!(self.get(root).kind, NodeKind::Document) { StructuralScope::Document(handle) }
        else { StructuralScope::Detached(handle) }
    }

    pub(crate) fn selector_tokens(&self, node: NodeIdx) -> SelectorTokens {
        let Some(attributes) = self.attributes_expanded(node) else { return SelectorTokens::default() };
        let mut tokens = SelectorTokens::default();
        for attribute in attributes {
            let value: Vec<u16> = attribute.value.encode_utf16().collect();
            if attribute.namespace_uri.is_none() && attribute.local_name == "id" {
                tokens.id = Some(value.clone());
            }
            if attribute.namespace_uri.is_none() && attribute.local_name == "class" {
                tokens.classes = value.split(|unit| matches!(*unit, 9 | 10 | 12 | 13 | 32))
                    .filter(|token| !token.is_empty()).map(|token| token.to_vec()).collect();
            }
            tokens.attributes.push(AttributeToken {
                namespace_uri: attribute.namespace_uri.map(str::to_owned),
                local_name: attribute.local_name.to_owned(), value,
            });
        }
        if self.local_name(node) == Some("input") && self.input_checked(node).unwrap_or(false) {
            tokens.states.push("checked");
        }
        if self.local_name(node) == Some("option") && self.option_selected(node).unwrap_or(false) {
            tokens.states.push("selected");
        }
        tokens
    }

    pub(crate) fn record_render_state(&mut self, target: NodeIdx, name: &'static str,
        old_value: bool, new_value: bool) {
        if old_value != new_value {
            self.record_render_mutation(target, RenderMutationKind::State {
                name, old_value, new_value,
            });
        }
    }

    pub(crate) fn record_render_full(&mut self, target: NodeIdx, reason: &'static str) {
        let document = self.owner_document(target).unwrap_or(target);
        let document = self.render_handle(document);
        self.record_render_mutation(target,
            RenderMutationKind::RequiresFullRebuild { document, reason });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};

    #[test]
    fn tree_attribute_text_and_state_are_ordered_and_drain_at_epoch() {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "html");
        let input = dom.create_element(root, "input");
        let initial = dom.render_mutation_epoch();
        let insertion = dom.take_render_mutations(initial);
        assert!(insertion.iter().any(|entry| entry.target.node == input
            && matches!(entry.kind, RenderMutationKind::Tree { inserted: true,
                scope: StructuralScope::Document(_), .. })));
        assert!(dom.take_render_mutations(initial).is_empty());

        dom.set_attribute(input, "id", "before").unwrap();
        let boundary = dom.render_mutation_epoch();
        dom.set_attribute(input, "id", "after").unwrap();
        let first = dom.take_render_mutations(boundary);
        assert_eq!(first.len(), 1);
        assert!(matches!(&first[0].kind, RenderMutationKind::Attribute {
            old_value: None, new_value: Some(_), new_tokens, ..
        } if new_tokens.id == Some("before".encode_utf16().collect())));
        let second = dom.take_render_mutations(dom.render_mutation_epoch());
        assert!(matches!(&second[0].kind, RenderMutationKind::Attribute {
            old_tokens, new_tokens, ..
        } if old_tokens.id == Some("before".encode_utf16().collect())
            && new_tokens.id == Some("after".encode_utf16().collect())));

        let text = dom.create_text(root, "old");
        dom.take_render_mutations(dom.render_mutation_epoch());
        dom.replace_character_data(text, 0, 3, &"new".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert!(dom.take_render_mutations(dom.render_mutation_epoch()).iter().any(|entry|
            matches!(&entry.kind, RenderMutationKind::CharacterData { old_value, new_value }
                if *old_value == "old".encode_utf16().collect::<Vec<_>>()
                    && *new_value == "new".encode_utf16().collect::<Vec<_>>())));
        dom.set_input_checked(input, true).unwrap();
        assert!(dom.take_render_mutations(dom.render_mutation_epoch()).iter().any(|entry|
            matches!(entry.kind, RenderMutationKind::State { name: "checked",
                old_value: false, new_value: true })));
        dom.detach(input);
        assert!(dom.take_render_mutations(dom.render_mutation_epoch()).iter().any(|entry|
            matches!(entry.kind, RenderMutationKind::Tree { inserted: false, .. })));
    }

    #[test]
    fn shadow_slot_journal_and_full_rebuild_fallback() {
        let mut dom = Dom::new();
        let host = dom.create_element(NodeIdx(0), "div");
        let root = dom.attach_shadow(host, ShadowOptions {
            mode: ShadowMode::Open, delegates_focus: false,
            slot_assignment: SlotAssignment::Named, clonable: false, serializable: false,
        }).unwrap();
        assert!(dom.take_render_mutations(dom.render_mutation_epoch()).iter().any(|entry|
            matches!(entry.kind, RenderMutationKind::RequiresFullRebuild {
                reason: "shadow root attached", .. })));
        let slot = dom.create_element(root, "slot");
        dom.take_render_mutations(dom.render_mutation_epoch());
        dom.create_element(host, "span");
        assert!(dom.take_render_mutations(dom.render_mutation_epoch()).iter().any(|entry|
            entry.target.node == slot && matches!(entry.kind,
                RenderMutationKind::SlotAssignment { .. })));
        assert_eq!(dom.node_generation(host), Some(1));
        assert_eq!(dom.node_generation(slot), Some(1));
    }

    #[test]
    fn recycled_slot_has_new_generation_and_old_journal_identity_is_stable() {
        let mut dom = Dom::new();
        let node = dom.alloc_unattached(NodeKind::Element {
            name: "old".into(), attrs: Vec::new(), namespace: crate::Namespace::Html,
        });
        let first = dom.node_generation(node).unwrap();
        dom.record_render_full(node, "old node");
        dom.retire_unreachable_node(node).unwrap();
        assert_eq!(dom.node_generation(node), None);
        let replacement = dom.alloc_unattached(NodeKind::Element {
            name: "new".into(), attrs: Vec::new(), namespace: crate::Namespace::Html,
        });
        assert_eq!(replacement, node);
        assert_eq!(dom.node_generation(node), Some(first + 1));
        let entries = dom.take_render_mutations(dom.render_mutation_epoch());
        assert_eq!(entries[0].target, NodeGeneration { node, generation: first });
        dom.record_render_full(replacement, "new node");
        let entries = dom.take_render_mutations(dom.render_mutation_epoch());
        assert_eq!(entries[0].target, NodeGeneration { node, generation: first + 1 });
    }
}
