use amazing_dom::{Namespace, NodeIdx, NodeKind, QuirksMode};
use crate::{HtmlFragmentAttribute, HtmlFragmentContext, XmlFragmentContext, parse_fragment_with_context, parse_xml_fragment};

fn context(name: &str, uri: Option<&str>) -> HtmlFragmentContext {
    HtmlFragmentContext { local_name: name.into(), namespace_uri: uri.map(str::to_owned),
        attributes: Vec::new(), has_form_ancestor: false, scripting_enabled: true,
        quirks_mode: QuirksMode::NoQuirks }
}
fn parse(context: &HtmlFragmentContext, src: &str) -> amazing_dom::Dom {
    parse_fragment_with_context(context, src, false).unwrap()
}
#[test]
fn actual_context_namespace_and_case_survive_arena_export() {
    for uri in [None, Some("urn:actual"), Some(crate::xml::SVG_NAMESPACE)] {
        let dom = parse(&context("table", uri), "<widget a='x'/><other/>");
        let children: Vec<_> = dom.children(NodeIdx(0)).collect();
        assert_eq!(children.len(), 2, "{uri:?}");
        for node in children { assert_eq!(dom.namespace_uri(node), uri); }
    }
    let html = Some(crate::xml::HTML_NAMESPACE);
    let upper = parse(&context("TITLE", html), "<b>x</b>");
    let node = upper.children(NodeIdx(0)).next().unwrap();
    assert_eq!(upper.local_name(node), Some("b"));
    let lower = parse(&context("title", html), "<b>x</b>");
    assert!(matches!(&lower.get(lower.children(NodeIdx(0)).next().unwrap()).kind,
        NodeKind::Text { data } if data == "<b>x</b>"));
    let integration = parse(&context("foreignObject", Some(crate::xml::SVG_NAMESPACE)), "<widget/>");
    let node = integration.children(NodeIdx(0)).next().unwrap();
    assert_eq!(integration.namespace(node), Some(Namespace::Html));
    let nonintegration = parse(&context("foreignobject", Some(crate::xml::SVG_NAMESPACE)), "<widget/>");
    let node = nonintegration.children(NodeIdx(0)).next().unwrap();
    assert_eq!(nonintegration.namespace(node), Some(Namespace::Svg));
}
#[test]
fn actual_attributes_form_and_document_modes_control_fragment_rules() {
    let mut math = context("annotation-xml", Some(crate::xml::MATHML_NAMESPACE));
    math.attributes.push(HtmlFragmentAttribute { namespace_uri: None, prefix: None,
        local_name: "encoding".into(), value: "TeXt/HtMl".encode_utf16().collect() });
    // A real DOMString attribute can include an unpaired surrogate.
    math.attributes.push(HtmlFragmentAttribute { namespace_uri: Some("urn:attr".into()), prefix: Some("a".into()),
        local_name: "data".into(), value: vec![0xd800] });
    let dom = parse(&math, "<widget/>");
    assert_eq!(dom.namespace(dom.children(NodeIdx(0)).next().unwrap()), Some(Namespace::Html));
    math.attributes[0].namespace_uri = Some("urn:wrong".into());
    math.attributes[0].prefix = Some("a".into());
    let dom = parse(&math, "<widget/>");
    assert_eq!(dom.namespace(dom.children(NodeIdx(0)).next().unwrap()), Some(Namespace::MathMl));
    let mut body = context("body", Some(crate::xml::HTML_NAMESPACE));
    body.has_form_ancestor = true;
    let dom = parse(&body, "<form><input></form>");
    assert_eq!(dom.local_name(dom.children(NodeIdx(0)).next().unwrap()), Some("input"));
    body.has_form_ancestor = false;
    assert_eq!(parse(&body, "<form><input></form>").local_name(NodeIdx(1)), Some("form"));
    body.quirks_mode = QuirksMode::Quirks;
    let quirks = parse(&body, "<p>x<table><tr><td>y");
    assert_eq!(quirks.document_quirks_mode(NodeIdx(0)).unwrap(), QuirksMode::Quirks);
    let p = quirks.children(NodeIdx(0)).next().unwrap();
    assert!(quirks.children(p).any(|node| quirks.local_name(node) == Some("table")));
    body.quirks_mode = QuirksMode::NoQuirks;
    let standards = parse(&body, "<p>x<table><tr><td>y");
    assert!(standards.children(NodeIdx(0)).any(|node| standards.local_name(node) == Some("table")));
    body.local_name = "noscript".into();
    let enabled = parse(&body, "<b>x</b>");
    assert!(enabled.children(NodeIdx(0)).all(|node| matches!(enabled.get(node).kind, NodeKind::Text { .. })));
    body.scripting_enabled = false;
    let dom = parse(&body, "<b>x</b>");
    assert_eq!(dom.local_name(dom.children(NodeIdx(0)).next().unwrap()), Some("b"));
    assert!(!dom.document_scripting_enabled(NodeIdx(0)).unwrap());
}

#[test]
fn template_fragment_form_rules_ignore_outer_form_pointer_but_preserve_namespace_and_case() {
    let html = Some(crate::xml::HTML_NAMESPACE);
    for ancestor in [false, true] {
        let mut template = context("template", html);
        template.has_form_ancestor = ancestor;
        for source in ["<div><form></div><input type=text></form>",
            "<table><form id=first></form><form id=second></form></table>",
            "<form id=first><form id=second></form></form>"] {
            let dom = parse(&template, source);
            let forms = dom.tree_order(NodeIdx(0)).into_iter()
                .filter(|node| dom.namespace(*node) == Some(Namespace::Html) && dom.local_name(*node) == Some("form"))
                .count();
            assert_eq!(forms, if source.starts_with("<div>") { 1 } else { 2 }, "{ancestor}: {source}");
        }
    }
    // Foreign or differently-cased contexts are not HTML template contexts.
    for mut other in [context("TEMPLATE", html), context("template", Some("urn:actual"))] {
        other.has_form_ancestor = true;
        let dom = parse(&other, "<form><input></form>");
        assert!(!dom.tree_order(NodeIdx(0)).into_iter().any(|node|
            dom.namespace(node) == Some(Namespace::Html) && dom.local_name(node) == Some("form")));
    }
}
#[test]
fn xml_fragment_direct_grammar_and_actual_namespace_scope() {
    let context = XmlFragmentContext { namespace_uri: Some("urn:context".into()), prefix: Some("c".into()),
        namespaces: vec![(Some("c".into()), "urn:context".into()), (None, "urn:default".into()),
            (Some("a".into()), "urn:attribute".into()), (Some("gone".into()), String::new())] };
    let dom = parse_xml_fragment(&context,
        "\u{feff}text&amp;<![CDATA[<raw>]]><!--comment--><?pi data?><item a:x='v'/><c:item/><item xmlns=''><child/></item><item/>").unwrap();
    let children: Vec<_> = dom.children(NodeIdx(0)).collect();
    assert_eq!(children.len(), 8);
    assert!(matches!(&dom.get(children[0]).kind, NodeKind::Text { data } if data == "\u{feff}text&"));
    assert!(matches!(&dom.get(children[1]).kind, NodeKind::CDATASection { data } if data == "<raw>"));
    assert_eq!(dom.namespace_uri(children[4]), Some("urn:default"));
    assert_eq!(dom.namespace_uri(children[5]), Some("urn:context"));
    assert_eq!(dom.namespace_uri(children[6]), None);
    assert_eq!(dom.namespace_uri(children[7]), Some("urn:default"));
    assert_eq!(dom.attribute_ns(children[4], Some("urn:attribute"), "x").unwrap().value, "v");
    assert!(parse_xml_fragment(&context, "").unwrap().children(NodeIdx(0)).next().is_none());
    assert!(parse_xml_fragment(&context, "<gone:item/>").is_err());
    assert!(parse_xml_fragment(&context, "<c:item xmlns:c='urn:inner'/><c:item/>").is_ok());
}
#[test]
fn xml_fragment_fatal_errors_never_use_html_recovery() {
    let context = XmlFragmentContext::default();
    for source in ["<a>", "</a>", "<a></b>", "<!DOCTYPE a><a/>", "<?xml version='1.0'?>",
        "<a/><p:x/>", "<a x='1' x='2'/>", "<a xmlns:p='urn:x' xmlns:q='urn:x' p:x='1' q:x='2'/>",
        "<a xmlns:p=''/>", "<a xmlns:xml='urn:bad'/>", "<a xmlns='http://www.w3.org/XML/1998/namespace'/>",
        "&unknown;", "&#0;", "text]]>", "<!--bad--comment-->", "<![CDATA[unclosed", "<a x='<bad'/>", "\0"] {
        assert!(parse_xml_fragment(&context, source).is_err(), "{source:?}");
    }
    assert!(parse_xml_fragment(&context, "<a/><b/>text").is_ok());
    assert!(crate::parse_xml("<a/><b/>").is_err());
    assert!(crate::parse_xml("text").is_err());
}

#[test]
fn xml_fragment_template_children_use_inert_contents_document() {
    let dom = parse_xml_fragment(&XmlFragmentContext::default(),
        "<template xmlns='http://www.w3.org/1999/xhtml'><b/><![CDATA[x]]></template><tail/>").unwrap();
    let template = dom.children(NodeIdx(0)).next().unwrap();
    assert!(dom.children(template).next().is_none());
    let contents = dom.template_contents(template).expect("XML template contents");
    let children: Vec<_> = dom.children(contents).collect();
    assert_eq!(children.len(), 2);
    assert_eq!(dom.namespace(children[0]), Some(Namespace::Html));
    assert!(matches!(dom.get(children[1]).kind, NodeKind::CDATASection { .. }));
    assert_eq!(dom.owner_document(children[0]), dom.owner_document(contents));
}
