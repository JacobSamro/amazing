use amazing_css::{compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn compute_styles(css: &str) -> (amazing_css::StyledDom, NodeIdx, NodeIdx) {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(root, "span");
    (compute(&dom, &[parse_stylesheet(css)]), root, child)
}

#[test]
fn font_features_validate_and_inherit_as_computed_values() {
    let (styles, root, child) = compute_styles(
        "div { font-feature-settings: 'liga' on, \"dlig\" 0; \
               font-variant-ligatures: no-common-ligatures discretionary-ligatures; }",
    );
    for node in [root, child] {
        let style = styles.get(node).unwrap();
        assert_eq!(
            style.get("font-feature-settings"),
            Some("\"liga\" on, \"dlig\" 0")
        );
        assert_eq!(
            style.get("font-variant-ligatures"),
            Some("no-common-ligatures discretionary-ligatures")
        );
    }
}

#[test]
fn invalid_font_features_do_not_override_valid_declarations() {
    for invalid in [
        "'lig'",
        "'éabc'",
        "liga",
        "'liga' -1",
        "'liga' 1.5",
        "'liga',",
        "'liga' 'kern'",
    ] {
        let (styles, root, _) = compute_styles(&format!(
            "div {{ font-feature-settings: 'kern'; font-feature-settings: {invalid}; }}"
        ));
        assert_eq!(
            styles.get(root).unwrap().get("font-feature-settings"),
            Some("\"kern\""),
            "{invalid}"
        );
    }
    for invalid in [
        "common-ligatures no-common-ligatures",
        "normal contextual",
        "none",
        "foo",
        "contextual contextual",
    ] {
        let css = format!(
            "div {{ font-variant-ligatures: historical-ligatures; font-variant-ligatures: {invalid}; }}"
        );
        let (styles, root, _) = compute_styles(&css);
        let expected = if invalid == "none" {
            "none"
        } else {
            "historical-ligatures"
        };
        assert_eq!(
            styles.get(root).unwrap().get("font-variant-ligatures"),
            Some(expected),
            "{invalid}"
        );
    }
}

#[test]
fn font_shorthand_resets_feature_properties_and_css_wide_values_work() {
    let (computed, root, child) = compute_styles(
        "div { font-feature-settings: 'liga' off; font-variant-ligatures: none; font-size-adjust: 0.5; } \
         span { font: italic 16px serif; }",
    );
    assert_eq!(
        computed.get(root).unwrap().get("font-feature-settings"),
        Some("\"liga\" off")
    );
    assert_eq!(
        computed.get(child).unwrap().get("font-feature-settings"),
        Some("normal")
    );
    assert_eq!(
        computed.get(child).unwrap().get("font-variant-ligatures"),
        Some("normal")
    );
    assert_eq!(
        computed.get(child).unwrap().get("font-size-adjust"),
        Some("none")
    );

    let (styles, root, child) = compute_styles(
        "div { font-feature-settings: 'kern'; font-variant-ligatures: no-contextual; } \
         span { font-feature-settings: inherit; font-variant-ligatures: initial; }",
    );
    assert_eq!(
        styles.get(child).unwrap().get("font-feature-settings"),
        styles.get(root).unwrap().get("font-feature-settings")
    );
    assert_eq!(
        styles.get(child).unwrap().get("font-variant-ligatures"),
        Some("normal")
    );

    let (computed, _, child) = compute_styles(
        "div { font-feature-settings: 'liga'; font-variant-ligatures: none; \
               --font: italic 18px serif; } span { font: var(--font); }",
    );
    assert_eq!(
        computed.get(child).unwrap().get("font-feature-settings"),
        Some("normal")
    );
    assert_eq!(
        computed.get(child).unwrap().get("font-variant-ligatures"),
        Some("normal")
    );
    assert_eq!(computed.get(child).unwrap().get("font-size"), Some("18px"));
}
