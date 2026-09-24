use super::*;
use crate::mutation::ObserveOptions;

fn element(dom: &mut Dom, document: NodeIdx) -> NodeIdx {
    dom.create_element_ns_in(document, Some("http://www.w3.org/1999/xhtml"), "div")
        .unwrap()
}
fn children(dom: &Dom, parent: NodeIdx) -> Vec<NodeIdx> {
    dom.children(parent).collect()
}
fn nodes(ids: &[NodeIdx]) -> Vec<NodeOrString> {
    ids.iter().copied().map(NodeOrString::Node).collect()
}

#[test]
fn element_navigation_skips_non_elements_and_tracks_tree_mutations() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let text = dom.create_text(parent, "x");
    let svg = dom.create_element_ns_in(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "svg").unwrap();
    dom.pre_insert(svg, parent, None).unwrap();
    let middle = dom.create_text(parent, "y");
    let html = dom.create_element(parent, "span");
    assert_eq!(dom.first_element_child(parent), Some(svg));
    assert_eq!(dom.last_element_child(parent), Some(html));
    assert_eq!(dom.child_element_count(parent), 2);
    assert_eq!(dom.previous_element_sibling(html), Some(svg));
    assert_eq!(dom.next_element_sibling(svg), Some(html));
    assert_eq!(dom.previous_element_sibling(middle), Some(svg));
    assert_eq!(dom.next_element_sibling(text), Some(svg));
    assert_eq!(dom.previous_element_sibling(svg), None);
    assert_eq!(dom.next_element_sibling(html), None);
    dom.detach(svg);
    assert_eq!(dom.child_element_count(parent), 1);
    assert_eq!(dom.first_element_child(parent), Some(html));
    assert_eq!(dom.previous_element_sibling(middle), None);
    dom.detach(html);
    assert_eq!(dom.child_element_count(parent), 0);
    assert_eq!(dom.first_element_child(parent), None);
    assert_eq!(dom.last_element_child(parent), None);
}

#[test]
fn append_and_prepend_move_duplicates_and_preserve_identity() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let a = element(&mut dom, NodeIdx(0));
    let b = element(&mut dom, NodeIdx(0));
    dom.append_nodes(parent, &nodes(&[a, b, a])).unwrap();
    assert_eq!(children(&dom, parent), [b, a]);
    dom.prepend_nodes(parent, &nodes(&[a, b])).unwrap();
    assert_eq!(children(&dom, parent), [a, b]);
}

#[test]
fn child_mixins_work_for_comment_text_element_and_doctype() {
    for kind in [
        NodeKind::Comment {
            data: "comment".into(),
        },
        NodeKind::Text {
            data: "text".into(),
        },
        NodeKind::Element {
            name: "a".into(),
            attrs: vec![],
            namespace: crate::Namespace::Html,
        },
        NodeKind::DocumentType {
            name: "html".into(),
            public_id: String::new(),
            system_id: String::new(),
        },
    ] {
        let mut dom = Dom::new();
        let is_doctype = matches!(kind, NodeKind::DocumentType { .. });
        let parent = if is_doctype {
            NodeIdx(0)
        } else {
            element(&mut dom, NodeIdx(0))
        };
        let target = dom.create_node_in(NodeIdx(0), kind).unwrap();
        let a = dom
            .create_node_in(NodeIdx(0), NodeKind::Comment { data: "a".into() })
            .unwrap();
        let b = dom
            .create_node_in(NodeIdx(0), NodeKind::Comment { data: "b".into() })
            .unwrap();
        dom.pre_insert(target, parent, None).unwrap();
        dom.before_nodes(target, &nodes(&[a])).unwrap();
        dom.after_nodes(target, &nodes(&[b])).unwrap();
        assert_eq!(children(&dom, parent), [a, target, b]);
        dom.remove(target).unwrap();
        assert_eq!(children(&dom, parent), [a, b]);
        dom.remove(target).unwrap();
    }
}

#[test]
fn before_and_after_skip_siblings_in_arguments_before_conversion() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let a = element(&mut dom, NodeIdx(0));
    let b = element(&mut dom, NodeIdx(0));
    let c = element(&mut dom, NodeIdx(0));
    let d = element(&mut dom, NodeIdx(0));
    dom.append_nodes(parent, &nodes(&[a, b, c, d])).unwrap();
    dom.after_nodes(b, &nodes(&[c, b])).unwrap();
    assert_eq!(children(&dom, parent), [a, c, b, d]);
    dom.before_nodes(b, &nodes(&[c, a, b])).unwrap();
    assert_eq!(children(&dom, parent), [c, a, b, d]);
}

#[test]
fn replace_with_target_in_arguments_uses_saved_viable_sibling() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let a = element(&mut dom, NodeIdx(0));
    let b = element(&mut dom, NodeIdx(0));
    let c = element(&mut dom, NodeIdx(0));
    dom.append_nodes(parent, &nodes(&[a, b, c])).unwrap();
    dom.replace_with(a, &nodes(&[b, a, b])).unwrap();
    assert_eq!(children(&dom, parent), [a, b, c]);
    dom.replace_with(a, &[]).unwrap();
    assert_eq!(children(&dom, parent), [b, c]);
}

#[test]
fn detached_child_methods_do_not_convert_or_move_arguments() {
    let mut dom = Dom::new();
    let detached = element(&mut dom, NodeIdx(0));
    let parent = element(&mut dom, NodeIdx(0));
    let child = element(&mut dom, NodeIdx(0));
    dom.pre_insert(child, parent, None).unwrap();
    let count = dom.node_count();
    let values = [
        NodeOrString::String("new text".into()),
        NodeOrString::Node(child),
    ];
    dom.before_nodes(detached, &values).unwrap();
    dom.after_nodes(detached, &values).unwrap();
    dom.replace_with(detached, &values).unwrap();
    assert_eq!(dom.node_count(), count);
    assert_eq!(children(&dom, parent), [child]);
}

#[test]
fn strings_are_distinct_text_nodes_and_cross_document_nodes_are_adopted() {
    let mut dom = Dom::new();
    let other = dom.create_document();
    let parent = element(&mut dom, other);
    let child = element(&mut dom, NodeIdx(0));
    dom.append_nodes(
        parent,
        &[
            NodeOrString::String("".into()),
            NodeOrString::Node(child),
            NodeOrString::String("x".into()),
        ],
    )
    .unwrap();
    let result = children(&dom, parent);
    assert_eq!(result.len(), 3);
    assert_eq!(result[1], child);
    assert_eq!(dom.character_data(result[0]), Some(""));
    assert_eq!(dom.character_data(result[2]), Some("x"));
    for id in result {
        assert_eq!(dom.owner_document(id), Some(other));
    }
}

#[test]
fn failed_final_insertion_keeps_observable_conversion_moves() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let a = element(&mut dom, NodeIdx(0));
    let b = element(&mut dom, NodeIdx(0));
    dom.append_nodes(parent, &nodes(&[a, b])).unwrap();
    // Conversion moves both children into a fragment; a document cannot then
    // accept that fragment's two element children. There is no rollback.
    assert_eq!(
        dom.append_nodes(NodeIdx(0), &nodes(&[a, b])),
        Err(DomError::HierarchyRequestError)
    );
    assert!(children(&dom, parent).is_empty());
    let fragment = dom.get(a).parent.unwrap();
    assert!(matches!(dom.get(fragment).kind, NodeKind::DocumentFragment));
    assert_eq!(children(&dom, fragment), [a, b]);
    assert!(children(&dom, NodeIdx(0)).is_empty());
}

#[test]
fn replace_children_emits_one_combined_record_and_empty_call_removes_all() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let old = element(&mut dom, NodeIdx(0));
    let a = element(&mut dom, NodeIdx(0));
    let b = element(&mut dom, NodeIdx(0));
    dom.pre_insert(old, parent, None).unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(
        observer,
        parent,
        ObserveOptions {
            child_list: true,
            ..Default::default()
        },
    )
    .unwrap();
    dom.replace_children(parent, &nodes(&[a, b])).unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].removed_nodes, [old]);
    assert_eq!(records[0].added_nodes, [a, b]);
    dom.replace_children(parent, &[]).unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].removed_nodes, [a, b]);
    assert!(records[0].added_nodes.is_empty());
}

#[test]
fn invalid_receivers_and_single_node_cycles_do_not_mutate_tree() {
    let mut dom = Dom::new();
    let parent = element(&mut dom, NodeIdx(0));
    let child = element(&mut dom, NodeIdx(0));
    dom.pre_insert(child, parent, None).unwrap();
    assert_eq!(
        dom.append_nodes(child, &nodes(&[parent])),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(children(&dom, parent), [child]);
    let text = dom
        .create_node_in(
            NodeIdx(0),
            NodeKind::Text {
                data: String::new(),
            },
        )
        .unwrap();
    assert_eq!(dom.append_nodes(text, &[]), Err(DomError::TypeError));
    assert_eq!(dom.remove(NodeIdx(0)), Err(DomError::TypeError));
    assert_eq!(
        dom.before_nodes(NodeIdx(u32::MAX), &[]),
        Err(DomError::InvalidNode)
    );
}
