use amazing_css::{compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn pinned_oklab_and_oklch_colors_reach_computed_style() {
    // WPT css/css-color/oklch-002/003/004 and oklab-006 at c2a2fce69e5c.
    let samples = [
        ("oklch(0% 0 0)", "#000000ff"),
        ("oklch(100% 0 0)", "#ffffffff"),
        ("oklch(50% 0.2 0)", "#b4065fff"),
        ("oklab(70% 0 0.125)", "#bd9a32ff"),
    ];
    for (input, expected) in samples {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "div");
        let sheet = parse_stylesheet(&format!("div {{ background-color: {input}; }}"));
        let styles = compute(&dom, &[sheet]);
        assert_eq!(
            styles.get(element).unwrap().get("background-color"),
            Some(expected),
            "{input}"
        );
    }
}

#[test]
fn pinned_predefined_color_spaces_reach_computed_style() {
    let samples = [
        ("color(xyz 0 0 0)", "#000000ff"),
        ("lab(0% 0 0)", "#000000ff"),
        ("lab(100% 0 0)", "#ffffffff"),
        ("color(display-p3-linear 0 0 0)", "#000000ff"),
        ("color(srgb-linear 0 0 0)", "#000000ff"),
        ("color(xyz-d50 0.08312 0.154746 0.020961)", "#008000ff"),
        ("color(display-p3 0.21604 0.49418 0.13151)", "#008000ff"),
    ];
    for (input, expected) in samples {
        let mut dom = Dom::new();
        let element = dom.create_element(NodeIdx(0), "div");
        let sheet = parse_stylesheet(&format!("div {{ background-color: {input}; }}"));
        let styles = compute(&dom, &[sheet]);
        assert_eq!(
            styles.get(element).unwrap().get("background-color"),
            Some(expected),
            "{input}"
        );
    }
}

#[test]
fn pinned_lab_reference_colors_match_predefined_spaces() {
    // These pairs come from css/css-color/*-004 at the pinned WPT revision.
    let pairs = [
        (
            "lab(86.61399% -106.539 102.871)",
            "color(display-p3-linear 0 1 0)",
        ),
        (
            "lab(87.5745% -186.6921 150.9905)",
            "color(prophoto-rgb 0 1 0)",
        ),
        ("lab(85.7729% -160.7259 109.2319)", "color(rec2020 0 1 0)"),
        ("lab(100% 0.0131 0.0085)", "color(prophoto-rgb 1 1 1)"),
    ];
    for (reference, test) in pairs {
        let mut dom = Dom::new();
        let reference_node = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(reference_node, vec![("class".into(), "reference".into())]);
        let test_node = dom.create_element(NodeIdx(0), "div");
        dom.set_attrs(test_node, vec![("class".into(), "test".into())]);
        let sheet = parse_stylesheet(&format!(
            ".reference {{ background-color: {reference}; }} .test {{ background-color: {test}; }}"
        ));
        let styles = compute(&dom, &[sheet]);
        assert_eq!(
            styles.get(reference_node).unwrap().get("background-color"),
            styles.get(test_node).unwrap().get("background-color"),
            "{reference} versus {test}"
        );
    }
}

#[test]
fn relative_currentcolor_identity_rebinds_after_explicit_inheritance() {
    // WPT css/css-color/relative-currentcolor-rec2020-02 at the pinned revision.
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "div");
    let sheet = parse_stylesheet(
        "div { color: red; background-color: color(from currentColor rec2020 r g b); } \
         div div { color: color(prophoto-rgb 0.23 0.43 0.13); background-color: inherit; }",
    );
    let styles = compute(&dom, &[sheet]);
    assert_eq!(
        styles.get(parent).unwrap().get("background-color"),
        Some("#ff0000ff")
    );
    let child_style = styles.get(child).unwrap();
    assert_eq!(
        child_style.get("background-color"),
        child_style.get("color")
    );
    assert_ne!(child_style.get("background-color"), Some("#ff0000ff"));
}
