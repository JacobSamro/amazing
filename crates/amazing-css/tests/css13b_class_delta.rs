use amazing_css::{compute, parse_stylesheet, AttributeStyleUpdate, CachedBoxSize, StyleChangeKind, StyleInvalidator};
use amazing_dom::{Dom, NodeIdx};

fn fixture(css: &str, class: &str) -> (Dom, amazing_css::parser::Stylesheet, NodeIdx) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let target = dom.create_element(html, "div");
    dom.create_element(target, "span");
    dom.create_element(html, "p");
    dom.set_attribute(target, "class", class).unwrap();
    (dom, parse_stylesheet(css), target)
}

#[test]
fn ia_opacity_class_edit_is_value_only_and_updates_retained_style() {
    let (mut dom, sheet, node) = fixture(".base { opacity:.4; color:red } .dim { opacity:.6; color:red }", "base");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 7);
    let mut styles = compute(&dom, &[sheet]);
    dom.set_attribute(node, "class", "dim").unwrap();
    let update = invalidator.update_attribute(&dom, &styles, node, "class", Some("base"), Some("dim"), 7);
    assert_eq!(update.change_kind(node), StyleChangeKind::CompositorOnly);
    let AttributeStyleUpdate::CompositorOnly(delta) = update else { panic!("class edit recomputed") };
    assert_eq!(delta.opacity, Some(0.6));
    assert!(delta.apply_to_styles(&mut styles));
    assert_eq!(styles.get(node).unwrap().get("opacity"), Some("0.6"));
    invalidator.compare_to_full(&dom, &styles).unwrap();
}

#[test]
fn class_transform_uses_cached_geometry_and_rejects_unresolved_geometry() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let node = dom.create_element(html, "div");
    dom.set_attribute(node, "class", "base").unwrap();
    let sheet = parse_stylesheet(".base { transform:translate(2px,0px) } .dim { transform:translate(10%,0px) }");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 3);
    let styles = compute(&dom, &[sheet]);
    dom.set_attribute(node, "class", "dim").unwrap();
    assert!(matches!(invalidator.update_attribute(&dom, &styles, node, "class", Some("base"), Some("dim"), 3), AttributeStyleUpdate::Recomputed(_)));
    let update = invalidator.update_attribute_with_box(&dom, &styles, node, "class", Some("base"), Some("dim"), 3,
        Some(CachedBoxSize { width: 100.0, height: 40.0 }));
    let AttributeStyleUpdate::CompositorOnly(delta) = update else { panic!("stable box rejected") };
    assert_eq!(delta.transform_matrix.unwrap()[0][3], 10.0);
}

#[test]
fn class_edits_with_selector_or_cascade_dependencies_recompute() {
    let cases = [
        ".base {opacity:.4} .hot {opacity:.6} .hot span {opacity:.7}",
        ".base {opacity:.4} .hot {opacity:.6} .hot + p {opacity:.7}",
        ".base {opacity:.4} .hot {opacity:.6} div:has(.hot) {opacity:.8}",
        ".base {opacity:.4} .hot {opacity:.6; color:red}",
        ".base {opacity:.4} @layer theme { .hot {opacity:.6} } .other {opacity:.8 !important}",
        ".base {opacity:.4} .hot {opacity:.6} [class~=hot] {color:red}",
        ".base {opacity:.4} .hot {opacity:.6} .hot::before {content:'changed'}",
    ];
    for css in cases {
        let (mut dom, sheet, node) = fixture(css, "base");
        let invalidator = StyleInvalidator::new(&[sheet.clone()], 5);
        let styles = compute(&dom, &[sheet]);
        dom.set_attribute(node, "class", "hot other").unwrap();
        let update = invalidator.update_attribute(&dom, &styles, node, "class", Some("base"), Some("hot other"), 5);
        assert!(matches!(update, AttributeStyleUpdate::Recomputed(_)), "{css}");
    }
}

#[test]
fn fixed_implicit_style_scope_keeps_class_edit_on_compositor_path() {
    let (mut dom, mut sheet, node) = fixture("@scope { .base {opacity:.4} .dim {opacity:.6} }", "base");
    sheet.scope_root = dom.get(node).parent;
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 9);
    let styles = compute(&dom, &[sheet]);
    dom.set_attribute(node, "class", "dim").unwrap();
    let update = invalidator.update_attribute(&dom, &styles, node, "class", Some("base"), Some("dim"), 9);
    let AttributeStyleUpdate::CompositorOnly(delta) = update else { panic!("fixed owner scope recomputed") };
    assert_eq!(delta.opacity, Some(0.6));
}

#[test]
fn edited_class_in_scope_start_or_end_recomputes() {
    for css in [
        "@scope (.base) { .base {opacity:.4} } .dim {opacity:.6}",
        "@scope (html) to (.dim) { .base {opacity:.4} .dim {opacity:.6} }",
    ] {
        let (mut dom, sheet, node) = fixture(css, "base");
        let invalidator = StyleInvalidator::new(&[sheet.clone()], 9);
        let styles = compute(&dom, &[sheet]);
        dom.set_attribute(node, "class", "dim").unwrap();
        assert!(matches!(invalidator.update_attribute(&dom, &styles, node, "class", Some("base"), Some("dim"), 9),
            AttributeStyleUpdate::Recomputed(_)), "{css}");
    }
}
