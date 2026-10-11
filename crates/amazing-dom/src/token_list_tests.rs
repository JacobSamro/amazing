use crate::{Dom, DomError, NodeIdx};
use crate::mutation::ObserveOptions;
use super::{TokenListCapture, TokenListSupport};
use crate::render_mutation::RenderMutationKind;
fn units(text: &str) -> Vec<u16> { text.encode_utf16().collect() }
fn fixture() -> (Dom, NodeIdx, TokenListCapture) {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let slot = dom.token_list(element, "class").unwrap();
    let capture = dom.token_list_capture(slot).unwrap();
    (dom, element, capture)
}
fn observe(dom: &mut Dom, element: NodeIdx) -> crate::mutation::ObserverId {
    let observer = dom.create_mutation_observer(1);
    dom.observe(observer, element, ObserveOptions {
        attributes: Some(true), attribute_old_value: Some(true), ..Default::default()
    }).unwrap();
    observer
}
#[test]
fn native_token_views_are_canonical_per_element_attribute_and_arena() {
    let (mut dom, node, capture) = fixture();
    assert_eq!(dom.token_list(node, "class").unwrap(), capture.slot());
    let rel = dom.token_list(node, "rel").unwrap();
    assert_ne!(rel, capture.slot());
    let second = dom.create_element(NodeIdx(0), "div");
    assert_ne!(dom.token_list(second, "class").unwrap(), capture.slot());
    assert_eq!(dom.token_list_owner(&capture).unwrap(), capture.element());
    assert_eq!(dom.token_list_attribute(&capture).unwrap(), "class");
    let (other, other_node, other_capture) = fixture();
    assert_eq!(node, other_node);
    assert_eq!(capture.slot(), other_capture.slot());
    assert_eq!(other.token_list_value(&capture), Err(DomError::InvalidStateError));
    assert_eq!(dom.token_list_value(&other_capture), Err(DomError::InvalidStateError));
    assert_eq!(dom.token_list(NodeIdx(0), "class"), Err(DomError::TypeError));
    assert_eq!(dom.token_list_capture(usize::MAX).unwrap_err(), DomError::InvalidStateError);
}
#[test]
fn native_token_captures_remain_valid_when_the_owning_dom_moves() {
    let (dom, _, capture) = fixture();
    let mut moved = Box::new(dom);
    moved.token_list_set_value(&capture, units("a b")).unwrap();
    assert_eq!(moved.token_list_length(&capture).unwrap(), 2);
}
#[test]
fn native_token_weak_metadata_never_retargets_a_recycled_element() {
    let (mut dom, node, capture) = fixture();
    // No script wrapper or owning side table exists in this native fixture.
    // The capture is a weak identity, not a node ownership root.
    dom.detach(node);
    dom.retire_unreachable_node(node).unwrap();
    assert_eq!(dom.token_list_info(capture.slot()), None);
    assert_eq!(dom.token_list_value(&capture), Err(DomError::InvalidStateError));
    let replacement = dom.create_element(NodeIdx(0), "div");
    assert_eq!(replacement, node);
    assert_ne!(dom.node_generation(replacement), Some(capture.element().generation));
    let slot = dom.token_list(replacement, "class").unwrap();
    assert_ne!(slot, capture.slot());
    let current = dom.token_list_capture(slot).unwrap();
    assert_eq!(current.view_generation(), 1);
    assert!(current.element().generation > 1);
    assert_eq!(dom.token_list_add(&capture, &[units("wrong")]), Err(DomError::InvalidStateError));
    dom.token_list_add(&current, &[units("right")]).unwrap();
    assert_eq!(dom.token_list_value(&current).unwrap(), units("right"));
    assert_eq!(dom.token_list_capture(capture.slot()).unwrap_err(), DomError::InvalidStateError);
}
#[test]
fn native_token_queries_are_live_ordered_and_do_not_normalize_raw_values() {
    let (mut dom, node, capture) = fixture();
    let raw = units("\t a a\r\nb\u{c}c \u{b}d\u{a0}e ");
    dom.set_attribute_ns_units(node, None, "class", raw.clone()).unwrap();
    let observer = observe(&mut dom, node);
    assert_eq!(dom.token_list_tokens(&capture).unwrap(), vec![units("a"), units("b"), units("c"), units("\u{b}d\u{a0}e")]);
    assert_eq!(dom.token_list_length(&capture).unwrap(), 4);
    assert_eq!(dom.token_list_item(&capture, 1).unwrap(), Some(units("b")));
    assert_eq!(dom.token_list_item(&capture, u32::MAX).unwrap(), None);
    assert!(!dom.token_list_contains(&capture, &[]).unwrap());
    assert!(!dom.token_list_contains(&capture, &units("a b")).unwrap());
    assert_eq!(dom.token_list_value(&capture).unwrap(), raw);
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn native_token_values_keep_surrogates_nulls_and_scalar_replacement_distinct() {
    let (mut dom, _, capture) = fixture();
    let first = vec![0xd800, 0];let second = vec![0xdc00, 0];let replacement = vec![0xfffd, 0];
    dom.token_list_add(&capture, &[first.clone(), second.clone(), replacement.clone(), first.clone()]).unwrap();
    assert_eq!(dom.token_list_tokens(&capture).unwrap(), vec![first.clone(), second.clone(), replacement.clone()]);
    assert!(dom.token_list_contains(&capture, &first).unwrap());
    dom.token_list_remove(&capture, &[second]).unwrap();
    assert_eq!(dom.token_list_tokens(&capture).unwrap(), vec![first, replacement]);
}
#[test]
fn native_token_set_value_is_raw_and_each_existing_set_queues_once() {
    let (mut dom, node, capture) = fixture();
    let observer = observe(&mut dom, node);
    let raw = vec![32, 0xd800, 32, 0xd800, 32];
    dom.token_list_set_value(&capture, raw.clone()).unwrap();
    dom.token_list_set_value(&capture, raw.clone()).unwrap();
    assert_eq!(dom.token_list_value(&capture).unwrap(), raw.clone());
    assert_eq!(dom.token_list_length(&capture).unwrap(), 1);
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].old_value_utf16, None);
    assert_eq!(records[1].old_value_utf16, Some(raw));
}
#[test]
fn native_token_zero_argument_updates_preserve_absence_but_normalize_presence() {
    let (mut dom, node, capture) = fixture();
    let observer = observe(&mut dom, node);
    dom.token_list_add(&capture, &[]).unwrap();dom.token_list_remove(&capture, &[]).unwrap();
    assert!(dom.get_attribute_ns(node, None, "class").is_none());
    assert!(dom.take_mutation_records(observer).is_empty());
    dom.token_list_set_value(&capture, units("  a a  b ")).unwrap();
    dom.take_mutation_records(observer);
    dom.token_list_add(&capture, &[]).unwrap();
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("a b"));
    assert_eq!(dom.take_mutation_records(observer).len(), 1);
    dom.token_list_set_value(&capture, units(" \t ")).unwrap();dom.take_mutation_records(observer);
    dom.token_list_remove(&capture, &[]).unwrap();
    assert_eq!(dom.get_attribute_ns(node, None, "class"), Some(""));
    assert_eq!(dom.take_mutation_records(observer).len(), 1);
}
#[test]
fn native_token_add_and_remove_validate_every_argument_before_mutation() {
    let (mut dom, node, capture) = fixture();
    dom.token_list_set_value(&capture, units(" a a ")).unwrap();let observer = observe(&mut dom, node);
    for tokens in [vec![units("new"), vec![]], vec![units("new"), units("bad token")]] {
        assert!(dom.token_list_add(&capture, &tokens).is_err());
        assert!(dom.token_list_remove(&capture, &tokens).is_err());
        assert_eq!(dom.token_list_value(&capture).unwrap(), units(" a a "));
    }
    assert_eq!(dom.token_list_add(&capture, &[units("bad token"), vec![]]), Err(DomError::InvalidCharacterError));
    assert_eq!(dom.token_list_remove(&capture, &[vec![], units("bad token")]), Err(DomError::SyntaxError));
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn native_token_add_remove_order_and_missing_removal_follow_update_steps() {
    let (mut dom, node, capture) = fixture();
    dom.token_list_set_value(&capture, units(" b a b ")).unwrap();let observer = observe(&mut dom, node);
    dom.token_list_add(&capture, &[units("a"), units("c"), units("c"), units("d")]).unwrap();
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("b a c d"));
    dom.token_list_remove(&capture, &[units("b"), units("c"), units("b")]).unwrap();
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("a d"));
    dom.token_list_remove(&capture, &[units("absent")]).unwrap();
    assert_eq!(dom.take_mutation_records(observer).len(), 3);
}
#[test]
fn native_token_force_noops_preserve_raw_value_presence_and_observers() {
    let (mut dom, node, capture) = fixture();let observer = observe(&mut dom, node);
    assert!(!dom.token_list_toggle(&capture, &units("a"), Some(false)).unwrap());
    assert!(dom.get_attribute_ns(node, None, "class").is_none());
    dom.token_list_set_value(&capture, units(" a a ")).unwrap();dom.take_mutation_records(observer);
    assert!(dom.token_list_toggle(&capture, &units("a"), Some(true)).unwrap());
    assert!(!dom.token_list_toggle(&capture, &units("b"), Some(false)).unwrap());
    assert_eq!(dom.token_list_value(&capture).unwrap(), units(" a a "));
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn native_token_toggle_transitions_serialize_and_retain_empty_attribute() {
    let (mut dom, node, capture) = fixture();let observer = observe(&mut dom, node);
    assert!(dom.token_list_toggle(&capture, &[0xd800], None).unwrap());
    assert!(!dom.token_list_toggle(&capture, &[0xd800], None).unwrap());
    assert_eq!(dom.get_attribute_ns(node, None, "class"), Some(""));
    assert!(dom.token_list_toggle(&capture, &units("a"), Some(true)).unwrap());
    assert!(!dom.token_list_toggle(&capture, &units("a"), Some(false)).unwrap());
    assert_eq!(dom.take_mutation_records(observer).len(), 4);
    for token in [vec![], units("a b")] { assert!(dom.token_list_toggle(&capture, &token, None).is_err()); }
}
#[test]
fn native_token_replacement_retains_earliest_existing_position() {
    let (mut dom, _, capture) = fixture();
    for (raw, old, new, result) in [("a b c d", "b", "d", "a d c"), ("a b c d", "d", "b", "a b c"), ("a b c", "b", "z", "a z c")] {
        dom.token_list_set_value(&capture, units(raw)).unwrap();
        assert!(dom.token_list_replace(&capture, &units(old), &units(new)).unwrap());
        assert_eq!(dom.token_list_value(&capture).unwrap(), units(result));
    }
}
#[test]
fn native_token_replace_validation_priority_and_compatibility_noops() {
    let (mut dom, node, capture) = fixture();
    dom.token_list_set_value(&capture, units(" a a ")).unwrap();let observer = observe(&mut dom, node);
    assert_eq!(dom.token_list_replace(&capture, &units("bad token"), &[]), Err(DomError::SyntaxError));
    assert_eq!(dom.token_list_replace(&capture, &[], &units("bad token")), Err(DomError::SyntaxError));
    assert_eq!(dom.token_list_replace(&capture, &units("a"), &units("b c")), Err(DomError::InvalidCharacterError));
    assert!(!dom.token_list_replace(&capture, &units("absent"), &units("b")).unwrap());
    assert_eq!(dom.token_list_value(&capture).unwrap(), units(" a a "));
    assert!(dom.take_mutation_records(observer).is_empty());
    assert!(dom.token_list_replace(&capture, &units("a"), &units("a")).unwrap());
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("a"));
    assert_eq!(dom.take_mutation_records(observer).len(), 1);
}
#[test]
fn native_token_supports_uses_explicit_policy_and_only_ascii_case_folding() {
    let (mut dom, _, capture) = fixture();
    let rel = dom.token_list(capture.element().node, "rel").unwrap();let rel = dom.token_list_capture(rel).unwrap();
    let words = [units("noopener"), units("É"), vec![0xd800]];
    let slices: Vec<&[u16]> = words.iter().map(Vec::as_slice).collect();
    for view in [&capture, &rel] {
        assert_eq!(dom.token_list_supports(view, &[], TokenListSupport::Undefined), Err(DomError::TypeError));
        assert!(dom.token_list_supports(view, &units("NOOPENER"), TokenListSupport::Defined(&slices)).unwrap());
        assert!(dom.token_list_supports(view, &units("É"), TokenListSupport::Defined(&slices)).unwrap());
        assert!(!dom.token_list_supports(view, &units("é"), TokenListSupport::Defined(&slices)).unwrap());
        assert!(dom.token_list_supports(view, &[0xd800], TokenListSupport::Defined(&slices)).unwrap());
        assert!(!dom.token_list_supports(view, &[0xfffd], TokenListSupport::Defined(&slices)).unwrap());
        assert!(!dom.token_list_supports(view, &[], TokenListSupport::Defined(&[])).unwrap());
        assert!(!dom.token_list_supports(view, &units("bad token"), TokenListSupport::Defined(&slices)).unwrap());
    }
}
#[test]
fn native_token_all_external_attribute_routes_refresh_the_live_view() {
    let (mut dom, node, capture) = fixture();
    dom.set_attribute(node, "class", "first second").unwrap();
    assert_eq!(dom.token_list_item(&capture, 0).unwrap(), Some(units("first")));
    let attr = dom.get_attribute_node(node, None, "class").unwrap();
    dom.set_attr_value(attr, vec![0xd800, 32, 0xdc00]).unwrap();
    assert_eq!(dom.token_list_tokens(&capture).unwrap(), vec![vec![0xd800], vec![0xdc00]]);
    dom.remove_attribute(node, "class").unwrap();
    assert_eq!(dom.token_list_length(&capture).unwrap(), 0);
    let attribute = dom.create_attribute_in(NodeIdx(0), "class").unwrap();
    dom.set_attr_value(attribute, units("via attr")).unwrap();
    dom.set_attribute_node(node, attribute).unwrap();
    assert_eq!(dom.token_list_tokens(&capture).unwrap(), vec![units("via"), units("attr")]);
}
#[test]
fn native_token_mutation_and_render_journals_keep_exact_single_write_values() {
    let (mut dom, node, capture) = fixture();let observer = observe(&mut dom, node);
    let first = vec![0xd800];let second = vec![0xdc00];
    dom.token_list_set_value(&capture, first.clone()).unwrap();dom.take_mutation_records(observer);
    dom.take_render_mutations(dom.render_mutation_epoch());
    dom.token_list_set_value(&capture, second.clone()).unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 1);assert_eq!(records[0].old_value_utf16, Some(first.clone()));
    let journal = dom.take_render_mutations(dom.render_mutation_epoch());
    assert_eq!(journal.len(), 1);
    let RenderMutationKind::Attribute { old_value, new_value, old_tokens, new_tokens, .. } = &journal[0].kind else { panic!("attribute mutation expected"); };
    assert_eq!(old_value, &Some(first.clone()));assert_eq!(new_value, &Some(second.clone()));
    assert_eq!(old_tokens.classes, vec![first]);assert_eq!(new_tokens.classes, vec![second]);
}
#[test]
fn native_token_invalid_mutators_leave_observer_and_render_queues_empty() {
    let (mut dom, node, capture) = fixture();
    dom.token_list_set_value(&capture, units("safe")).unwrap();let observer = observe(&mut dom, node);
    dom.take_render_mutations(dom.render_mutation_epoch());
    assert_eq!(dom.token_list_add(&capture, &[units("new"), vec![]]), Err(DomError::SyntaxError));
    assert_eq!(dom.token_list_remove(&capture, &[units("safe"), units("bad token")]), Err(DomError::InvalidCharacterError));
    assert_eq!(dom.token_list_toggle(&capture, &units("bad token"), Some(true)), Err(DomError::InvalidCharacterError));
    assert_eq!(dom.token_list_replace(&capture, &units("safe"), &[]), Err(DomError::SyntaxError));
    assert!(dom.take_mutation_records(observer).is_empty());
    assert!(dom.take_render_mutations(dom.render_mutation_epoch()).is_empty());
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("safe"));
}
#[test]
fn native_token_cloning_and_import_keep_values_but_create_distinct_views() {
    let (mut dom, node, capture) = fixture();
    let value = vec![0xd800, 32, 0xdc00];dom.token_list_set_value(&capture, value.clone()).unwrap();
    let observer = observe(&mut dom, node);
    let clone = dom.clone_node(node, false).unwrap();let imported = dom.import_node(NodeIdx(0), node, false).unwrap();
    for copy in [clone, imported] {
        let slot = dom.token_list(copy, "class").unwrap();assert_ne!(slot, capture.slot());
        let view = dom.token_list_capture(slot).unwrap();assert_eq!(dom.token_list_value(&view).unwrap(), value);
        dom.token_list_add(&view, &[units("new")]).unwrap();
        assert_eq!(dom.token_list_value(&capture).unwrap(), value);
    }
    assert!(dom.take_mutation_records(observer).is_empty());
}
#[test]
fn native_token_namespaced_attributes_do_not_enter_the_null_namespace_set() {
    let (mut dom, node, capture) = fixture();
    dom.set_attribute_expanded(node, Some("urn:test"), Some("p"), "class", "foreign").unwrap();
    assert_eq!(dom.token_list_length(&capture).unwrap(), 0);
    dom.token_list_add(&capture, &[units("actual")]).unwrap();
    assert_eq!(dom.get_attribute_ns(node, Some("urn:test"), "class"), Some("foreign"));
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("actual"));
}
#[test]
fn native_token_capture_reads_mutations_after_author_conversion_and_adoption() {
    let (mut dom, node, capture) = fixture();
    dom.token_list_set_value(&capture, units("before")).unwrap();
    // The embedding finishes conversion before entering the native operation.
    dom.set_attribute_ns_units(node, None, "class", units("after conversion")).unwrap();
    let document = dom.create_document();
    dom.adopt(node, document).unwrap();
    dom.token_list_add(&capture, &[units("new")]).unwrap();
    assert_eq!(dom.token_list_value(&capture).unwrap(), units("after conversion new"));
    assert_eq!(dom.token_list_owner(&capture).unwrap(), capture.element());
}
