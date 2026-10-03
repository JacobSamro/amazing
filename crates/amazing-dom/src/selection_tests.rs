use super::*;

#[test]
fn selection_direction_and_replacement_retain_actual_old_ranges() {
    let mut dom = Dom::new(); let document = NodeIdx(0);
    let parent = dom.create_element(document, "p"); let text = dom.create_text(parent, "abcdef");
    let id = dom.document_selection(document).unwrap();
    assert_eq!(dom.document_selection(document), Ok(id));
    assert_eq!(dom.selection_state(id).unwrap().direction, SelectionDirection::Directionless);
    dom.selection_set_base_and_extent(id, BoundaryPoint::new(text, 5), BoundaryPoint::new(text, 1)).unwrap();
    assert_eq!(dom.selection_points(id).unwrap(), Some((BoundaryPoint::new(text, 5), BoundaryPoint::new(text, 1))));
    assert_eq!(dom.selection_state(id).unwrap().direction, SelectionDirection::Backwards);
    let old = dom.selection_state(id).unwrap().range.unwrap();
    dom.range_set_start(old, BoundaryPoint::new(text, 2)).unwrap();
    assert_eq!(dom.selection_state(id).unwrap().range, Some(old));
    assert_eq!(dom.selection_state(id).unwrap().direction, SelectionDirection::Backwards);
    dom.selection_extend(id, BoundaryPoint::new(text, 6)).unwrap();
    assert_eq!(dom.selection_state(id).unwrap().direction, SelectionDirection::Forwards);
    let newer = dom.selection_state(id).unwrap().range.unwrap(); assert_ne!(old, newer);
    assert_eq!(dom.range(old).unwrap(), AbstractRange { start: BoundaryPoint::new(text, 2), end: BoundaryPoint::new(text, 5) });
    dom.selection_collapse_to(id, false).unwrap();
    assert_ne!(dom.selection_state(id).unwrap().range, Some(newer));
    assert_eq!(dom.selection_points(id).unwrap(), Some((BoundaryPoint::new(text, 6), BoundaryPoint::new(text, 6))));
    dom.selection_clear(id).unwrap();
    assert_eq!(dom.selection_collapse_to(id, true), Err(DomError::InvalidStateError));
    assert_eq!(dom.selection_remove(id, old), Err(DomError::NotFoundError));
}

#[test]
fn selection_task_coalescing_live_mutations_and_reentrant_delivery() {
    let mut dom = Dom::new(); let document = NodeIdx(0);
    let parent = dom.create_element(document, "p"); let text = dom.create_text(parent, "abcd");
    let id = dom.document_selection(document).unwrap();
    let range = dom.create_range(document).unwrap();
    dom.range_set_start(range, BoundaryPoint::new(text, 1)).unwrap();
    dom.range_set_end(range, BoundaryPoint::new(text, 3)).unwrap();
    dom.selection_add(id, range).unwrap();
    dom.replace_character_data(text, 0, 0, &['x' as u16]).unwrap();
    assert_eq!(dom.range(range).unwrap().start.offset, 2);
    dom.range_set_end(range, BoundaryPoint::new(text, 4)).unwrap();
    let tasks = dom.drain_selection_change_tasks(); assert_eq!(tasks.len(), 1);
    assert!(dom.drain_selection_change_tasks().is_empty());
    assert!(dom.begin_selection_change(tasks[0]));
    // Listener reentry must queue a new task after the old document flag is cleared.
    dom.selection_collapse_at(id, Some(BoundaryPoint::new(text, 0))).unwrap();
    let reentrant = dom.drain_selection_change_tasks(); assert_eq!(reentrant.len(), 1);
    assert!(!dom.begin_selection_change(tasks[0]));
    assert!(dom.begin_selection_change(reentrant[0]));
    let selected = dom.selection_state(id).unwrap().range.unwrap();
    dom.release_range(selected); assert!(dom.range(selected).is_ok());
    dom.range_set_start(selected, BoundaryPoint::new(text, 0)).unwrap();
    assert!(dom.drain_selection_change_tasks().is_empty());
}

#[test]
fn document_open_style_replace_preserves_selection_range_and_pending_task() {
    let mut dom = Dom::new(); let document = NodeIdx(0);
    let parent = dom.create_element(document, "p"); dom.create_text(parent, "abc");
    let id = dom.document_selection(document).unwrap();
    dom.selection_select_children(id, parent).unwrap();
    let range = dom.selection_state(id).unwrap().range.unwrap();
    let tasks = dom.drain_selection_change_tasks(); assert_eq!(tasks.len(), 1);
    dom.replace_all(document, None).unwrap();
    assert_eq!(dom.document_selection(document), Ok(id));
    assert_eq!(dom.selection_state(id).unwrap().range, Some(range));
    assert_eq!(dom.selection_visible_points(id).unwrap(), Some((BoundaryPoint::new(document, 0), BoundaryPoint::new(document, 0))));
    assert!(dom.drain_selection_change_tasks().is_empty());
    assert!(dom.begin_selection_change(tasks[0]));
}

#[test]
fn selection_foreign_detached_validation_is_not_owner_document_membership() {
    let mut dom = Dom::new(); let doc = NodeIdx(0);
    let text = dom.create_text(doc, "abc"); let id = dom.document_selection(doc).unwrap();
    dom.selection_collapse_at(id, Some(BoundaryPoint::new(text, 1))).unwrap();
    let range = dom.selection_state(id).unwrap().range;
    let foreign = dom.create_document(); let foreign_text = dom.create_text(foreign, "other");
    dom.selection_collapse_at(id, Some(BoundaryPoint::new(foreign_text, 0))).unwrap();
    assert_eq!(dom.selection_state(id).unwrap().range, range);
    assert_eq!(dom.selection_collapse_at(id, Some(BoundaryPoint::new(foreign_text, 6))), Err(DomError::IndexSizeError));
    let detached = dom.create_element_local_in(doc, Some("http://www.w3.org/1999/xhtml"), "span").unwrap();
    dom.selection_select_children(id, detached).unwrap(); assert_eq!(dom.selection_state(id).unwrap().range, range);
    dom.selection_extend(id, BoundaryPoint::new(detached, 0)).unwrap(); assert_eq!(dom.selection_state(id).unwrap().range, range);
    dom.selection_clear(id).unwrap();
    assert_eq!(dom.selection_extend(id, BoundaryPoint::new(text, 0)), Err(DomError::InvalidStateError));
    assert_eq!(dom.selection_document(SelectionId::from_slot(usize::MAX)), Err(DomError::InvalidStateError));
}

#[test]
fn selection_generation_does_not_follow_recycled_document_and_stale_tasks_do_not_dispatch() {
    let mut dom = Dom::new(); let document = dom.create_document();
    let id = dom.document_selection(document).unwrap();
    dom.selection_collapse_at(id, Some(BoundaryPoint::new(document, 0))).unwrap();
    let task = dom.drain_selection_change_tasks()[0];
    let generation = dom.node_generation(document).unwrap();
    // Exercise the collector seam's explicit invalidation guard; normal detachment never retires.
    dom.retire_unreachable_node(document).unwrap();
    let replacement = dom.create_document(); assert_eq!(replacement, document);
    assert_ne!(dom.node_generation(replacement), Some(generation));
    assert!(dom.selection_state(id).is_err());
    assert!(!dom.begin_selection_change(task));
    let replacement_id = dom.document_selection(replacement).unwrap(); assert_ne!(replacement_id, id);
    assert_eq!(dom.selection_state(replacement_id).unwrap().range, None);
}

#[test]
fn composed_ranges_rescope_nested_closed_roots_only_when_not_explicitly_allowed() {
    use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    let mut dom = Dom::new(); let document = NodeIdx(0);
    let parent = dom.create_element(document, "body"); let host = dom.create_element(parent, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: false, slot_assignment: SlotAssignment::Named, clonable: false, serializable: false };
    let root = dom.attach_shadow(host, options).unwrap();
    let nested_host = dom.create_element(root, "div"); let nested = dom.attach_shadow(nested_host, options).unwrap();
    let text = dom.create_text(nested, "abc"); let id = dom.document_selection(document).unwrap();
    dom.selection_set_base_and_extent(id, BoundaryPoint::new(text, 0), BoundaryPoint::new(text, 3)).unwrap();
    assert_eq!(dom.selection_visible_points(id).unwrap(), Some((BoundaryPoint::new(text, 0), BoundaryPoint::new(text, 3))));
    let external = dom.selection_composed_ranges(id, &[]).unwrap()[0];
    assert_eq!(dom.static_range(external).unwrap().boundaries(), AbstractRange { start: BoundaryPoint::new(parent, 0), end: BoundaryPoint::new(parent, 1) });
    let exposed = dom.selection_composed_ranges(id, &[nested]).unwrap()[0];
    assert_eq!(dom.static_range(exposed).unwrap().boundaries(), AbstractRange { start: BoundaryPoint::new(text, 0), end: BoundaryPoint::new(text, 3) });
    let outer = dom.selection_composed_ranges(id, &[root]).unwrap()[0];
    assert_eq!(dom.static_range(outer).unwrap().boundaries(), AbstractRange { start: BoundaryPoint::new(root, 0), end: BoundaryPoint::new(root, 1) });
    assert_eq!(dom.selection_composed_ranges(id, &[host]), Err(DomError::TypeError));
    // Static snapshot remains unchanged while live selected Range adjusts.
    dom.replace_character_data(text, 0, 1, &[]).unwrap();
    assert_eq!(dom.static_range(exposed).unwrap().boundaries().end.offset, 3);
    assert_eq!(dom.selection_points(id).unwrap().unwrap().1.offset, 2);
}

#[test]
fn selected_deletion_keeps_range_identity_in_document_and_connected_shadow() {
    let mut dom = Dom::new(); let parent = dom.create_element(NodeIdx(0), "p"); let text = dom.create_text(parent, "abcdef");
    let id = dom.document_selection(NodeIdx(0)).unwrap();
    dom.selection_set_base_and_extent(id, BoundaryPoint::new(text, 1), BoundaryPoint::new(text, 4)).unwrap();
    let range = dom.selection_state(id).unwrap().range;
    dom.selection_delete_from_document(id).unwrap();
    assert_eq!(dom.character_data_utf16(text).unwrap(), "aef".encode_utf16().collect::<Vec<_>>());
    assert_eq!(dom.selection_state(id).unwrap().range, range);
    assert!(dom.range(range.unwrap()).unwrap().collapsed());
    use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    let host = dom.create_element(parent, "div");
    let root = dom.attach_shadow(host, ShadowOptions { mode: ShadowMode::Closed,
        delegates_focus: false, slot_assignment: SlotAssignment::Named, clonable: false, serializable: false }).unwrap();
    let shadow_text = dom.create_text(root, "inside");
    dom.selection_set_base_and_extent(id, BoundaryPoint::new(shadow_text, 0), BoundaryPoint::new(shadow_text, 6)).unwrap();
    let shadow_range = dom.selection_state(id).unwrap().range;
    dom.selection_delete_from_document(id).unwrap();
    assert_eq!(dom.character_data_utf16(shadow_text).unwrap(), Vec::<u16>::new());
    assert_eq!(dom.selection_state(id).unwrap().range, shadow_range);
}


#[test]
fn connected_shadow_membership_is_actual_and_detached_owner_document_does_not_suffice() {
    use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    for mode in [ShadowMode::Open, ShadowMode::Closed] {
        let mut dom = Dom::new(); let document = NodeIdx(0);
        let body = dom.create_element(document, "body"); let host = dom.create_element(body, "div");
        let root = dom.attach_shadow(host, ShadowOptions { mode, delegates_focus: false,
            slot_assignment: SlotAssignment::Named, clonable: false, serializable: false }).unwrap();
        let text = dom.create_text(root, "abc"); let id = dom.document_selection(document).unwrap();
        dom.selection_select_children(id, text).unwrap();
        let selected = dom.selection_state(id).unwrap().range.unwrap();
        assert_eq!(dom.range(selected).unwrap(), AbstractRange {
            start: BoundaryPoint::new(text, 0), end: BoundaryPoint::new(text, 3) });
        assert!(dom.selection_visible_points(id).unwrap().is_some());
        dom.selection_clear(id).unwrap(); dom.selection_add(id, selected).unwrap();
        assert_eq!(dom.selection_state(id).unwrap().range, Some(selected));
        dom.remove(host).unwrap();
        assert_eq!(dom.owner_document(text), Some(document));
        assert_eq!(dom.selection_visible_points(id).unwrap(), None);
        dom.selection_clear(id).unwrap(); dom.selection_add(id, selected).unwrap();
        assert_eq!(dom.selection_state(id).unwrap().range, None);
        dom.selection_select_children(id, text).unwrap();
        assert_eq!(dom.selection_state(id).unwrap().range, None);
        let detached = dom.create_node_in(document, NodeKind::Text { data: "detached".into() }).unwrap();
        dom.selection_collapse_at(id, Some(BoundaryPoint::new(detached, 0))).unwrap();
        assert_eq!(dom.selection_state(id).unwrap().range, None);
    }
}
