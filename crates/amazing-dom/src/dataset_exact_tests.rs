use crate::{Dom, DomError, NodeIdx};
use crate::mutation::ObserveOptions;
fn units(text: &str) -> Vec<u16> { text.encode_utf16().collect() }
fn fixture() -> (Dom, NodeIdx) {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    (dom, element)
}
#[test]
fn dataset_exact_mutation_custody_and_single_change() {
    let (mut dom, element) = fixture();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions {
        attributes: Some(true), attribute_old_value: Some(true), ..Default::default()
    }).unwrap();
    let first = vec![0xd800, 0, 0xdc00];
    let second = vec![0xdc00, 0xd800];
    dom.set_dataset_value(element, &units("value"), first.clone()).unwrap();
    dom.set_dataset_value(element, &units("value"), second.clone()).unwrap();
    dom.set_dataset_value(element, &units("value"), second.clone()).unwrap();
    dom.delete_dataset_value(element, &units("value")).unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 4);
    assert_eq!(records.iter().map(|r| r.old_value_utf16.clone()).collect::<Vec<_>>(),
        vec![None, Some(first), Some(second.clone()), Some(second)]);
    assert!(records.iter().all(|r| r.attribute_name.as_deref() == Some("data-value")));
}
#[test]
fn dataset_mutation_old_value_option_is_respected() {
    let (mut dom, element) = fixture();
    dom.set_dataset_value(element, &units("value"), vec![0xd800]).unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions { attributes: Some(true), ..Default::default() }).unwrap();
    dom.delete_dataset_value(element, &units("value")).unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].old_value, None);
    assert_eq!(records[0].old_value_utf16, None);
}
#[test]
fn dataset_clone_and_import_preserve_units_without_reactions() {
    let (mut dom, element) = fixture();
    let value = vec![0xd800, 0, 0xdc00];
    dom.set_dataset_value(element, &units("aB"), value.clone()).unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, NodeIdx(0), ObserveOptions { attributes: Some(true), subtree: true, ..Default::default() }).unwrap();
    let clone = dom.clone_node(element, true).unwrap();
    let imported = dom.import_node(NodeIdx(0), element, true).unwrap();
    for copy in [clone, imported] {
        assert_eq!(dom.dataset_value(copy, &units("aB")).unwrap(), Some(value.clone()));
        assert_eq!(dom.dataset_names(copy).unwrap(), vec![units("aB")]);
    }
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn attr_and_scalar_setters_update_live_dataset_and_exact_old_value() {
    let (mut dom, element) = fixture();
    dom.set_dataset_value(element, &units("value"), vec![0xd800]).unwrap();
    let attr = dom.get_attribute_node(element, None, "data-value").unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions { attribute_old_value: Some(true), ..Default::default() }).unwrap();
    dom.set_attr_value(attr, vec![0xdc00]).unwrap();
    assert_eq!(dom.dataset_value(element, &units("value")).unwrap(), Some(vec![0xdc00]));
    dom.set_attribute(element, "data-value", "scalar").unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].old_value_utf16, Some(vec![0xd800]));
    assert_eq!(records[1].old_value_utf16, Some(vec![0xdc00]));
    assert_eq!(dom.dataset_value(element, &units("value")).unwrap(), Some(units("scalar")));
}
#[test]
fn dataset_complete_ascii_conversion_and_unicode_boundaries() {
    let (mut dom, element) = fixture();
    for name in ["", "A", "AZ", "-A", "foo-Bar", "a9", "éÉ", "𝄞", "x_y", "x:y", "x.y"] {
        dom.set_dataset_value(element, &units(name), units(name)).unwrap();
        assert_eq!(dom.dataset_value(element, &units(name)).unwrap(), Some(units(name)));
    }
    for name in ["-a", "a-b", "--a"] {
        assert_eq!(dom.set_dataset_value(element, &units(name), vec![]), Err(DomError::SyntaxError));
    }
}
#[test]
fn dataset_foreign_elements_and_qualified_attribute_names() {
    let mut dom = Dom::new();
    for namespace in ["http://www.w3.org/1999/xhtml", "http://www.w3.org/2000/svg", "http://www.w3.org/1998/Math/MathML"] {
        let element = dom.create_element_ns_in(NodeIdx(0), Some(namespace), "x").unwrap();
        dom.set_attribute_expanded(element, Some("urn:custom"), None, "data-first", "visible").unwrap();
        dom.set_attribute_expanded(element, Some("urn:custom"), Some("p"), "data-second", "hidden").unwrap();
        dom.set_attribute(element, "data-UPPER", "hidden").unwrap();
        assert_eq!(dom.dataset_pairs(element).unwrap(), vec![(units("first"), units("visible"))]);
    }
}
#[test]
fn dataset_failures_are_atomic_and_do_not_notify() {
    let (mut dom, element) = fixture();
    dom.set_dataset_value(element, &units("kept"), units("old")).unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions { attributes: Some(true), ..Default::default() }).unwrap();
    for name in [vec![0xd800], units("bad key"), units("bad=key"), units("bad\0key")] {
        assert_eq!(dom.set_dataset_value(element, &name, vec![0xdc00]), Err(DomError::InvalidCharacterError));
    }
    assert_eq!(dom.dataset_pairs(element).unwrap(), vec![(units("kept"), units("old"))]);
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn dataset_wrong_kind_and_dead_node_do_not_mutate() {
    let (mut dom, element) = fixture();
    assert_eq!(dom.dataset_names(NodeIdx(0)), Err(DomError::TypeError));
    assert_eq!(dom.set_dataset_value(NodeIdx(0), &units("x"), vec![]), Err(DomError::NotSupportedError));
    dom.pre_remove(element, NodeIdx(0)).unwrap();
    dom.retire_unreachable_node(element).unwrap();
    assert_eq!(dom.dataset_names(element), Err(DomError::InvalidNode));
    assert_eq!(dom.set_dataset_value(element, &units("x"), vec![]), Err(DomError::InvalidNode));
}

#[test]
fn dataset_render_receipt_keeps_exact_old_and_new_tokens() {
    use crate::render_mutation::RenderMutationKind;
    let (mut dom, element) = fixture();
    let first = vec![0xd800];
    let second = vec![0xdc00];
    dom.set_dataset_value(element, &units("value"), first.clone()).unwrap();
    dom.take_render_mutations(dom.render_mutation_epoch());
    dom.set_dataset_value(element, &units("value"), second.clone()).unwrap();
    let changes = dom.take_render_mutations(dom.render_mutation_epoch());
    let values: Vec<_> = changes.into_iter().filter_map(|entry| match entry.kind {
        RenderMutationKind::Attribute { local_name, old_value, new_value, old_tokens, new_tokens, .. }
            if local_name == "data-value" => Some((old_value, new_value, old_tokens, new_tokens)),
        _ => None,
    }).collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, Some(first.clone()));
    assert_eq!(values[0].1, Some(second.clone()));
    assert_eq!(values[0].2.attributes[0].value, first);
    assert_eq!(values[0].3.attributes[0].value, second);
}

#[test]
fn replacing_attribute_node_preserves_exact_dataset_and_detached_old_attr() {
    let (mut dom, element) = fixture();
    dom.set_dataset_value(element, &units("value"), vec![0xd800]).unwrap();
    let old = dom.get_attribute_node(element, None, "data-value").unwrap();
    let replacement = dom.create_attribute_in(NodeIdx(0), "data-value").unwrap();
    dom.set_attr_value(replacement, vec![0xdc00]).unwrap();
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions { attribute_old_value: Some(true), ..Default::default() }).unwrap();
    assert_eq!(dom.set_attribute_node(element, replacement).unwrap(), Some(old));
    assert_eq!(dom.dataset_value(element, &units("value")).unwrap(), Some(vec![0xdc00]));
    assert_eq!(dom.attr_snapshot(old).unwrap().value, vec![0xd800]);
    assert_eq!(dom.attr_snapshot(old).unwrap().owner_element, None);
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].old_value_utf16, Some(vec![0xd800]));
}

#[test]
fn dataset_generation_fences_recycled_element_after_author_conversion() {
    let (mut dom, element) = fixture();
    let generation = dom.node_generation(element).unwrap();
    assert_eq!(dom.dataset_pairs_for_generation(element, generation).unwrap(), Vec::<(Vec<u16>, Vec<u16>)>::new());
    dom.pre_remove(element, NodeIdx(0)).unwrap();
    dom.retire_unreachable_node(element).unwrap();
    let replacement = dom.create_element(NodeIdx(0), "span");
    assert_eq!(replacement, element);
    assert_ne!(dom.node_generation(replacement), Some(generation));
    assert_eq!(dom.dataset_value_for_generation(element, generation, &units("x")), Err(DomError::InvalidStateError));
    assert_eq!(dom.set_dataset_value_for_generation(element, generation, &units("x"), units("bad")), Err(DomError::InvalidStateError));
    assert_eq!(dom.delete_dataset_value_for_generation(element, generation, &units("x")), Err(DomError::InvalidStateError));
    assert_eq!(dom.dataset_pairs_for_generation(element, generation), Err(DomError::InvalidStateError));
    assert_eq!(dom.dataset_names(replacement).unwrap(), Vec::<Vec<u16>>::new());
    let current = dom.node_generation(replacement).unwrap();
    dom.set_dataset_value_for_generation(replacement, current, &units("x"), units("ok")).unwrap();
    assert_eq!(dom.dataset_value_for_generation(replacement, current, &units("x")).unwrap(), Some(units("ok")));
}
