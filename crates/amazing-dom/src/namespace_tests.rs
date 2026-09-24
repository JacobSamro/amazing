use super::*;
use crate::range::BoundaryPoint;

#[test]
fn attributes_keep_one_insertion_order_across_namespaces_and_clones() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    dom.set_attribute_qualified_ns(element, Some("urn:a"), "a:first", "1").unwrap();
    dom.set_attribute(element, "plain", "2").unwrap();
    dom.set_attribute_qualified_ns(element, Some("urn:b"), "b:last", "3").unwrap();
    let names = |dom: &Dom, element| dom.attributes_expanded(element).unwrap().iter()
        .map(AttributeRef::qualified_name).collect::<Vec<_>>();
    assert_eq!(names(&dom, element), ["a:first", "plain", "b:last"]);
    dom.set_attribute_ns(element, Some("urn:a"), "first", "updated").unwrap();
    assert_eq!(names(&dom, element), ["a:first", "plain", "b:last"]);
    let nodes = dom.attribute_nodes(element).unwrap();
    assert_eq!(nodes.iter().map(|node| dom.attr_snapshot(*node).unwrap().name()).collect::<Vec<_>>(),
        ["a:first", "plain", "b:last"]);
    let copy = dom.clone_node(element, false).unwrap();
    assert_eq!(names(&dom, copy), ["a:first", "plain", "b:last"]);
    dom.remove_attribute_ns(element, Some("urn:a"), "first").unwrap();
    dom.set_attribute_qualified_ns(element, Some("urn:a"), "a:first", "4").unwrap();
    assert_eq!(names(&dom, element), ["plain", "b:last", "a:first"]);
}

#[test]
fn create_element_local_name_does_not_extract_colon_prefix() {
    let mut dom = Dom::new();
    let element = dom.create_element_local_in(NodeIdx(0), Some(HTML), ":good:times:").unwrap();
    assert_eq!(dom.local_name(element), Some(":good:times:"));
    assert_eq!(dom.prefix(element), None);
    assert_eq!(dom.namespace(element), Some(Namespace::Html));
    assert_eq!(dom.create_element_local_in(NodeIdx(0), Some(HTML), ""),
        Err(DomError::InvalidCharacterError));
    assert_eq!(dom.create_element_ns_in(NodeIdx(0), Some(HTML), ":good:times:"),
        Err(DomError::InvalidCharacterError));
}

#[test]
fn null_empty_and_arbitrary_namespaces_keep_identity_and_case() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let a = dom.create_element_ns(root, None, "MixedCase").unwrap();
    let b = dom.create_element_ns(root, Some(""), "MixedCase").unwrap();
    let c = dom
        .create_element_ns(root, Some("urn:example:A"), "p:MixedCase")
        .unwrap();
    let d = dom
        .create_element_ns(root, Some("urn:example:B"), "p:MixedCase")
        .unwrap();
    let e = dom
        .create_element_ns(root, Some("urn:example:A"), "q:MixedCase")
        .unwrap();
    assert_eq!(dom.namespace(a), Some(Namespace::Null));
    assert_eq!(dom.namespace(a), dom.namespace(b));
    assert_eq!(dom.namespace_uri(a), None);
    assert_eq!(dom.namespace_uri(c), Some("urn:example:A"));
    assert_ne!(dom.namespace(c), dom.namespace(d));
    assert_eq!(dom.namespace(c), dom.namespace(e));
    assert_eq!(dom.prefix(c), Some("p"));
    assert_eq!(dom.name(c), Some("MixedCase"));
    assert_eq!(dom.local_name(c), Some("MixedCase"));
    assert_eq!(dom.qualified_name(c).as_deref(), Some("p:MixedCase"));
    assert_eq!(dom.qualified_name(a).as_deref(), Some("MixedCase"));
    assert_eq!(dom.prefix(a), None);
    assert_eq!(dom.namespace(NodeIdx(0)), None);
    assert_eq!(dom.namespace_uri(NodeIdx(0)), None);
    assert_eq!(dom.prefix(NodeIdx(0)), None);
    assert_eq!(dom.qualified_name(NodeIdx(0)), None);
}

#[test]
fn known_uris_preserve_existing_enum_values_and_parser_constructors() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    for (uri, ns) in [
        (HTML, Namespace::Html),
        (SVG, Namespace::Svg),
        (MATHML, Namespace::MathMl),
    ] {
        let node = dom.create_element_ns(root, Some(uri), "pre:Mixed").unwrap();
        assert_eq!(dom.namespace(node), Some(ns));
        assert_eq!(dom.namespace_uri(node), Some(uri));
        assert_eq!(dom.local_name(node), Some("Mixed"));
        assert_eq!(dom.prefix(node), Some("pre"));
        let parser = dom.alloc_unattached(NodeKind::Element {
            name: "Local".into(),
            attrs: vec![],
            namespace: ns,
        });
        assert_eq!(dom.namespace_uri(parser), Some(uri));
        assert_eq!(dom.prefix(parser), None);
    }
    assert_eq!(dom.namespace(root), Some(Namespace::Html));
    assert_eq!(dom.namespace_uri(root), Some(HTML));
    let almost = dom
        .create_element_ns(root, Some("http://www.w3.org/1999/XHTML"), "x")
        .unwrap();
    assert_ne!(dom.namespace(almost), Some(Namespace::Html));
}

#[test]
fn invalid_names_and_reserved_prefixes_do_not_mutate_the_tree() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let before = dom.node_count();
    for name in ["", ":x", "p:", "a b", "a/b", "a>b", "a\0b", "1bad", "_a="] {
        assert_eq!(
            dom.create_element_ns(root, Some("urn:x"), name),
            Err(DomError::InvalidCharacterError),
            "{name:?}"
        );
    }
    for (uri, name) in [
        (None, "p:x"),
        (Some(""), "p:x"),
        (Some("urn:x"), "xml:x"),
        (None, "xmlns"),
        (Some("urn:x"), "xmlns:x"),
        (Some(XMLNS), "x"),
        (Some(XMLNS), "p:x"),
    ] {
        assert_eq!(
            dom.create_element_ns(root, uri, name),
            Err(DomError::NamespaceError)
        );
    }
    assert_eq!(dom.node_count(), before);
    assert_eq!(dom.children(root).count(), 0);
    for (uri, name) in [(XML, "xml:lang"), (XMLNS, "xmlns"), (XMLNS, "xmlns:p")] {
        let node = dom.create_element_ns(root, Some(uri), name).unwrap();
        assert_eq!(dom.namespace_uri(node), Some(uri));
        assert_eq!(dom.qualified_name(node).as_deref(), Some(name));
    }
}

#[test]
fn dom_name_validation_is_distinct_from_xml_tokenization() {
    // DOM Standard 2026 name validation is intentionally less strict than XML.
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    for name in [
        "a=b", "a!", "1:local", "p:a:b", "p:é", "p:😀", "éclair", "_name",
    ] {
        let node = dom.create_element_ns(root, Some("urn:x"), name).unwrap();
        assert_eq!(dom.qualified_name(node).as_deref(), Some(name));
    }
    let node = dom.create_element_ns(root, Some("urn:x"), "p:a:b").unwrap();
    assert_eq!(dom.prefix(node), Some("p"));
    assert_eq!(dom.local_name(node), Some("a:b"));
}

#[test]
fn namespace_survives_deep_clone_and_adoption() {
    let mut dom = Dom::new();
    let node = dom
        .create_element_ns(NodeIdx(0), Some("urn:outer"), "p:Root")
        .unwrap();
    let child = dom.create_element_ns(node, None, "Child").unwrap();
    dom.create_element_ns(child, Some("urn:inner"), "q:Leaf")
        .unwrap();
    let copy = dom.clone_node(node, true).unwrap();
    let originals = dom.tree_order(node);
    let copies = dom.tree_order(copy);
    for (&a, &b) in originals.iter().zip(&copies) {
        assert_eq!(dom.namespace(a), dom.namespace(b));
        assert_eq!(dom.namespace_uri(a), dom.namespace_uri(b));
        assert_eq!(dom.qualified_name(a), dom.qualified_name(b));
    }
    let other_document = dom.create_document();
    dom.pre_insert(copy, other_document, None).unwrap();
    for node in copies {
        assert_eq!(dom.owner_document(node), Some(other_document));
    }
    assert_eq!(dom.namespace_uri(copy), Some("urn:outer"));
    assert_eq!(dom.prefix(copy), Some("p"));
}

#[test]
fn namespace_survives_partial_range_cloning() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let a = dom.create_element_ns(root, Some("urn:a"), "p:One").unwrap();
    let at = dom.create_text(a, "abc");
    let b = dom.create_element_ns(root, Some("urn:b"), "q:Two").unwrap();
    let bt = dom.create_text(b, "def");
    let range = dom.create_range(NodeIdx(0)).unwrap();
    dom.range_set_start(range, BoundaryPoint::new(at, 1))
        .unwrap();
    dom.range_set_end(range, BoundaryPoint::new(bt, 2)).unwrap();
    let fragment = dom.range_clone_contents(range).unwrap();
    let copies: Vec<_> = dom.children(fragment).collect();
    assert_eq!(copies.len(), 2);
    for (copy, original) in copies.into_iter().zip([a, b]) {
        assert_eq!(dom.namespace_uri(copy), dom.namespace_uri(original));
        assert_eq!(dom.qualified_name(copy), dom.qualified_name(original));
    }
}

#[test]
fn detached_constructor_uses_requested_document_and_checked_attachment() {
    let mut dom = Dom::new();
    let document = dom.create_document();
    let node = dom
        .create_element_ns_in(document, Some("urn:x"), "x:Root")
        .unwrap();
    assert_eq!(dom.owner_document(node), Some(document));
    assert_eq!(dom.get(node).parent, None);
    dom.pre_insert(node, document, None).unwrap();
    assert_eq!(
        dom.create_element_ns(document, None, "second"),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(dom.children(document).collect::<Vec<_>>(), vec![node]);
    assert_eq!(
        dom.create_element_ns_in(node, None, "x"),
        Err(DomError::NotSupportedError)
    );
    assert_eq!(
        dom.create_element_ns(NodeIdx(u32::MAX), None, "x"),
        Err(DomError::InvalidNode)
    );
}

#[test]
fn xml_parser_can_create_unprefixed_xmlns_without_relaxing_dom_api() {
    let mut dom = Dom::new();
    assert_eq!(
        dom.create_element_ns(NodeIdx(0), None, "xmlns"),
        Err(DomError::NamespaceError)
    );
    let root = dom
        .create_element_expanded(NodeIdx(0), None, None, "xmlns")
        .unwrap();
    assert_eq!(dom.namespace(root), Some(Namespace::Null));
    assert_eq!(dom.qualified_name(root).as_deref(), Some("xmlns"));
    let child = dom
        .create_element_expanded(root, Some("urn:default"), None, "xmlns")
        .unwrap();
    assert_eq!(dom.namespace_uri(child), Some("urn:default"));
    assert_eq!(dom.prefix(child), None);
    assert_eq!(
        dom.create_element_ns(root, Some("urn:default"), "xmlns"),
        Err(DomError::NamespaceError)
    );
}

#[test]
fn expanded_parser_creation_keeps_names_case_and_checked_tree_ownership() {
    let mut dom = Dom::new();
    let document = dom.create_document();
    let root = dom
        .create_element_expanded_in(document, Some("urn:a"), Some("P"), "Mixed")
        .unwrap();
    assert_eq!(dom.get(root).parent, None);
    assert_eq!(dom.owner_document(root), Some(document));
    assert_eq!(dom.qualified_name(root).as_deref(), Some("P:Mixed"));
    dom.pre_insert(root, document, None).unwrap();
    assert_eq!(
        dom.create_element_expanded(document, None, None, "second"),
        Err(DomError::HierarchyRequestError)
    );
    assert_eq!(
        dom.create_element_expanded_in(root, None, None, "x"),
        Err(DomError::NotSupportedError)
    );
    let clone = dom.clone_node(root, false).unwrap();
    assert_eq!(dom.qualified_name(clone), dom.qualified_name(root));
}

#[test]
fn qualified_attributes_keep_prefix_and_expanded_lookup_identity() {
    let mut dom = Dom::new();
    let root = dom
        .create_element_ns(NodeIdx(0), Some("urn:element"), "root")
        .unwrap();
    dom.set_attribute_qualified_ns(root, Some("urn:a"), "p:a", "first")
        .unwrap();
    dom.set_attribute_qualified_ns(root, Some("urn:b"), "p:a", "other")
        .unwrap();
    dom.set_attribute_qualified_ns(root, None, "a", "null")
        .unwrap();
    assert_eq!(
        dom.get_attribute_ns(root, Some("urn:a"), "a"),
        Some("first")
    );
    assert_eq!(dom.get_attribute_ns(root, Some("urn:a"), "p:a"), None);
    let a = dom.attribute_ns(root, Some("urn:a"), "a").unwrap();
    assert_eq!(a.namespace_uri, Some("urn:a"));
    assert_eq!(a.prefix, Some("p"));
    assert_eq!(a.qualified_name(), "p:a");
    assert_eq!(
        dom.attribute_ns(root, None, "a").unwrap().namespace_uri,
        None
    );
    // DOM set-an-attribute-value changes value, not an existing Attr's prefix.
    dom.set_attribute_qualified_ns(root, Some("urn:a"), "q:a", "second")
        .unwrap();
    assert_eq!(
        dom.attribute_ns(root, Some("urn:a"), "a")
            .unwrap()
            .qualified_name(),
        "p:a"
    );
    assert_eq!(
        dom.get_attribute_ns(root, Some("urn:b"), "a"),
        Some("other")
    );
    dom.remove_attribute_ns(root, Some("urn:a"), "a").unwrap();
    assert_eq!(dom.attribute_ns(root, Some("urn:a"), "a"), None);
    dom.set_attribute_qualified_ns(root, Some("urn:a"), "q:a", "new")
        .unwrap();
    assert_eq!(
        dom.attribute_ns(root, Some("urn:a"), "a").unwrap().prefix,
        Some("q")
    );
}

#[test]
fn qualified_attribute_validation_precedes_mutation_and_supports_xmlns_attributes() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let mut initial = dom.begin_mutation_checkpoint();
    while dom.next_mutation_delivery(&mut initial).is_some() {}
    let observer = dom.create_mutation_observer(1);
    dom.observe(
        observer,
        root,
        crate::mutation::ObserveOptions {
            attributes: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    for name in ["", "p:", "p:a=b", "p:a b"] {
        assert_eq!(
            dom.set_attribute_qualified_ns(root, Some("urn:a"), name, "x"),
            Err(DomError::InvalidCharacterError)
        );
    }
    for (uri, name) in [
        (None, "p:a"),
        (Some("urn:a"), "xml:lang"),
        (None, "xmlns"),
        (Some(XMLNS), "p:a"),
    ] {
        assert_eq!(
            dom.set_attribute_qualified_ns(root, uri, name, "x"),
            Err(DomError::NamespaceError)
        );
    }
    assert!(!dom.mutation_microtask_pending());
    assert!(dom.take_mutation_records(observer).is_empty());
    dom.set_attribute_qualified_ns(root, Some(XMLNS), "xmlns", "urn:default")
        .unwrap();
    dom.set_attribute_qualified_ns(root, Some(XMLNS), "xmlns:p", "urn:p")
        .unwrap();
    assert_eq!(
        dom.attribute_ns(root, Some(XMLNS), "xmlns")
            .unwrap()
            .qualified_name(),
        "xmlns"
    );
    assert_eq!(
        dom.attribute_ns(root, Some(XMLNS), "p")
            .unwrap()
            .qualified_name(),
        "xmlns:p"
    );
    // Attribute local-name validation permits a leading digit; element validation does not.
    dom.set_attribute_qualified_ns(root, None, "1", "allowed by DOM")
        .unwrap();
}

#[test]
fn expanded_attributes_clone_and_notify_by_local_name() {
    use crate::mutation::ObserveOptions;
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "root");
    let observer = dom.create_mutation_observer(1);
    dom.observe(
        observer,
        root,
        ObserveOptions {
            attributes: Some(true),
            attribute_old_value: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    dom.set_attribute_expanded(root, Some("urn:a"), Some("p"), "a", "old")
        .unwrap();
    dom.set_attribute_ns(root, Some("urn:a"), "a", "new")
        .unwrap();
    let records = dom.take_mutation_records(observer);
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].attribute_name.as_deref(), Some("a"));
    assert_eq!(records[1].attribute_namespace.as_deref(), Some("urn:a"));
    assert_eq!(records[1].old_value.as_deref(), Some("old"));
    let clone = dom.clone_node(root, true).unwrap();
    let attr = dom.attribute_ns(clone, Some("urn:a"), "a").unwrap();
    assert_eq!(attr.prefix, Some("p"));
    assert_eq!(attr.value, "new");
    let document = dom.create_document();
    dom.pre_insert(clone, document, None).unwrap();
    assert_eq!(
        dom.attribute_ns(clone, Some("urn:a"), "a")
            .unwrap()
            .qualified_name(),
        "p:a"
    );
}
#[test]
fn wpt_doctype_name_allows_empty_and_punctuation_but_rejects_ascii_whitespace_and_gt() {
    let mut dom = crate::Dom::new();
    let document = crate::NodeIdx(0);
    for name in ["", "1foo", "@foo", "edi:/", "{"] {
        assert!(dom.create_document_type_in(document, name, "", "").is_ok(), "{name:?}");
    }
    for name in ["a b", "edi:>", "a\0b", "a\tb"] {
        assert_eq!(dom.create_document_type_in(document, name, "", ""),
            Err(crate::DomError::InvalidCharacterError), "{name:?}");
    }
}
