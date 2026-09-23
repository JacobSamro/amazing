use crate::{Dom, NodeIdx, NodeKind};
use crate::shadow::{ShadowMode, ShadowOptions, SlotAssignment};

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
    let contents = dom.ensure_template_contents(template).unwrap();
    dom.create_element(contents, "br");
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

// HTML §13.3 at cd8ac6f1bbf86dd0bd09ef75d27dacaebe7b4c1d and
// shadow-dom/declarative WPT at c2a2fce69e5c2509d9dc28440ec8b42532e5c293.
#[test]
fn declarative_shadow_serialization_selects_roots_and_preserves_template_flags() {
    let mut dom = Dom::new();
    let container = dom.create_element(NodeIdx(0), "main");
    let host = dom.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: true,
        slot_assignment: SlotAssignment::Manual, clonable: true, serializable: true };
    let root = dom.attach_declarative_shadow(host, options, true).unwrap();
    assert_eq!(dom.attach_declarative_shadow(host, options, false),
        Err(crate::DomError::NotSupportedError));
    let shadow_text = dom.create_text(root, "shadow");
    assert_eq!(dom.get(shadow_text).parent, Some(root));
    dom.create_element(host, "span");
    assert_eq!(markup(&dom, container), "<div><span></span></div>");
    let serialized = String::from_utf16(&dom.serialize_children_with_shadow(container, true, &[]).unwrap()).unwrap();
    assert_eq!(serialized, "<div><template shadowrootmode=\"closed\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\">shadow</template><span></span></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, true, &[]).unwrap()).unwrap(),
        "<template shadowrootmode=\"closed\" shadowrootdelegatesfocus=\"\" shadowrootserializable=\"\" shadowrootslotassignment=\"manual\" shadowrootclonable=\"\" shadowrootcustomelementregistry=\"\">shadow</template><span></span>");
    let copy = dom.clone_node(host, false).unwrap();
    let copy_root = dom.shadow_root_of_host(copy).unwrap();
    assert!(dom.shadow_data(copy_root).unwrap().declarative);
    assert!(dom.shadow_data(copy_root).unwrap().keep_custom_element_registry_null);
    assert_eq!(String::from_utf16(&dom.serialize_children(copy_root).unwrap()).unwrap(), "shadow");
    let wrong_mode = ShadowOptions { mode: ShadowMode::Open, ..options };
    assert_eq!(dom.attach_shadow(host, wrong_mode), Err(crate::DomError::NotSupportedError));
    assert!(dom.shadow_data(root).unwrap().declarative);
    assert_eq!(dom.children(root).count(), 1);
    assert_eq!(dom.attach_shadow(host, options), Ok(root));
    assert!(!dom.shadow_data(root).unwrap().declarative);
    assert_eq!(dom.children(root).count(), 0);
}

#[test]
fn explicit_shadow_roots_include_nonserializable_closed_roots() {
    let mut dom = Dom::new();
    let container = dom.create_element(NodeIdx(0), "main");
    let host = dom.create_element(container, "div");
    let options = ShadowOptions { mode: ShadowMode::Closed, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: false, serializable: false };
    let root = dom.attach_declarative_shadow(host, options, false).unwrap();
    dom.create_text(root, "inside");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(container, true, &[]).unwrap()).unwrap(), "<div></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(container, false, &[root]).unwrap()).unwrap(),
        "<div><template shadowrootmode=\"closed\">inside</template></div>");
    assert_eq!(String::from_utf16(&dom.serialize_children_with_shadow(host, false, &[root]).unwrap()).unwrap(),
        "<template shadowrootmode=\"closed\">inside</template>");
}

#[test]
fn parser_fragment_import_keeps_declarative_shadow_contents() {
    let mut source = Dom::new();
    let host = source.create_element(NodeIdx(0), "div");
    let options = ShadowOptions { mode: ShadowMode::Open, delegates_focus: false,
        slot_assignment: SlotAssignment::Named, clonable: true, serializable: true };
    let root = source.attach_declarative_shadow(host, options, false).unwrap();
    source.create_element(root, "slot");
    let mut target = Dom::new();
    let fragment = target.import_fragment(&source, NodeIdx(0)).unwrap();
    let imported_host = target.children(fragment).next().unwrap();
    let imported_root = target.shadow_root_of_host(imported_host).unwrap();
    assert!(target.shadow_data(imported_root).unwrap().declarative);
    assert_eq!(String::from_utf16(&target.serialize_children_with_shadow(fragment, true, &[]).unwrap()).unwrap(),
        "<div><template shadowrootmode=\"open\" shadowrootserializable=\"\" shadowrootclonable=\"\"><slot></slot></template></div>");
}
