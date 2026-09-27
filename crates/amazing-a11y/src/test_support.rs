//! Shared helpers for the crate's table-driven fixtures.
#![cfg(test)]

use amazing_dom::NodeIdx;

use crate::role::{implicit_role, role_for};
use crate::{AxDocument, AxNode, AxNodeId, Role, build_document};

/// Builds the full pipeline for a fixture at an 800x600 viewport.
pub fn document(html: &str) -> AxDocument {
    build_document(html, "", 800.0, Some(600.0))
}

/// Builds the full pipeline with an explicit author stylesheet.
pub fn document_with_css(html: &str, css: &str) -> AxDocument {
    build_document(html, css, 800.0, Some(600.0))
}

/// Builds the full pipeline with the first element of `focus_tag` focused.
pub fn document_with_focus(html: &str, focus_tag: &str) -> AxDocument {
    let mut doc = build_document(html, "", 800.0, Some(600.0));
    let target = doc
        .dom
        .tree_order(NodeIdx(0))
        .into_iter()
        .find(|node| doc.dom.name(*node) == Some(focus_tag));
    if let Some(node) = target {
        doc.dom.set_active_element(NodeIdx(0), Some(node), true);
        doc.tree = crate::build(&doc.dom, &doc.styled, &doc.layout);
    }
    doc
}

/// A11Y-15: rebuilds `doc.tree` after mutating `doc.dom`'s live form-control
/// state directly (`set_input_checked`, `set_option_selected`, ...) —
/// mirrors `document_with_focus`'s own rebuild step, generalized for tests
/// that mutate live state other than focus.
pub fn rebuild(doc: &mut AxDocument) {
    doc.tree = crate::build(&doc.dom, &doc.styled, &doc.layout);
}

/// The first DOM node with the given tag name.
pub fn dom_of_tag(doc: &AxDocument, tag: &str) -> Option<NodeIdx> {
    doc.dom
        .tree_order(NodeIdx(0))
        .into_iter()
        .find(|node| doc.dom.name(*node) == Some(tag))
}

/// The first accessibility node backed by an element with the given tag name.
pub fn ax_of_tag<'a>(doc: &'a AxDocument, tag: &str) -> Option<&'a AxNode> {
    let dom = dom_of_tag(doc, tag)?;
    let id = doc.tree.find_dom(dom)?;
    doc.tree.node(id)
}

/// Role of the first element with the given tag name, if exposed.
pub fn role_of_tag(html: &str, tag: &str) -> Option<Role> {
    let doc = document(html);
    ax_of_tag(&doc, tag).map(|node| node.role)
}

/// HTML-AAM implicit role of the first element with the given tag, independent
/// of tree pruning and explicit `role` attributes.
pub fn implicit_role_of_tag(html: &str, tag: &str) -> Option<Role> {
    let doc = document(html);
    let node = dom_of_tag(&doc, tag)?;
    let (ids, _labels) = crate::tree::collect_indices(&doc.dom);
    Some(implicit_role(&doc.dom, &ids, node))
}

/// Resolved role (`role` attribute if valid, else implicit) of the first element
/// with the given tag, independent of tree pruning.
pub fn role_for_tag(html: &str, tag: &str) -> Option<Role> {
    let doc = document(html);
    let node = dom_of_tag(&doc, tag)?;
    let (ids, _labels) = crate::tree::collect_indices(&doc.dom);
    Some(role_for(&doc.dom, &ids, node))
}

/// The exposed accessibility node backed by the element with the given `id`.
pub fn ax_of_id<'a>(doc: &'a AxDocument, id: &str) -> Option<&'a AxNode> {
    let node = doc
        .dom
        .tree_order(NodeIdx(0))
        .into_iter()
        .find(|n| doc.dom.get_attribute_ns(*n, None, "id") == Some(id))?;
    let ax_id = doc.tree.find_dom(node)?;
    doc.tree.node(ax_id)
}

/// Accessible name of the first element with the given tag name.
pub fn name_of_tag(html: &str, tag: &str) -> Option<String> {
    let doc = document(html);
    ax_of_tag(&doc, tag).map(|node| node.name.clone())
}

/// The first node with the given role.
pub fn first_role(doc: &AxDocument, role: Role) -> Option<&AxNode> {
    doc.tree.nodes().iter().find(|node| node.role == role)
}

/// The first node with the given role and non-empty name.
pub fn first_named_role<'a>(doc: &'a AxDocument, role: Role, name: &str) -> Option<&'a AxNode> {
    doc.tree
        .nodes()
        .iter()
        .find(|node| node.role == role && node.name == name)
}

/// Number of nodes with the given role.
pub fn count_role(doc: &AxDocument, role: Role) -> usize {
    doc.tree.nodes().iter().filter(|node| node.role == role).count()
}

/// Preorder list of node ids as a convenience for assertions.
pub fn ids(doc: &AxDocument) -> Vec<AxNodeId> {
    doc.tree.walk(None)
}

/// All node names in preorder.
pub fn names(doc: &AxDocument) -> Vec<String> {
    doc.tree
        .walk(None)
        .into_iter()
        .filter_map(|id| doc.tree.node(id).map(|node| node.name.clone()))
        .collect()
}

/// All `(role, name)` pairs in preorder.
pub fn shape(doc: &AxDocument) -> Vec<(&'static str, String)> {
    doc.tree
        .walk(None)
        .into_iter()
        .filter_map(|id| doc.tree.node(id))
        .map(|node| (node.role.as_str(), node.name.clone()))
        .collect()
}
