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
            NodeKind::Text { data } => Some(data.as_str()),
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
    assert_eq!(
        parse_xml("<?xml version='1.1'?><r/>").unwrap_err().kind,
        XmlErrorKind::Unsupported
    );
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
    assert_eq!(
        parse_xml("<!DOCTYPE r [<!ENTITY a 'x'>]><r>&a;</r>")
            .unwrap_err()
            .kind,
        XmlErrorKind::Unsupported
    );
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
