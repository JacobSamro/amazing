use super::*;
use BoundaryPoint as P;
fn setup() -> (Dom, NodeIdx, NodeIdx, NodeIdx) {
    let mut d = Dom::new();
    let root = d.create_element(NodeIdx(0), "root");
    let a = d.create_text(root, "abcd");
    let b = d.create_text(root, "efgh");
    (d, root, a, b)
}
fn range(d: &mut Dom, start: (NodeIdx, usize), end: (NodeIdx, usize)) -> Range {
    let r = d.create_range(NodeIdx(0)).unwrap();
    d.range_set_start(r, P::new(start.0, start.1)).unwrap();
    d.range_set_end(r, P::new(end.0, end.1)).unwrap();
    r
}
#[test]
fn boundary_order_uses_ancestor_offsets() {
    let (d, root, a, b) = setup();
    let points = [
        P::new(root, 0),
        P::new(a, 0),
        P::new(a, 4),
        P::new(root, 1),
        P::new(b, 0),
        P::new(b, 4),
        P::new(root, 2),
    ];
    for (i, a) in points.iter().enumerate() {
        for (j, b) in points.iter().enumerate() {
            assert_eq!(d.compare_boundary_points(*a, *b), Ok(i.cmp(&j)));
        }
    }
}
#[test]
fn static_range_can_be_invalid_and_never_moves() {
    let (mut d, root, a, _) = setup();
    let invalid = d
        .create_static_range(P::new(a, 99), P::new(root, 0))
        .unwrap();
    assert!(!d.static_range_valid(invalid));
    let r = d.create_static_range(P::new(a, 1), P::new(a, 4)).unwrap();
    assert!(d.static_range_valid(r));
    d.set_character_data(a, "x".into()).unwrap();
    assert_eq!(r.boundaries().end.offset, 4);
    assert!(!d.static_range_valid(r));
}
#[test]
fn setters_validate_then_collapse_across_roots_or_order() {
    let (mut d, root, a, b) = setup();
    let r = range(&mut d, (a, 1), (b, 2));
    let old = d.range(r).unwrap();
    assert_eq!(
        d.range_set_start(r, P::new(a, 9)),
        Err(DomError::IndexSizeError)
    );
    assert_eq!(d.range(r).unwrap(), old);
    d.range_set_start(r, P::new(b, 3)).unwrap();
    assert!(d.range(r).unwrap().collapsed());
    d.range_set_end_before(r, a).unwrap();
    assert_eq!(d.range(r).unwrap().start, P::new(root, 0));
    let detached = d.alloc_unattached(NodeKind::Text { data: "x".into() });
    d.range_set_end(r, P::new(detached, 1)).unwrap();
    assert!(d.range(r).unwrap().collapsed());
    assert_eq!(
        d.range_set_start_before(r, detached),
        Err(DomError::InvalidNodeTypeError)
    );
}
#[test]
fn compare_modes_and_intersection() {
    let (mut d, root, a, b) = setup();
    let r = range(&mut d, (a, 1), (b, 2));
    let s = range(&mut d, (a, 2), (b, 3));
    for (how, order) in [
        (0, Ordering::Less),
        (1, Ordering::Greater),
        (2, Ordering::Less),
        (3, Ordering::Less),
    ] {
        assert_eq!(d.range_compare_boundary_points(r, how, s), Ok(order));
    }
    assert_eq!(d.range_common_ancestor(r), Ok(root));
    assert_eq!(d.range_intersects_node(r, a), Ok(true));
    d.range_select_node(r, a).unwrap();
    assert_eq!(d.range_intersects_node(r, b), Ok(false));
    assert_eq!(
        d.range_compare_boundary_points(r, 9, s),
        Err(DomError::NotSupportedError)
    );
    let other = d.create_document();
    let t = d.create_range(other).unwrap();
    assert_eq!(
        d.range_compare_boundary_points(r, 0, t),
        Err(DomError::WrongDocumentError)
    );
    assert_eq!(d.range_intersects_node(r, other), Ok(false));
}
#[test]
fn insertion_at_boundary_does_not_shift_it() {
    let (mut d, root, a, b) = setup();
    let r = range(&mut d, (root, 1), (root, 2));
    let n = d.alloc_unattached(NodeKind::Text { data: "x".into() });
    d.pre_insert(n, root, Some(b)).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 1),
            end: P::new(root, 3)
        }
    );
    d.pre_insert(a, root, None).unwrap();
    assert_eq!(d.range(r).unwrap().start.offset, 0);
    assert_eq!(d.range(r).unwrap().end.offset, 2);
}
#[test]
fn fragment_removal_and_insertion_update_all_ranges() {
    let (mut d, root, _, b) = setup();
    let f = d.create_document_fragment(NodeIdx(0)).unwrap();
    let t = d.create_text(f, "x");
    d.create_text(f, "y");
    let r = range(&mut d, (t, 0), (t, 1));
    let parent_range = range(&mut d, (root, 1), (root, 2));
    d.pre_insert(f, root, Some(b)).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(f, 0),
            end: P::new(f, 0)
        }
    );
    assert_eq!(d.range(parent_range).unwrap().end.offset, 4);
}
#[test]
fn removal_relocates_descendants_and_decrements_following_offsets() {
    let (mut d, root, a, _) = setup();
    let r = range(&mut d, (a, 2), (root, 2));
    d.detach(a);
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 0),
            end: P::new(root, 1)
        }
    );
}
#[test]
fn replace_data_endpoint_rules_and_utf16() {
    let (mut d, _, a, _) = setup();
    d.set_character_data(a, "a😀bc".into()).unwrap();
    assert_eq!(d.node_length(a), 5);
    let r = range(&mut d, (a, 2), (a, 5));
    d.replace_character_data(a, 1, 2, &[120]).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(a, 1),
            end: P::new(a, 4)
        }
    );
    d.replace_character_data(a, 1, 0, &[121, 122]).unwrap();
    assert_eq!(d.range(r).unwrap().start.offset, 1);
    assert_eq!(d.range(r).unwrap().end.offset, 6);
}
#[test]
fn split_text_preserves_surrogate_halves_and_parent_boundaries() {
    let (mut d, root, a, _) = setup();
    d.set_character_data(a, "😀".into()).unwrap();
    let r = range(&mut d, (a, 1), (a, 2));
    let p = range(&mut d, (root, 1), (root, 2));
    let new = d.split_text(a, 1).unwrap();
    assert_eq!(d.character_data_utf16(a), Some(vec![0xd83d]));
    assert_eq!(d.character_data_utf16(new), Some(vec![0xde00]));
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(a, 1),
            end: P::new(new, 1)
        }
    );
    assert_eq!(
        d.range(p).unwrap(),
        AbstractRange {
            start: P::new(root, 2),
            end: P::new(root, 3)
        }
    );
    d.append_text_data(a, "!");
    assert_eq!(d.character_data_utf16(a), Some(vec![0xd83d, 33]));
}
#[test]
fn detached_split_clamps_instead_of_relocating() {
    let mut d = Dom::new();
    let a = d.alloc_unattached(NodeKind::Text {
        data: "abcd".into(),
    });
    let r = range(&mut d, (a, 2), (a, 4));
    d.split_text(a, 1).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(a, 1),
            end: P::new(a, 1)
        }
    );
}
fn partial_tree() -> (Dom, NodeIdx, NodeIdx, NodeIdx, NodeIdx, NodeIdx) {
    let mut d = Dom::new();
    let root = d.create_element(NodeIdx(0), "root");
    let left = d.create_element(root, "left");
    let a = d.create_text(left, "abcd");
    let mid = d.create_element(root, "mid");
    d.create_text(mid, "middle");
    let right = d.create_element(root, "right");
    let b = d.create_text(right, "efgh");
    (d, root, left, mid, a, b)
}
#[test]
fn clone_partial_contents_preserves_source_and_node_attributes() {
    let (mut d, root, left, _, a, b) = partial_tree();
    d.set_attribute_ns(left, Some("urn:x"), "a", "v").unwrap();
    let r = range(&mut d, (a, 2), (b, 2));
    let before = d.dump(root);
    let bounds = d.range(r).unwrap();
    let f = d.range_clone_contents(r).unwrap();
    assert_eq!(
        d.dump(f),
        "#document-fragment\n  <left>\n    \"cd\"\n  <mid>\n    \"middle\"\n  <right>\n    \"ef\"\n"
    );
    assert_eq!(d.dump(root), before);
    assert_eq!(d.range(r).unwrap(), bounds);
    assert_eq!(
        d.get_attribute_ns(d.get(f).first_child.unwrap(), Some("urn:x"), "a"),
        Some("v")
    );
}
#[test]
fn extract_partial_contents_moves_whole_children_and_collapses_before_mutation() {
    let (mut d, root, _, mid, a, b) = partial_tree();
    let r = range(&mut d, (a, 2), (b, 2));
    let watching = range(&mut d, (mid, 0), (mid, 1));
    let f = d.range_extract_contents(r).unwrap();
    assert_eq!(d.get(mid).parent, Some(f));
    assert_eq!(d.character_data(a), Some("ab"));
    assert_eq!(d.character_data(b), Some("gh"));
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 1),
            end: P::new(root, 1)
        }
    );
    assert_eq!(d.range(watching).unwrap().start, P::new(root, 1));
}
#[test]
fn delete_partial_contents_leaves_partial_ancestors() {
    let (mut d, root, left, mid, a, b) = partial_tree();
    let r = range(&mut d, (a, 2), (b, 2));
    d.range_delete_contents(r).unwrap();
    assert_eq!(d.get(left).parent, Some(root));
    assert_eq!(d.get(mid).parent, None);
    assert_eq!(d.character_data(a), Some("ab"));
    assert_eq!(d.character_data(b), Some("gh"));
    assert!(d.range(r).unwrap().collapsed());
}
#[test]
fn extract_with_ancestor_start_and_character_data_end() {
    let (mut d, root, a, b) = setup();
    let r = range(&mut d, (root, 0), (b, 2));
    let f = d.range_extract_contents(r).unwrap();
    assert_eq!(d.get(a).parent, Some(f));
    assert_eq!(d.dump(f), "#document-fragment\n  \"abcd\"\n  \"ef\"\n");
    assert_eq!(d.range(r).unwrap().start, P::new(root, 0));
}
#[test]
fn doctype_extract_and_clone_throw_before_mutating_but_delete_works() {
    let mut d = Dom::new();
    let doc = NodeIdx(0);
    let dt = d.append_child(
        doc,
        NodeKind::DocumentType {
            name: "html".into(),
            public_id: "".into(),
            system_id: "".into(),
        },
    );
    let r = range(&mut d, (doc, 0), (doc, 1));
    let old = d.range(r).unwrap();
    assert_eq!(
        d.range_extract_contents(r),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.range_clone_contents(r),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(d.range(r).unwrap(), old);
    assert_eq!(
        d.range_select_node_contents(r, dt),
        Err(DomError::InvalidNodeTypeError)
    );
    d.range_delete_contents(r).unwrap();
    assert_eq!(d.get(dt).parent, None);
}
#[test]
fn surround_rejects_partial_nontext_before_changes() {
    let (mut d, root, _, _, a, b) = partial_tree();
    let r = range(&mut d, (a, 1), (b, 1));
    let wrapper = d.alloc_unattached(NodeKind::Element {
        name: "wrap".into(),
        attrs: vec![],
        namespace: crate::Namespace::Html,
    });
    let before = d.dump(root);
    assert_eq!(
        d.range_surround_contents(r, wrapper),
        Err(DomError::InvalidStateError)
    );
    assert_eq!(d.dump(root), before);
}
#[test]
fn surround_text_splits_and_selects_wrapper() {
    let (mut d, root, a, _) = setup();
    let r = range(&mut d, (a, 1), (a, 3));
    let wrapper = d.create_element(root, "wrap");
    d.create_text(wrapper, "discard");
    d.range_surround_contents(r, wrapper).unwrap();
    assert_eq!(d.dump(wrapper), "<wrap>\n  \"bc\"\n");
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 1),
            end: P::new(root, 2)
        }
    );
}
#[test]
fn insert_validates_before_text_split_and_expands_collapsed_range() {
    let (mut d, root, a, _) = setup();
    let r = range(&mut d, (a, 2), (a, 2));
    let old_count = d.node_count();
    assert_eq!(
        d.range_insert_node(r, root),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(d.node_count(), old_count);
    assert_eq!(d.character_data(a), Some("abcd"));
    let x = d.alloc_unattached(NodeKind::Comment { data: "x".into() });
    d.range_insert_node(r, x).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(a, 2),
            end: P::new(root, 2)
        }
    );
}
#[test]
fn character_data_old_value_retains_lone_surrogate() {
    use crate::mutation::ObserveOptions;
    let (mut d, _, a, _) = setup();
    d.replace_character_data(a, 0, usize::MAX, &[0xd800])
        .unwrap();
    let observer = d.create_mutation_observer(1);
    d.observe(
        observer,
        a,
        ObserveOptions {
            character_data_old_value: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_character_data(a, "new".into()).unwrap();
    assert_eq!(
        d.take_mutation_records(observer)[0].old_value_utf16,
        Some(vec![0xd800])
    );
}
#[test]
fn clone_and_release_live_handles() {
    let (mut d, _, a, _) = setup();
    let r = range(&mut d, (a, 1), (a, 3));
    let clone = d.clone_range(r).unwrap();
    d.detach_range(r).unwrap();
    d.replace_character_data(a, 0, 2, &[]).unwrap();
    assert_eq!(d.range(r), d.range(clone));
    d.release_range(r);
    assert_eq!(d.range(r), Err(DomError::InvalidStateError));
    assert!(d.range(clone).is_ok());
}
#[test]
fn normalize_merges_ranges_before_removing_text_nodes() {
    let (mut d, root, a, b) = setup();
    let empty = d.create_text(root, "");
    let c = d.create_text(root, "ij");
    let r = range(&mut d, (b, 1), (c, 2));
    let parent = range(&mut d, (root, 1), (root, 3));
    let e = range(&mut d, (empty, 0), (empty, 0));
    d.normalize(root).unwrap();
    assert_eq!(d.character_data(a), Some("abcdefghij"));
    assert_eq!(d.children(root).collect::<Vec<_>>(), vec![a]);
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(a, 5),
            end: P::new(a, 10)
        }
    );
    assert_eq!(
        d.range(parent).unwrap(),
        AbstractRange {
            start: P::new(a, 4),
            end: P::new(a, 8)
        }
    );
    assert_eq!(d.range(e).unwrap().start, P::new(a, 8));
}

#[test]
fn normalize_only_touches_exclusive_text_nodes() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let first = dom.create_text(root, "a");
    let pi = dom.append_child(root, NodeKind::ProcessingInstruction {
        target: "pi".into(), data: String::new(),
    });
    let second = dom.create_text(root, "b");
    let cdata = dom.append_child(root, NodeKind::CDATASection { data: String::new() });
    let third = dom.create_text(root, "c");
    let comment = dom.append_child(root, NodeKind::Comment { data: String::new() });
    let fourth = dom.create_text(root, "d");
    let expected = vec![first, pi, second, cdata, third, comment, fourth];
    dom.normalize(root).unwrap();
    assert_eq!(dom.children(root).collect::<Vec<_>>(), expected);
    assert_eq!(dom.character_data(cdata), Some(""));
    assert_eq!(dom.character_data(first), Some("a"));
    assert_eq!(dom.character_data(second), Some("b"));
}
#[test]
fn normalize_reunites_surrogates_and_ignores_self() {
    let (mut d, root, a, b) = setup();
    d.replace_character_data(a, 0, 99, &[0xd83d]).unwrap();
    d.replace_character_data(b, 0, 99, &[0xde00]).unwrap();
    d.normalize(a).unwrap();
    assert_eq!(d.node_length(a), 1);
    d.normalize(root).unwrap();
    assert_eq!(d.character_data(a), Some("😀"));
    assert_eq!(d.node_length(a), 2);
}
#[test]
fn replace_all_and_adoption_use_live_removal_hooks() {
    let (mut d, root, a, b) = setup();
    let r = range(&mut d, (a, 1), (b, 2));
    let doc = d.create_document();
    d.adopt(root, doc).unwrap();
    assert_eq!(d.range(r).unwrap().start, P::new(NodeIdx(0), 0));
    assert!(d.range(r).unwrap().collapsed());
    let r = range(&mut d, (a, 1), (b, 2));
    d.replace_all(root, None).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 0),
            end: P::new(root, 0)
        }
    );
}
#[test]
fn partially_contained_comment_is_not_a_text_exception_for_surround() {
    let (mut d, root, _, b) = setup();
    let c = d.alloc_unattached(NodeKind::Comment {
        data: "comment".into(),
    });
    d.pre_insert(c, root, Some(b)).unwrap();
    let r = range(&mut d, (c, 2), (b, 2));
    let wrapper = d.alloc_unattached(NodeKind::Element {
        name: "w".into(),
        attrs: vec![],
        namespace: crate::Namespace::Html,
    });
    assert_eq!(
        d.range_surround_contents(r, wrapper),
        Err(DomError::InvalidStateError)
    );
    let f = d.range_extract_contents(r).unwrap();
    assert_eq!(
        d.dump(f),
        "#document-fragment\n  <!-- mment -->\n  \"ef\"\n"
    );
    assert_eq!(d.character_data(c), Some("co"));
}
#[test]
fn surround_character_data_parent_errors_after_extraction() {
    let (mut d, _, a, _) = setup();
    let r = range(&mut d, (a, 1), (a, 3));
    let comment = d.alloc_unattached(NodeKind::Comment { data: "w".into() });
    assert_eq!(
        d.range_surround_contents(r, comment),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(d.character_data(a), Some("a"));
    assert!(d.get(comment).parent.is_some());
}
#[test]
fn collapsed_extract_clone_delete_are_empty_and_preserve_boundaries() {
    let (mut d, _, a, _) = setup();
    let r = range(&mut d, (a, 2), (a, 2));
    let old = d.range(r).unwrap();
    for f in [
        d.range_extract_contents(r).unwrap(),
        d.range_clone_contents(r).unwrap(),
    ] {
        assert_eq!(d.get(f).kind, NodeKind::DocumentFragment);
        assert_eq!(d.node_length(f), 0);
    }
    d.range_delete_contents(r).unwrap();
    assert_eq!(d.range(r).unwrap(), old);
    assert_eq!(d.character_data(a), Some("abcd"));
}
#[test]
fn select_collapse_and_set_after_variants() {
    let (mut d, root, a, b) = setup();
    let r = d.create_range(NodeIdx(0)).unwrap();
    d.range_select_node_contents(r, root).unwrap();
    d.range_set_start_after(r, a).unwrap();
    d.range_set_end_after(r, b).unwrap();
    assert_eq!(
        d.range(r).unwrap(),
        AbstractRange {
            start: P::new(root, 1),
            end: P::new(root, 2)
        }
    );
    d.range_collapse(r, false).unwrap();
    assert_eq!(d.range(r).unwrap().start, P::new(root, 2));
    d.range_set_start_before(r, a).unwrap();
    d.range_collapse(r, true).unwrap();
    assert_eq!(d.range(r).unwrap().end, P::new(root, 0));
}
