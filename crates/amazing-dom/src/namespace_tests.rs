use super::*;
use crate::range::BoundaryPoint;

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
