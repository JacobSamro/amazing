use amazing_css::{compute, parse_stylesheet, values::{ComputedValue, LengthPercentage, RelativeUnit}};
use amazing_dom::{Dom, NodeIdx};

#[test]
fn font_relative_metric_units_reach_layout_in_computed_styles() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let child = dom.create_element(html, "div");
    let style = compute(&dom, &[parse_stylesheet(
        "div { width: 2ic; height: calc(1rcap + 1rlh); \
         margin-left: 3rex; padding-top: 4ric; line-height: 2lh; \
         font-size: 1cap; word-spacing: calc(1rch + 1px) }",
    )]);
    let style = style.get(child).unwrap();
    assert_eq!(style.get("width"), Some("2ic"));
    assert_eq!(style.typed("width"), Some(&ComputedValue::LengthPercentage(
        LengthPercentage::Relative(2.0, RelativeUnit::Ic))));
    assert_eq!(style.get("height"), Some("calc(1rcap + 1rlh)"));
    assert!(matches!(style.typed("height"),
        Some(ComputedValue::LengthPercentageExpression(value)) if value == "calc(1rcap + 1rlh)"));
    assert_eq!(style.typed("padding-top"), Some(&ComputedValue::LengthPercentage(
        LengthPercentage::Relative(4.0, RelativeUnit::Ric))));
    assert_eq!(style.typed("line-height"), Some(&ComputedValue::LengthPercentage(
        LengthPercentage::Relative(2.0, RelativeUnit::Lh))));
    assert_eq!(style.typed("font-size"), Some(&ComputedValue::LengthPercentage(
        LengthPercentage::Relative(1.0, RelativeUnit::Cap))));
    assert!(matches!(style.typed("word-spacing"),
        Some(ComputedValue::LengthPercentageExpression(value)) if value == "calc(1rch + 1px)"));
}
