use amazing_css::{compute, parse_stylesheet, values::ComputedValue};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    compute(&dom, &[parse_stylesheet(&format!("div{{{css}}}"))]).get(node).unwrap().clone()
}

#[test]
fn complete_atomic_factor_pair_and_basis_order_produces_longhands() {
    for (value, expected) in [
        ("none", "0 0 auto"), ("AUTO", "1 1 auto"), ("n\\6f ne", "0 0 auto"),
        ("1", "1 1 0%"), ("0", "0 1 0%"), ("2 3", "2 3 0%"),
        ("4 5 6px", "4 5 6px"), ("7% 8", "8 1 7%"), ("8 auto", "8 1 auto"),
        ("auto 2 3", "2 3 auto"), ("5px 2 3", "2 3 5px"), ("1 1 0", "1 1 0px"),
        ("1 1 0%", "1 1 0%"), ("1/**/2/**/3px", "1 2 3px"),
    ] {
        assert!(amazing_css::values::parses("flex", value), "{value}");
        assert_eq!(style(&format!("flex:{value}")).get("flex"), Some(expected), "{value}");
    }
}

#[test]
fn invalid_atomic_shorthand_never_overwrites_any_winning_longhand() {
    for value in ["", "1 2 calc(0)", "1 2 calc(3 - 3)", "2 3 4", "5px 7%", "9 none", "none 1", "-1", "infinity", "nan", "1 -2", "1 2 -3px", "1 2 -3%", "1px 2px", "1 2 3px 4", "calc(1px) 2 3px", "calc(1 + ) 2 3px", "'1'"] {
        assert!(!amazing_css::values::parses("flex", value), "{value}");
        let current = style(&format!("flex:4 5 6px;flex:{value}"));
        assert_eq!(current.get("flex"), Some("4 5 6px"), "{value}");
        assert_eq!(current.get("flex-grow"), Some("4"));
        assert_eq!(current.get("flex-shrink"), Some("5"));
        assert_eq!(current.get("flex-basis"), Some("6px"));
    }
}

#[test]
fn finite_factor_math_computes_and_clamps_before_actual_layout_consumer() {
    for (css, expected, grow, shrink) in [
        ("calc(1) calc(2 + 1) calc(3px)", "1 3 3px", 1.0, 3.0),
        ("calc(-1) calc(-2) 0px", "0 0 0px", 0.0, 0.0),
        ("max(1, 4) clamp(1, 2, 3) 5px", "4 2 5px", 4.0, 2.0),
        ("calc(8 / 2) min(5, 2) auto", "4 2 auto", 4.0, 2.0),
    ] {
        let current = style(&format!("flex:{css}"));
        assert_eq!(current.get("flex"), Some(expected));
        assert_eq!(current.typed("flex-grow"), Some(&ComputedValue::Number(grow)));
        assert_eq!(current.typed("flex-shrink"), Some(&ComputedValue::Number(shrink)));
    }
}

#[test]
fn basis_computation_uses_actual_element_font_and_absolute_units() {
    for (css, expected) in [
        ("1 1 2em", "1 1 40px"), ("1 1 calc(-10em)", "1 1 0px"),
        ("1 1 calc(10px + .5em)", "1 1 20px"), ("1 1 1in", "1 1 96px"),
        ("1 1 2pc", "1 1 32px"), ("1 1 6pt", "1 1 8px"),
    ] {
        assert_eq!(style(&format!("font-size:20px;flex:{css}")).get("flex"), Some(expected), "{css}");
    }
}

#[test]
fn root_and_descendant_rem_use_the_document_root_context() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let node = dom.create_element(html, "div");
    let current = compute(&dom, &[parse_stylesheet("html{font-size:30px;flex:1 1 2rem}div{font-size:10px;flex:1 1 calc(3rem)}")]);
    assert_eq!(current.get(html).unwrap().get("flex"), Some("1 1 60px"));
    assert_eq!(current.get(node).unwrap().get("flex"), Some("1 1 90px"));
}

#[test]
fn percentages_keep_real_box_handoff_and_negative_pure_math_clamps() {
    for (css, expected) in [("1 1 20%", "1 1 20%"), ("1 1 calc(-20%)", "1 1 0%"), ("1 1 calc(10% + 15%)", "1 1 25%")] {
        assert_eq!(style(&format!("flex:{css}")).get("flex"), Some(expected));
    }
    let current = style("flex:1 1 calc(10px + 20%)");
    assert!(current.get("flex").unwrap().contains("20%"));
    assert!(matches!(current.typed("flex-basis"), Some(ComputedValue::LengthPercentageExpression(_))));
}

#[test]
fn intrinsic_and_sizing_basis_keywords_reach_existing_consumers() {
    for basis in ["auto", "content", "min-content", "max-content", "fit-content", "stretch", "calc-size(auto, size + 1px)"] {
        let input = format!("2 3 {basis}");
        assert!(amazing_css::values::parses("flex", &input), "{input}");
        let current = style(&format!("flex:{input}"));
        assert_eq!(current.get("flex-grow"), Some("2"));
        assert_eq!(current.get("flex-shrink"), Some("3"));
        assert!(current.get("flex-basis").is_some(), "{basis}");
    }
}

#[test]
fn deferred_variables_keep_atomic_invalid_at_computed_value_behavior() {
    assert_eq!(style("--f:calc(2 + 1) min(4, 2) calc(5px);flex:var(--f)").get("flex"), Some("3 2 5px"));
    for css in ["flex:4 5 6px;flex:var(--missing)", "--f:1 2 garbage;flex:4 5 6px;flex:var(--f)"] {
        assert_eq!(style(css).get("flex"), Some("0 1 auto"), "{css}");
    }
    assert_eq!(style("flex:var(--missing, 2 3 4px)").get("flex"), Some("2 3 4px"));
}

#[test]
fn css_wide_inheritance_and_layer_winners_apply_to_every_component() {
    let mut dom = Dom::new();
    let section = dom.create_element(NodeIdx(0), "section");
    let child = dom.create_element(section, "div");
    for keyword in ["inherit", "var(--wide, inherit)"] {
        let css = format!("section{{flex:4 5 6px}}div{{--wide:inherit;flex:{keyword}}}");
        assert_eq!(compute(&dom, &[parse_stylesheet(&css)]).get(child).unwrap().get("flex"), Some("4 5 6px"));
    }
    for keyword in ["initial", "unset"] {
        assert_eq!(style(&format!("flex:4 5 6px;flex:{keyword}")).get("flex"), Some("0 1 auto"));
    }
    assert_eq!(style("flex:4 5 6px!important;flex-grow:8;flex-shrink:9!important").get("flex"), Some("4 9 6px"));
    assert_eq!(style("flex:4 5 6px;all:initial").get("flex"), Some("0 1 auto"));
    let css = "@layer a,b;@layer a{div{flex:2 3 4px}}@layer b{div{flex:7 8 9px;flex:revert-layer}}";
    let node = dom.create_element(section, "div");
    assert_eq!(compute(&dom, &[parse_stylesheet(css)]).get(node).unwrap().get("flex"), Some("2 3 4px"));
}

#[test]
fn public_setter_and_cloning_derive_shorthand_from_current_longhand_winners() {
    let mut current = style("font-size:20px;flex:2 3 4px");
    let original = current.clone();
    current.set("flex-grow", "calc(8 / 2)");
    current.set("flex-basis", "2em");
    assert_eq!(current.get("flex"), Some("4 3 40px"));
    current.set("flex", "2 3 4");
    assert_eq!(current.get("flex"), Some("4 3 40px"));
    assert_eq!(original.get("flex"), Some("2 3 4px"));
    current.set("flex", "NONE");
    assert_eq!(current.get("flex"), Some("0 0 auto"));
}

#[test]
fn existing_nonfinite_factor_contract_stays_raw_and_unparsed() {
    for constant in ["infinity", "-infinity", "nan"] {
        let css = format!("calc({constant})");
        let current = style(&format!("flex:{css} 0 0px"));
        assert_eq!(current.get("flex-grow"), Some(css.as_str()));
        assert_eq!(current.typed("flex-grow"), Some(&ComputedValue::Unparsed));
    }
}

#[test]
fn typed_attribute_and_real_dom_inline_mutation_recompute_atomic_values() {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    dom.set_attrs(node, vec![("data-factor".into(), "2".into()), ("data-size".into(), "10".into())]);
    let sheet = parse_stylesheet("div{font-size:20px;flex:attr(data-factor type(<number>)) 3 calc(attr(data-size type(<number>)) * 1px)}");
    let before = compute(&dom, &[sheet.clone()]);
    assert_eq!(before.get(node).unwrap().get("flex"), Some("2 3 10px"));
    dom.set_attrs(node, vec![("data-factor".into(), "4".into()), ("data-size".into(), "20".into())]);
    assert_eq!(compute(&dom, &[sheet]).get(node).unwrap().get("flex"), Some("4 3 20px"));
    dom.set_attrs(node, vec![("style".into(), "font-size:30px;flex:2 3 2em".into())]);
    assert_eq!(compute(&dom, &[]).get(node).unwrap().get("flex"), Some("2 3 60px"));
    dom.set_attrs(node, vec![("style".into(), "font-size:10px;flex:2 3 2em".into())]);
    assert_eq!(compute(&dom, &[]).get(node).unwrap().get("flex"), Some("2 3 20px"));
    assert_eq!(before.get(node).unwrap().get("flex"), Some("2 3 10px"));
}


#[test]
fn whole_percentage_handoff_preserves_zero_type_and_actual_font_dimensions() {
    for (css, expected) in [
        ("calc(0% + 10px)", "calc(0% + 10px)"),
        ("calc(10% + 0px)", "10%"),
        ("calc(10px + 20%)", "calc(20% + 10px)"),
        ("calc(2em + 10%)", "calc(10% + 40px)"),
        ("calc(10% + calc(2em))", "calc(10% + 40px)"),
    ] {
        let current = style(&format!("font-size:20px;flex:1 1 {css}"));
        assert_eq!(current.get("flex-basis"), Some(expected), "{css}");
        assert_eq!(current.get("flex"), Some(format!("1 1 {expected}").as_str()));
    }
    let current = style("font-size:20px;flex:1 1 calc(2em + 10%)");
    let basis = current.get("flex-basis").unwrap();
    for (actual_container_inline_size, expected) in [(200.0, 60.0), (400.0, 80.0)] {
        assert_eq!(amazing_css::used_position_axis(basis, actual_container_inline_size), Some(expected));
    }
    let current = style("font-size:20px;flex:1 1 min(10%, 2em)");
    assert!(matches!(current.typed("flex-basis"), Some(ComputedValue::LengthPercentageExpression(_))));
    assert!(current.get("flex-basis").unwrap().contains("10%"));
    assert!(current.get("flex-basis").unwrap().contains("40px"));
}


#[test]
fn inherited_custom_property_without_fallback_keeps_guaranteed_invalid_defaulting() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "section");
    let child = dom.create_element(parent, "div");
    let css = "section{flex:4 5 6px}div{--wide:inherit;flex:var(--wide)}";
    assert_eq!(compute(&dom, &[parse_stylesheet(css)]).get(child).unwrap().get("flex"), Some("0 1 auto"));
    for keyword in ["initial", "unset"] {
        assert_eq!(style(&format!("flex:4 5 6px;flex:var(--missing, {keyword})")).get("flex"), Some("0 1 auto"));
    }
}
