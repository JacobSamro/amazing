use amazing_css::{color::parse_css_color, compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn lch_mix_normalizes_omitted_and_oversized_percentages() {
    // WPT css/css-color/color-mix-percents-01 at c2a2fce.
    let reference = parse_css_color("rgb(68.4898% 36.015% 68.3102%)").unwrap();
    for css in [
        "color-mix(in lch, purple 50%, plum 50%)",
        "color-mix(in lch, purple 50%, plum)",
        "color-mix(in lch, purple, plum 50%)",
        "color-mix(in lch, purple, plum)",
        "color-mix(in lch, plum, purple)",
        "color-mix(in lch, purple 80%, plum 80%)",
    ] {
        assert_eq!(parse_css_color(css), Some(reference), "{css}");
    }
    assert_eq!(
        parse_css_color("color-mix(in srgb, red 30%, blue 30%)"),
        Some([128, 0, 128, 153])
    );
    assert_eq!(parse_css_color("color-mix(in srgb, red -1%, blue)"), None);
}

#[test]
fn contrast_color_picks_black_on_light_and_white_on_dark() {
    for color in [
        "white",
        "aliceblue",
        "mistyrose",
        "lightyellow",
        "palegreen",
    ] {
        assert_eq!(
            parse_css_color(&format!("contrast-color({color})")),
            Some([0, 0, 0, 255])
        );
    }
    for color in ["darkblue", "maroon", "purple", "brown", "black"] {
        assert_eq!(
            parse_css_color(&format!("contrast-color({color})")),
            Some([255, 255, 255, 255])
        );
    }
}

#[test]
fn relative_hsl_currentcolor_rebinds_after_inheritance() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "div");
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "div { color:red; background-color:hsl(from currentColor h s l) } \
         div div { color:green; background-color:inherit }",
        )],
    );
    assert_eq!(
        styles.get(parent).unwrap().get("background-color"),
        Some("#ff0000ff")
    );
    assert_eq!(
        parse_css_color(styles.get(child).unwrap().get("background-color").unwrap()),
        parse_css_color(styles.get(child).unwrap().get("color").unwrap())
    );
}

#[test]
fn deprecated_system_color_is_supported_and_matches_modern_alias() {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    let styles = compute(&dom, &[parse_stylesheet(
        "div { background-color:red } @supports (color: InactiveBorder) { div { background-color:InactiveBorder } }",
    )]);
    assert_eq!(
        styles.get(node).unwrap().get("background-color"),
        Some("#808080ff")
    );
    assert_eq!(
        parse_css_color("InactiveBorder"),
        parse_css_color("ButtonBorder")
    );
}

#[test]
fn color_functions_survive_cascade_and_custom_property_substitution() {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "p");
    let styles = compute(&dom, &[parse_stylesheet(
        "p { --bgcolor: aliceblue; background-color: color-mix(in lch, purple, plum); color: contrast-color(var(--bgcolor)) }",
    )]);
    assert_eq!(styles.get(node).unwrap().get("color"), Some("#000000ff"));
    assert_eq!(
        parse_css_color(styles.get(node).unwrap().get("background-color").unwrap()),
        parse_css_color("rgb(68.4898% 36.015% 68.3102%)")
    );
}

#[test]
fn currentcolor_inside_color_mix_uses_element_color_and_inherits_source() {
    // WPT color-mix-currentcolor-002: inherited background recomputes on child.
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "div");
    let styles = compute(&dom, &[parse_stylesheet(
        "div { color: red; background-color: color-mix(in lch, currentColor 50%, blue) } \
         div div { color: green; background-color: inherit }",
    )]);
    assert_eq!(
        parse_css_color(styles.get(parent).unwrap().get("background-color").unwrap()),
        parse_css_color("color-mix(in lch, red 50%, blue)")
    );
    assert_eq!(
        parse_css_color(styles.get(child).unwrap().get("background-color").unwrap()),
        parse_css_color("color-mix(in lch, green 50%, blue)")
    );

    // WPT color-mix-currentcolor-003: color's currentColor is inherited input.
    let mut dom = Dom::new();
    let body = dom.create_element(NodeIdx(0), "body");
    let parent = dom.create_element(body, "div");
    let child = dom.create_element(parent, "div");
    let styles = compute(&dom, &[parse_stylesheet(
        "body { color: green } div { color: color-mix(in srgb, currentColor 50%, white) } \
         div div { color: inherit }",
    )]);
    let expected = parse_css_color("color-mix(in srgb, green 50%, white)");
    assert_eq!(parse_css_color(styles.get(parent).unwrap().get("color").unwrap()), expected);
    assert_eq!(parse_css_color(styles.get(child).unwrap().get("color").unwrap()), expected);
}
