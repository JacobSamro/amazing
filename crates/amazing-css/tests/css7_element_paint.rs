use amazing_css::{
    ElementFill, ElementStyleSource as Source, ElementStyleValue as Value,
    compute_element_paint_style, parse_declarations,
};

fn paint(parent: Option<&amazing_css::ComputedStyle>, hints: &str, author: &str, inline: &str) -> amazing_css::ElementPaintStyle {
    compute_element_paint_style(parent, &parse_declarations(hints), &parse_declarations(author), &parse_declarations(inline))
}

#[test]
fn cascade_sources_and_inheritance() {
    let parent = paint(None, "", "color: blue; fill: red; fill-opacity: .4; opacity: .2", "");
    let child = paint(Some(&parent.computed), "fill: green; color: green", "fill: blue; color: blue", "fill: red; color: red; opacity: .5");
    assert_eq!(child.color, Value::Supported { value: [255, 0, 0, 255], source: Source::Inline });
    assert_eq!(child.fill, Value::Supported { value: ElementFill::Color([255, 0, 0, 255]), source: Source::Inline });
    assert_eq!(child.fill_opacity, Value::Supported { value: 0.4, source: Source::Inherited });
    assert_eq!(child.opacity, Value::Supported { value: 0.5, source: Source::Inline });
    let hint = paint(None, "fill: green", "", "");
    assert_eq!(hint.fill, Value::Supported { value: ElementFill::Color([0, 128, 0, 255]), source: Source::PresentationHint });
}

#[test]
fn wide_keywords_and_variables_use_the_computed_resolver() {
    let parent = paint(None, "", "color: blue; fill: red; fill-opacity: .4", "");
    for (keyword, expected_color, expected_fill, expected_alpha) in [
        ("inherit", [0, 0, 255, 255], [255, 0, 0, 255], 0.4),
        ("initial", [0, 0, 0, 255], [0, 0, 0, 255], 1.0),
        ("unset", [0, 0, 255, 255], [255, 0, 0, 255], 0.4),
    ] {
        let child = paint(Some(&parent.computed), "", "", &format!("color: {keyword}; fill: {keyword}; fill-opacity: {keyword}"));
        assert_eq!(child.color, Value::Supported { value: expected_color, source: Source::Inline }, "{keyword}");
        assert_eq!(child.fill, Value::Supported { value: ElementFill::Color(expected_fill), source: Source::Inline }, "{keyword}");
        assert_eq!(child.fill_opacity, Value::Supported { value: expected_alpha, source: Source::Inline }, "{keyword}");
    }
    for keyword in ["revert", "revert-layer"] {
        let child = paint(Some(&parent.computed), "fill: green", "fill: blue", &format!("fill: {keyword}"));
        let expected = if keyword == "revert" { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
        assert_eq!(child.fill, Value::Supported { value: ElementFill::Color(expected), source: if keyword == "revert" { Source::Inherited } else { Source::Author } });
    }
    let variable = paint(None, "", "--ink: red; fill: var(--ink, blue)", "color: var(--missing, green); fill-opacity: var(--missing, 25%)");
    assert_eq!(variable.color, Value::Supported { value: [0, 128, 0, 255], source: Source::Inline });
    assert_eq!(variable.fill, Value::Supported { value: ElementFill::Color([255, 0, 0, 255]), source: Source::Author });
    assert_eq!(variable.fill_opacity, Value::Supported { value: 0.25, source: Source::Inline });
}

#[test]
fn unsupported_paint_is_explicit() {
    let style = paint(None, "", "fill: url(#gradient); color: color(from currentcolor xyz x y z)", "");
    assert_eq!(style.fill, Value::Unsupported { css: "url(#gradient)".into(), source: Source::Author });
    assert!(matches!(style.color, Value::Unsupported { source: Source::Author, .. }));
    let child = paint(Some(&style.computed), "", "fill: currentcolor", "");
    assert!(matches!(child.fill, Value::Unsupported { source: Source::Author, .. }));
}
