//! Attr nodes are detached Node arena entries with a live owner association.
use crate::{namespace::{valid_attribute_name, validate_and_extract}, Dom, DomError, NodeIdx, NodeKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrSnapshot {
    pub local_name: String,
    pub namespace_uri: Option<String>,
    pub prefix: Option<String>,
    pub value: Vec<u16>,
    pub owner_element: Option<NodeIdx>,
}
impl AttrSnapshot {
    pub fn name(&self) -> String {
        match &self.prefix { Some(prefix) => format!("{prefix}:{}", self.local_name),
            None => self.local_name.clone() }
    }
}
impl Dom {
    pub fn create_attribute_in(&mut self, document: NodeIdx, name: &str) -> Result<NodeIdx, DomError> {
        if !valid_attribute_name(name) { return Err(DomError::InvalidCharacterError); }
        self.create_node_in(document, NodeKind::Attr {
            local_name: name.into(), namespace_uri: None, prefix: None,
            value: Vec::new(), owner_element: None,
        })
    }
    pub fn create_attribute_ns_in(&mut self, document: NodeIdx, uri: Option<&str>, qualified: &str) -> Result<NodeIdx, DomError> {
        let uri = uri.filter(|s| !s.is_empty());
        let (prefix, local) = validate_and_extract(uri, qualified, true)?;
        self.create_node_in(document, NodeKind::Attr {
            local_name: local.into(), namespace_uri: uri.map(str::to_owned),
            prefix: prefix.map(str::to_owned), value: Vec::new(), owner_element: None,
        })
    }
    pub fn attr_snapshot(&self, attr: NodeIdx) -> Result<AttrSnapshot, DomError> {
        self.check_node(attr)?;
        let NodeKind::Attr { local_name, namespace_uri, prefix, value, owner_element } = &self.get(attr).kind
            else { return Err(DomError::TypeError) };
        Ok(AttrSnapshot { local_name: local_name.clone(), namespace_uri: namespace_uri.clone(),
            prefix: prefix.clone(), value: value.clone(), owner_element: *owner_element })
    }
    pub fn set_attr_value(&mut self, attr: NodeIdx, units: Vec<u16>) -> Result<(), DomError> {
        let snapshot = self.attr_snapshot(attr)?;
        if let Some(element) = snapshot.owner_element {
            let display = String::from_utf16_lossy(&units);
            self.set_attribute_expanded(element, snapshot.namespace_uri.as_deref(),
                snapshot.prefix.as_deref(), &snapshot.local_name, &display)?;
        }
        if let NodeKind::Attr { value, .. } = &mut self.nodes[attr.0 as usize].kind { *value = units; }
        Ok(())
    }
    /// Preserve exact code units after an Element setter has already changed
    /// the attribute; do not repeat the mutation or its reactions.
    pub fn store_attr_value_units(&mut self, attr: NodeIdx, units: Vec<u16>) -> Result<(), DomError> {
        self.check_node(attr)?;
        let NodeKind::Attr { value, .. } = &mut self.nodes[attr.0 as usize].kind
            else { return Err(DomError::TypeError) };
        *value = units;
        Ok(())
    }
    fn attached_attr_slot(&self, element: NodeIdx, uri: Option<&str>, local: &str) -> Option<NodeIdx> {
        self.nodes.iter().enumerate().find_map(|(index, node)| match &node.kind {
            NodeKind::Attr { local_name, namespace_uri, owner_element: Some(owner), .. }
                if *owner == element && namespace_uri.as_deref() == uri && local_name == local => Some(NodeIdx(index as u32)),
            _ => None,
        })
    }
    pub fn get_attribute_node(&mut self, element: NodeIdx, uri: Option<&str>, local: &str) -> Option<NodeIdx> {
        let attr = self.attribute_ns(element, uri, local)?;
        let (namespace_uri, prefix, local_name, value) = (
            attr.namespace_uri.map(str::to_owned), attr.prefix.map(str::to_owned),
            attr.local_name.to_owned(), attr.value.encode_utf16().collect(),
        );
        if let Some(slot) = self.attached_attr_slot(element, namespace_uri.as_deref(), &local_name) {
            return Some(slot);
        }
        let document = self.owner_document(element)?;
        let slot = self.create_node_in(document, NodeKind::Attr {
            local_name, namespace_uri, prefix, value, owner_element: Some(element),
        }).ok()?;
        Some(slot)
    }
    pub fn attribute_nodes(&mut self, element: NodeIdx) -> Result<Vec<NodeIdx>, DomError> {
        self.check_node(element)?;
        if !matches!(self.get(element).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        let names: Vec<_> = self.attributes_expanded(element).unwrap().into_iter()
            .map(|attr| (attr.namespace_uri.map(str::to_owned), attr.local_name.to_owned()))
            .collect();
        Ok(names.into_iter().filter_map(|(uri, local)| self.get_attribute_node(element, uri.as_deref(), &local)).collect())
    }
    pub fn set_attribute_node(&mut self, element: NodeIdx, attr: NodeIdx) -> Result<Option<NodeIdx>, DomError> {
        self.check_node(element)?;
        if !matches!(self.get(element).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        let snapshot = self.attr_snapshot(attr)?;
        if snapshot.owner_element.is_some_and(|owner| owner != element) { return Err(DomError::InUseAttributeError); }
        let old = self.get_attribute_node(element, snapshot.namespace_uri.as_deref(), &snapshot.local_name);
        if old == Some(attr) { return Ok(old); }
        let display = String::from_utf16_lossy(&snapshot.value);
        self.set_attribute_expanded(element, snapshot.namespace_uri.as_deref(), snapshot.prefix.as_deref(),
            &snapshot.local_name, &display)?;
        if let Some(old) = old {
            if let NodeKind::Attr { owner_element, .. } = &mut self.nodes[old.0 as usize].kind { *owner_element = None; }
        }
        if let NodeKind::Attr { owner_element, .. } = &mut self.nodes[attr.0 as usize].kind { *owner_element = Some(element); }
        Ok(old)
    }
    pub fn remove_attribute_node(&mut self, element: NodeIdx, attr: NodeIdx) -> Result<NodeIdx, DomError> {
        let snapshot = self.attr_snapshot(attr)?;
        if snapshot.owner_element != Some(element) { return Err(DomError::NotFoundError); }
        self.remove_attribute_ns(element, snapshot.namespace_uri.as_deref(), &snapshot.local_name)?;
        Ok(attr)
    }
    pub(crate) fn sync_attached_attr_value(&mut self, element: NodeIdx, uri: Option<&str>, local: &str, value: &str) {
        if let Some(slot) = self.attached_attr_slot(element, uri, local) {
            if let NodeKind::Attr { value: stored, .. } = &mut self.nodes[slot.0 as usize].kind {
                *stored = value.encode_utf16().collect();
            }
        }
    }
    pub(crate) fn sync_attached_attr_prefix(&mut self, element: NodeIdx, uri: Option<&str>, local: &str, prefix: Option<&str>) {
        if let Some(slot) = self.attached_attr_slot(element, uri, local) {
            if let NodeKind::Attr { prefix: stored, .. } = &mut self.nodes[slot.0 as usize].kind {
                *stored = prefix.map(str::to_owned);
            }
        }
    }
    pub(crate) fn detach_attached_attr(&mut self, element: NodeIdx, uri: Option<&str>, local: &str) {
        if let Some(slot) = self.attached_attr_slot(element, uri, local) {
            if let NodeKind::Attr { owner_element, .. } = &mut self.nodes[slot.0 as usize].kind {
                *owner_element = None;
            }
        }
    }
}
