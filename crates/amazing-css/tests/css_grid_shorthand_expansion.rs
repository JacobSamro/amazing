use amazing_css::{ComputedStyle, compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let element = dom.create_element(html, "div");
    compute(&dom, &[parse_stylesheet(&format!("div{{{css}}}"))])
        .get(element)
        .unwrap()
        .clone()
}

#[test]
fn template_strings_expand_areas_tracks_and_line_boundaries_together() {
    let s = style("grid-template:[start]\"a a\"50px[middle]\"b b\"minmax(10px,1fr)[end]/20px 30px");
    assert_eq!(
        s.get("grid-template-rows"),
        Some("[start] 50px [middle] minmax(10px, 1fr) [end]")
    );
    assert_eq!(s.get("grid-template-columns"), Some("20px 30px"));
    assert_eq!(s.get("grid-template-areas"), Some("\"a a\" \"b b\""));
    let s = style("grid-template:[a]\"first\"[b][c]\"second\"[d]");
    assert_eq!(s.get("grid-template-columns"), Some("none"));
    assert_eq!(
        s.get("grid-template-rows"),
        Some("[a] auto [b][c] auto [d]")
    );
    assert_eq!(s.get("grid-template-areas"), Some("\"first\" \"second\""));
}

#[test]
fn template_and_grid_reset_only_their_own_complete_longhand_sets() {
    let s = style(
        "grid-template-areas:\"old\";grid-auto-rows:7px;grid-auto-columns:8px;grid-auto-flow:column dense;grid-template:20px/30px",
    );
    assert_eq!(s.get("grid-template-areas"), Some("none"));
    assert_eq!(s.get("grid-auto-rows"), Some("7px"));
    assert_eq!(s.get("grid-auto-columns"), Some("8px"));
    assert_eq!(s.get("grid-auto-flow"), Some("column dense"));
    let s = style(
        "grid-auto-rows:7px;grid-auto-columns:8px;grid-auto-flow:column dense;grid:20px/30px",
    );
    assert_eq!(s.get("grid-template-rows"), Some("20px"));
    assert_eq!(s.get("grid-template-columns"), Some("30px"));
    assert_eq!(s.get("grid-template-areas"), Some("none"));
    assert_eq!(s.get("grid-auto-rows"), Some("auto"));
    assert_eq!(s.get("grid-auto-columns"), Some("auto"));
    assert_eq!(s.get("grid-auto-flow"), Some("row"));
}

#[test]
fn both_auto_flow_axes_expand_default_and_multiple_implicit_tracks() {
    for (css, axis, explicit, implicit) in [
        (
            "grid:10px 20px/auto-flow dense minmax(1px,1fr)fit-content(50px)",
            "column dense",
            "grid-template-rows",
            "grid-auto-columns",
        ),
        (
            "grid:dense auto-flow minmax(1px,1fr)fit-content(50px)/10px 20px",
            "row dense",
            "grid-template-columns",
            "grid-auto-rows",
        ),
    ] {
        let s = style(css);
        assert_eq!(s.get("grid-auto-flow"), Some(axis));
        assert_eq!(s.get(explicit), Some("10px 20px"));
        assert_eq!(s.get(implicit), Some("minmax(1px, 1fr) fit-content(50px)"));
        assert_eq!(s.get("grid-template-areas"), Some("none"));
    }
    for (css, axis) in [
        ("grid:none/auto-flow", "column"),
        ("grid:auto-flow/none", "row"),
    ] {
        let s = style(css);
        assert_eq!(s.get("grid-auto-flow"), Some(axis));
        assert_eq!(s.get("grid-auto-rows"), Some("auto"));
        assert_eq!(s.get("grid-auto-columns"), Some("auto"));
    }
}

#[test]
fn all_six_grid_longhands_reset_for_none_and_css_wide_values() {
    for value in ["none", "initial", "unset"] {
        let s = style(&format!(
            "grid-template-areas:\"old\";grid-auto-columns:10px;grid-auto-rows:20px;grid-auto-flow:column dense;grid:{value}"
        ));
        for name in [
            "grid-template-rows",
            "grid-template-columns",
            "grid-template-areas",
        ] {
            assert_eq!(s.get(name), Some("none"), "{value} {name}");
        }
        for name in ["grid-auto-rows", "grid-auto-columns"] {
            assert_eq!(s.get(name), Some("auto"));
        }
        assert_eq!(s.get("grid-auto-flow"), Some("row"));
    }
}

#[test]
fn deferred_template_and_grid_values_resolve_complete_shapes() {
    for (property, tracks, rows, cols, flow, areas) in [
        (
            "grid-template",
            "\"a a\"50px/10px 20px",
            "50px",
            "10px 20px",
            "row",
            "\"a a\"",
        ),
        (
            "grid",
            "auto-flow dense 25px/10px 20px",
            "none",
            "10px 20px",
            "row dense",
            "none",
        ),
        (
            "grid",
            "50px/auto-flow 10px 20px",
            "50px",
            "none",
            "column",
            "none",
        ),
    ] {
        let s = style(&format!("--tracks:{tracks};{property}:var(--tracks)"));
        assert_eq!(s.get("grid-template-rows"), Some(rows));
        assert_eq!(s.get("grid-template-columns"), Some(cols));
        assert_eq!(s.get("grid-auto-flow"), Some(flow));
        assert_eq!(s.get("grid-template-areas"), Some(areas));
    }
    let s = style("--g:initial;grid:var(--g)");
    assert_eq!(s.get("grid-auto-columns"), Some("auto"));
    assert_eq!(s.get("grid-template-areas"), Some("none"));
    let s = style("grid:var(--absent)");
    assert_eq!(s.get("grid-template-rows"), Some("none"));
    assert_eq!(s.get("grid-auto-flow"), Some("row"));
}

#[test]
fn named_cell_tokens_null_runs_and_digit_names_serialize_canonically() {
    for (specified, expected) in [
        ("\"head...main\"", "\"head . main\""),
        ("\"1st  2nd\"", "\"1st 2nd\""),
        ("\".... ....\"", "\". .\""),
        ("\"\u{65e5} \u{672c}\"", "\"\u{65e5} \u{672c}\""),
        ("\"head head\"\"body body\"", "\"head head\" \"body body\""),
    ] {
        let s = style(&format!("grid-template-areas:{specified}"));
        assert_eq!(s.get("grid-template-areas"), Some(expected), "{specified}");
    }
}

#[test]
fn area_rectangles_reject_disconnected_regions_and_trash_tokens() {
    for value in [
        "\"a a\"\"a .\"",
        "\"a . a\"",
        "\"a b\"\"b a\"",
        "\"a b\"\"a\"",
        "\"a/b\"",
        "\"a,b\"",
        "\" \"",
        "\"\"",
    ] {
        let s = style(&format!("grid-template-areas:{value}"));
        assert_eq!(s.get("grid-template-areas"), Some("none"), "{value}");
    }
    let s = style("grid-template:\"head head\"\"head head\"/10px 20px");
    assert_eq!(
        s.get("grid-template-areas"),
        Some("\"head head\" \"head head\"")
    );
    assert_eq!(s.get("grid-template-rows"), Some("auto auto"));
    let s = style("grid-template:\"a a\"\"a .\"/10px 20px");
    assert_eq!(s.get("grid-template-rows"), Some("none"));
}

#[test]
fn invalid_shorthand_arity_names_and_both_auto_axes_leave_previous_values() {
    for value in [
        "auto-flow/auto-flow",
        "10px/auto-flow dense dense",
        "10px/auto-flow repeat(2,10px)",
        "\"a\" repeat(2,10px)/10px",
        "10px/20px/30px",
        "[span]10px/20px",
    ] {
        let s = style(&format!("grid:1px/2px;grid:{value}"));
        assert_eq!(s.get("grid-template-rows"), Some("1px"), "{value}");
        assert_eq!(s.get("grid-template-columns"), Some("2px"), "{value}");
    }
}

#[test]
fn area_and_implicit_longhand_overrides_preserve_cascade_order() {
    let s = style("grid:\"a\"50px/20px;grid-template-areas:\"later\";grid-auto-columns:33px");
    assert_eq!(s.get("grid-template-areas"), Some("\"later\""));
    assert_eq!(s.get("grid-template-rows"), Some("50px"));
    assert_eq!(s.get("grid-auto-columns"), Some("33px"));
    let s = style("grid-template-areas:\"first\";grid:\"last\"/20px");
    assert_eq!(s.get("grid-template-areas"), Some("\"last\""));
}

#[test]
fn explicit_inheritance_carries_areas_and_flow_without_implicit_inheritance() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let element = dom.create_element(html, "div");
    let sheet = parse_stylesheet(
        "html{grid:\"parent\"30px/40px;grid-auto-flow:column dense}div{grid:inherit}",
    );
    let s = compute(&dom, &[sheet]);
    assert_eq!(
        s.get(element).unwrap().get("grid-template-areas"),
        Some("\"parent\"")
    );
    assert_eq!(
        s.get(element).unwrap().get("grid-template-rows"),
        Some("30px")
    );
    assert_eq!(
        s.get(element).unwrap().get("grid-auto-flow"),
        Some("column dense")
    );
    let s = compute(
        &dom,
        &[parse_stylesheet(
            "html{grid-template-areas:\"parent\"}div{}",
        )],
    );
    assert_eq!(
        s.get(element).unwrap().get("grid-template-areas"),
        Some("none")
    );
}
