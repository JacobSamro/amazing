use super::*;

#[test]
fn dataset_identity_live_values_and_binding_coercion() {
    let mut h = Host::default();
    let element = h.dom.create_element_ns(NodeIdx(0), Some("http://www.w3.org/1999/xhtml"), "div").unwrap();
    let map = call(&mut h, O::ElementDataset, V::Node(element), &[]);
    assert_eq!(map, call(&mut h, O::ElementDataset, V::Node(element), &[]));
    h.dom.set_attribute(element, "data-first-name", "Ada").unwrap();
    assert_eq!(call(&mut h, O::DatasetGet, map.clone(), &[str_value("firstName")]), str_value("Ada"));
    let units = V::String(vec![0xd800, 0xdc00, 0xd800]);
    call(&mut h, O::DatasetSet, map.clone(), &[str_value("firstName"), units.clone()]);
    assert_eq!(call(&mut h, O::DatasetGet, map.clone(), &[str_value("firstName")]), units);
    assert_eq!(supported_names(&mut h, O::DatasetSupportedNames, &map).unwrap(), vec!["firstName".encode_utf16().collect::<Vec<_>>()]);
    call(&mut h, O::DatasetDelete, map.clone(), &[str_value("firstName")]);
    assert_eq!(call(&mut h, O::DatasetGet, map.clone(), &[str_value("firstName")]), V::Undefined);
    assert!(supported_names(&mut h, O::DatasetSupportedNames, &map).unwrap().is_empty());
    h.conversions.clear();
    call(&mut h, O::DatasetSet, map.clone(), &[V::Coerce("name"), V::Coerce("value")]);
    assert_eq!(h.conversions, ["name", "value"]);
    let result = invoke(&mut h, O::DatasetSet, map, &[str_value("invalid-name"), V::Throw]);
    assert_eq!(result, Err("coercion sentinel".into()));
}

#[test]
fn dataset_receiver_and_legacy_hook_contract() {
    let mut h = Host::default();
    for receiver in [V::Undefined, V::Null, V::Node(NodeIdx(0)), V::Dictionary(vec![])] {
        assert!(invoke(&mut h, O::ElementDataset, receiver, &[]).is_err());
    }
    let svg = h.dom.create_element_ns(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "g").unwrap();
    let map = call(&mut h, O::ElementDataset, V::Node(svg), &[]);
    assert_eq!(supported_index_count(&mut h, O::DatasetIndexCount, &map).unwrap(), 0);
    install(&mut h).unwrap();
    let hooks = h.legacy.iter().find(|(brand, _)| *brand == InterfaceBrand::DOM_STRING_MAP).unwrap().1;
    assert!(hooks.override_builtins);
    assert!(!hooks.unenumerable_names);
    assert_eq!(hooks.named_getter, Some(O::DatasetGet));
    assert_eq!(hooks.named_setter, Some(O::DatasetSet));
    assert_eq!(hooks.named_deleter, Some(O::DatasetDelete));
}
