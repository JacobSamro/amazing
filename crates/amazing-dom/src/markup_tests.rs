use crate::{Dom, NodeIdx, NodeKind};

fn markup(dom: &Dom, node: NodeIdx) -> String {
    String::from_utf16(&dom.serialize_children(node).unwrap()).unwrap()
}

#[test]
fn html_fragment_serialization_escapes_text_and_attributes_but_not_raw_text() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let img = dom.create_element(root, "img");
    dom.set_attribute(img, "alt", "a&\"b").unwrap();
    dom.append_child(root, NodeKind::Text { data: "<&>\u{a0}".into() });
    let script = dom.create_element(root, "script");
    dom.append_child(script, NodeKind::Text { data: "if (a < b && c > d) x()".into() });
    assert_eq!(markup(&dom, root), "<img alt=\"a&amp;&quot;b\">&lt;&amp;&gt;&nbsp;<script>if (a < b && c > d) x()</script>");
}

#[test]
fn foreign_namespaces_and_template_children_serialize() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let svg = dom.create_element_ns(root, Some("http://www.w3.org/2000/svg"), "svg").unwrap();
    let path = dom.create_element_ns(svg, Some("http://www.w3.org/2000/svg"), "path").unwrap();
    dom.set_attribute_qualified_ns(path, Some("http://www.w3.org/1999/xlink"), "xlink:href", "a&b").unwrap();
    let math = dom.create_element_ns(root, Some("http://www.w3.org/1998/Math/MathML"), "math").unwrap();
    dom.create_element_ns(math, Some("http://www.w3.org/1998/Math/MathML"), "mi").unwrap();
    let template = dom.create_element(root, "template");
    dom.create_element(template, "br");
    assert_eq!(markup(&dom, root), "<svg><path xlink:href=\"a&amp;b\"></path></svg><math><mi></mi></math><template><br></template>");
}

#[test]
fn parser_arena_import_preserves_namespace_and_tree_order() {
    let mut source = Dom::new();
    let foreign = source.create_element_ns(NodeIdx(0), Some("http://www.w3.org/2000/svg"), "svg").unwrap();
    source.create_element_ns(foreign, Some("http://www.w3.org/2000/svg"), "circle").unwrap();
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let fragment = dom.import_fragment(&source, NodeIdx(0)).unwrap();
    dom.replace_all(root, Some(fragment)).unwrap();
    assert_eq!(markup(&dom, root), "<svg><circle></circle></svg>");
}

#[test]
fn serialization_preserves_utf16_text_units() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let text = dom.append_child(root, NodeKind::Text { data: String::new() });
    dom.replace_character_data(text, 0, 0, &[0xD800, b'&' as u16]).unwrap();
    assert_eq!(dom.serialize_children(root).unwrap(), vec![0xD800, b'&' as u16, b'a' as u16,
        b'm' as u16, b'p' as u16, b';' as u16]);
}
