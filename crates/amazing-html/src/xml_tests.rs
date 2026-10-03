use super::*;

fn elements(dom: &Dom) -> Vec<NodeIdx> {
    (0..dom.node_count())
        .map(|n| NodeIdx(n as u32))
        .filter(|&n| dom.name(n).is_some())
        .collect()
}
fn text(dom: &Dom) -> String {
    (0..dom.node_count())
        .filter_map(|n| match &dom.get(NodeIdx(n as u32)).kind {
            NodeKind::Text { data } | NodeKind::CDATASection { data } => Some(data.as_str()),
            _ => None,
        })
        .collect()
}
fn malformed(src: &str) {
    let error = parse_xml(src).expect_err(src);
    assert_eq!(error.kind, XmlErrorKind::Malformed, "{src}: {error}");
}

#[test]
fn document_structure_is_xml_not_html() {
    let dom = parse_xml(" <!--before--> <Root><P/><P>text</P></Root> <?tail?> ").unwrap();
    let nodes = elements(&dom);
    assert_eq!(
        nodes
            .iter()
            .map(|&n| dom.name(n).unwrap())
            .collect::<Vec<_>>(),
        ["Root", "P", "P"]
    );
    assert_eq!(dom.get(nodes[1]).parent, Some(nodes[0]));
    assert_eq!(dom.get(nodes[2]).parent, Some(nodes[0]));
    for src in [
        "",
        "  ",
        "<!--alone-->",
        "x<r/>",
        "<r/>x",
        "<r/><s/>",
        "<r>",
        "</r>",
        "<r></R>",
        "<r><s></r></s>",
        "<r><br></r>",
        "<r / >",
        "<r></r x>",
    ] {
        malformed(src);
    }
}

#[test]
fn attributes_require_quotes_separators_and_unique_names() {
    let dom = parse_xml("<r a='one' b=\"two\"/>").unwrap();
    assert_eq!(
        dom.get_attribute_ns(elements(&dom)[0], None, "a"),
        Some("one")
    );
    for src in [
        "<r a=x/>",
        "<r a/>",
        "<r a='x'b='y'/>",
        "<r a='x' a='y'/>",
        "<r a='<x'/>",
        "<r a='unterminated/>",
        "<r a = />",
    ] {
        malformed(src);
    }
}

#[test]
fn references_and_attribute_normalization() {
    let dom = parse_xml(
        "<r a='x\t\r\ny&#9;&#10;&#13;&amp;'>&lt;&gt;&amp;&quot;&apos;&#65;&#x1F642; &amp;lt;</r>",
    )
    .unwrap();
    assert_eq!(
        dom.get_attribute_ns(elements(&dom)[0], None, "a"),
        Some("x  y\t\n\r&")
    );
    assert_eq!(text(&dom), "<>&\"'A🙂 &lt;");
    for src in [
        "<r>&nbsp;</r>",
        "<r>&amp</r>",
        "<r>&#;</r>",
        "<r>&#x;</r>",
        "<r>&#X41;</r>",
        "<r>&#+65;</r>",
        "<r>&#-1;</r>",
        "<r>&#0;</r>",
        "<r>&#xD800;</r>",
        "<r>&#xFFFE;</r>",
        "<r>&#x110000;</r>",
        "<r>&#999999999999999999999;</r>",
        "<r>&bogus;</r>",
        "<r a='&bad;'/>",
        "<r>\0</r>",
    ] {
        malformed(src);
    }
}

#[test]
fn cdata_is_literal_and_only_allowed_inside_elements() {
    let dom = parse_xml("<r>a<![CDATA[<b>&unknown;\r\n]]>z</r>").unwrap();
    assert_eq!(text(&dom), "a<b>&unknown;\nz");
    for src in [
        "<![CDATA[x]]><r/>",
        "<r/><![CDATA[x]]>",
        "<r><![CDATA[x</r>",
        "<r>]]></r>",
    ] {
        malformed(src);
    }
}

#[test]
fn comments_and_processing_instructions() {
    let dom =
        parse_xml("<?xml-stylesheet href='a.css'?><!--ok--><r><?pi a > & b?><!--x--></r>").unwrap();
    assert_eq!(
        dom.get(NodeIdx(1)).kind,
        NodeKind::ProcessingInstruction {
            target: "xml-stylesheet".into(),
            data: "href='a.css'".into()
        }
    );
    assert_eq!(
        dom.get(NodeIdx(2)).kind,
        NodeKind::Comment { data: "ok".into() }
    );
    assert_eq!(
        dom.get(NodeIdx(4)).kind,
        NodeKind::ProcessingInstruction {
            target: "pi".into(),
            data: "a > & b".into()
        }
    );
    assert_eq!(
        dom.get(NodeIdx(5)).kind,
        NodeKind::Comment { data: "x".into() }
    );
    for src in [
        "<!--a--b--><r/>",
        "<!--a---><r/>",
        "<!--<r/>",
        "<?XML?><r/>",
        "<?xml?><r/>",
        "<r><?xml version='1.0'?></r>",
        "<?p:x?><r/>",
        "<?pi!x?><r/>",
        "<?pi <r/>",
    ] {
        malformed(src);
    }
}

#[test]
fn declaration_grammar_and_encoding_metadata() {
    let (dom, declaration) = parse_xml_with_declaration(
        "\u{feff}<?xml version = '1.0' encoding='UTF-16' standalone='yes'?><r/>",
    )
    .unwrap();
    let declaration = declaration.unwrap();
    assert_eq!(declaration.encoding.as_deref(), Some("UTF-16"));
    assert_eq!(declaration.standalone, Some(true));
    assert_eq!(dom.node_count(), 2);
    parse_xml("<?xml version='1.0'?><r/>").unwrap();
    for src in [
        " <?xml version='1.0'?><r/>",
        "<?xml encoding='UTF-8' version='1.0'?><r/>",
        "<?xml version='1.0'encoding='UTF-8'?><r/>",
        "<?xml version='1.0' encoding='1bad'?><r/>",
        "<?xml version='1.0' standalone='maybe'?><r/>",
        "<?xml version='1.0' standalone='no' encoding='UTF-8'?><r/>",
        "<?xml version='1.0' encoding='UTF-8' encoding='UTF-8'?><r/>",
    ] {
        malformed(src);
    }
    // Forward version declarations retain XML 1.0 processing; accepting the
    // declaration does not enable XML 1.1-only control character references.
    parse_xml("<?xml version='1.1'?><r/>").unwrap();
    malformed("<?xml version='1.1'?><r>&#x1;</r>");
}

#[test]
fn namespace_defaults_prefix_shadowing_and_scope_restoration() {
    let dom = parse_xml("<r xmlns='urn:a' xmlns:p='urn:outer'><p:x p:a='1' a='2'/><inner xmlns='urn:b' xmlns:p='urn:inner'><p:x/></inner><p:x/><plain xmlns=''/></r>").unwrap();
    let nodes = elements(&dom);
    assert_eq!(
        nodes
            .iter()
            .map(|&n| dom.namespace_uri(n))
            .collect::<Vec<_>>(),
        [
            Some("urn:a"),
            Some("urn:outer"),
            Some("urn:b"),
            Some("urn:inner"),
            Some("urn:outer"),
            None
        ]
    );
    assert_eq!(
        dom.get_attribute_ns(nodes[1], Some("urn:outer"), "a"),
        Some("1")
    );
    assert_eq!(dom.get_attribute_ns(nodes[1], None, "a"), Some("2"));
    assert_eq!(dom.prefix(nodes[1]), Some("p"));
    assert_eq!(
        dom.get_attribute_ns(nodes[0], Some(XMLNS_NAMESPACE), "xmlns"),
        Some("urn:a")
    );
    let dom = parse_xml("<p:r p:a='1' xmlns:p='urn:p' xml:lang='en'/>").unwrap();
    let root = elements(&dom)[0];
    assert_eq!(dom.namespace_uri(root), Some("urn:p"));
    assert_eq!(
        dom.get_attribute_ns(root, Some(XML_NAMESPACE), "lang"),
        Some("en")
    );
}

#[test]
fn namespace_constraints_and_expanded_attribute_uniqueness() {
    for src in [
        "<p:r/>",
        "<r p:a='1'/>",
        "<r xmlns:p=''/>",
        "<r xmlns:xml='urn:wrong'/>",
        "<r xmlns:p='http://www.w3.org/XML/1998/namespace'/>",
        "<r xmlns='http://www.w3.org/XML/1998/namespace'/>",
        "<r xmlns:xmlns='urn:a'/>",
        "<r xmlns='http://www.w3.org/2000/xmlns/'/>",
        "<xmlns:r/>",
        "<r xmlns:a='urn:x' xmlns:b='urn:x' a:n='1' b:n='2'/>",
        "<a:b:c/>",
        "<:r/>",
        "<r:/>",
        "<r xmlns:1p='urn:x'/>",
    ] {
        malformed(src);
    }
    parse_xml("<r xmlns:xml='http://www.w3.org/XML/1998/namespace' xmlns:a='urn:x' xmlns:b='urn:y' a:n='1' b:n='2' n='3'/>").unwrap();
}

#[test]
fn xml_name_unicode_ranges_and_case_are_preserved() {
    parse_xml("<é:根 xmlns:é='urn:x' a·b='1'><𐀀/></é:根>").unwrap();
    for src in [
        "<1r/>",
        "<\u{b7}r/>",
        "<\u{37e}/>",
        "<\u{f0000}/>",
        "<r \u{300}a='x'/>",
    ] {
        malformed(src);
    }
}

#[test]
fn doctype_public_system_and_non_fetching_policy() {
    let dom = parse_xml("<!DOCTYPE html PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'http://example.invalid/external.dtd'><html/>").unwrap();
    assert_eq!(
        dom.get(NodeIdx(1)).kind,
        NodeKind::DocumentType {
            name: "html".into(),
            public_id: "-//W3C//DTD XHTML 1.0 Strict//EN".into(),
            system_id: "http://example.invalid/external.dtd".into()
        }
    );
    parse_xml("<!DOCTYPE r SYSTEM 'file:///not-read'><r/>").unwrap();
    for src in [
        "<!DOCTYPEr><r/>",
        "<!DOCTYPE r><!DOCTYPE r><r/>",
        "<r/><!DOCTYPE r>",
        "<!DOCTYPE r PUBLIC 'bad^' 'sys'><r/>",
        "<!DOCTYPE r SYSTEM><r/>",
    ] {
        malformed(src);
    }
    assert_eq!(text(&parse_xml("<!DOCTYPE r [<!ENTITY a 'x'>]><r>&a;</r>").unwrap()), "x");
}

#[test]
fn fatal_error_returns_no_partial_or_recovered_dom() {
    malformed("<r><a></wrong><never/></r>");
    malformed("<html xmlns='http://www.w3.org/1999/xhtml'><body></html>");
}

#[test]
fn xhtml_uses_existing_dom_namespaces_and_preserves_script_text() {
    let dom = parse_xml("<?xml version='1.0'?><html xmlns='http://www.w3.org/1999/xhtml'><body id='b'><svg xmlns='http://www.w3.org/2000/svg'/><math xmlns='http://www.w3.org/1998/Math/MathML'/><script><![CDATA[a < b && c]]></script></body></html>").unwrap();
    let html = dom.children(NodeIdx(0)).next().unwrap();
    assert_eq!(dom.namespace(html), Some(Namespace::Html));
    let body = dom.children(html).next().unwrap();
    assert_eq!(dom.get_attribute_ns(body, None, "id"), Some("b"));
    let children: Vec<_> = dom.children(body).collect();
    assert_eq!(dom.namespace(children[0]), Some(Namespace::Svg));
    assert_eq!(dom.namespace(children[1]), Some(Namespace::MathMl));
    assert_eq!(text(&dom), "a < b && c");
}

#[test]
fn depth_is_bounded_without_recursive_parser_stack() {
    let src = format!("{}{}", "<r>".repeat(513), "</r>".repeat(513));
    assert_eq!(parse_xml(&src).unwrap_err().kind, XmlErrorKind::Limit);
}

#[test]
fn empty_element_namespace_declarations_do_not_leak_to_siblings() {
    let dom = parse_xml("<r xmlns='urn:outer' xmlns:p='urn:a'><p:x xmlns:p='urn:b'/><p:x/><plain xmlns=''/><again/></r>").unwrap();
    assert_eq!(
        elements(&dom)
            .iter()
            .map(|&n| dom.namespace_uri(n))
            .collect::<Vec<_>>(),
        [
            Some("urn:outer"),
            Some("urn:b"),
            Some("urn:a"),
            None,
            Some("urn:outer")
        ]
    );
    malformed("<r><x xmlns:p='urn:a'/><p:y/></r>");
}

#[test]
fn whitespace_and_illegal_characters_follow_xml_ranges() {
    parse_xml("<r>\u{85}\u{2028}\u{1fffe}</r>").unwrap();
    for src in [
        "<r\u{a0}a='1'/>",
        "\u{85}<r/>",
        "<r>\u{1}</r>",
        "<r>\u{b}</r>",
        "<r>\u{ffff}</r>",
        "<!--\u{1}--><r/>",
    ] {
        malformed(src);
    }
    assert_eq!(text(&parse_xml("<r>\r\nx\ry</r>").unwrap()), "\nx\ny");
}

#[test]
fn attributes_are_bounded_before_growing_unbounded_dom_state() {
    let mut src = String::from("<r");
    for n in 0..1025 {
        src.push_str(&format!(" a{n}='x'"));
    }
    src.push_str("/>");
    assert_eq!(parse_xml(&src).unwrap_err().kind, XmlErrorKind::Limit);
}

#[test]
fn unloaded_external_entities_are_unsupported_not_silently_recovered() {
    let src = "<!DOCTYPE r SYSTEM 'unloaded.dtd'><r>&nbsp;</r>";
    assert_eq!(parse_xml(src).unwrap_err().kind, XmlErrorKind::Unsupported);
    malformed("<r>&nbsp;</r>");
}

#[test]
fn external_dtd_does_not_hide_malformed_reference_syntax_or_standalone_errors() {
    for entity in ["&;", "&1bad;", "&bad name;", "&p:name;", "&#xZZ;"] {
        malformed(&format!(
            "<!DOCTYPE r SYSTEM 'unloaded.dtd'><r>{entity}</r>"
        ));
    }
    malformed(
        "<?xml version='1.0' standalone='yes'?><!DOCTYPE r SYSTEM 'unloaded.dtd'><r>&unknown;</r>",
    );
}

#[test]
fn dom_retains_null_custom_and_prefixed_element_identity() {
    let dom = parse_xml("<r xmlns:p='urn:outer'><p:x/><x xmlns='urn:inner'/><plain/></r>").unwrap();
    let root = dom.children(NodeIdx(0)).next().unwrap();
    assert_eq!(dom.namespace(root), Some(Namespace::Null));
    assert_eq!(dom.namespace_uri(root), None);
    let children: Vec<_> = dom.children(root).collect();
    assert_eq!(dom.namespace_uri(children[0]), Some("urn:outer"));
    assert_eq!(dom.prefix(children[0]), Some("p"));
    assert_eq!(dom.local_name(children[0]), Some("x"));
    assert_eq!(dom.qualified_name(children[0]).as_deref(), Some("p:x"));
    assert_eq!(dom.namespace_uri(children[1]), Some("urn:inner"));
    assert_eq!(dom.namespace(children[2]), Some(Namespace::Null));
}

#[test]
fn qualified_attributes_retain_prefix_and_expanded_identity() {
    let dom =
        parse_xml("<r xmlns='urn:element' xmlns:p='urn:attr' p:a='one' a='two' xml:lang='en'/>")
            .unwrap();
    let root = elements(&dom)[0];
    let prefixed = dom.attribute_ns(root, Some("urn:attr"), "a").unwrap();
    assert_eq!(prefixed.namespace_uri, Some("urn:attr"));
    assert_eq!(prefixed.prefix, Some("p"));
    assert_eq!(prefixed.local_name, "a");
    assert_eq!(prefixed.qualified_name(), "p:a");
    assert_eq!(prefixed.value, "one");
    let plain = dom.attribute_ns(root, None, "a").unwrap();
    assert_eq!(plain.namespace_uri, None);
    assert_eq!(plain.prefix, None);
    assert_eq!(plain.qualified_name(), "a");
    assert_eq!(plain.value, "two");
    let binding = dom.attribute_ns(root, Some(XMLNS_NAMESPACE), "p").unwrap();
    assert_eq!(binding.prefix, Some("xmlns"));
    assert_eq!(binding.qualified_name(), "xmlns:p");
    let default = dom
        .attribute_ns(root, Some(XMLNS_NAMESPACE), "xmlns")
        .unwrap();
    assert_eq!(default.prefix, None);
    assert_eq!(default.qualified_name(), "xmlns");
    let lang = dom.attribute_ns(root, Some(XML_NAMESPACE), "lang").unwrap();
    assert_eq!(lang.prefix, Some("xml"));
    assert_eq!(lang.qualified_name(), "xml:lang");
    assert_eq!(lang.value, "en");
}

#[test]
fn xmlns_is_valid_as_an_element_local_name() {
    for (source, uri, prefix, qualified) in [
        ("<xmlns/>", None, None, "xmlns"),
        ("<xmlns xmlns='urn:x'/>", Some("urn:x"), None, "xmlns"),
        (
            "<p:xmlns xmlns:p='urn:y'/>",
            Some("urn:y"),
            Some("p"),
            "p:xmlns",
        ),
    ] {
        let dom = parse_xml(source).unwrap();
        let root = elements(&dom)[0];
        assert_eq!(dom.local_name(root), Some("xmlns"));
        assert_eq!(dom.namespace_uri(root), uri);
        assert_eq!(dom.prefix(root), prefix);
        assert_eq!(dom.qualified_name(root).as_deref(), Some(qualified));
    }
    malformed("<xmlns:r/>");
    malformed("<r xmlns:xmlns='urn:bad'/>");
}

#[test]
fn version_info_and_declaration_family_follow_xml_fifth_edition() {
    for version in ["1.0", "1.1", "1.2", "1.00", "1.999999999999999999999"] {
        for quote in ['\'', '"'] {
            let source = format!("<?xml\tversion\r\n= {quote}{version}{quote} encoding = 'UTF-8' standalone='yes' ?><r>&#x85;</r>");
            let (dom, declaration) = parse_xml_with_declaration(&source).unwrap();
            assert_eq!(text(&dom), "\u{85}"); // XML 1.0 does not normalize NEL.
            let declaration = declaration.unwrap();
            assert_eq!(declaration.encoding.as_deref(), Some("UTF-8"));
            assert_eq!(declaration.standalone, Some(true));
            let decoded = crate::xml_encoding::decode(source.as_bytes(), None).unwrap();
            assert_eq!(decoded.encoding, amazing_encoding::Encoding::Utf8);
            assert!(parse_xml_decoded(&decoded.text).is_ok());
        }
    }
    for version in ["", "1", "1.", "1.a", "01.0", "0.1", "2.0", "1.٠", "1.0x", "1.0 ", " 1.0", "1..0"] {
        let source = format!("<?xml version='{version}'?><r/>");
        malformed(&source);
        assert!(crate::xml_encoding::decode(source.as_bytes(), None).is_err());
    }
    for source in [
        "<?xml version='1.2' version='1.2'?><r/>",
        "<?xml Version='1.2'?><r/>",
        "<?xml version='1.2'encoding='UTF-8'?><r/>",
        "<?xml version='1.2' standalone='yes' encoding='UTF-8'?><r/>",
        "<?xml version='1.2' encoding='UTF-8' standalone='true'?><r/>",
        "<?xml version='1.2' encoding='UTF 8'?><r/>",
        "<?xml version='1.2' encoding='é'?><r/>",
        "<?xml version='1.2' standalone='yes' standalone='no'?><r/>",
        " <?xml version='1.2'?><r/>",
        "<!--pre--><?xml version='1.2'?><r/>",
        "<r/><?xml version='1.2'?>",
        "<?xml version='1.1'?><r>&#x1;</r>",
    ] { malformed(source); }
    assert!(parse_xml_fragment(&XmlFragmentContext::default(), "<?xml version='1.2'?><r/>").is_err());
}

#[test]
fn xml_cdata_preserves_each_lexical_section_and_ordinary_text_boundaries() {
    let dom = parse_xml("<r xmlns='urn:actual'>before<![CDATA[]]><![CDATA[<x>&unknown;\r\n]]><![CDATA[more]]>after</r>").unwrap();
    let root = dom.children(NodeIdx(0)).next().unwrap();
    assert_eq!(dom.namespace_uri(root), Some("urn:actual"));
    let children: Vec<_> = dom.children(root).collect();
    assert_eq!(children.len(), 5);
    for (node, expected) in children.iter().zip([
        NodeKind::Text { data: "before".into() },
        NodeKind::CDATASection { data: "".into() },
        NodeKind::CDATASection { data: "<x>&unknown;\n".into() },
        NodeKind::CDATASection { data: "more".into() },
        NodeKind::Text { data: "after".into() },
    ]) { assert_eq!(dom.get(*node).kind, expected); assert_eq!(dom.get(*node).parent, Some(root)); }
    assert_eq!(text(&dom), "before<x>&unknown;\nmoreafter");
    let context = XmlFragmentContext::default();
    let fragment = parse_xml_fragment(&context, "a<![CDATA[]]><![CDATA[<&not-an-entity;]]>b").unwrap();
    let nodes: Vec<_> = fragment.children(NodeIdx(0)).collect();
    assert_eq!(nodes.len(), 4);
    assert!(matches!(&fragment.get(nodes[1]).kind, NodeKind::CDATASection { data } if data.as_str().is_empty()));
    assert!(matches!(&fragment.get(nodes[2]).kind, NodeKind::CDATASection { data } if data.as_str() == "<&not-an-entity;"));
    let large = "<>&not-an-entity;".repeat(1024);
    let dom = parse_xml(&format!("<r><![CDATA[{large}]]></r>")).unwrap();
    assert_eq!(text(&dom), large);
    for source in ["<![CDATA[x]]><r/>", "<r/><![CDATA[x]]>", "<r><![CDATA[x</r>",
        "<r><![cdata[x]]></r>", "<r><![CDATA[x]]>]]></r>", "<r><![CDATA[\0]]></r>"] { malformed(source); }
}


fn assert_xml_node_documents(dom: &Dom, node: NodeIdx, document: NodeIdx) {
    assert_eq!(dom.owner_document(node), Some(document), "{:?}", dom.get(node).kind);
    for child in dom.children(node) { assert_xml_node_documents(dom, child, document); }
    if let Some(contents) = dom.template_contents(node) {
        assert_eq!(dom.children(node).count(), 0);
        let inert = dom.owner_document(contents).unwrap();
        assert_ne!(inert, NodeIdx(0));
        assert_eq!(dom.document_metadata(inert).unwrap().unwrap().content_type, "application/xml");
        assert!(!dom.document_scripting_enabled(inert).unwrap());
        assert_xml_node_documents(dom, contents, inert);
    }
}

#[test]
fn xml_document_templates_use_actual_inert_contents_and_node_documents() {
    let source = "<?before data?><r xmlns:h='http://www.w3.org/1999/xhtml'><h:template id='t'>text<![CDATA[<&literal;]]><!--comment--><?inside data?><x a='v'/><h:template id='nested'>inner</h:template><h:template id='empty'/></h:template><h:TEMPLATE>ordinary</h:TEMPLATE><template xmlns='urn:foreign'><x/></template></r>";
    let dom = parse_xml(source).unwrap();
    assert_eq!(dom.document_metadata(NodeIdx(0)).unwrap().unwrap().content_type, "application/xml");
    for child in dom.children(NodeIdx(0)) { assert_xml_node_documents(&dom, child, NodeIdx(0)); }
    let template = elements(&dom).into_iter().find(|node| dom.get_attribute_ns(*node, None, "id") == Some("t")).unwrap();
    let contents = dom.template_contents(template).unwrap();
    let children: Vec<_> = dom.children(contents).collect();
    assert_eq!(children.len(), 7);
    assert!(matches!(&dom.get(children[0]).kind, NodeKind::Text { data } if data.as_str() == "text"));
    assert!(matches!(&dom.get(children[1]).kind, NodeKind::CDATASection { data } if data.as_str() == "<&literal;"));
    assert!(matches!(&dom.get(children[2]).kind, NodeKind::Comment { data } if data.as_str() == "comment"));
    assert!(matches!(&dom.get(children[3]).kind, NodeKind::ProcessingInstruction { target, data }
        if target == "inside" && data.as_str() == "data"));
    let document = dom.owner_document(contents).unwrap();
    for node in [children[5],children[6]] {
        assert_eq!(dom.owner_document(dom.template_contents(node).unwrap()), Some(document));
    }
    assert_eq!(dom.children(dom.template_contents(children[6]).unwrap()).count(), 0);
    for node in elements(&dom).into_iter().filter(|node| dom.local_name(*node) == Some("TEMPLATE")
        || dom.namespace_uri(*node) == Some("urn:foreign")) {
        assert_eq!(dom.template_contents(node), None);
        assert_eq!(dom.owner_document(node), Some(NodeIdx(0)));
    }
    for source in ["<template xmlns='http://www.w3.org/1999/xhtml'/>",
        "<h:template xmlns:h='http://www.w3.org/1999/xhtml'></h:template>"] {
        let dom = parse_xml(source).unwrap();
        let root = dom.children(NodeIdx(0)).next().unwrap();
        let contents = dom.template_contents(root).unwrap();
        assert_eq!(dom.children(contents).count(), 0);
        assert_xml_node_documents(&dom, root, NodeIdx(0));
    }
}

#[test]
fn xml_fragment_character_data_in_nested_templates_inherits_actual_insertion_document() {
    let context = XmlFragmentContext { namespace_uri: Some(HTML_NAMESPACE.into()), prefix: None,
        namespaces: vec![(None, HTML_NAMESPACE.into())] };
    let dom = parse_xml_fragment(&context, "before<template><!--x--><?p y?><![CDATA[z]]><template>inner</template><span a='v'/></template>after").unwrap();
    for child in dom.children(NodeIdx(0)) { assert_xml_node_documents(&dom, child, NodeIdx(0)); }
    let top: Vec<_> = dom.children(NodeIdx(0)).collect();
    assert_eq!(top.len(), 3);
    let contents = dom.template_contents(top[1]).unwrap();
    assert_eq!(dom.children(contents).count(), 5);
}


#[test]
fn canonical_xhtml_public_identifiers_select_complete_character_entity_subset() {
    for public_id in XHTML_PUBLIC_IDS {
        let source = format!("<!DOCTYPE r PUBLIC '{public_id}' 'https://never-fetch.invalid/a.dtd'><r a='&COPY;&LT;&Tab;&NewLine;&#9;'>&copy;&NotEqualTilde;&AMP;&LT;&Tab;&NewLine;</r>");
        let dom = parse_xml(&source).unwrap();
        assert_eq!(text(&dom), "©≂̸&<\t\n");
        let root = elements(&dom)[0];
        assert_eq!(dom.get_attribute_ns(root, None, "a"), Some("©<  \t"));
        let doctype = dom.children(NodeIdx(0)).next().unwrap();
        assert!(matches!(&dom.get(doctype).kind, NodeKind::DocumentType { public_id: id, system_id, .. }
            if id == *public_id && system_id == "https://never-fetch.invalid/a.dtd"));
    }
    let id = "  -//W3C//DTD\nXHTML 1.0   Strict//EN  ";
    let dom = parse_xml(&format!("<!DOCTYPE r PUBLIC '{id}' 'unloaded.dtd'><r>&copy;</r>")).unwrap();
    assert_eq!(text(&dom), "©");
}

#[test]
fn canonical_subset_preserves_every_standard_name_and_scalar_sequence() {
    for (name, expected) in crate::entities::NAMED {
        let source = format!("<!DOCTYPE r PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'unloaded.dtd'><r>{name}</r>");
        let dom = parse_xml(&source).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(text(&dom), *expected, "{name}");
    }
}

#[test]
fn xhtml_entity_subset_never_comes_from_mime_namespace_system_url_or_fragment_context() {
    for source in ["<html xmlns='http://www.w3.org/1999/xhtml'>&copy;</html>",
        "<!DOCTYPE r PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'unloaded.dtd'><r>&Copy;</r>",
        "<?xml version='1.0' standalone='yes'?><!DOCTYPE r PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'unloaded.dtd'><r>&copy;</r>"] {
        malformed(source);
    }
    for public_id in ["-//W3C//DTD XHTML 1.0 strict//EN", "-//W3C//DTD SVG 1.1//EN", "unknown"] {
        let source = format!("<!DOCTYPE r PUBLIC '{public_id}' 'unloaded.dtd'><r>&copy;</r>");
        assert_eq!(parse_xml(&source).unwrap_err().kind, XmlErrorKind::Unsupported);
    }
    assert_eq!(parse_xml("<!DOCTYPE r SYSTEM 'https://html.spec.whatwg.org/entities.dtd'><r>&copy;</r>")
        .unwrap_err().kind, XmlErrorKind::Unsupported);
    assert_eq!(text(&parse_xml("<!DOCTYPE r PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'unloaded.dtd' [<!ENTITY x 'y'>]><r>&x;&copy;</r>").unwrap()), "y©");
    let dom = parse_xml("<!DOCTYPE r PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'unloaded.dtd'><r><![CDATA[&copy;]]><!--&copy;--><?p &copy;?></r>").unwrap();
    assert_eq!(text(&dom), "&copy;");
    assert_eq!(parse_xml_fragment(&XmlFragmentContext::default(), "&copy;").unwrap_err().kind, XmlErrorKind::Malformed);
}

#[test]
fn internal_dtd_declaration_grammar_and_nonvalidating_defaults() {
    let source = "<!DOCTYPE r [<!ELEMENT r (a,(b|c)+,d?)><!ELEMENT a EMPTY><!ELEMENT b ANY><!ELEMENT c (#PCDATA|a)*><!ELEMENT d (#PCDATA)><!NOTATION image PUBLIC 'image format'><!ENTITY logo SYSTEM 'logo.bin' NDATA image><!ATTLIST r plain CDATA 'first' plain CDATA 'later' token NMTOKENS '  x   y ' fixed CDATA #FIXED 'default' required ID #REQUIRED implied IDREF #IMPLIED choice (yes|no) 'yes' picture NOTATION (image) #IMPLIED><!--DTD--><?dtd test?>]><r fixed='explicit' choice='not-enumerated'/>";
    let dom = parse_xml(source).unwrap();
    let root = elements(&dom)[0];
    for (name, value) in [("plain", "first"), ("token", "x y"), ("fixed", "explicit"), ("choice", "not-enumerated")] {
        assert_eq!(dom.get_attribute_ns(root, None, name), Some(value));
    }
    assert_eq!(dom.get_attribute_ns(root, None, "required"), None);
    assert_eq!(dom.get_attribute_ns(root, None, "implied"), None);
    // Content validity and root/type correspondence are not WF constraints.
    parse_xml("<!DOCTYPE unrelated [<!ELEMENT unrelated EMPTY>]><r>text</r>").unwrap();
}

#[test]
fn internal_dtd_entity_phases_quotes_whitespace_and_first_binding() {
    let dom = parse_xml("<!DOCTYPE r [<!ENTITY value 'first'><!ENTITY value 'later'><!ENTITY quote '&#34;'><!ENTITY generated '&#13;&#9;'><!ENTITY escaped '&#38;#60;'><!ATTLIST r a NMTOKENS #IMPLIED>]><r a='  x &generated; &#9; &quote; &escaped;  '>&value;&generated;&escaped;</r>").unwrap();
    let root = elements(&dom)[0];
    assert_eq!(dom.get_attribute_ns(root, None, "a"), Some("x \t \" <"));
    assert_eq!(String::from_utf16(&dom.inner_text_approx(root).unwrap()).unwrap(), "first\r\t<");
    malformed("<!DOCTYPE r [<!ENTITY literal '&#60;'>]><r a='&literal;'/>");
    malformed("<!DOCTYPE r [<!ENTITY unused '&missing;'>]><r/>");
    malformed("<!DOCTYPE r [<!ENTITY external SYSTEM 'unloaded'>]><r a='&external;'/>");
    malformed("<!DOCTYPE r [<!ATTLIST r a CDATA '&later;'><!ENTITY later 'x'>]><r/>");
    for declaration in ["<!ENTITY lt '<'>", "<!ENTITY amp '&#38;'>", "<!ENTITY gt 'x'>"] {
        malformed(&format!("<!DOCTYPE r [{declaration}]><r/>"));
    }
    parse_xml("<!DOCTYPE r [<!ENTITY lt '&#38;#60;'><!ENTITY amp '&#38;#38;'><!ENTITY gt '&#62;'><!ENTITY apos '\"'><!ENTITY quot '&#34;'>]><r/>").expect_err("wrong apos redeclaration");
}

#[test]
fn internal_dtd_defaults_bind_namespaces_before_expanded_names() {
    let dom = parse_xml("<!DOCTYPE p:r [<!ENTITY uri 'urn:actual'><!ATTLIST p:r xmlns:p CDATA '&uri;' p:a CDATA 'value' xmlns CDATA 'urn:default'>]><p:r><child/></p:r>").unwrap();
    let nodes = elements(&dom);
    assert_eq!(dom.namespace_uri(nodes[0]), Some("urn:actual"));
    assert_eq!(dom.namespace_uri(nodes[1]), Some("urn:default"));
    assert_eq!(dom.get_attribute_ns(nodes[0], Some("urn:actual"), "a"), Some("value"));
    malformed("<!DOCTYPE r [<!ATTLIST r xmlns:xml CDATA 'urn:wrong'>]><r/>");
    malformed("<!DOCTYPE r [<!ATTLIST r xmlns:a CDATA 'urn:x' xmlns:b CDATA 'urn:x' a:c CDATA 'x'>]><r b:c='y'/>");
}

#[test]
fn internal_dtd_content_entities_have_actual_balanced_xml_nodes() {
    let dom = parse_xml("<!DOCTYPE r [<!ENTITY inner '<p:a><![CDATA[&#38;literal;]]></p:a>'><!ENTITY outer 'before&inner;after'>]><r xmlns:p='urn:x'>start&outer;end</r>").unwrap();
    let nodes = elements(&dom);
    assert_eq!(dom.namespace_uri(nodes[1]), Some("urn:x"));
    let children: Vec<_> = dom.children(nodes[0]).collect();
    assert_eq!(children.len(), 3);
    assert!(matches!(&dom.get(children[0]).kind, NodeKind::Text { data } if data == "startbefore"));
    assert!(matches!(&dom.get(children[2]).kind, NodeKind::Text { data } if data == "afterend"));
    let cdata = dom.children(nodes[1]).next().unwrap();
    assert!(matches!(&dom.get(cdata).kind, NodeKind::CDATASection { data } if data == "&literal;"));
    assert_eq!(dom.owner_document(cdata), Some(NodeIdx(0)));
    for body in ["</r>", "<a>", "<?xml version='1.0'?>", "<!DOCTYPE x>", "]]>"] {
        malformed(&format!("<!DOCTYPE r [<!ENTITY e \"{body}\">]><r>&e;</r>"));
    }
}

#[test]
fn internal_parameter_entities_are_complete_declaration_sequences() {
    let source = "<!DOCTYPE r [<!ENTITY % declarations \"<!ENTITY value 'good'><!ATTLIST r a CDATA 'default'>\">%declarations;]><r>&value;</r>";
    let dom = parse_xml(source).unwrap();
    let root = elements(&dom)[0];
    assert_eq!(String::from_utf16(&dom.inner_text_approx(root).unwrap()).unwrap(), "good");
    assert_eq!(dom.get_attribute_ns(root, None, "a"), Some("default"));
    for subset in ["<!ENTITY % p 'EMPTY'><!ELEMENT r %p;>", "<!ENTITY % p 'x'><!ENTITY e '%p;'>", "<!ENTITY % p '<!ELEMENT'>%p; r EMPTY>", "<!ENTITY % p '&#37;p;'>%p;"] {
        malformed(&format!("<!DOCTYPE r [{subset}]><r/>"));
    }
    let standalone = format!("<?xml version='1.0' standalone='yes'?>{source}");
    malformed(&standalone);
    // A later direct duplicate satisfies the direct-declaration WF rule without
    // replacing the value from the first parameter-supplied declaration.
    let source = source.replace("]><r>", "<!ENTITY value 'later'>]><r>");
    let dom = parse_xml(&format!("<?xml version='1.0' standalone='yes'?>{source}")).unwrap();
    assert_eq!(String::from_utf16(&dom.inner_text_approx(elements(&dom)[0]).unwrap()).unwrap(), "good");
}

#[test]
fn internal_dtd_malformed_families_and_external_boundaries() {
    for subset in ["<!ELEMENT r>", "<!ELEMENT r (a|b,c)>", "<!ELEMENT r (#PCDATA|a)>", "<!ELEMENT r (a,)>", "<!ATTLIST r a UNKNOWN 'x'>", "<!ATTLIST r a (x|) 'x'>", "<!ATTLIST r a CDATA #FIXED>", "<!NOTATION n PUBLIC 'bad^'>", "<!ENTITY e SYSTEM 'x'NDATA n>", "<!ENTITY % e SYSTEM 'x' NDATA n>", "<![IGNORE[x]]>", "<!--a--b-->", "<?xml bad?>"] {
        malformed(&format!("<!DOCTYPE r [{subset}]><r/>"));
    }
    for subset in ["<!ENTITY e SYSTEM 'unloaded'>", "<!ENTITY e PUBLIC 'public' 'unloaded'>"] {
        assert_eq!(parse_xml(&format!("<!DOCTYPE r [{subset}]><r>&e;</r>")).unwrap_err().kind, XmlErrorKind::Unsupported);
        parse_xml(&format!("<!DOCTYPE r [{subset}]><r/>")).unwrap();
    }
    malformed("<!DOCTYPE r [<!NOTATION n SYSTEM 'n'><!ENTITY e SYSTEM 'x' NDATA n>]><r>&e;</r>");
    assert_eq!(parse_xml("<!DOCTYPE r [<!ENTITY % e SYSTEM 'unloaded'>%e;]><r/>").unwrap_err().kind, XmlErrorKind::Unsupported);
    assert_eq!(parse_xml("<!DOCTYPE r [<!ENTITY a '&b;'><!ENTITY b '&a;'>]><r>&a;</r>").unwrap_err().kind, XmlErrorKind::Malformed);
    let mut subset = String::new();
    for n in 0..34 { subset.push_str(&format!("<!ENTITY e{n} '&e{};'>", n+1)); }
    subset.push_str("<!ENTITY e34 'end'>");
    assert_eq!(parse_xml(&format!("<!DOCTYPE r [{subset}]><r>&e0;</r>")).unwrap_err().kind, XmlErrorKind::Limit);
}


#[test]
fn internal_dtd_entities_keep_template_contents_and_public_fragment_boundaries() {
    let dom = parse_xml("<!DOCTYPE r [<!ENTITY e '<h:template>x<![CDATA[y]]><h:template/></h:template>'>]><r xmlns:h='http://www.w3.org/1999/xhtml'>&e;</r>").unwrap();
    let root = dom.children(NodeIdx(0)).find(|node| dom.name(*node) == Some("r")).unwrap();
    let template = dom.children(root).next().unwrap();
    assert_eq!(dom.children(template).count(), 0);
    let contents = dom.template_contents(template).unwrap();
    let owner = dom.owner_document(contents).unwrap();
    assert_ne!(owner, NodeIdx(0));
    let nodes: Vec<_> = dom.children(contents).collect();
    assert_eq!(nodes.len(), 3);
    assert!(matches!(&dom.get(nodes[1]).kind, NodeKind::CDATASection { data } if data == "y"));
    for node in nodes { assert_eq!(dom.owner_document(node), Some(owner)); }
    for parser in [parse_xml as fn(&str) -> Result<Dom, XmlError>, parse_xml_decoded] {
        let dom = parser("<!DOCTYPE r [<!ENTITY e 'works'>]><r>&e;</r>").unwrap();
        assert_eq!(text(&dom), "works");
    }
    malformed("<!DOCTYPE r [<!ENTITY unused '&unused;'>]><r/>");
    assert_eq!(parse_xml_fragment(&XmlFragmentContext::default(), "&e;").unwrap_err().kind, XmlErrorKind::Malformed);
    assert_eq!(parse_xml_fragment(&XmlFragmentContext::default(), "<!DOCTYPE r [<!ENTITY e 'x'>]><r/>").unwrap_err().kind, XmlErrorKind::Malformed);
}


#[test]
fn internal_entity_recursion_checks_ignore_actual_literal_markup_regions() {
    let literal = parse_xml("<!DOCTYPE r [<!ENTITY e '<![CDATA[&missing;]]><!--&missing;--><?p &missing;?>'>]><r>&e;</r>").unwrap();
    assert_eq!(text(&literal), "&missing;");
    for body in ["<![CDATA[&#38;e;]]>", "<!--&#38;e;-->", "<?p &#38;e;?>"] {
        let dom = parse_xml(&format!("<!DOCTYPE r [<!ENTITY e '{body}'>]><r>&e;</r>")).unwrap();
        let root = elements(&dom)[0];
        assert_eq!(dom.children(root).count(), 1);
    }
}
