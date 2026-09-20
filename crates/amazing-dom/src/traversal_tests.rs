use super::*;
fn setup() -> (Dom, NodeIdx, NodeIdx, NodeIdx, NodeIdx, NodeIdx) {
    let mut d = Dom::new();
    let root = d.create_element(NodeIdx(0), "root");
    let a = d.create_element(root, "a");
    let text = d.create_text(a, "t");
    let b = d.create_element(root, "b");
    let c = d.create_element(b, "c");
    (d, root, a, text, b, c)
}
fn run(
    d: &mut Dom,
    id: impl Into<TraversalId>,
    direction: Traverse,
    filter: impl Fn(NodeIdx) -> u16,
) -> Option<NodeIdx> {
    let mut step = d.begin_traversal(id, direction).unwrap();
    loop {
        match step {
            TraversalStep::Done(n) => return n,
            TraversalStep::Filter {
                traversal, node, ..
            } => step = d.resume_traversal(traversal, Ok(filter(node))).unwrap(),
        }
    }
}
#[test]
fn iterator_visits_root_and_reverses_pointer() {
    let (mut d, root, a, text, b, c) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, None)
        .unwrap();
    for n in [root, a, text, b, c] {
        assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(n));
    }
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), None);
    for n in [c, b, text, a, root] {
        assert_eq!(run(&mut d, i, Traverse::Previous, |_| 1), Some(n));
    }
    assert_eq!(run(&mut d, i, Traverse::Previous, |_| 1), None);
}
#[test]
fn iterator_reject_does_not_prune_children() {
    let (mut d, root, a, text, _, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, Some(4))
        .unwrap();
    assert_eq!(
        run(&mut d, i, Traverse::Next, |n| if n == root || n == a {
            2
        } else {
            1
        }),
        Some(text)
    );
}
#[test]
fn what_to_show_skips_callback_but_descends() {
    let (mut d, root, _, text, _, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_TEXT, Some(4))
        .unwrap();
    assert_eq!(
        d.begin_traversal(i, Traverse::Next),
        Ok(TraversalStep::Filter {
            traversal: i.into(),
            callback: 4,
            node: text
        })
    );
    assert_eq!(
        d.resume_traversal(i.into(), Ok(1)),
        Ok(TraversalStep::Done(Some(text)))
    );
}
#[test]
fn removal_adjusts_before_pointer_to_following_node() {
    let (mut d, root, a, text, b, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, None)
        .unwrap();
    for _ in 0..3 {
        run(&mut d, i, Traverse::Next, |_| 1);
    }
    run(&mut d, i, Traverse::Previous, |_| 1);
    assert_eq!(
        d.iterator_reference(i).unwrap(),
        NodePointer {
            node: text,
            before: true
        }
    );
    d.detach(a);
    assert_eq!(
        d.iterator_reference(i).unwrap(),
        NodePointer {
            node: b,
            before: true
        }
    );
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(b));
}
#[test]
fn removal_adjusts_after_pointer_to_preceding_node() {
    let (mut d, root, a, _, _, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, None)
        .unwrap();
    run(&mut d, i, Traverse::Next, |_| 1);
    run(&mut d, i, Traverse::Next, |_| 1);
    d.detach(a);
    assert_eq!(
        d.iterator_reference(i).unwrap(),
        NodePointer {
            node: root,
            before: false
        }
    );
}
#[test]
fn removing_root_or_ancestor_keeps_iterator_inside_detached_tree() {
    let (mut d, root, a, _, _, _) = setup();
    let i = d
        .create_node_iterator(a, NodeFilter::SHOW_ALL, None)
        .unwrap();
    let reference = d.iterator_reference(i).unwrap();
    d.detach(root);
    assert_eq!(d.iterator_reference(i).unwrap(), reference);
}
#[test]
fn filter_removal_adjusts_candidate_but_returns_original_node() {
    let (mut d, root, a, _, b, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    run(&mut d, i, Traverse::Next, |_| 1);
    assert!(
        matches!(d.begin_traversal(i, Traverse::Next).unwrap(), TraversalStep::Filter { node, .. } if node == a)
    );
    d.detach(a);
    assert_eq!(
        d.resume_traversal(i.into(), Ok(1)),
        Ok(TraversalStep::Done(Some(a)))
    );
    assert_eq!(
        d.iterator_reference(i).unwrap(),
        NodePointer {
            node: root,
            before: false
        }
    );
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(b));
}
#[test]
fn callback_errors_clear_active_state_and_preserve_reference() {
    let (mut d, root, _, _, _, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    d.begin_traversal(i, Traverse::Next).unwrap();
    assert_eq!(
        d.begin_traversal(i, Traverse::Next),
        Err(DomError::InvalidStateError)
    );
    assert_eq!(
        d.resume_traversal(i.into(), Err(DomError::TypeError)),
        Err(DomError::TypeError)
    );
    assert!(d.iterator_reference(i).unwrap().before);
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(root));
}
#[test]
fn walker_reject_prunes_and_skip_descends() {
    let (mut d, root, a, text, b, _) = setup();
    let w = d
        .create_tree_walker(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    assert_eq!(
        run(&mut d, w, Traverse::Next, |n| if n == a { 2 } else { 1 }),
        Some(b)
    );
    d.walker_set_current(w, root).unwrap();
    assert_eq!(
        run(&mut d, w, Traverse::Next, |n| if n == a { 3 } else { 1 }),
        Some(text)
    );
}
#[test]
fn walker_children_siblings_and_parents_follow_filtered_tree() {
    let (mut d, root, a, text, b, c) = setup();
    let w = d
        .create_tree_walker(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    let f = |n| if n == a || n == b { 3 } else { 1 };
    assert_eq!(run(&mut d, w, Traverse::FirstChild, f), Some(text));
    assert_eq!(run(&mut d, w, Traverse::NextSibling, f), Some(c));
    assert_eq!(run(&mut d, w, Traverse::PreviousSibling, f), Some(text));
    assert_eq!(run(&mut d, w, Traverse::Parent, f), Some(root));
    assert_eq!(run(&mut d, w, Traverse::LastChild, f), Some(c));
    assert_eq!(run(&mut d, w, Traverse::NextSibling, f), None);
}
#[test]
fn walker_previous_descends_before_accepting_sibling() {
    let (mut d, root, a, text, b, _) = setup();
    let w = d
        .create_tree_walker(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    d.walker_set_current(w, b).unwrap();
    assert_eq!(run(&mut d, w, Traverse::Previous, |_| 1), Some(text));
    assert_eq!(run(&mut d, w, Traverse::Previous, |_| 1), Some(a));
    assert_eq!(run(&mut d, w, Traverse::Previous, |_| 1), Some(root));
    assert_eq!(run(&mut d, w, Traverse::Previous, |_| 1), None);
}
#[test]
fn walker_current_is_unrestricted_and_not_relocated_by_removal() {
    let (mut d, root, a, text, _, _) = setup();
    let w = d
        .create_tree_walker(root, NodeFilter::SHOW_ALL, None)
        .unwrap();
    d.walker_set_current(w, text).unwrap();
    d.detach(a);
    assert_eq!(d.walker_current(w), Ok(text));
    assert_eq!(run(&mut d, w, Traverse::Parent, |_| 1), Some(a));
    d.walker_set_current(w, NodeIdx(0)).unwrap();
    assert_eq!(d.walker_current(w), Ok(NodeIdx(0)));
}
#[test]
fn legacy_detach_is_noop_and_gc_release_is_explicit() {
    let (mut d, root, _, _, _, _) = setup();
    let i = d.create_node_iterator(root, 0, None).unwrap();
    d.iterator_detach(i).unwrap();
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), None);
    d.release_traversal(i);
    assert_eq!(d.iterator_reference(i), Err(DomError::InvalidStateError));
}
#[test]
fn nested_traversal_without_a_filter_candidate_returns_null() {
    let (mut d, root, a, _, _, _) = setup();
    let w = d
        .create_tree_walker(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    assert!(
        matches!(d.begin_traversal(w, Traverse::Next), Ok(TraversalStep::Filter { node, .. }) if node == a)
    );
    assert_eq!(
        d.begin_traversal(w, Traverse::Parent),
        Ok(TraversalStep::Done(None))
    );
    assert_eq!(
        d.begin_traversal(w, Traverse::Next),
        Err(DomError::InvalidStateError)
    );
    assert_eq!(
        d.resume_traversal(w.into(), Ok(1)),
        Ok(TraversalStep::Done(Some(a)))
    );
}
#[test]
fn reverse_filter_removal_moves_candidate_before_next_node() {
    let (mut d, root, a, text, b, _) = setup();
    let i = d
        .create_node_iterator(root, NodeFilter::SHOW_ALL, Some(1))
        .unwrap();
    for _ in 0..3 {
        run(&mut d, i, Traverse::Next, |_| 1);
    }
    assert!(
        matches!(d.begin_traversal(i, Traverse::Previous), Ok(TraversalStep::Filter { node, .. }) if node == text)
    );
    d.detach(a);
    assert_eq!(
        d.resume_traversal(i.into(), Ok(2)),
        Ok(TraversalStep::Filter {
            traversal: i.into(),
            callback: 1,
            node: root
        })
    );
    assert_eq!(
        d.resume_traversal(i.into(), Ok(1)),
        Ok(TraversalStep::Done(Some(root)))
    );
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(root));
    assert_eq!(run(&mut d, i, Traverse::Next, |_| 1), Some(b));
}
