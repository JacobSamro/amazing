//! Native DOMTokenList (DOM §7.1; Infra ordered sets).
//!
//! WebIDL conversion belongs to the embedding. Capture the original view before
//! any author-defined conversion, then pass the converted UTF16 arguments here.
//! Captures carry the actual arena identity and live element generation; they
//! cannot be constructed by callers or retargeted to a replacement arena/slot.
//! The intern table and captures contain weak generation-bearing node identities.
//! Embeddings root the associated element while its JS view is reachable; a
//! native capture does not substitute for that script-visible ownership root.
use std::sync::Arc;
use crate::{Dom, DomError, NodeIdx, NodeKind, render_mutation::NodeGeneration};

#[derive(Debug)]
pub(crate) struct TokenListRecord {
    owner: NodeGeneration,
    attribute: String,
    generation: u64,
}

/// A genuine native view capture, valid only in its original owning Dom.
#[derive(Clone, Debug)]
pub struct TokenListCapture {
    arena: Arc<()>,
    slot: usize,
    owner: NodeGeneration,
    generation: u64,
}
impl TokenListCapture {
    pub fn slot(&self) -> usize { self.slot }
    /// Generation of the native view slot, independent of element recycling.
    pub fn view_generation(&self) -> u64 { self.generation }
    pub fn element(&self) -> NodeGeneration { self.owner }
}

/// The embedding supplies the policy actually defined for this element and
/// associated attribute. The generic DOM never infers HTML policy from a tag.
#[derive(Clone, Copy, Debug)]
pub enum TokenListSupport<'a> {
    Undefined,
    Defined(&'a [&'a [u16]]),
}

fn whitespace(unit: u16) -> bool { matches!(unit, 9 | 10 | 12 | 13 | 32) }
fn parse(value: &[u16]) -> Vec<Vec<u16>> {
    let mut tokens = Vec::new();
    for token in value.split(|unit| whitespace(*unit)).filter(|token| !token.is_empty()) {
        if !tokens.iter().any(|existing| existing == token) { tokens.push(token.to_vec()); }
    }
    tokens
}
fn serialize(tokens: &[Vec<u16>]) -> Vec<u16> {
    let mut value = Vec::new();
    for token in tokens {
        if !value.is_empty() { value.push(32); }
        value.extend_from_slice(token);
    }
    value
}
fn validate(token: &[u16]) -> Result<(), DomError> {
    if token.is_empty() { return Err(DomError::SyntaxError); }
    if token.iter().any(|unit| whitespace(*unit)) { return Err(DomError::InvalidCharacterError); }
    Ok(())
}

impl Dom {
    /// Canonical identity belongs to an actual element lifetime and attribute.
    /// Native view slots are append-only; their initial generation is never
    /// recycled. Element generations remain independently checked on every use.
    pub fn token_list(&mut self, node: NodeIdx, attribute: &str) -> Result<usize, DomError> {
        self.check_node(node)?;
        if !matches!(self.get(node).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        let owner = self.render_handle(node);
        if let Some(slot) = self.token_lists.iter().position(|record| record.owner == owner && record.attribute == attribute) {
            return Ok(slot);
        }
        let slot = self.token_lists.len();
        self.token_lists.push(TokenListRecord { owner, attribute: attribute.to_owned(), generation: 1 });
        Ok(slot)
    }
    fn live_token_list(&self, slot: usize) -> Result<&TokenListRecord, DomError> {
        let record = self.token_lists.get(slot).ok_or(DomError::InvalidStateError)?;
        if self.node_generation(record.owner.node) != Some(record.owner.generation) {
            return Err(DomError::InvalidStateError);
        }
        if !matches!(self.get(record.owner.node).kind, NodeKind::Element { .. }) { return Err(DomError::TypeError); }
        Ok(record)
    }
    pub fn token_list_info(&self, slot: usize) -> Option<(NodeIdx, &str)> {
        let record = self.live_token_list(slot).ok()?;
        Some((record.owner.node, &record.attribute))
    }
    /// Call before author conversion, on the receiver's actual owning arena.
    pub fn token_list_capture(&self, slot: usize) -> Result<TokenListCapture, DomError> {
        let record = self.live_token_list(slot)?;
        Ok(TokenListCapture { arena: self.token_list_arena.clone(), slot, owner: record.owner, generation: record.generation })
    }
    fn checked_token_list(&self, capture: &TokenListCapture) -> Result<&TokenListRecord, DomError> {
        if !Arc::ptr_eq(&capture.arena, &self.token_list_arena) { return Err(DomError::InvalidStateError); }
        let record = self.live_token_list(capture.slot)?;
        if record.owner != capture.owner || record.generation != capture.generation { return Err(DomError::InvalidStateError); }
        Ok(record)
    }
    pub fn token_list_attribute(&self, capture: &TokenListCapture) -> Result<&str, DomError> {
        Ok(&self.checked_token_list(capture)?.attribute)
    }
    pub fn token_list_owner(&self, capture: &TokenListCapture) -> Result<NodeGeneration, DomError> {
        Ok(self.checked_token_list(capture)?.owner)
    }
    pub fn token_list_value(&self, capture: &TokenListCapture) -> Result<Vec<u16>, DomError> {
        let record = self.checked_token_list(capture)?;
        Ok(self.attribute_utf16(record.owner.node, &record.attribute).unwrap_or_default())
    }
    pub fn token_list_tokens(&self, capture: &TokenListCapture) -> Result<Vec<Vec<u16>>, DomError> {
        Ok(parse(&self.token_list_value(capture)?))
    }
    pub fn token_list_length(&self, capture: &TokenListCapture) -> Result<usize, DomError> {
        Ok(self.token_list_tokens(capture)?.len())
    }
    pub fn token_list_item(&self, capture: &TokenListCapture, index: u32) -> Result<Option<Vec<u16>>, DomError> {
        Ok(self.token_list_tokens(capture)?.get(index as usize).cloned())
    }
    pub fn token_list_contains(&self, capture: &TokenListCapture, token: &[u16]) -> Result<bool, DomError> {
        Ok(self.token_list_tokens(capture)?.iter().any(|item| item == token))
    }
    pub fn token_list_set_value(&mut self, capture: &TokenListCapture, value: Vec<u16>) -> Result<(), DomError> {
        let record = self.checked_token_list(capture)?;
        let (node, attribute) = (record.owner.node, record.attribute.clone());
        self.set_attribute_ns_units(node, None, &attribute, value)
    }
    fn update_token_list(&mut self, capture: &TokenListCapture, tokens: &[Vec<u16>]) -> Result<(), DomError> {
        let record = self.checked_token_list(capture)?;
        if tokens.is_empty() && self.get_attribute_ns(record.owner.node, None, &record.attribute).is_none() { return Ok(()); }
        self.token_list_set_value(capture, serialize(tokens))
    }
    pub fn token_list_add(&mut self, capture: &TokenListCapture, tokens: &[Vec<u16>]) -> Result<(), DomError> {
        self.checked_token_list(capture)?;
        for token in tokens { validate(token)?; }
        let mut current = self.token_list_tokens(capture)?;
        for token in tokens { if !current.contains(token) { current.push(token.clone()); } }
        self.update_token_list(capture, &current)
    }
    pub fn token_list_remove(&mut self, capture: &TokenListCapture, tokens: &[Vec<u16>]) -> Result<(), DomError> {
        self.checked_token_list(capture)?;
        for token in tokens { validate(token)?; }
        let mut current = self.token_list_tokens(capture)?;
        current.retain(|token| !tokens.contains(token));
        self.update_token_list(capture, &current)
    }
    pub fn token_list_toggle(&mut self, capture: &TokenListCapture, token: &[u16], force: Option<bool>) -> Result<bool, DomError> {
        self.checked_token_list(capture)?;
        validate(token)?;
        let mut current = self.token_list_tokens(capture)?;
        if current.iter().any(|item| item == token) {
            if force != Some(true) {
                current.retain(|item| item != token);
                self.update_token_list(capture, &current)?;
                return Ok(false);
            }
            return Ok(true);
        }
        if force != Some(false) {
            current.push(token.to_vec());
            self.update_token_list(capture, &current)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn token_list_replace(&mut self, capture: &TokenListCapture, token: &[u16], replacement: &[u16]) -> Result<bool, DomError> {
        self.checked_token_list(capture)?;
        if token.is_empty() || replacement.is_empty() { return Err(DomError::SyntaxError); }
        validate(token)?;validate(replacement)?;
        let mut current = self.token_list_tokens(capture)?;
        let Some(old) = current.iter().position(|item| item == token) else { return Ok(false); };
        if let Some(new) = current.iter().position(|item| item == replacement) {
            if old != new { current[old.min(new)] = replacement.to_vec();current.remove(old.max(new)); }
        } else { current[old] = replacement.to_vec(); }
        self.update_token_list(capture, &current)?;
        Ok(true)
    }
    pub fn token_list_supports(&self, capture: &TokenListCapture, token: &[u16], support: TokenListSupport<'_>) -> Result<bool, DomError> {
        self.checked_token_list(capture)?;
        let TokenListSupport::Defined(supported) = support else { return Err(DomError::TypeError); };
        let folded: Vec<u16> = token.iter().map(|unit| if (65..=90).contains(unit) { unit + 32 } else { *unit }).collect();
        Ok(supported.iter().any(|candidate| *candidate == folded.as_slice()))
    }
}

#[cfg(test)]
#[path = "token_list_tests.rs"]
mod tests;
