//! DOM Standard §4.3. Owned observer deliveries; the embedding owns scheduling.
use crate::{Dom, DomError, NodeIdx, NodeKind};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObserverId(usize);
impl ObserverId {
    pub fn slot(self) -> usize { self.0 }
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutationRecordId(usize);
impl MutationRecordId {
    pub fn slot(self) -> usize { self.0 }
    pub fn from_slot(slot: usize) -> Self { Self(slot) }
}
#[derive(Debug, Clone)]
pub(crate) struct RegisteredRecord {
    record: MutationRecord,
    added_nodes: usize,
    removed_nodes: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObserveOptions {
    pub child_list: bool,
    pub attributes: Option<bool>,
    pub character_data: Option<bool>,
    pub subtree: bool,
    pub attribute_old_value: Option<bool>,
    pub character_data_old_value: Option<bool>,
    pub attribute_filter: Option<Vec<String>>,
}
impl ObserveOptions {
    fn normalize(mut self) -> Result<Self, DomError> {
        if self.attributes.is_none()
            && (self.attribute_old_value.is_some() || self.attribute_filter.is_some())
        {
            self.attributes = Some(true);
        }
        if self.character_data.is_none() && self.character_data_old_value.is_some() {
            self.character_data = Some(true);
        }
        if (!self.child_list && self.attributes != Some(true) && self.character_data != Some(true))
            || (self.attributes == Some(false)
                && (self.attribute_old_value == Some(true) || self.attribute_filter.is_some()))
            || (self.character_data == Some(false) && self.character_data_old_value == Some(true))
        {
            return Err(DomError::TypeError);
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationType {
    ChildList,
    Attributes,
    CharacterData,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationRecord {
    pub kind: MutationType,
    pub target: NodeIdx,
    pub attribute_name: Option<String>,
    pub attribute_namespace: Option<String>,
    pub old_value: Option<String>,
    /// Exact CharacterData old value for DOMString bindings (including lone surrogates).
    pub old_value_utf16: Option<Vec<u16>>,
    pub added_nodes: Vec<NodeIdx>,
    pub removed_nodes: Vec<NodeIdx>,
    pub previous_sibling: Option<NodeIdx>,
    pub next_sibling: Option<NodeIdx>,
}
impl MutationRecord {
    fn new(kind: MutationType, target: NodeIdx) -> Self {
        Self {
            kind,
            target,
            attribute_name: None,
            attribute_namespace: None,
            old_value: None,
            old_value_utf16: None,
            added_nodes: vec![],
            removed_nodes: vec![],
            previous_sibling: None,
            next_sibling: None,
        }
    }
}
#[derive(Debug)]
struct Observer {
    callback: u64,
    records: Vec<MutationRecord>,
}
#[derive(Debug, Clone)]
struct Registration {
    id: u64,
    target: NodeIdx,
    observer: ObserverId,
    options: ObserveOptions,
    source: Option<u64>,
}
#[derive(Debug, Default)]
pub(crate) struct MutationObservers {
    observers: Vec<Observer>,
    registered: Vec<Registration>,
    pending: Vec<ObserverId>,
    signal_slots: Vec<NodeIdx>,
    queued: bool,
    next_registration: u64,
}
/// A frozen notify-set, not pre-drained records. Callback A may mutate records
/// for callback B before B's turn; taking every queue at once would be wrong.
#[derive(Debug)]
pub struct MutationCheckpoint {
    remaining: VecDeque<ObserverId>,
    slots: VecDeque<NodeIdx>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct MutationDelivery {
    pub observer: ObserverId,
    pub callback: u64,
    pub records: Vec<MutationRecord>,
}

impl MutationObservers {
    fn queue(&mut self, ancestors: &[NodeIdx], record: MutationRecord) {
        let mut interested: Vec<(ObserverId, bool)> = vec![];
        for node in ancestors {
            for reg in self.registered.iter().filter(|r| r.target == *node) {
                let opts = &reg.options;
                if *node != record.target && !opts.subtree {
                    continue;
                }
                let old_value = match record.kind {
                    MutationType::ChildList if opts.child_list => false,
                    MutationType::Attributes if opts.attributes == Some(true) => {
                        if opts.attribute_filter.as_ref().is_some_and(|filter| {
                            record.attribute_namespace.is_some()
                                || !record
                                    .attribute_name
                                    .as_ref()
                                    .is_some_and(|n| filter.contains(n))
                        }) {
                            continue;
                        }
                        opts.attribute_old_value == Some(true)
                    }
                    MutationType::CharacterData if opts.character_data == Some(true) => {
                        opts.character_data_old_value == Some(true)
                    }
                    _ => continue,
                };
                if let Some((_, old)) = interested.iter_mut().find(|(id, _)| *id == reg.observer) {
                    *old |= old_value;
                } else {
                    interested.push((reg.observer, old_value));
                }
            }
        }
        for (id, include_old) in interested {
            let mut record = record.clone();
            if !include_old {
                record.old_value = None;
                record.old_value_utf16 = None;
            }
            self.observers[id.0].records.push(record);
            if !self.pending.contains(&id) {
                self.pending.push(id);
            }
        }
        // A record that interested no observer must not schedule a delivery.
        // Embeddings observe the false -> true edge after a native operation
        // and enqueue that microtask in the same stream as JS jobs.
        if !self.pending.is_empty() {
            self.queued = true;
        }
    }
    pub(crate) fn register_transients(&mut self, node: NodeIdx, ancestors: &[NodeIdx]) {
        let mut sources = vec![];
        for ancestor in ancestors {
            sources.extend(
                self.registered
                    .iter()
                    .filter(|r| r.target == *ancestor && r.options.subtree)
                    .cloned(),
            );
        }
        for source in sources {
            let id = self.next_registration;
            self.next_registration += 1;
            self.registered.push(Registration {
                id,
                target: node,
                observer: source.observer,
                options: source.options,
                source: Some(source.id),
            });
        }
    }
}
impl Dom {
    pub fn observer_callback(&self, observer: ObserverId) -> Option<u64> {
        self.mutations.observers.get(observer.0).map(|entry| entry.callback)
    }
    pub fn register_mutation_record(&mut self, record: MutationRecord) -> MutationRecordId {
        let added_nodes = self.static_node_lists.len();
        self.static_node_lists.push(record.added_nodes.clone());
        let removed_nodes = self.static_node_lists.len();
        self.static_node_lists.push(record.removed_nodes.clone());
        let id = MutationRecordId(self.mutation_records.len());
        self.mutation_records.push(RegisteredRecord { record, added_nodes, removed_nodes });
        id
    }
    pub fn mutation_record(&self, id: MutationRecordId) -> Option<&MutationRecord> {
        self.mutation_records.get(id.0).map(|entry| &entry.record)
    }
    pub fn mutation_record_lists(&self, id: MutationRecordId) -> Option<(usize, usize)> {
        self.mutation_records.get(id.0).map(|entry| (entry.added_nodes, entry.removed_nodes))
    }
    pub fn static_node_list(&self, slot: usize) -> Option<&[NodeIdx]> {
        self.static_node_lists.get(slot).map(Vec::as_slice)
    }
    pub fn create_static_node_list(&mut self, nodes: Vec<NodeIdx>) -> usize {
        let slot = self.static_node_lists.len();
        self.static_node_lists.push(nodes);
        slot
    }
    pub fn create_mutation_observer(&mut self, callback: u64) -> ObserverId {
        let id = ObserverId(self.mutations.observers.len());
        self.mutations.observers.push(Observer {
            callback,
            records: vec![],
        });
        id
    }
    pub fn observe(
        &mut self,
        observer: ObserverId,
        target: NodeIdx,
        options: ObserveOptions,
    ) -> Result<(), DomError> {
        self.check_node(target)?;
        if observer.0 >= self.mutations.observers.len() {
            return Err(DomError::InvalidStateError);
        }
        let options = options.normalize()?;
        let existing: Vec<_> = self
            .mutations
            .registered
            .iter()
            .filter(|r| r.target == target && r.observer == observer)
            .map(|r| r.id)
            .collect();
        if existing.is_empty() {
            let id = self.mutations.next_registration;
            self.mutations.next_registration += 1;
            self.mutations.registered.push(Registration {
                id,
                target,
                observer,
                options,
                source: None,
            });
        } else {
            self.mutations
                .registered
                .retain(|r| !r.source.is_some_and(|s| existing.contains(&s)));
            for reg in &mut self.mutations.registered {
                if existing.contains(&reg.id) {
                    reg.options = options.clone();
                }
            }
        }
        Ok(())
    }
    pub fn disconnect_observer(&mut self, observer: ObserverId) {
        self.mutations.registered.retain(|r| r.observer != observer);
        if let Some(o) = self.mutations.observers.get_mut(observer.0) {
            o.records.clear();
        }
    }
    pub fn take_mutation_records(&mut self, observer: ObserverId) -> Vec<MutationRecord> {
        self.mutations
            .observers
            .get_mut(observer.0)
            .map(|o| std::mem::take(&mut o.records))
            .unwrap_or_default()
    }
    /// One mutation-observer delivery is needed when this changes false -> true.
    /// A JS embedding must record that transition before the next JS statement,
    /// in the same ordered outbox as Promise and queueMicrotask jobs. Inspect
    /// the state even if the native operation returns an error after mutation.
    /// Queueing the same pending delivery again would change observable order.
    /// The DOM borrow must end before the embedding posts to its outbox.
    /// A checkpoint must finish before another starts (microtasks do not nest).
    pub fn mutation_microtask_pending(&self) -> bool {
        self.mutations.queued
    }
    pub fn begin_mutation_checkpoint(&mut self) -> MutationCheckpoint {
        self.mutations.queued = false;
        MutationCheckpoint {
            remaining: std::mem::take(&mut self.mutations.pending).into(),
            slots: std::mem::take(&mut self.mutations.signal_slots).into(),
        }
    }
    pub(crate) fn queue_slotchange(&mut self, slot: NodeIdx) {
        if !self.mutations.signal_slots.contains(&slot) {
            self.mutations.signal_slots.push(slot);
        }
        self.mutations.queued = true;
    }
    /// Slotchange fires after observer callbacks in the same checkpoint.
    pub fn next_slotchange(&mut self, checkpoint: &mut MutationCheckpoint) -> Option<NodeIdx> {
        checkpoint.slots.pop_front()
    }
    pub fn next_mutation_delivery(
        &mut self,
        checkpoint: &mut MutationCheckpoint,
    ) -> Option<MutationDelivery> {
        while let Some(observer) = checkpoint.remaining.pop_front() {
            let records = self.take_mutation_records(observer);
            self.mutations
                .registered
                .retain(|r| r.observer != observer || r.source.is_none());
            if !records.is_empty() {
                return Some(MutationDelivery {
                    observer,
                    callback: self.mutations.observers[observer.0].callback,
                    records,
                });
            }
        }
        None
    }
    pub(crate) fn queue_tree_record(
        &mut self,
        target: NodeIdx,
        added: Vec<NodeIdx>,
        removed: Vec<NodeIdx>,
        previous: Option<NodeIdx>,
        next: Option<NodeIdx>,
    ) {
        if added.is_empty() && removed.is_empty() {
            return;
        }
        let mut record = MutationRecord::new(MutationType::ChildList, target);
        record.added_nodes = added;
        record.removed_nodes = removed;
        record.previous_sibling = previous;
        record.next_sibling = next;
        self.mutations
            .queue(&self.inclusive_ancestors(target), record);
    }
    /// Normalized local name/namespace seam; WebIDL name validation belongs to
    /// the future Element binding entry point. Empty namespace means null.
    pub fn set_attribute_ns(
        &mut self,
        node: NodeIdx,
        namespace: Option<&str>,
        name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { .. }) {
            return Err(DomError::NotSupportedError);
        }
        let namespace = namespace.filter(|s| !s.is_empty());
        let old = self
            .get_attribute_ns(node, namespace, name)
            .map(str::to_owned);
        if let Some(ns) = namespace {
            let attrs = &mut self.nodes[node.0 as usize].namespaced_attributes;
            if let Some(current) = attrs
                .iter_mut()
                .find(|attr| attr.namespace == ns && attr.local_name == name)
            {
                current.value = value.into();
            } else {
                attrs.push(crate::namespace::NamespacedAttribute {
                    namespace: ns.into(),
                    local_name: name.into(),
                    prefix: None,
                    value: value.into(),
                });
            }
        } else if let NodeKind::Element { attrs, .. } = &mut self.nodes[node.0 as usize].kind {
            if let Some((_, current)) = attrs.iter_mut().find(|(local, _)| local == name) {
                *current = value.into();
            } else {
                attrs.push((name.into(), value.into()));
            }
        }
        self.sync_attached_attr_value(node, namespace, name, value);
        self.queue_attribute_record(node, namespace, name, old);
        if namespace.is_none() && matches!(name, "name" | "slot") { self.reassign_all_slots(); }
        Ok(())
    }
    pub fn set_attribute(
        &mut self,
        node: NodeIdx,
        name: &str,
        value: &str,
    ) -> Result<(), DomError> {
        self.set_attribute_ns(node, None, name, value)
    }
    pub fn get_attribute_ns(
        &self,
        node: NodeIdx,
        namespace: Option<&str>,
        name: &str,
    ) -> Option<&str> {
        if let Some(ns) = namespace.filter(|s| !s.is_empty()) {
            self.get(node)
                .namespaced_attributes
                .iter()
                .find(|attr| attr.namespace == ns && attr.local_name == name)
                .map(|attr| attr.value.as_str())
        } else {
            self.attrs_of(node)?
                .iter()
                .find(|(local, _)| local == name)
                .map(|(_, value)| value.as_str())
        }
    }
    pub fn remove_attribute(&mut self, node: NodeIdx, name: &str) -> Result<(), DomError> {
        self.remove_attribute_ns(node, None, name)
    }
    pub fn remove_attribute_ns(
        &mut self,
        node: NodeIdx,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<(), DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { .. }) {
            return Err(DomError::NotSupportedError);
        }
        let namespace = namespace.filter(|s| !s.is_empty());
        let Some(old) = self
            .get_attribute_ns(node, namespace, name)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        if let Some(ns) = namespace {
            self.nodes[node.0 as usize]
                .namespaced_attributes
                .retain(|attr| attr.namespace != ns || attr.local_name != name);
        } else if let NodeKind::Element { attrs, .. } = &mut self.nodes[node.0 as usize].kind {
            attrs.retain(|(local, _)| local != name);
        }
        self.detach_attached_attr(node, namespace, name);
        self.queue_attribute_record(node, namespace, name, Some(old));
        if namespace.is_none() && matches!(name, "name" | "slot") { self.reassign_all_slots(); }
        Ok(())
    }
    fn queue_attribute_record(
        &mut self,
        target: NodeIdx,
        namespace: Option<&str>,
        name: &str,
        old: Option<String>,
    ) {
        self.mutation_generation = self.mutation_generation.wrapping_add(1);
        let mut record = MutationRecord::new(MutationType::Attributes, target);
        record.attribute_name = Some(name.into());
        record.attribute_namespace = namespace.map(str::to_owned);
        record.old_value = old;
        self.mutations
            .queue(&self.inclusive_ancestors(target), record);
    }
    pub fn character_data(&self, node: NodeIdx) -> Option<&str> {
        match &self.get(node).kind {
            NodeKind::Text { data }
            | NodeKind::CDATASection { data }
            | NodeKind::Comment { data }
            | NodeKind::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        }
    }
    pub fn set_character_data(&mut self, node: NodeIdx, value: String) -> Result<(), DomError> {
        self.check_node(node)?;
        self.replace_character_data(
            node,
            0,
            self.node_length(node),
            &value.encode_utf16().collect::<Vec<_>>(),
        )
    }
    /// DOMString code units. The legacy String accessor is a lossy display projection.
    pub fn character_data_utf16(&self, node: NodeIdx) -> Option<Vec<u16>> {
        let data = self.character_data(node)?;
        Some(
            self.get(node)
                .character_data_units
                .clone()
                .unwrap_or_else(|| data.encode_utf16().collect()),
        )
    }
    pub(crate) fn store_character_data(&mut self, node: NodeIdx, value: Vec<u16>) {
        let string = String::from_utf16_lossy(&value);
        self.nodes[node.0 as usize].character_data_units = if String::from_utf16(&value).is_err() {
            Some(value)
        } else {
            None
        };
        match &mut self.nodes[node.0 as usize].kind {
            NodeKind::Text { data }
            | NodeKind::CDATASection { data }
            | NodeKind::Comment { data }
            | NodeKind::ProcessingInstruction { data, .. } => *data = string,
            _ => unreachable!(),
        }
    }
    pub fn replace_character_data(
        &mut self,
        node: NodeIdx,
        offset: usize,
        count: usize,
        value: &[u16],
    ) -> Result<(), DomError> {
        self.check_node(node)?;
        let old = self
            .character_data_utf16(node)
            .ok_or(DomError::NotSupportedError)?;
        if offset > old.len() {
            return Err(DomError::IndexSizeError);
        }
        let count = count.min(old.len() - offset);
        let mut record = MutationRecord::new(MutationType::CharacterData, node);
        record.old_value = Some(String::from_utf16_lossy(&old));
        record.old_value_utf16 = Some(old.clone());
        self.mutations
            .queue(&self.inclusive_ancestors(node), record);
        let mut data = old;
        data.splice(offset..offset + count, value.iter().copied());
        self.store_character_data(node, data);
        self.ranges_replace_data(node, offset, count, value.len());
        Ok(())
    }
}
#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
