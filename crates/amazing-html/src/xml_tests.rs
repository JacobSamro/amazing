use super::*;

#[derive(Default)]
struct Recording {
    elements: Vec<(usize, XmlName, Vec<XmlAttribute>)>,
    text: Vec<String>,
    comments: Vec<String>,
    pis: Vec<(String, String)>,
    doctypes: Vec<(String, String, String)>,
}
impl XmlTreeSink for Recording {
    type Node = usize;
    fn document(&self) -> usize {
        0
    }
    fn element(
        &mut self,
        parent: usize,
        name: &XmlName,
        attrs: &[XmlAttribute],
    ) -> Result<usize, String> {
        self.elements.push((parent, name.clone(), attrs.to_vec()));
        Ok(self.elements.len())
    }
    fn text(&mut self, _: usize, data: &str) -> Result<(), String> {
        self.text.push(data.into());
        Ok(())
    }
    fn comment(&mut self, _: usize, data: &str) -> Result<(), String> {
        self.comments.push(data.into());
        Ok(())
    }
    fn processing_instruction(&mut self, _: usize, target: &str, data: &str) -> Result<(), String> {
        self.pis.push((target.into(), data.into()));
        Ok(())
    }
    fn doctype(&mut self, name: &str, public: &str, system: &str) -> Result<(), String> {
        self.doctypes
            .push((name.into(), public.into(), system.into()));
        Ok(())
    }
}
fn record(src: &str) -> Recording {
    let mut sink = Recording::default();
    parse_xml_into(src, &mut sink).unwrap();
    sink
}
fn malformed(src: &str) {
    let error = parse_xml_into(src, &mut Recording::default()).expect_err(src);
    assert_eq!(error.kind, XmlErrorKind::Malformed, "{src}: {error}");
}

#[test]
fn document_structure_is_xml_not_html() {
    let r = record(" <!--before--> <Root><P/><P>text</P></Root> <?tail?> ");
    assert_eq!(
        r.elements
            .iter()
            .map(|e| e.1.local.as_str())
            .collect::<Vec<_>>(),
        ["Root", "P", "P"]
    );
    assert_eq!(
        r.elements.iter().map(|e| e.0).collect::<Vec<_>>(),
        [0, 1, 1]
    );
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
    let r = record("<r a='one' b=\"two\"/>");
    assert_eq!(r.elements[0].2[0].value, "one");
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
    let r = record(
        "<r a='x\t\r\ny&#9;&#10;&#13;&amp;'>&lt;&gt;&amp;&quot;&apos;&#65;&#x1F642; &amp;lt;</r>",
    );
    assert_eq!(r.elements[0].2[0].value, "x  y\t\n\r&");
    assert_eq!(r.text.concat(), "<>&\"'A🙂 &lt;");
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
    let r = record("<r>a<![CDATA[<b>&unknown;\r\n]]>z</r>");
    assert_eq!(r.text, ["a", "<b>&unknown;\n", "z"]);
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
    let r = record("<?xml-stylesheet href='a.css'?><!--ok--><r><?pi a > & b?><!--x--></r>");
    assert_eq!(r.comments, ["ok", "x"]);
    assert_eq!(
        r.pis,
        [
            ("xml-stylesheet".into(), "href='a.css'".into()),
            ("pi".into(), "a > & b".into())
        ]
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
    let mut sink = Recording::default();
    let d = parse_xml_into(
        "\u{feff}<?xml version = '1.0' encoding='UTF-16' standalone='yes'?><r/>",
        &mut sink,
    )
    .unwrap()
    .unwrap();
    assert_eq!(d.encoding.as_deref(), Some("UTF-16"));
    assert_eq!(d.standalone, Some(true));
    assert!(record("<?xml version='1.0'?><r/>").pis.is_empty());
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
        parse_xml_into("<?xml version='1.1'?><r/>", &mut sink)
            .unwrap_err()
            .kind,
        XmlErrorKind::Unsupported
    );
}

#[test]
fn namespace_defaults_prefix_shadowing_and_scope_restoration() {
    let r = record("<r xmlns='urn:a' xmlns:p='urn:outer'><p:x p:a='1' a='2'/><inner xmlns='urn:b' xmlns:p='urn:inner'><p:x/></inner><p:x/><plain xmlns=''/></r>");
    let uris: Vec<_> = r
        .elements
        .iter()
        .map(|e| e.1.namespace_uri.as_deref())
        .collect();
    assert_eq!(
        uris,
        [
            Some("urn:a"),
            Some("urn:outer"),
            Some("urn:b"),
            Some("urn:inner"),
            Some("urn:outer"),
            None
        ]
    );
    let attrs = &r.elements[1].2;
    assert_eq!(attrs[0].name.namespace_uri.as_deref(), Some("urn:outer"));
    assert_eq!(attrs[0].name.prefix.as_deref(), Some("p"));
    assert_eq!(attrs[1].name.namespace_uri, None);
    assert_eq!(
        r.elements[0].2[0].name.namespace_uri.as_deref(),
        Some(XMLNS_NAMESPACE)
    );
    let r = record("<p:r p:a='1' xmlns:p='urn:p' xml:lang='en'/>");
    assert_eq!(r.elements[0].1.namespace_uri.as_deref(), Some("urn:p"));
    assert_eq!(
        r.elements[0].2[2].name.namespace_uri.as_deref(),
        Some(XML_NAMESPACE)
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
    record("<r xmlns:xml='http://www.w3.org/XML/1998/namespace' xmlns:a='urn:x' xmlns:b='urn:y' a:n='1' b:n='2' n='3'/>");
}

#[test]
fn xml_name_unicode_ranges_and_case_are_preserved() {
    record("<é:根 xmlns:é='urn:x' a·b='1'><𐀀/></é:根>");
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
    let r = record("<!DOCTYPE html PUBLIC '-//W3C//DTD XHTML 1.0 Strict//EN' 'http://example.invalid/external.dtd'><html/>");
    assert_eq!(r.doctypes[0].2, "http://example.invalid/external.dtd");
    record("<!DOCTYPE r SYSTEM 'file:///not-read'><r/>");
    for src in [
        "<!DOCTYPEr><r/>",
        "<!DOCTYPE r><!DOCTYPE r><r/>",
        "<r/><!DOCTYPE r>",
        "<!DOCTYPE r PUBLIC 'bad^' 'sys'><r/>",
        "<!DOCTYPE r SYSTEM><r/>",
    ] {
        malformed(src);
    }
    let e = parse_xml_into(
        "<!DOCTYPE r [<!ENTITY a 'x'>]><r>&a;</r>",
        &mut Recording::default(),
    )
    .unwrap_err();
    assert_eq!(e.kind, XmlErrorKind::Unsupported);
}

#[test]
fn fatal_error_stops_sink_callbacks_and_dom_is_not_returned() {
    let mut r = Recording::default();
    assert!(parse_xml_into("<r><a></wrong><never/></r>", &mut r).is_err());
    assert_eq!(r.elements.len(), 2);
    assert!(parse_xml("<html xmlns='http://www.w3.org/1999/xhtml'><body></html>").is_err());
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
    assert!(dom.dump(NodeIdx(0)).contains("a < b && c"));
}

#[test]
fn depth_is_bounded_without_recursive_parser_stack() {
    let src = format!("{}{}", "<r>".repeat(513), "</r>".repeat(513));
    assert_eq!(
        parse_xml_into(&src, &mut Recording::default())
            .unwrap_err()
            .kind,
        XmlErrorKind::Limit
    );
}

#[test]
fn empty_element_namespace_declarations_do_not_leak_to_siblings() {
    let r = record("<r xmlns='urn:outer' xmlns:p='urn:a'><p:x xmlns:p='urn:b'/><p:x/><plain xmlns=''/><again/></r>");
    let uris: Vec<_> = r
        .elements
        .iter()
        .map(|e| e.1.namespace_uri.as_deref())
        .collect();
    assert_eq!(
        uris,
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
    record("<r>\u{85}\u{2028}\u{1fffe}</r>");
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
    let r = record("<r>\r\nx\ry</r>");
    assert_eq!(r.text, ["\nx\ny"]);
}

#[test]
fn attributes_are_bounded_before_growing_unbounded_sink_state() {
    let mut src = String::from("<r");
    for n in 0..1025 {
        src.push_str(&format!(" a{n}='x'"));
    }
    src.push_str("/>");
    let mut sink = Recording::default();
    assert_eq!(
        parse_xml_into(&src, &mut sink).unwrap_err().kind,
        XmlErrorKind::Limit
    );
    assert!(sink.elements.is_empty());
}

#[test]
fn unloaded_external_entities_are_unsupported_not_silently_recovered() {
    let src = "<!DOCTYPE r SYSTEM 'unloaded.dtd'><r>&nbsp;</r>";
    assert_eq!(
        parse_xml_into(src, &mut Recording::default())
            .unwrap_err()
            .kind,
        XmlErrorKind::Unsupported
    );
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
