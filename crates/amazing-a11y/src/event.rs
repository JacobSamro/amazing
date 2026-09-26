//! Accessibility events: a diff between two trees that classifies node
//! additions, removals and per-aspect property changes, and resolves the live
//! region that governs each change.
//!
//! Nodes are compared by their DOM arena index (`dom_node`), not by
//! `AxNodeId`, because ids are reassigned in preorder after pruning. This makes
//! the diff exact for append-mostly mutations (streaming text, added rows); a
//! reparent or a pre-order insert can shift indices, so an index that used to
//! identify one node comes to identify a different one and the shift shows
//! up as name/property changes (or, past the end of the shared prefix, an
//! addition) rather than a clean move
//! (`inserting_at_the_head_shifts_dom_identity` in `event_tests.rs` pins
//! this).
//!
//! A changed node can differ in more than one aspect at once (its name *and*
//! its role, say), so unlike an earlier version of this module a persisting
//! node no longer collapses every difference into one opaque `Changed`
//! event: each aspect that differs — name, description, role, states, typed
//! properties, active-descendant — becomes its own [`AxEvent`] with its own
//! [`AxEventKind`]. This is deliberately shaped for two downstream
//! consumers that both want to know *which* aspect changed, not just *that*
//! something did:
//! - a CDP `Accessibility` bridge (today's `Accessibility.getFullAXTree` in
//!   `crates/amazing/src/runtime_host_devtools.rs` is pull-based; a future
//!   incremental `Accessibility.*` event would want the same granularity
//!   DevTools' own frontend renders separately, e.g. a name edit vs. a state
//!   toggle);
//! - a platform AT bridge (AT-SPI on Linux emits distinct D-Bus signals per
//!   aspect — `object:property-change:accessible-name`,
//!   `object:property-change:accessible-role`, `object:state-changed`,
//!   `object:active-descendant-changed` — so a bridge can map one
//!   [`AxEventKind`] onto one signal instead of re-diffing an opaque bundle).
//!
//! [`rebuild_and_diff`] is the incremental-on-mutation entry point: a driver
//! that just applied a DOM mutation calls it once with the (whole-document)
//! recomputed DOM/style/layout and the previously built tree, and gets back
//! the new tree plus the events that separate it from the old one. The build
//! step itself is not partial — Stage 1 has no incremental style/layout
//! invalidation yet (see ARCHITECTURE.md) — but rebuilding the whole tree and
//! diffing it against the previous snapshot is also what makes "name or role
//! invalidation" correct for free: if a mutation changes a node's text and
//! that node is the target of another node's `aria-labelledby`, the *referring*
//! node's name is recomputed too on the rebuild (accname resolves the
//! reference fresh every time), and the diff reports it as its own
//! `NameChanged` event even though the referring node's own subtree never
//! changed.

use amazing_css::StyledDom;
use amazing_dom::Dom;
use amazing_layout::LayoutBox;

use crate::aria::Relevant;
use crate::role::Role;
use crate::tree::{self, AxNodeId, AxTree};

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
    /// change to the node's name; every other change (including a role,
    /// state or other property change on an existing node) counts as an
    /// addition, matching this method's pre-existing behavior from when
    /// those were all folded into one `Changed` kind.
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
            AxEventKind::NameChanged => self.relevant.text || self.relevant.additions,
            AxEventKind::DescriptionChanged
            | AxEventKind::RoleChanged
            | AxEventKind::StateChanged
            | AxEventKind::PropertyChanged
            | AxEventKind::ActiveDescendantChanged => self.relevant.additions,
        }
    }
}

/// What happened to a node. A node that persists across the diff can produce
/// more than one of the `*Changed` kinds at once — each is reported as its
/// own event (see the module doc comment).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AxEventKind {
    Added,
    Removed,
    /// The computed accessible name differs.
    NameChanged,
    /// The computed accessible description differs.
    DescriptionChanged,
    /// The resolved role differs (an explicit `role` attribute changed, or
    /// conflict resolution now picks a different fallback).
    RoleChanged,
    /// A boolean/tristate flag in [`crate::aria::AxStates`] differs (e.g.
    /// `aria-checked`, `aria-expanded`, focus).
    StateChanged,
    /// A typed value in [`crate::aria::AxProperties`] differs (e.g.
    /// `aria-valuenow`, `aria-level`, `aria-live` itself).
    PropertyChanged,
    /// The resolved `aria-activedescendant` target differs.
    ActiveDescendantChanged,
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
    /// The name before the change, set only for `NameChanged`.
    pub previous_name: Option<String>,
    /// The role before the change, set only for `RoleChanged`.
    pub previous_role: Option<Role>,
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

/// Diffs two trees, returning removals, additions and per-aspect property
/// changes in previous-then-next order.
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
                previous_name: None,
                previous_role: None,
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
                previous_name: None,
                previous_role: None,
                live: live_region(next, node.id),
            }),
            Some(old) => {
                let live = live_region(next, node.id);
                let template = AxEvent {
                    kind: AxEventKind::NameChanged, // overwritten per push below
                    node: node.id,
                    dom_node: Some(dom.0),
                    role: node.role,
                    name: node.name.clone(),
                    previous_name: None,
                    previous_role: None,
                    live: live.clone(),
                };
                // Each aspect that differs is reported as its own event, most
                // to least visible, so a bridge never has to re-diff an
                // opaque bundle to learn which one actually changed.
                if old.name != node.name {
                    events.push(AxEvent {
                        previous_name: Some(old.name.clone()),
                        ..template.clone()
                    });
                }
                if old.role != node.role {
                    events.push(AxEvent {
                        kind: AxEventKind::RoleChanged,
                        previous_role: Some(old.role),
                        ..template.clone()
                    });
                }
                if old.description != node.description {
                    events.push(AxEvent { kind: AxEventKind::DescriptionChanged, ..template.clone() });
                }
                if old.states != node.states {
                    events.push(AxEvent { kind: AxEventKind::StateChanged, ..template.clone() });
                }
                if properties_changed(&old.properties, &node.properties) {
                    events.push(AxEvent { kind: AxEventKind::PropertyChanged, ..template.clone() });
                }
                if old.active_descendant != node.active_descendant {
                    events.push(AxEvent { kind: AxEventKind::ActiveDescendantChanged, ..template });
                }
            }
        }
    }
    events
}

/// Rebuilds the accessibility tree from a document's current DOM/style/layout
/// and diffs it against `previous`. The entry point a driver calls once per
/// batch of applied DOM mutations (see the module doc comment for why a whole
/// rebuild still yields a correctly incremental *diff*, including
/// invalidating a node's name when a node it references changed instead).
pub fn rebuild_and_diff(
    dom: &Dom,
    styled: &StyledDom,
    layout: &LayoutBox,
    previous: &AxTree,
) -> (AxTree, Vec<AxEvent>) {
    let next = tree::build(dom, styled, layout);
    let events = diff(previous, &next);
    (next, events)
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

/// Whether the *typed* properties differ, ignoring `raw`. `raw` mirrors every
/// non-empty `aria-*` attribute verbatim, including ones already covered by a
/// dedicated signal — `aria-label`/`aria-labelledby` by `NameChanged`,
/// `aria-describedby` by `DescriptionChanged`, `aria-pressed`/`aria-checked`/…
/// by `StateChanged` — so comparing it here would make almost every name or
/// state edit also spuriously fire `PropertyChanged`, defeating the point of
/// giving each aspect its own kind. Excluding it means a change to an
/// `aria-*` attribute with no dedicated typed field (a rare, obscure tail —
/// most well-known ones already have one) is not reported as its own event;
/// that is an accepted, documented gap rather than universal event noise.
fn properties_changed(old: &crate::aria::AxProperties, new: &crate::aria::AxProperties) -> bool {
    let mut old_typed = old.clone();
    old_typed.raw = Default::default();
    let mut new_typed = new.clone();
    new_typed.raw = Default::default();
    old_typed != new_typed
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
