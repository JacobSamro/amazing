use amazing_css::{ComputedStyle, compute, parse_stylesheet, values};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let div = dom.create_element(html, "div");
    compute(
        &dom,
        &[parse_stylesheet(&format!(
            "html{{font-size:20px}} div{{{css}}}"
        ))],
    )
    .get(div)
    .unwrap()
    .clone()
}

#[test]
fn full_keyword_length_percentage_and_math_grammar() {
    for css in [
        "normal",
        "initial",
        r"\69nitial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "infinite",
        "NORMAL",
        "0",
        "0px",
        "10%",
        "1in",
        "2em",
        "calc(-1em + 10px)",
        "min(2em, 50px)",
        "clamp(1px, 4em, 100px)",
    ] {
        assert!(values::parses("flow-tolerance", css), "{css}");
    }
    for css in [
        "",
        "-1px",
        "-2%",
        "-3em",
        "1",
        "auto",
        "none",
        "row",
        "1fr",
        "1deg",
        "normal infinite",
        "10px 20px",
        "normal 10%",
        "calc(2px + 3s)",
        "min()",
        "calc()",
    ] {
        assert!(!values::parses("flow-tolerance", css), "{css}");
    }
}

#[test]
fn specified_negative_lengths_reject_but_math_results_clamp() {
    assert_eq!(
        style("flow-tolerance:10px;flow-tolerance:-1px").get("flow-tolerance"),
        Some("10px")
    );
    assert_eq!(
        style("flow-tolerance:calc(-1em + 10px)").get("flow-tolerance"),
        Some("0px")
    );
    assert_eq!(
        style("flow-tolerance:calc(0.5em + 10px)").get("flow-tolerance"),
        Some("20px")
    );
}

#[test]
fn absolute_and_font_units_use_real_computed_font_context() {
    for (css, expected) in [
        ("1in", "96px"),
        ("2em", "40px"),
        ("2rem", "40px"),
        ("min(2em,50px)", "40px"),
        ("max(2em,50px)", "50px"),
        ("clamp(1px,4em,100px)", "80px"),
    ] {
        assert_eq!(
            style(&format!("flow-tolerance:{css}")).get("flow-tolerance"),
            Some(expected),
            "{css}"
        );
    }
    assert_eq!(
        style("font-size:30px;flow-tolerance:2em").get("flow-tolerance"),
        Some("60px")
    );
}

#[test]
fn root_and_descendant_rem_use_the_actual_root_computed_font() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let div = dom.create_element(html, "div");
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "html{font-size:20px;flow-tolerance:2rem} div{font-size:30px;flow-tolerance:1rem}",
        )],
    );
    assert_eq!(
        styles.get(html).unwrap().get("flow-tolerance"),
        Some("40px")
    );
    assert_eq!(styles.get(div).unwrap().get("flow-tolerance"), Some("20px"));
}

#[test]
fn grid_axis_percentages_stay_symbolic_and_pure_math_clamps() {
    assert_eq!(
        style("flow-tolerance:50%").get("flow-tolerance"),
        Some("50%")
    );
    assert_eq!(
        style("flow-tolerance:calc(10% + 5%)").get("flow-tolerance"),
        Some("15%")
    );
    assert_eq!(
        style("flow-tolerance:calc(-10%)").get("flow-tolerance"),
        Some("0%")
    );
    assert_eq!(
        style("flow-tolerance:calc(10% + 2px)").get("flow-tolerance"),
        Some("calc(10% + 2px)")
    );
}

#[test]
fn custom_properties_share_validation_and_computed_value_fallback() {
    assert_eq!(
        style("--t:2em;flow-tolerance:var(--t)").get("flow-tolerance"),
        Some("40px")
    );
    assert_eq!(
        style("flow-tolerance:var(--missing, 2em)").get("flow-tolerance"),
        Some("40px")
    );
    assert_eq!(
        style("--t:-1px;flow-tolerance:10px;flow-tolerance:var(--t)").get("flow-tolerance"),
        Some("normal")
    );
}

#[test]
fn keywords_canonicalize_and_defaults_do_not_accidentally_inherit() {
    assert_eq!(style("").get("flow-tolerance"), Some("normal"));
    assert_eq!(
        style("flow-tolerance:INFINITE").get("flow-tolerance"),
        Some("infinite")
    );
    assert_eq!(
        style("flow-tolerance:NORMAL").get("flow-tolerance"),
        Some("normal")
    );
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let child = dom.create_element(html, "div");
    let inherited = dom.create_element(html, "span");
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "html{font-size:20px;flow-tolerance:2em} span{font-size:30px;flow-tolerance:inherit}",
        )],
    );
    assert_eq!(
        styles.get(child).unwrap().get("flow-tolerance"),
        Some("normal")
    );
    assert_eq!(
        styles.get(inherited).unwrap().get("flow-tolerance"),
        Some("40px")
    );
}

#[test]
fn author_set_and_csswide_reset_share_the_installed_producer() {
    let mut s = style("flow-tolerance:10px");
    s.set("flow-tolerance", "2em");
    assert_eq!(s.get("flow-tolerance"), Some("40px"));
    s.set("flow-tolerance", "-1px");
    assert_eq!(s.get("flow-tolerance"), Some("40px"));
    s.set("flow-tolerance", "INFINITE/**/");
    assert_eq!(s.get("flow-tolerance"), Some("infinite"));
    s.set("flow-tolerance", r"\6e ormal");
    assert_eq!(s.get("flow-tolerance"), Some("normal"));
    s.set("flow-tolerance", "normal");
    assert_eq!(s.get("flow-tolerance"), Some("normal"));
}
