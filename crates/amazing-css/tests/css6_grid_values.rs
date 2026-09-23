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
fn grid_lanes_display_stays_typed() {
    for (value, expected) in [
        ("grid-lanes", Display::GridLanes),
        ("inline grid-lanes", Display::InlineGridLanes),
        ("inline   grid-lanes", Display::InlineGridLanes),
        ("inline-grid-lanes", Display::InlineGridLanes),
    ] {
        let result = style("display", value);
        assert_eq!(result.get("display"), Some(value.split_whitespace().collect::<Vec<_>>().join(" ").as_str()));
        assert_eq!(result.typed("display"), Some(&ComputedValue::Display(expected)));
    }
    for value in ["grid-lanes inline", "inline-grid-lanes grid", "inline grid-lanes block"] {
        let result = style("display", value);
        assert_eq!(result.get("display"), Some("block"), "{value}");
    }
}
