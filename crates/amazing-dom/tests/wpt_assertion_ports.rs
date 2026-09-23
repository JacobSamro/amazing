//! Native assertion ports from WPT c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
//! These exercise DOM algorithms, not the WPT runner or JS bindings.
use amazing_dom::{Dom, DomError, NodeIdx, NodeKind, DocumentPosition as P};
use amazing_dom::range::BoundaryPoint as B;
use amazing_dom::traversal::{NodeFilter, Traverse, TraversalStep};

#[test]
fn range_point_checks_follow_wpt_root_type_offset_and_position_order() {
    // dom/ranges/Range-comparePoint.html and Range-isPointInRange.html:
    // wrong-root precedes doctype/offset validation; endpoints are included.
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "main");
    let text = dom.create_text(parent, "abcd");
    let range = dom.create_range(NodeIdx(0)).unwrap();
    dom.range_set_start(range, B::new(text, 1)).unwrap();
    dom.range_set_end(range, B::new(text, 3)).unwrap();
    for (offset, comparison, inside) in [(0, -1, false), (1, 0, true),
        (2, 0, true), (3, 0, true), (4, 1, false)] {
        assert_eq!(dom.range_compare_point(range, B::new(text, offset)), Ok(comparison));
        assert_eq!(dom.range_is_point_in_range(range, B::new(text, offset)), Ok(inside));
    }
    assert_eq!(dom.range_compare_point(range, B::new(text, 5)), Err(DomError::IndexSizeError));
    assert_eq!(dom.range_is_point_in_range(range, B::new(text, 5)), Err(DomError::IndexSizeError));
    let detached = dom.create_node_in(NodeIdx(0), NodeKind::Text { data: "x".into() }).unwrap();
    assert_eq!(dom.range_compare_point(range, B::new(detached, 99)), Err(DomError::WrongDocumentError));
    assert_eq!(dom.range_is_point_in_range(range, B::new(detached, 99)), Ok(false));
    let foreign_doctype = dom.create_document_type_in(NodeIdx(0), "html", "", "").unwrap();
    assert_eq!(dom.range_compare_point(range, B::new(foreign_doctype, 0)), Err(DomError::WrongDocumentError));
    assert_eq!(dom.range_is_point_in_range(range, B::new(foreign_doctype, 0)), Ok(false));
    let doctype = dom.create_document_type_in(NodeIdx(0), "html", "", "").unwrap();
    dom.pre_insert(doctype, NodeIdx(0), Some(parent)).unwrap();
    assert_eq!(dom.range_compare_point(range, B::new(doctype, 0)), Err(DomError::InvalidNodeTypeError));
    assert_eq!(dom.range_is_point_in_range(range, B::new(doctype, 0)), Err(DomError::InvalidNodeTypeError));
}

#[test]
fn node_pre_insert_errors_precede_missing_reference_and_position_is_antisymmetric() {
    // dom/nodes/pre-insertion-validation-notfound.js: parent/ancestor checks
    // precede NotFoundError. Node-compareDocumentPosition.html: tree bits.
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let a = dom.create_element(root, "a");
    let b = dom.create_element(root, "b");
    let detached = dom.create_element(a, "detached");
    let missing = dom.create_node_in(NodeIdx(0), NodeKind::Text { data: "".into() }).unwrap();
    assert_eq!(dom.pre_insert(root, detached, Some(missing)), Err(DomError::HierarchyRequestError));
    assert_eq!(dom.pre_insert(missing, root, Some(detached)), Err(DomError::NotFoundError));
    assert_eq!(dom.pre_insert(a, missing, Some(detached)), Err(DomError::HierarchyRequestError));
    assert_eq!(dom.compare_document_position(a, a), 0);
    assert_eq!(dom.compare_document_position(root, a), P::CONTAINED_BY | P::FOLLOWING);
    assert_eq!(dom.compare_document_position(a, root), P::CONTAINS | P::PRECEDING);
    assert_eq!(dom.compare_document_position(a, b), P::FOLLOWING);
    assert_eq!(dom.compare_document_position(b, a), P::PRECEDING);
    let ab = dom.compare_document_position(a, missing);
    let ba = dom.compare_document_position(missing, a);
    assert_eq!(ab & (P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC), P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC);
    assert_eq!(ba & (P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC), P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC);
    assert_ne!(ab & (P::PRECEDING | P::FOLLOWING), ba & (P::PRECEDING | P::FOLLOWING));
}

#[test]
fn normalize_merges_only_adjacent_nonempty_text() {
    // dom/nodes/Node-normalize.html: detached fragment is unaffected by
    // document.normalize; empty Text is removed and adjacent Text merged.
    let mut dom = Dom::new();
    let fragment = dom.create_document_fragment(NodeIdx(0)).unwrap();
    let first = dom.create_text(fragment, "1");
    let second = dom.create_text(fragment, "2");
    let element = dom.create_element(fragment, "x");
    let nested_a = dom.create_text(element, "3");
    let nested_b = dom.create_text(element, "4");
    dom.normalize(NodeIdx(0)).unwrap();
    assert_eq!(dom.children(fragment).collect::<Vec<_>>(), vec![first, second, element]);
    dom.normalize(fragment).unwrap();
    assert_eq!(dom.children(fragment).collect::<Vec<_>>(), vec![first, element]);
    assert_eq!(dom.character_data_utf16(first), Some("12".encode_utf16().collect()));
    assert_eq!(dom.children(element).collect::<Vec<_>>(), vec![nested_a]);
    assert_eq!(dom.character_data_utf16(nested_a), Some("34".encode_utf16().collect()));
    assert_eq!(dom.character_data_utf16(nested_b), Some("4".encode_utf16().collect()));
    let empty = dom.create_text(element, "");
    dom.normalize(element).unwrap();
    assert_eq!(dom.get(empty).parent, None);
}

#[test]
fn iterator_removal_moves_reference_to_next_or_previous() {
    // dom/traversal/NodeIterator-removal.html: the two targeted
    // pointer-before-reference cases at the end of the source.
    for has_following in [true, false] {
        let mut dom = Dom::new();
        let root = dom.create_element(NodeIdx(0), "root");
        let a = dom.create_element(root, "a");
        let a1 = dom.create_element(a, "a1");
        let b = dom.create_element(root, "b");
        let b1 = dom.create_element(b, "b1");
        let c = has_following.then(|| dom.create_element(root, "c"));
        let iter = dom.create_node_iterator(root, NodeFilter::SHOW_ALL, None).unwrap();
        for _ in 0..5 { assert!(matches!(dom.begin_traversal(iter, Traverse::Next), Ok(TraversalStep::Done(Some(_))))); }
        assert_eq!(dom.iterator_reference(iter).unwrap().node, b1);
        assert_eq!(dom.begin_traversal(iter, Traverse::Previous).unwrap(), TraversalStep::Done(Some(b1)));
        assert!(dom.iterator_reference(iter).unwrap().before);
        dom.pre_remove(b, root).unwrap();
        let pointer = dom.iterator_reference(iter).unwrap();
        assert_eq!(pointer.node, c.unwrap_or(a1));
        assert_eq!(pointer.before, has_following);
    }
}

#[test]
fn range_contents_on_one_text_node_clone_extract_and_delete() {
    // dom/ranges/Range-cloneContents.html, Range-extractContents.html,
    // Range-deleteContents.html: the same-CharacterData branch creates a
    // fragment with the selected substring; clone leaves source unchanged.
    for mode in 0..3 {
        let mut dom = Dom::new();
        let parent = dom.create_element(NodeIdx(0), "p");
        let text = dom.create_text(parent, "abcdef");
        let range = dom.create_range(NodeIdx(0)).unwrap();
        dom.range_set_start(range, B::new(text, 1)).unwrap();
        dom.range_set_end(range, B::new(text, 4)).unwrap();
        let fragment = match mode {
            0 => Some(dom.range_clone_contents(range).unwrap()),
            1 => Some(dom.range_extract_contents(range).unwrap()),
            _ => { dom.range_delete_contents(range).unwrap(); None },
        };
        if let Some(fragment) = fragment {
            let copied = dom.children(fragment).collect::<Vec<_>>();
            assert_eq!(copied.len(), 1);
            assert_ne!(copied[0], text);
            assert_eq!(dom.character_data_utf16(copied[0]), Some("bcd".encode_utf16().collect()));
        }
        assert_eq!(dom.character_data_utf16(text),
            Some(if mode == 0 { "abcdef" } else { "aef" }.encode_utf16().collect()));
        if mode != 0 { assert!(dom.range(range).unwrap().collapsed()); }
    }
}

#[test]
fn clone_node_preserves_attributes_and_obeys_deep_flag() {
    // dom/nodes/Node-cloneNode.html: a clone has distinct identity, matching
    // attributes, no parent; descendants appear only for cloneNode(true).
    let mut dom = Dom::new();
    let original = dom.create_element(NodeIdx(0), "div");
    dom.set_attribute(original, "data-port", "v").unwrap();
    let child = dom.create_text(original, "child");
    let shallow = dom.clone_node(original, false).unwrap();
    let deep = dom.clone_node(original, true).unwrap();
    assert_ne!(original, shallow);
    assert_eq!(dom.get(shallow).parent, None);
    assert_eq!(dom.get(deep).parent, None);
    assert_eq!(dom.attribute_utf16(shallow, "data-port"), Some("v".encode_utf16().collect()));
    assert_eq!(dom.children(shallow).count(), 0);
    let copied = dom.children(deep).collect::<Vec<_>>();
    assert_eq!(copied.len(), 1);
    assert_ne!(copied[0], child);
    assert_eq!(dom.character_data_utf16(copied[0]), Some("child".encode_utf16().collect()));
}

#[test]
fn tree_walker_filter_error_keeps_current_node_and_can_retry() {
    // dom/traversal/TreeWalker-acceptNode-filter.html: a throwing filter
    // leaves currentNode at root and the next call asks the filter again.
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let child = dom.create_element(root, "child");
    let walker = dom.create_tree_walker(root, NodeFilter::SHOW_ELEMENT, Some(7)).unwrap();
    for direction in [Traverse::FirstChild, Traverse::Next] {
        let request = dom.begin_traversal(walker, direction).unwrap();
        let TraversalStep::Filter { traversal, callback, node } = request else { panic!("expected filter"); };
        assert_eq!(callback, 7);
        assert_eq!(node, child);
        assert_eq!(dom.resume_traversal(traversal, Err(DomError::TypeError)), Err(DomError::TypeError));
        assert_eq!(dom.walker_current(walker), Ok(root));
    }
    let TraversalStep::Filter { traversal, .. } = dom.begin_traversal(walker, Traverse::Next).unwrap()
        else { panic!("expected filter"); };
    assert_eq!(dom.resume_traversal(traversal, Ok(NodeFilter::ACCEPT)), Ok(TraversalStep::Done(Some(child))));
    assert_eq!(dom.walker_current(walker), Ok(child));
}
