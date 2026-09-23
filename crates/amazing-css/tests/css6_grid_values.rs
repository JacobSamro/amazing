use amazing_css::{compute, parse_stylesheet, values::{ComputedValue, Display, SubgridLineNames, SubgridRepeatCount}};
use amazing_dom::{Dom, NodeIdx};

fn style(property: &str, value: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(&format!("div {{ {property}: {value}; }}"));
    compute(&dom, &[sheet]).get(element).unwrap().clone()
}

#[test]
fn subgrid_accepts_and_types_both_axes() {
    for property in ["grid-template-columns", "grid-template-rows"] {
        for value in [
            "subgrid", "subgrid [] [a b] [c]", "subgrid repeat(2, [a] [b c])",
            "subgrid [start] repeat(auto-fill, [middle] []) [end]",
        ] {
            let result = style(property, value);
            assert!(result.get(property).unwrap().starts_with("subgrid"), "{property}: {value}");
            assert!(matches!(result.typed(property), Some(ComputedValue::Subgrid(_))), "{property}: {value}");
        }
    }
    let result = style("grid-template-columns", "subgrid [a b] repeat(3, [c] [d e])");
    let Some(ComputedValue::Subgrid(subgrid)) = result.typed("grid-template-columns") else { panic!("subgrid type missing") };
    assert_eq!(subgrid.line_names[0], SubgridLineNames::Names(vec!["a".into(), "b".into()]));
    assert_eq!(subgrid.line_names[1], SubgridLineNames::Repeat {
        count: SubgridRepeatCount::Integer(3), names: vec![vec!["c".into()], vec!["d".into(), "e".into()]],
    });
}

#[test]
fn invalid_subgrid_forms_default_to_none() {
    for value in [
        "subgrid 10px", "subgrid [a] 1fr", "10px subgrid", "subgrid repeat(2, 1fr)",
        "subgrid repeat(auto-fit, [a])", "subgrid repeat(0, [a])",
        "subgrid repeat(2, []) 10px", "subgrid [span]", "subgrid [a] repeat(auto-fill, [b]) repeat(auto-fill, [c])",
        "subgrid repeat(2,)", "subgrid [a", "subgrid [a] minmax(1px, 2px)",
    ] {
        let result = style("grid-template-columns", value);
        assert_eq!(result.get("grid-template-columns"), Some("none"), "{value}");
    }
}

#[test]
fn grid_shorthand_carries_subgrid_and_resets_implicit_tracks() {
    for (property, value) in [
        ("grid", "subgrid / auto"),
        ("grid", "auto / subgrid"),
        ("grid", "subgrid / subgrid"),
        ("grid-template", "subgrid [a] / auto"),
    ] {
        let result = style(property, value);
        assert!(matches!(result.typed("grid-template-rows"), Some(ComputedValue::Subgrid(_)))
            || matches!(result.typed("grid-template-columns"), Some(ComputedValue::Subgrid(_))), "{property}: {value}");
        if property == "grid" {
            assert_eq!(result.get("grid-auto-rows"), Some("auto"));
            assert_eq!(result.get("grid-auto-columns"), Some("auto"));
            assert_eq!(result.get("grid-auto-flow"), Some("row"));
        }
    }
    let result = style("grid", "subgrid / subgrid");
    assert!(matches!(result.typed("grid-template-rows"), Some(ComputedValue::Subgrid(_))));
    assert!(matches!(result.typed("grid-template-columns"), Some(ComputedValue::Subgrid(_))));
    let invalid = style("grid", "subgrid 10px / auto");
    assert_eq!(invalid.get("grid-template-rows"), Some("none"));
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let variable = compute(&dom, &[parse_stylesheet("div { --tracks: subgrid / auto; grid: var(--tracks); }")]);
    assert!(matches!(variable.get(element).unwrap().typed("grid-template-rows"), Some(ComputedValue::Subgrid(_))));
}

#[test]
fn grid_lanes_display_stays_typed() {
    for (value, expected) in [
        ("grid-lanes", Display::GridLanes),
        ("inline grid-lanes", Display::InlineGridLanes),
        ("inline   grid-lanes", Display::InlineGridLanes),
        ("inline-grid-lanes", Display::InlineGridLanes),
    ] {
        let result = style("display", value);
        assert_eq!(result.get("display"), Some(if value.starts_with("inline ") { "inline-grid-lanes" } else { value }));
        assert_eq!(result.typed("display"), Some(&ComputedValue::Display(expected)));
    }
    for value in ["grid-lanes inline", "inline-grid-lanes grid", "inline grid-lanes block"] {
        let result = style("display", value);
        assert_eq!(result.get("display"), Some("block"), "{value}");
    }
}

#[test]
fn two_keyword_display_computes_to_existing_short_forms() {
    for (specified, computed, variant) in [
        ("inline flow-root", "inline-block", Display::InlineBlock),
        ("block flow-root", "flow-root", Display::FlowRoot),
        ("inline flow", "inline", Display::Inline),
        ("block flow", "block", Display::Block),
        ("inline flex", "inline-flex", Display::InlineFlex),
        ("block flex", "flex", Display::Flex),
        ("inline grid", "inline-grid", Display::InlineGrid),
        ("block grid", "grid", Display::Grid),
        ("inline table", "inline-table", Display::InlineTable),
        ("block table", "table", Display::Table),
        ("inline grid-lanes", "inline-grid-lanes", Display::InlineGridLanes),
    ] {
        let result = style("display", specified);
        assert_eq!(result.get("display"), Some(computed), "{specified}");
        assert_eq!(result.typed("display"), Some(&ComputedValue::Display(variant)), "{specified}");
    }
}
