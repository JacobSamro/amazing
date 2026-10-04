use super::*;

fn style(values: &[(&str, &str)], parent: Option<&ComputedStyle>) -> ComputedStyle {
    let declarations = values.iter().flat_map(|(name, value)| {
        if name.starts_with("--") { vec![(name.to_string(), value.to_string())] }
        else { expand_declaration(name, value) }
    }).collect();
    let root = parent.map_or(16.0, ComputedStyle::root_font_size_px);
    super::super::resolve_with_root_attrs_environment(parent, declarations, root, None, &MediaEnvironment::default())
}

#[test]
fn complete_quad_and_slash_grammar_is_atomic() {
    for css in ["0", "1px", "1px 2%", "1em 2rem 3cm", "1px 2% 3px 4% / 5% 6px 7% 8px",
        "calc(2em - 10px) / min(10%, 20px)", "0 / 0", "1PX"] {
        assert!(valid("border-radius", css), "{css}");
        assert_eq!(expand_declaration("border-radius", css).len(), 4);
        assert!(crate::values::parses("border-radius", css));
    }
    for css in ["auto", "none", "1", "-1px", "1px -2%", "1px 2px 3px 4px 5px", "/ 1px",
        "1px /", "1px // 2px", "1px / 2px / 3px", "1px, 2px", "calc(1px + 2)", "1px []"] {
        assert!(!crate::values::parses("border-radius", css), "{css}");
        assert!(expand_declaration("border-radius", css).is_empty(), "{css}");
    }
    for name in CORNERS {
        for css in ["1px 2%", "1cm", "calc(-1px)"] { assert!(valid(name, css)); }
        for css in ["1px 2px 3px", "1px / 2px", "-1%", "auto"] { assert!(!valid(name, css)); }
    }
}

#[test]
fn every_quad_cardinality_serializes_independent_axes() {
    for (css, expected) in [
        ("1px", "1px"), ("1px 2%", "1px 2%"), ("1px 2% 3px", "1px 2% 3px"),
        ("1px 2% 3px 4%", "1px 2% 3px 4%"),
        ("1px 2px 1px 2px / 3px 4px 3px 4px", "1px 2px / 3px 4px"),
        ("1px 1px 1px 1px / 1px", "1px"),
        ("1px 2% 3px 4% / 5px", "1px 2% 3px 4% / 5px"),
    ] {
        let computed = style(&[("border-radius", css)], None);
        assert_eq!(computed.cssom_property_value("border-radius").as_deref(), Some(expected), "{css}");
        assert!(computed.cssom_property_value("border-top-left-radius").is_some());
    }
}

#[test]
fn absolute_font_and_root_lengths_compute_without_box_percentage_basis() {
    let parent = style(&[("font-size", "20px")], None);
    let child = style(&[("font-size", "40px"), ("border-radius", "5em / 1rem 2% 3px 4%")], Some(&parent));
    assert_eq!(child.cssom_property_value("border-radius").as_deref(), Some("200px / 20px 2% 3px 4%"));
    let mixed = style(&[("font-size", "40px"), ("border-top-left-radius", "calc(.5em + 10px) 40%")], None);
    assert_eq!(mixed.get("border-top-left-radius"), Some("30px 40%"));
    assert_eq!(computed_pair("1in 72pt", None, 20.0).as_deref(), Some("96px"));
    assert_eq!(computed_pair("calc(50% + 2em)", Some(10.0), 20.0).as_deref(), Some("calc(50% + 20px)"));
}

#[test]
fn calculated_range_clamps_only_after_contextual_computation() {
    for (css, expected) in [("calc(-.5em + 10px)", "0px"), ("min(1em, 30px)", "30px"),
        ("max(-20px, -10px)", "0px"), ("clamp(10px, 2em, 50px)", "50px"),
        ("calc(-10%)", "0%"), ("calc(50% - 60%)", "0%") ] {
        assert_eq!(computed_pair(css, Some(40.0), 20.0).as_deref(), Some(expected), "{css}");
    }
    assert_eq!(computed_pair("1ex", None, 20.0).as_deref(), Some("1ex"));
    assert_eq!(computed_pair("calc(1ex + 10px)", Some(40.0), 20.0).as_deref(), Some("calc(1ex + 10px)"));
    assert_eq!(computed_pair("calc(50% - 10px)", Some(40.0), 20.0).as_deref(), Some("calc(50% - 10px)"));
}

#[test]
fn variable_shorthand_is_whole_and_inheritance_keeps_originating_lengths() {
    let parent = style(&[("font-size", "20px"), ("border-radius", "2em / 10%")], None);
    let child = style(&[("font-size", "40px"), ("border-radius", "inherit")], Some(&parent));
    assert_eq!(child.cssom_property_value("border-radius").as_deref(), Some("40px / 10%"));
    let substitution = style(&[("font-size", "20px"), ("--r", "1em 2% / 3px"), ("border-radius", "var(--r)")], None);
    assert_eq!(substitution.cssom_property_value("border-radius").as_deref(), Some("20px 2% / 3px"));
    let invalid = style(&[("--r", "1px / auto"), ("border-radius", "var(--r)")], None);
    assert_eq!(invalid.cssom_property_value("border-radius").as_deref(), Some("0px"));
    for css in ["initial", "unset"] {
        assert_eq!(style(&[("border-radius", css)], Some(&parent)).cssom_property_value("border-radius").as_deref(), Some("0px"));
    }
}

#[test]
fn declaration_order_specificity_and_important_share_cascade_expansion() {
    use amazing_dom::{Dom, NodeIdx};
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let div = dom.create_element(html, "div");
    let sheets = [crate::parse_stylesheet("div { border-radius: 1em / 20%; font-size:20px; border-top-left-radius:3px 4px !important } div { border-radius:5px; border-radius:1px / auto }")];
    let styles = crate::compute(&dom, &sheets);
    let value = styles.get(div).unwrap();
    assert_eq!(value.get("border-top-left-radius"), Some("3px 4px"));
    assert_eq!(value.cssom_property_value("border-radius").as_deref(), Some("3px 5px 5px / 4px 5px 5px"));
}

#[test]
fn style_mutation_and_clone_keep_complete_radius_serialization() {
    let mut value = style(&[("font-size", "20px"), ("border-radius", "2em")], None);
    value.set("border-radius", "1em 2% / 3px");
    assert_eq!(value.clone().cssom_property_value("border-radius").as_deref(), Some("20px 2% / 3px"));
    value.set("border-top-left-radius", "4px");
    let before = value.cssom_property_value("border-radius");
    value.set("border-radius", "1px / auto");
    assert_eq!(value.cssom_property_value("border-radius"), before);
}
