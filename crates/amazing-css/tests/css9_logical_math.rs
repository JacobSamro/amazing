use amazing_css::{compute, parse_stylesheet, values::ComputedValue};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let target = dom.create_element(body, "div");
    compute(&dom, &[parse_stylesheet(css)]).get(target).unwrap().clone()
}

#[test]
fn logical_sizing_maps_through_computed_writing_mode_and_preserves_cascade_rank() {
    let horizontal = style("div{inline-size:32px;block-size:41px;min-inline-size:4px;max-block-size:90px}");
    for (property, expected) in [("width", "32px"), ("height", "41px"), ("min-width", "4px"), ("max-height", "90px")] {
        assert_eq!(horizontal.get(property), Some(expected), "{property}");
    }
    let vertical = style("div{writing-mode:vertical-lr;inline-size:32px;block-size:41px;min-inline-size:4px;max-block-size:90px}");
    for (property, expected) in [("height", "32px"), ("width", "41px"), ("min-height", "4px"), ("max-width", "90px")] {
        assert_eq!(vertical.get(property), Some(expected), "{property}");
    }
    assert_eq!(style("div{width:10px;inline-size:20px}").get("width"), Some("20px"));
    assert_eq!(style("div{width:10px}").get("inline-size"), Some("10px"));
    assert_eq!(style("div{inline-size:20px;width:10px}").get("width"), Some("10px"));
    assert_eq!(style("div{width:10px!important;inline-size:20px}").get("width"), Some("10px"));
    assert_eq!(style("div{--axis:vertical-rl;writing-mode:var(--axis);inline-size:20px}").get("height"), Some("20px"));
}

#[test]
fn logical_sides_respect_writing_mode_direction_and_shorthands() {
    let s = style("div{direction:rtl;writing-mode:vertical-rl;margin-inline:3px 5px;padding-block:7px 9px;inset-inline:11px 13px}");
    for (property, expected) in [("margin-bottom", "3px"), ("margin-top", "5px"), ("padding-right", "7px"), ("padding-left", "9px"), ("bottom", "11px"), ("top", "13px")] {
        assert_eq!(s.get(property), Some(expected), "{property}");
    }
}

#[test]
fn mixed_math_is_typed_and_preserved_for_layout_basis() {
    let s = style("div{width:calc(100% - 2px);height:min(40px, calc(20% + 5px));min-width:clamp(10px, 50%, 100px);margin-left:calc(1em + 2%);padding-top:calc(5% * 2);border-radius:calc(10% + 2px) / calc(20px + 5%)}");
    for property in ["width", "height", "min-width", "margin-left", "padding-top"] {
        assert!(matches!(s.typed(property), Some(ComputedValue::LengthPercentageExpression(_))), "{property}");
        assert!(s.get(property).unwrap().contains('('), "{property}");
    }
    assert_eq!(s.get("border-top-left-radius"), Some("calc(10% + 2px) calc(20px + 5%)"));
    assert_eq!(style("div{margin-inline:calc(10% + 2px) 4px}").get("margin-left"), Some("calc(10% + 2px)"));
    assert_eq!(style("div{--amount:20%;max-height:calc(var(--amount) + 5px)}").get("max-height"), Some("calc(20% + 5px)"));
}

#[test]
fn invalid_math_cannot_displace_valid_declarations() {
    for invalid in ["calc(2px * 3px)", "calc(2px + 3deg)", "calc(2px +)", "clamp(2px,3px)", "calc(2px/1px)"] {
        assert_eq!(style(&format!("div{{width:17px;width:{invalid}}}")).get("width"), Some("17px"), "{invalid}");
    }
    assert_eq!(style("div{--bad:3deg;width:17px;width:calc(2px + var(--bad))}").get("width"), Some("auto"));
}

#[test]
fn pinned_css_values_cases_reach_computed_style() {
    // The declarations are from the pinned WPT calc block and radius cases.
    for expression in [
        "calc(50px)", "calc(50%)", "calc(25px + 50%)",
        "calc(150% / 2 - 30px)", "calc(40px + 10% - 20% / 2)",
        "calc(40px - 10%)",
    ] {
        assert_eq!(style(&format!("div{{max-height:{expression}}}")).get("max-height"), Some(expression), "{expression}");
    }
    for expression in [
        "calc(50% - 3px)", "calc(5em - 3px)", "calc(5em - 0%)",
        "calc(50%)", "calc(50px)", "calc(25% + 25%)",
    ] {
        assert_eq!(style(&format!("div{{min-width:{expression}}}")).get("min-width"), Some(expression), "{expression}");
    }
    let radius = style("div{border-radius:calc((1/32 * 100%) + 5px) calc((1/64 * 100%) - 2px) calc(10px + (1/256 * 100%)) calc((1/16 * 100%) - 3px) / calc((1/32 * 100%) - (1px + (1/128 * 100%))) calc(1/16 * 100%) calc(10px)}");
    assert_eq!(radius.get("border-top-left-radius"), Some("calc((1/32 * 100%) + 5px) calc((1/32 * 100%) - (1px + (1/128 * 100%)))"));
    assert_eq!(radius.get("border-bottom-right-radius"), Some("calc(10px + (1/256 * 100%)) 10px"));
    assert_eq!(style("div{--margin:2px;width:calc(50% - 2 * var(--margin));margin-inline:var(--margin)}").get("width"), Some("calc(50% - 2 * 2px)"));
}

#[test]
fn pinned_flex_gap_logical_sizing_reaches_physical_layout_inputs() {
    // css-flexbox/gap-006-rtl.html and gap-006-lr.html at the pinned WPT revision.
    for (axes, width, height) in [
        ("direction:rtl", "200px", "100px"),
        ("writing-mode:vertical-lr", "100px", "200px"),
    ] {
        let s = style(&format!("div{{{axes};block-size:100px;inline-size:200px}}"));
        assert_eq!(s.get("width"), Some(width), "{axes}");
        assert_eq!(s.get("height"), Some(height), "{axes}");
    }
}

#[test]
fn box_shorthands_keep_spaced_math_functions_whole() {
    let s = style("div{margin:calc(1em / 8) 3px;padding:calc(2px + 1px) min(1px, 2px) 4px}");
    assert_eq!(s.get("margin-top"), Some("calc(1em / 8)"));
    assert_eq!(s.get("margin-left"), Some("3px"));
    assert_eq!(s.get("margin-bottom"), Some("calc(1em / 8)"));
    assert_eq!(s.get("padding-top").map(|v| v.contains("calc") || v == "3px"), Some(true));
    assert_eq!(s.get("padding-right").map(|v| v.starts_with("min(") || v == "1px"), Some(true));
    assert_eq!(s.get("padding-bottom"), Some("4px"));
}
