use super::*;
fn fixture() -> (Dom, NodeIdx, NodeIdx) {
    let mut d = Dom::new();
    let p = d.create_element(NodeIdx(0), "p");
    let t = d.create_text(p, "old");
    (d, p, t)
}
#[test]
fn image_relevant_attribute_mutations_are_queued_including_same_src() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let img = dom.create_element(root, "img");
    let source = dom.create_element(root, "source");
    dom.set_attribute(img, "src", "photo.png").unwrap();
    dom.set_attribute(img, "src", "photo.png").unwrap();
    dom.set_attribute(img, "alt", "label").unwrap();
    dom.set_attribute(source, "media", "screen").unwrap();
    dom.remove_attribute(source, "media").unwrap();
    let events = dom.take_image_relevant_mutations();
    assert_eq!(events.iter().map(|event| (event.node, event.attribute.as_str())).collect::<Vec<_>>(),
        [(img, "src"), (img, "src"), (source, "media"), (source, "media")]);
    assert!(dom.take_image_relevant_mutations().is_empty());
}
#[test]
fn delivery_signal_only_changes_for_interested_records_and_rearms_after_checkpoint() {
    let (mut d, p, _) = fixture();
    d.set_attribute(p, "unobserved", "1").unwrap();
    assert!(!d.mutation_microtask_pending());

    let o = d.create_mutation_observer(7);
    d.observe(
        o,
        p,
        ObserveOptions {
            attribute_filter: Some(vec!["watched".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    let second_observer = d.create_mutation_observer(8);
    d.observe(
        second_observer,
        p,
        ObserveOptions {
            attribute_filter: Some(vec!["watched".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute(p, "ignored", "2").unwrap();
    assert!(!d.mutation_microtask_pending());
    d.set_attribute(p, "watched", "3").unwrap();
    assert!(d.mutation_microtask_pending());
    d.set_attribute(p, "watched", "4").unwrap();
    assert!(d.mutation_microtask_pending());

    let mut first = d.begin_mutation_checkpoint();
    assert!(!d.mutation_microtask_pending());
    assert_eq!(d.next_mutation_delivery(&mut first).unwrap().records.len(), 2);
    assert_eq!(d.next_mutation_delivery(&mut first).unwrap().records.len(), 2);
    assert!(d.next_mutation_delivery(&mut first).is_none());
    d.set_attribute(p, "watched", "5").unwrap();
    assert!(d.mutation_microtask_pending());
    let mut second = d.begin_mutation_checkpoint();
    assert_eq!(d.next_mutation_delivery(&mut second).unwrap().records.len(), 1);
    assert_eq!(d.next_mutation_delivery(&mut second).unwrap().records.len(), 1);
}
#[test]
fn options_validate_presence_not_only_truth() {
    let (mut d, p, _) = fixture();
    let o = d.create_mutation_observer(1);
    for invalid in [
        ObserveOptions::default(),
        ObserveOptions {
            attributes: Some(false),
            attribute_old_value: Some(true),
            ..Default::default()
        },
        ObserveOptions {
            attributes: Some(false),
            attribute_filter: Some(vec![]),
            ..Default::default()
        },
        ObserveOptions {
            character_data: Some(false),
            character_data_old_value: Some(true),
            ..Default::default()
        },
    ] {
        assert_eq!(d.observe(o, p, invalid), Err(DomError::TypeError));
    }
    d.observe(
        o,
        p,
        ObserveOptions {
            attribute_old_value: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute(p, "x", "1").unwrap();
    assert_eq!(d.take_mutation_records(o).len(), 1);
}
#[test]
fn checkpoint_batches_records_in_mutation_order() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(42);
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            attributes: Some(true),
            character_data: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute(p, "x", "1").unwrap();
    d.set_character_data(t, "new".into()).unwrap();
    let n = d.create_text(p, "tail");
    assert!(d.mutation_microtask_pending());
    let mut checkpoint = d.begin_mutation_checkpoint();
    assert!(!d.mutation_microtask_pending());
    let delivery = d.next_mutation_delivery(&mut checkpoint).unwrap();
    assert_eq!(delivery.callback, 42);
    assert_eq!(
        delivery.records.iter().map(|r| r.kind).collect::<Vec<_>>(),
        vec![
            MutationType::Attributes,
            MutationType::CharacterData,
            MutationType::ChildList
        ]
    );
    assert_eq!(delivery.records[2].added_nodes, vec![n]);
    assert!(d.next_mutation_delivery(&mut checkpoint).is_none());
}
#[test]
fn attribute_filter_ignores_other_names_and_non_null_namespaces() {
    let (mut d, p, _) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            attribute_filter: Some(vec!["x".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute(p, "y", "1").unwrap();
    d.set_attribute_ns(p, Some("urn:test"), "x", "1").unwrap();
    d.set_attribute(p, "x", "2").unwrap();
    let r = d.take_mutation_records(o);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].attribute_name.as_deref(), Some("x"));
    assert_eq!(r[0].attribute_namespace, None);
}
#[test]
fn namespaced_attribute_old_value_and_removal() {
    let (mut d, p, _) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            attribute_old_value: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute_ns(p, Some("urn:a"), "x", "first").unwrap();
    d.set_attribute_ns(p, Some("urn:b"), "x", "other").unwrap();
    d.set_attribute_ns(p, Some("urn:a"), "x", "second").unwrap();
    d.remove_attribute_ns(p, Some("urn:a"), "x").unwrap();
    let r = d.take_mutation_records(o);
    assert_eq!(r.len(), 4);
    assert_eq!(r[2].old_value.as_deref(), Some("first"));
    assert_eq!(r[3].old_value.as_deref(), Some("second"));
    assert_eq!(r[3].attribute_namespace.as_deref(), Some("urn:a"));
    assert_eq!(d.get_attribute_ns(p, Some("urn:b"), "x"), Some("other"));
}
#[test]
fn overlapping_registrations_deliver_once_with_old_value_union() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            character_data_old_value: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.observe(
        o,
        t,
        ObserveOptions {
            character_data: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_character_data(t, "new".into()).unwrap();
    let r = d.take_mutation_records(o);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].old_value.as_deref(), Some("old"));
}
#[test]
fn old_values_are_per_observer_and_unchanged_set_still_records() {
    let (mut d, p, _) = fixture();
    d.set_attribute(p, "x", "old").unwrap();
    let a = d.create_mutation_observer(1);
    let b = d.create_mutation_observer(2);
    d.observe(
        a,
        p,
        ObserveOptions {
            attribute_old_value: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.observe(
        b,
        p,
        ObserveOptions {
            attributes: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_attribute(p, "x", "old").unwrap();
    d.remove_attribute(p, "absent").unwrap();
    assert_eq!(
        d.take_mutation_records(a)[0].old_value.as_deref(),
        Some("old")
    );
    let r = d.take_mutation_records(b);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].old_value, None);
}
#[test]
fn subtree_false_does_not_observe_descendant_changes() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            character_data: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_character_data(t, "new".into()).unwrap();
    assert!(d.take_mutation_records(o).is_empty());
}
#[test]
fn detached_subtree_is_observed_until_its_delivery() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            character_data_old_value: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.pre_remove(t, p).unwrap();
    d.set_character_data(t, "detached".into()).unwrap();
    let mut checkpoint = d.begin_mutation_checkpoint();
    let delivery = d.next_mutation_delivery(&mut checkpoint).unwrap();
    assert_eq!(delivery.records.len(), 2);
    assert_eq!(delivery.records[1].old_value.as_deref(), Some("old"));
    d.set_character_data(t, "after".into()).unwrap();
    assert!(d.take_mutation_records(o).is_empty());
}
#[test]
fn reobserve_removes_transients_of_replaced_registration() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    let opts = ObserveOptions {
        child_list: true,
        character_data: Some(true),
        subtree: true,
        ..Default::default()
    };
    d.observe(o, p, opts.clone()).unwrap();
    d.pre_remove(t, p).unwrap();
    d.observe(o, p, opts).unwrap();
    d.set_character_data(t, "after".into()).unwrap();
    assert_eq!(d.take_mutation_records(o).len(), 1);
}
#[test]
fn disconnect_discards_records_and_transients() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            character_data: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.pre_remove(t, p).unwrap();
    d.disconnect_observer(o);
    d.set_character_data(t, "new".into()).unwrap();
    let mut c = d.begin_mutation_checkpoint();
    assert!(d.next_mutation_delivery(&mut c).is_none());
}
#[test]
fn take_records_drains_without_cancelling_scheduled_cleanup() {
    let (mut d, p, t) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            child_list: true,
            character_data: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.pre_remove(t, p).unwrap();
    assert_eq!(d.take_mutation_records(o).len(), 1);
    assert!(d.mutation_microtask_pending());
    let mut c = d.begin_mutation_checkpoint();
    assert!(d.next_mutation_delivery(&mut c).is_none());
    d.set_character_data(t, "new".into()).unwrap();
    assert!(d.take_mutation_records(o).is_empty());
}
#[test]
fn callback_mutations_reach_later_observer_in_same_checkpoint() {
    let (mut d, p, _) = fixture();
    let a = d.create_mutation_observer(1);
    let b = d.create_mutation_observer(2);
    for o in [a, b] {
        d.observe(
            o,
            p,
            ObserveOptions {
                attributes: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    }
    d.set_attribute(p, "x", "1").unwrap();
    let mut c = d.begin_mutation_checkpoint();
    assert_eq!(d.next_mutation_delivery(&mut c).unwrap().observer, a);
    d.set_attribute(p, "y", "2").unwrap();
    let second = d.next_mutation_delivery(&mut c).unwrap();
    assert_eq!(second.observer, b);
    assert_eq!(second.records.len(), 2);
    assert!(d.mutation_microtask_pending());
    assert!(d.next_mutation_delivery(&mut c).is_none());
    let mut c = d.begin_mutation_checkpoint();
    let delivery = d.next_mutation_delivery(&mut c).unwrap();
    assert_eq!(delivery.observer, a);
    assert_eq!(delivery.records.len(), 1);
    assert!(d.next_mutation_delivery(&mut c).is_none());
}
#[test]
fn pending_observer_order_follows_first_interested_registration() {
    let (mut d, p, t) = fixture();
    let a = d.create_mutation_observer(1);
    let b = d.create_mutation_observer(2);
    d.observe(
        a,
        p,
        ObserveOptions {
            character_data: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    d.observe(
        b,
        t,
        ObserveOptions {
            character_data: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    d.set_character_data(t, "new".into()).unwrap();
    let mut c = d.begin_mutation_checkpoint();
    assert_eq!(d.next_mutation_delivery(&mut c).unwrap().observer, b);
    assert_eq!(d.next_mutation_delivery(&mut c).unwrap().observer, a);
}
#[test]
fn tree_records_capture_previous_and_next_siblings() {
    let (mut d, p, a) = fixture();
    let b = d.create_text(p, "b");
    let c = d.create_text(p, "c");
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
    d.pre_remove(b, p).unwrap();
    let r = d.take_mutation_records(o);
    assert_eq!(r[0].previous_sibling, Some(a));
    assert_eq!(r[0].next_sibling, Some(c));
}
#[test]
fn comments_and_processing_instructions_are_character_data() {
    let (mut d, p, _) = fixture();
    let o = d.create_mutation_observer(1);
    d.observe(
        o,
        p,
        ObserveOptions {
            character_data_old_value: Some(true),
            subtree: true,
            ..Default::default()
        },
    )
    .unwrap();
    for kind in [
        NodeKind::Comment {
            data: "before".into(),
        },
        NodeKind::ProcessingInstruction {
            target: "xml".into(),
            data: "before".into(),
        },
    ] {
        let n = d.append_child(p, kind);
        d.set_character_data(n, "after".into()).unwrap();
    }
    let r = d.take_mutation_records(o);
    assert_eq!(r.len(), 2);
    assert!(r.iter().all(|r| r.old_value.as_deref() == Some("before")));
}

#[test]
fn bulk_parser_attribute_updates_preserve_supplied_order() {
    let (mut d, p, _) = fixture();
    d.set_attrs(p, vec![("a".into(), "1".into()), ("b".into(), "2".into())]);
    let reversed = vec![("b".into(), "3".into()), ("a".into(), "4".into())];
    d.set_attrs(p, reversed.clone());
    assert_eq!(d.attrs_of(p).unwrap(), reversed);
}
