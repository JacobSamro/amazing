//! Accessibility events: a diff between two trees that classifies node
//! additions, removals and property changes, and resolves the live region that
//! governs each change.
//!
//! Nodes are compared by their DOM arena index (`dom_node`), not by
//! `AxNodeId`, because ids are reassigned in preorder after pruning. This makes
//! the diff exact for append-mostly mutations (streaming text, added rows); a
//! reparent or a pre-order insert can shift indices and is reported as a
//! removal plus an addition.

use crate::aria::Relevant;
use crate::role::Role;
use crate::tree::{AxNodeId, AxTree};

/// `aria-live` politeness.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Live {
    #[default]
    Off,
    Polite,
    Assertive,
}

/// The nearest live region that governs a changed node.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LiveRegionInfo {
    pub node: AxNodeId,
    pub role: Role,
    pub live: Live,
    pub atomic: bool,
    pub relevant: Relevant,
}

impl LiveRegionInfo {
    /// Whether the region accepts an event of this kind. `text` matches a
    /// change to the node's name; other property changes count as additions.
    pub fn accepts(&self, event: &AxEvent) -> bool {
        if self.live == Live::Off {
            return false;
        }
        if self.relevant.all {
            return true;
        }
        match event.kind {
            AxEventKind::Added => self.relevant.additions,
            AxEventKind::Removed => self.relevant.removals,
            AxEventKind::Changed => {
                if event.text_changed {
                    self.relevant.text || self.relevant.additions
                } else {
                    self.relevant.additions
                }
            }
        }
    }
}

/// What happened to a node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AxEventKind {
    Added,
    Removed,
    Changed,
}

/// One accessibility change.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AxEvent {
    pub kind: AxEventKind,
    /// Node id in the tree that produced the event (new tree except for
    /// `Removed`, where it is the old tree).
    pub node: AxNodeId,
    pub dom_node: Option<u32>,
    pub role: Role,
    pub name: String,
    /// True when the node's accessible name changed, so a live region with
    /// `text` relevance accepts it.
    pub text_changed: bool,
    /// Nearest live region governing the node, from the owning tree.
    pub live: Option<LiveRegionInfo>,
}

impl AxEvent {
    /// The event filtered through its live region: an event with no live region
    /// is dropped, and a governed event is kept only when the region accepts it.
    pub fn as_announcement(&self) -> Option<&Self> {
        let region = self.live.as_ref()?;
        if region.accepts(self) { Some(self) } else { None }
    }
}

/// Diffs two trees, returning removals, additions and property changes in
/// previous-then-next order.
pub fn diff(previous: &AxTree, next: &AxTree) -> Vec<AxEvent> {
    use std::collections::HashMap;
    let previous_by_dom: HashMap<u32, &crate::tree::AxNode> = previous
        .nodes()
        .iter()
        .filter_map(|node| node.dom_node.map(|dom| (dom.0, node)))
        .collect();
    let next_by_dom: HashMap<u32, &crate::tree::AxNode> = next
        .nodes()
        .iter()
        .filter_map(|node| node.dom_node.map(|dom| (dom.0, node)))
        .collect();

    let mut events = Vec::new();
    for node in previous.nodes() {
        let Some(dom) = node.dom_node else { continue };
        if !next_by_dom.contains_key(&dom.0) {
            events.push(AxEvent {
                kind: AxEventKind::Removed,
                node: node.id,
                dom_node: Some(dom.0),
                role: node.role,
                name: node.name.clone(),
                text_changed: false,
                live: live_region(previous, node.id),
            });
        }
    }
    for node in next.nodes() {
        let Some(dom) = node.dom_node else { continue };
        match previous_by_dom.get(&dom.0) {
            None => events.push(AxEvent {
                kind: AxEventKind::Added,
                node: node.id,
                dom_node: Some(dom.0),
                role: node.role,
                name: node.name.clone(),
                text_changed: false,
                live: live_region(next, node.id),
            }),
            Some(old) => {
                let text_changed = old.name != node.name;
                if text_changed || property_changed(old, node) {
                    events.push(AxEvent {
                        kind: AxEventKind::Changed,
                        node: node.id,
                        dom_node: Some(dom.0),
                        role: node.role,
                        name: node.name.clone(),
                        text_changed,
                        live: live_region(next, node.id),
                    });
                }
            }
        }
    }
    events
}

fn property_changed(old: &crate::tree::AxNode, new: &crate::tree::AxNode) -> bool {
    old.role != new.role
        || old.description != new.description
        || old.states != new.states
        || old.properties != new.properties
        || old.active_descendant != new.active_descendant
}

/// The nearest live region at or above `id`, if the node is in one.
pub fn live_region(tree: &AxTree, id: AxNodeId) -> Option<LiveRegionInfo> {
    let mut current = Some(id);
    while let Some(node_id) = current {
        let node = tree.node(node_id)?;
        if let Some(live) = explicit_live(node) {
            return Some(LiveRegionInfo {
                node: node.id,
                role: node.role,
                live,
                atomic: node.properties.atomic,
                relevant: node.properties.relevant_flags,
            });
        }
        current = node.parent;
    }
    None
}

fn explicit_live(node: &crate::tree::AxNode) -> Option<Live> {
    if let Some(value) = node.properties.live.as_deref() {
        return Some(match value.trim().to_ascii_lowercase().as_str() {
            "assertive" => Live::Assertive,
            "polite" => Live::Polite,
            "off" => Live::Off,
            // An unrecognised or empty `aria-live` is polite per ARIA's default
            // for a live region role, but `off` means explicitly silent.
            _ => Live::Polite,
        });
    }
    match node.role {
        Role::Alert => Some(Live::Assertive),
        Role::Status | Role::Log => Some(Live::Polite),
        _ => None,
    }
}
