use super::*;
use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};

fn options(mode: ShadowMode, assignment: SlotAssignment) -> ShadowOptions {
    ShadowOptions { mode, delegates_focus: false, slot_assignment: assignment,
        clonable: false, serializable: false }
}
fn document(dom: &mut Dom, root: NodeIdx) -> NodeIdx { dom.create_element(root, "html") }

#[test]
fn named_slots_cross_closed_roots_and_exclude_entire_unassigned_branches() {
    for mode in [ShadowMode::Open, ShadowMode::Closed] {
        let mut dom = Dom::new(); let html = document(&mut dom, NodeIdx(0));
        let host = dom.create_element(html, "div");
        let assigned = dom.create_element(host, "span");
        let assigned_child = dom.create_element(assigned, "b");
        let hidden = dom.create_element(host, "section");
        let hidden_child = dom.create_element(hidden, "em");
        dom.set_attribute(assigned, "slot", "chosen").unwrap();
        let root = dom.attach_shadow(host, options(mode, SlotAssignment::Named)).unwrap();
        let slot = dom.create_element(root, "slot");
        dom.set_attribute(slot, "name", "chosen").unwrap();
        assert_eq!(dom.flat_tree_parent(assigned), Ok(Some(slot)));
        assert_eq!(dom.flat_tree_parent(assigned_child), Ok(Some(assigned)));
        assert_eq!(dom.flat_tree_parent(slot), Ok(Some(host)));
        assert_eq!(dom.flat_tree_parent(hidden), Ok(None));
        assert_eq!(dom.flat_tree_parent(hidden_child), Ok(None));
        assert_eq!(dom.assigned_slot(assigned), (mode == ShadowMode::Open).then_some(slot));
        assert_eq!(dom.flat_tree_parent(root), Ok(None));
        assert_eq!(dom.flat_tree_parent(html), Ok(None));
        assert_eq!(dom.flat_tree_parent(NodeIdx(0)), Ok(None));
    }
}

#[test]
fn fallback_suppression_checks_full_chain_and_restores_after_assignment_changes() {
    let mut dom = Dom::new(); let html = document(&mut dom, NodeIdx(0));
    let host = dom.create_element(html, "div");
    let root = dom.attach_shadow(host, options(ShadowMode::Closed, SlotAssignment::Named)).unwrap();
    let slot = dom.create_element(root, "slot");
    let fallback = dom.create_element(slot, "section");
    let nested_host = dom.create_element(fallback, "div");
    let nested_root = dom.attach_shadow(nested_host, options(ShadowMode::Open, SlotAssignment::Named)).unwrap();
    let deep = dom.create_element(nested_root, "span");
    assert_eq!(dom.flat_tree_parent(fallback), Ok(Some(slot)));
    assert_eq!(dom.flat_tree_parent(deep), Ok(Some(nested_host)));
    let assigned = dom.create_text(host, "visible");
    assert_eq!(dom.flat_tree_parent(assigned), Ok(Some(slot)));
    assert_eq!(dom.flat_tree_parent(fallback), Ok(None));
    assert_eq!(dom.flat_tree_parent(nested_host), Ok(None));
    assert_eq!(dom.flat_tree_parent(deep), Ok(None));
    dom.detach(assigned);
    assert_eq!(dom.flat_tree_parent(fallback), Ok(Some(slot)));
    assert_eq!(dom.flat_tree_parent(deep), Ok(Some(nested_host)));
    // Detached hosts retain internal native ancestry without acquiring an
    // active Document or crossing a browsing-context boundary.
    dom.detach(host);
    assert_eq!(dom.flat_tree_parent(slot), Ok(Some(host)));
    assert_eq!(dom.flat_tree_parent(deep), Ok(Some(nested_host)));
    assert_eq!(dom.flat_tree_parent(host), Ok(None));
}

#[test]
fn manual_assignment_and_nested_slotted_slots_keep_actual_parent_identity() {
    let mut dom = Dom::new(); let html = document(&mut dom, NodeIdx(0));
    let host = dom.create_element(html, "div");
    let light = dom.create_element(host, "span");
    let root = dom.attach_shadow(host, options(ShadowMode::Closed, SlotAssignment::Manual)).unwrap();
    let outer = dom.create_element(root, "slot");
    let alternate = dom.create_element(root, "slot");
    let fallback = dom.create_element(outer, "b");
    assert_eq!(dom.flat_tree_parent(light), Ok(None));
    dom.assign_slot(outer, &[light]).unwrap();
    assert_eq!(dom.flat_tree_parent(light), Ok(Some(outer)));
    assert_eq!(dom.flat_tree_parent(fallback), Ok(None));
    dom.assign_slot(alternate, &[light]).unwrap();
    assert_eq!(dom.flat_tree_parent(light), Ok(Some(alternate)));
    assert_eq!(dom.flat_tree_parent(fallback), Ok(Some(outer)));
    let inner_host = dom.create_element(root, "div");
    dom.pre_insert(outer, inner_host, None).unwrap();
    let inner_root = dom.attach_shadow(inner_host, options(ShadowMode::Closed, SlotAssignment::Named)).unwrap();
    let inner_slot = dom.create_element(inner_root, "slot");
    dom.assign_slot(outer, &[light]).unwrap();
    assert_eq!(dom.flat_tree_parent(light), Ok(Some(outer)));
    assert_eq!(dom.flat_tree_parent(outer), Ok(Some(inner_slot)));
    assert_eq!(dom.flat_tree_parent(inner_slot), Ok(Some(inner_host)));
}

#[test]
fn document_fragment_template_and_recycled_node_boundaries_are_native() {
    let mut dom = Dom::new(); let html = document(&mut dom, NodeIdx(0));
    let child_doc = dom.create_document(); let child_html = document(&mut dom, child_doc);
    let child = dom.create_element(child_html, "span");
    assert_eq!(dom.flat_tree_parent(child), Ok(Some(child_html)));
    assert_eq!(dom.flat_tree_parent(child_html), Ok(None));
    let fragment = dom.create_document_fragment(NodeIdx(0)).unwrap();
    let detached = dom.create_element(fragment, "div");
    let descendant = dom.create_element(detached, "span");
    assert_eq!(dom.flat_tree_parent(detached), Ok(None));
    assert_eq!(dom.flat_tree_parent(descendant), Ok(Some(detached)));
    let template = dom.create_element(html, "template");
    let contents = dom.ensure_template_contents(template).unwrap();
    let inert = dom.create_element(contents, "i");
    assert_eq!(dom.flat_tree_parent(inert), Ok(None));
    let leaf = dom.create_element(html, "p"); let generation = dom.node_generation(leaf).unwrap();
    dom.detach(leaf); dom.retire_unreachable_node(leaf).unwrap();
    assert_eq!(dom.flat_tree_parent(leaf), Err(DomError::InvalidNode));
    let reused = dom.create_element(html, "p");
    assert_eq!(reused, leaf); assert_ne!(dom.node_generation(reused), Some(generation));
    assert_eq!(dom.flat_tree_parent(reused), Ok(Some(html)));
    assert_eq!(dom.flat_tree_parent(NodeIdx(u32::MAX)), Err(DomError::InvalidNode));
}
