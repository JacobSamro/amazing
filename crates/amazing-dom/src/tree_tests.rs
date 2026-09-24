use super::*;
use crate::{Namespace, mutation::ObserveOptions};
fn element(d: &mut Dom, name: &str) -> NodeIdx {
    d.alloc_unattached(NodeKind::Element {
        name: name.into(),
        attrs: vec![],
        namespace: Namespace::Html,
    })
}
fn dt(d: &mut Dom) -> NodeIdx {
    d.alloc_unattached(NodeKind::DocumentType {
        name: "html".into(),
        public_id: "".into(),
        system_id: "".into(),
    })
}
fn assert_links(d: &Dom, parent: NodeIdx, expected: &[NodeIdx]) {
    assert_eq!(d.children(parent).collect::<Vec<_>>(), expected);
    assert_eq!(d.get(parent).first_child, expected.first().copied());
    assert_eq!(d.get(parent).last_child, expected.last().copied());
    for (i, id) in expected.iter().enumerate() {
        assert_eq!(d.get(*id).parent, Some(parent));
        assert_eq!(
            d.get(*id).prev_sibling,
            i.checked_sub(1).map(|j| expected[j])
        );
        assert_eq!(d.get(*id).next_sibling, expected.get(i + 1).copied());
    }
}
#[test]
fn default_has_a_document() {
    assert_eq!(Dom::default().node_count(), 1);
}
#[test]
fn invalid_parent_precedes_reference_not_found() {
    let mut d = Dom::new();
    let text = d.alloc_unattached(NodeKind::Text { data: "x".into() });
    let n = element(&mut d, "x");
    assert_eq!(
        d.pre_insert(n, text, Some(NodeIdx(0))),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(d.get(n).parent, None);
}
#[test]
fn cycles_and_document_insertion_are_rejected_without_changes() {
    let mut d = Dom::new();
    let a = d.create_element(NodeIdx(0), "a");
    let b = d.create_element(a, "b");
    assert_eq!(
        d.pre_insert(a, b, None),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.pre_insert(a, a, None),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.pre_insert(NodeIdx(0), b, None),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, a, &[b]);
    assert_links(&d, NodeIdx(0), &[a]);
}
#[test]
fn wrong_reference_and_wrong_removal_are_not_found() {
    let mut d = Dom::new();
    let a = element(&mut d, "a");
    let b = element(&mut d, "b");
    assert_eq!(
        d.pre_insert(a, NodeIdx(0), Some(b)),
        Err(DomError::NotFoundError)
    );
    assert_eq!(d.pre_remove(a, NodeIdx(0)), Err(DomError::NotFoundError));
}
#[test]
fn document_has_one_element_and_no_text() {
    let mut d = Dom::new();
    let a = element(&mut d, "a");
    let b = element(&mut d, "b");
    let t = d.alloc_unattached(NodeKind::Text { data: "".into() });
    assert_eq!(
        d.pre_insert(t, NodeIdx(0), None),
        Err(DomError::HierarchyRequestError)
    );
    d.pre_insert(a, NodeIdx(0), None).unwrap();
    assert_eq!(
        d.pre_insert(b, NodeIdx(0), None),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.pre_insert(a, NodeIdx(0), Some(a)),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, NodeIdx(0), &[a]);
}
#[test]
fn doctype_order_uniqueness_and_parent_validity() {
    let mut d = Dom::new();
    let a = element(&mut d, "html");
    let typ = dt(&mut d);
    let other = dt(&mut d);
    d.pre_insert(a, NodeIdx(0), None).unwrap();
    assert_eq!(
        d.pre_insert(typ, NodeIdx(0), None),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.pre_insert(typ, a, None),
        Err(DomError::HierarchyRequestError)
    );
    d.pre_insert(typ, NodeIdx(0), Some(a)).unwrap();
    assert_eq!(
        d.pre_insert(other, NodeIdx(0), Some(a)),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, NodeIdx(0), &[typ, a]);
}
#[test]
fn element_cannot_precede_doctype_even_through_comment() {
    let mut d = Dom::new();
    let comment = d.alloc_unattached(NodeKind::Comment { data: "".into() });
    let typ = dt(&mut d);
    let a = element(&mut d, "html");
    d.pre_insert(comment, NodeIdx(0), None).unwrap();
    d.pre_insert(typ, NodeIdx(0), None).unwrap();
    assert_eq!(
        d.pre_insert(a, NodeIdx(0), Some(typ)),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        d.pre_insert(a, NodeIdx(0), Some(comment)),
        Err(DomError::HierarchyRequestError)
    );
    d.pre_insert(a, NodeIdx(0), None).unwrap();
}
#[test]
fn fragments_validate_document_children_before_emptying() {
    let mut d = Dom::new();
    let f = d.create_document_fragment(NodeIdx(0)).unwrap();
    let a = element(&mut d, "a");
    let b = element(&mut d, "b");
    d.pre_insert(a, f, None).unwrap();
    d.pre_insert(b, f, None).unwrap();
    assert_eq!(
        d.pre_insert(f, NodeIdx(0), None),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, f, &[a, b]);
    d.pre_remove(b, f).unwrap();
    let t = d.alloc_unattached(NodeKind::Text { data: "x".into() });
    d.pre_insert(t, f, None).unwrap();
    assert_eq!(
        d.pre_insert(f, NodeIdx(0), None),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, f, &[a, t]);
    d.pre_remove(t, f).unwrap();
    d.pre_insert(f, NodeIdx(0), None).unwrap();
    assert_links(&d, f, &[]);
    assert_links(&d, NodeIdx(0), &[a]);
}
#[test]
fn empty_fragment_does_not_queue_records_or_change_its_document() {
    let mut d = Dom::new();
    let second = d.create_document();
    let f = d.create_document_fragment(second).unwrap();
    d.pre_insert(f, NodeIdx(0), None).unwrap();
    assert_eq!(d.owner_document(f), Some(second));
    assert!(!d.mutation_microtask_pending());
}
#[test]
fn fragment_insertion_batches_both_lists() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let f = d.create_document_fragment(NodeIdx(0)).unwrap();
    let a = d.create_text(f, "a");
    let b = d.create_text(f, "b");
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        f,
        ObserveOptions {
            child_list: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.pre_insert(f, p, None).unwrap();
    assert_links(&d, p, &[a, b]);
    assert_links(&d, f, &[]);
    let records = d.take_mutation_records(o);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].removed_nodes, vec![a, b]);
    assert_eq!(records[1].added_nodes, vec![a, b]);
}
#[test]
fn move_before_self_and_adjacent_sibling_preserves_links() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let b = d.create_text(p, "b");
    let c = d.create_text(p, "c");
    d.pre_insert(b, p, Some(b)).unwrap();
    assert_links(&d, p, &[a, b, c]);
    d.pre_insert(c, p, Some(a)).unwrap();
    assert_links(&d, p, &[c, a, b]);
    d.pre_insert(c, p, None).unwrap();
    assert_links(&d, p, &[a, b, c]);
}
#[test]
fn removal_repairs_first_middle_last_and_only_child() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let b = d.create_text(p, "b");
    let c = d.create_text(p, "c");
    d.pre_remove(b, p).unwrap();
    assert_links(&d, p, &[a, c]);
    d.pre_remove(a, p).unwrap();
    assert_links(&d, p, &[c]);
    d.pre_remove(c, p).unwrap();
    assert_links(&d, p, &[]);
    for n in [a, b, c] {
        assert_eq!(d.get(n).parent, None);
        assert_eq!(d.get(n).prev_sibling, None);
        assert_eq!(d.get(n).next_sibling, None);
    }
}
#[test]
fn replace_checks_old_child_membership_before_new_node_hierarchy() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "parent");
    let detached = element(&mut dom, "detached");
    let invalid_for_element = dt(&mut dom);
    assert_eq!(dom.replace(detached, invalid_for_element, parent), Err(DomError::NotFoundError));
    assert_eq!(dom.get(detached).parent, None);
}

#[test]
fn replace_self_keeps_identity_and_records_adoption_removal() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(d.replace(a, a, p), Ok(a));
    assert_links(&d, p, &[a]);
    let records = d.take_mutation_records(o);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].removed_nodes, vec![a]);
    assert_eq!(records[1].added_nodes, vec![a]);
}
#[test]
fn replace_with_next_sibling_and_fragment() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let b = d.create_text(p, "b");
    let c = d.create_text(p, "c");
    d.replace(a, b, p).unwrap();
    assert_links(&d, p, &[b, c]);
    let f = d.create_document_fragment(NodeIdx(0)).unwrap();
    let x = d.create_text(f, "x");
    let y = d.create_text(f, "y");
    d.replace(b, f, p).unwrap();
    assert_links(&d, p, &[x, y, c]);
    assert_links(&d, f, &[]);
}
#[test]
fn replace_document_element_and_doctype_respects_exclusion() {
    let mut d = Dom::new();
    let typ = dt(&mut d);
    let a = element(&mut d, "a");
    d.pre_insert(typ, NodeIdx(0), None).unwrap();
    d.pre_insert(a, NodeIdx(0), None).unwrap();
    let b = element(&mut d, "b");
    d.replace(a, b, NodeIdx(0)).unwrap();
    assert_links(&d, NodeIdx(0), &[typ, b]);
    let another = dt(&mut d);
    d.replace(typ, another, NodeIdx(0)).unwrap();
    assert_links(&d, NodeIdx(0), &[another, b]);
    assert_eq!(
        d.replace(another, a, NodeIdx(0)),
        Err(DomError::HierarchyRequestError)
    );
}
#[test]
fn replace_all_is_one_parent_record_and_preserves_fragment_record() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let b = d.create_text(p, "b");
    let f = d.create_document_fragment(NodeIdx(0)).unwrap();
    let x = d.create_text(f, "x");
    let o = d.create_mutation_observer(1);
    for n in [p, f] {
        d.observe(
            o,
            n,
            ObserveOptions {
                child_list: true,
                ..Default::default()
            },
        )
        .unwrap();
    }
    d.replace_all(p, Some(f)).unwrap();
    assert_links(&d, p, &[x]);
    let r = d.take_mutation_records(o);
    assert_eq!(r.len(), 2);
    assert_eq!(r[1].removed_nodes, vec![a, b]);
    assert_eq!(r[1].added_nodes, vec![x]);
}
#[test]
fn adoption_preserves_subtree_ids_and_changes_every_owner() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let c = d.create_text(p, "x");
    let other = d.create_document();
    d.adopt(p, other).unwrap();
    assert_eq!(d.get(p).parent, None);
    assert_links(&d, p, &[c]);
    assert_eq!(d.owner_document(p), Some(other));
    assert_eq!(d.owner_document(c), Some(other));
    assert_eq!(d.adopt(NodeIdx(0), other), Err(DomError::NotSupportedError));
    assert_eq!(d.owner_document(other), None);
    d.pre_insert(p, other, None).unwrap();
    assert!(d.is_connected(c));
}
#[test]
fn insertion_adopts_children_but_not_the_fragment_itself() {
    let mut d = Dom::new();
    let other = d.create_document();
    let f = d.create_document_fragment(other).unwrap();
    let a = element(&mut d, "a");
    d.pre_insert(a, f, None).unwrap();
    d.pre_insert(f, NodeIdx(0), None).unwrap();
    assert_eq!(d.owner_document(f), Some(other));
    assert_eq!(d.owner_document(a), Some(NodeIdx(0)));
}
#[test]
fn compare_position_and_tree_predicates() {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let a = d.create_text(p, "a");
    let b = d.create_text(p, "b");
    let detached = element(&mut d, "x");
    use DocumentPosition as P;
    assert_eq!(d.tree_order(NodeIdx(0)), vec![NodeIdx(0), p, a, b]);
    assert!(d.contains(p, Some(p)));
    assert!(!d.contains(p, None));
    assert!(d.is_ancestor(p, a));
    assert!(d.is_descendant(a, p));
    assert!(d.is_sibling(a, b));
    assert!(!d.is_sibling(a, a));
    assert_eq!(
        d.compare_document_position(p, a),
        P::FOLLOWING | P::CONTAINED_BY
    );
    assert_eq!(
        d.compare_document_position(a, p),
        P::PRECEDING | P::CONTAINS
    );
    assert_eq!(d.compare_document_position(a, b), P::FOLLOWING);
    assert_eq!(d.compare_document_position(b, a), P::PRECEDING);
    assert_eq!(d.compare_document_position(a, a), 0);
    assert_eq!(
        d.compare_document_position(a, detached),
        P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC | P::FOLLOWING
    );
    assert_eq!(
        d.compare_document_position(detached, a),
        P::DISCONNECTED | P::IMPLEMENTATION_SPECIFIC | P::PRECEDING
    );
}
#[test]
fn invalid_handles_return_error_at_checked_boundaries() {
    let mut d = Dom::new();
    assert_eq!(
        d.pre_insert(NodeIdx(99), NodeIdx(0), None),
        Err(DomError::InvalidNode)
    );
}
#[test]
fn invalid_replace_is_atomic_including_observer_queues() {
    let mut d = Dom::new();
    let a = element(&mut d, "a");
    d.pre_insert(a, NodeIdx(0), None).unwrap();
    let text = d.alloc_unattached(NodeKind::Text { data: "bad".into() });
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        NodeIdx(0),
        ObserveOptions {
            child_list: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        d.replace(a, text, NodeIdx(0)),
        Err(DomError::HierarchyRequestError)
    );
    assert_links(&d, NodeIdx(0), &[a]);
    assert!(d.take_mutation_records(o).is_empty());
}
#[test]
fn comments_and_processing_instructions_can_surround_document_root() {
    let mut d = Dom::new();
    let c = d.alloc_unattached(NodeKind::Comment {
        data: "before".into(),
    });
    let pi = d.alloc_unattached(NodeKind::ProcessingInstruction {
        target: "xml-stylesheet".into(),
        data: "".into(),
    });
    let root = element(&mut d, "root");
    d.pre_insert(root, NodeIdx(0), None).unwrap();
    d.pre_insert(c, NodeIdx(0), Some(root)).unwrap();
    d.pre_insert(pi, NodeIdx(0), None).unwrap();
    assert_links(&d, NodeIdx(0), &[c, root, pi]);
}
