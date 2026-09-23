use amazing_css::{
    compute, parse_stylesheet,
    selectors::{PseudoElement, parse_selector},
};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn list_style_shorthand_resets_and_inherits_all_three_longhands() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let list = dom.create_element(root, "ul");
    let item = dom.create_element(list, "li");
    let styles = compute(&dom, &[parse_stylesheet("ul { list-style: none }")]);
    for node in [list, item] {
        let style = styles.get(node).unwrap();
        assert_eq!(style.get("list-style-type"), Some("none"));
        assert_eq!(style.get("list-style-image"), Some("none"));
        assert_eq!(style.get("list-style-position"), Some("outside"));
    }
    assert!(styles.get_pseudo(item, PseudoElement::Marker).is_none());

    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "ul { list-style: square inside url(icons/bullet.svg) }",
        )],
    );
    let style = styles.get(item).unwrap();
    assert_eq!(style.get("list-style-type"), Some("square"));
    assert_eq!(style.get("list-style-position"), Some("inside"));
    assert_eq!(style.get("list-style-image"), Some("url(icons/bullet.svg)"));

    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "ul { list-style: square inside; list-style: none }",
        )],
    );
    assert_eq!(
        styles.get(item).unwrap().get("list-style-type"),
        Some("none")
    );
    assert_eq!(
        styles.get(item).unwrap().get("list-style-position"),
        Some("outside")
    );
}

#[test]
fn nth_of_type_compound_before_marker_matches_only_target_item() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let list = dom.create_element(root, "ul");
    let first = dom.create_element(list, "li");
    let _other = dom.create_element(list, "div");
    let second = dom.create_element(list, "li");
    let third = dom.create_element(list, "li");
    let selector = parse_selector("ul > li:nth-of-type(2)::marker").unwrap();
    assert_eq!(selector.pseudo_element(), Some(PseudoElement::Marker));
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "ul > li:nth-of-type(2)::marker { content: 'SECOND' }",
        )],
    );
    assert!(!styles.pseudo_matched(first, PseudoElement::Marker));
    assert!(styles.pseudo_matched(second, PseudoElement::Marker));
    assert!(!styles.pseudo_matched(third, PseudoElement::Marker));
    assert_eq!(
        styles
            .generated_content(second, PseudoElement::Marker)
            .unwrap()
            .text(),
        "SECOND"
    );
    assert_eq!(
        styles
            .generated_content(first, PseudoElement::Marker)
            .unwrap()
            .text(),
        "• "
    );
}

#[test]
fn list_style_shorthand_resolves_variables_and_drops_invalid_atomically() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let list = dom.create_element(root, "ul");
    let item = dom.create_element(list, "li");
    let css = "ul { --marker: square inside; list-style: var(--marker) }";
    let styles = compute(&dom, &[parse_stylesheet(css)]);
    assert_eq!(
        styles.get(item).unwrap().get("list-style-type"),
        Some("square")
    );
    assert_eq!(
        styles.get(item).unwrap().get("list-style-position"),
        Some("inside")
    );
    let css = "ul { list-style: square inside; list-style: square circle }";
    let styles = compute(&dom, &[parse_stylesheet(css)]);
    assert_eq!(
        styles.get(item).unwrap().get("list-style-type"),
        Some("square")
    );
    assert_eq!(
        styles.get(item).unwrap().get("list-style-position"),
        Some("inside")
    );
}
