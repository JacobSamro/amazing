use amazing_css::{compute, parse_stylesheet, values::ComputedValue};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn unsupported_color_spaces_preserve_the_existing_cascade_winner() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for color in [
        "darkred",
        // CSS-22 added `xyz`/predefined-RGB reverse conversions for relative
        // `color()`; a custom-ident color space (unimplemented) still hits
        // the preserved-unsupported-grammar path.
        "color(from currentcolor --custom-space x y z)",
    ] {
        let sheet = parse_stylesheet(&format!(
            "div {{ background-color: red; background-color: {color}; }}"
        ));
        let styled = compute(&dom, &[sheet]);
        let style = styled.get(element).unwrap();
        assert_eq!(style.get("background-color"), Some(color));
        assert_eq!(
            style.typed("background-color"),
            Some(&ComputedValue::Unparsed)
        );
    }
}

#[test]
fn deprecated_system_color_alias_has_a_typed_value() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let styled = compute(&dom, &[parse_stylesheet("div { color: InfoText; }")]);
    assert_eq!(styled.get(element).unwrap().get("color"), Some("#000000ff"));
    assert_eq!(styled.get(element).unwrap().typed("color"),
        Some(&ComputedValue::Color(amazing_css::values::Color::Rgba([0, 0, 0, 255]))));
}

#[test]
fn invalid_supported_color_after_var_defaults_without_reviving_prior_winner() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(
        "div { --bad: rgb(1,2); background-color: red; background-color: var(--bad); }",
    );
    let styled = compute(&dom, &[sheet]);
    assert_eq!(
        styled.get(element).unwrap().get("background-color"),
        Some("transparent")
    );
}

#[test]
fn invalid_color_identifier_does_not_get_the_unsupported_color_escape_hatch() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let styled = compute(
        &dom,
        &[parse_stylesheet(
            "div { --bad: notacolor; color: red; color: var(--bad); }",
        )],
    );
    assert_eq!(styled.get(element).unwrap().get("color"), Some("black"));
}

#[test]
fn existing_flex_math_constants_remain_available_to_layout() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for constant in ["infinity", "-infinity", "nan"] {
        let css = format!("div {{ flex: calc({constant}) 0 0px; }}");
        let styled = compute(&dom, &[parse_stylesheet(&css)]);
        let value = format!("calc({constant})");
        assert_eq!(
            styled.get(element).unwrap().get("flex-grow"),
            Some(value.as_str())
        );
        assert_eq!(
            styled.get(element).unwrap().typed("flex-grow"),
            Some(&ComputedValue::Unparsed)
        );
    }
}

#[test]
fn ruby_display_roles_survive_typed_computation() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for role in [
        "ruby",
        "ruby-base",
        "ruby-text",
        "ruby-base-container",
        "ruby-text-container",
    ] {
        let styled = compute(
            &dom,
            &[parse_stylesheet(&format!("div {{ display: {role}; }}"))],
        );
        let style = styled.get(element).unwrap();
        assert_eq!(style.get("display"), Some(role));
        assert!(matches!(
            style.typed("display"),
            Some(ComputedValue::Display(_))
        ));
    }
}
