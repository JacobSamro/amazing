//! CSS-23: `light-dark()` (CSS Color 5) and the `color-scheme` property
//! (CSS Color Adjustment 1) that selects its used branch.
use amazing_css::{ColorScheme, MediaEnvironment, compute, compute_with_environment, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn env(scheme: ColorScheme) -> MediaEnvironment {
    MediaEnvironment { prefers_color_scheme: scheme, ..Default::default() }
}

#[test]
fn light_dark_follows_environment_when_color_scheme_is_normal() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet("div { background-color: light-dark(white, black); }");
    let light = compute_with_environment(&dom, &[sheet.clone()], &env(ColorScheme::Light));
    assert_eq!(light.get(element).unwrap().get("background-color"), Some("#ffffffff"));
    let dark = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Dark));
    assert_eq!(dark.get(element).unwrap().get("background-color"), Some("#000000ff"));
}

#[test]
fn color_scheme_forces_a_branch_regardless_of_environment_preference() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(
        "div { color-scheme: dark; background-color: light-dark(white, black); }",
    );
    // Even though the environment prefers light, the forced `dark` scheme wins.
    let styled = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Light));
    assert_eq!(styled.get(element).unwrap().get("background-color"), Some("#000000ff"));

    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(
        "div { color-scheme: only light; background-color: light-dark(white, black); }",
    );
    let styled = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Dark));
    assert_eq!(styled.get(element).unwrap().get("background-color"), Some("#ffffffff"));
}

#[test]
fn color_scheme_with_both_schemes_still_follows_the_environment() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(
        "div { color-scheme: light dark; background-color: light-dark(white, black); }",
    );
    let styled = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Dark));
    assert_eq!(styled.get(element).unwrap().get("background-color"), Some("#000000ff"));
}

#[test]
fn color_scheme_inherits_to_descendants() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let child = dom.create_element(html, "div");
    let sheet = parse_stylesheet(
        "html { color-scheme: dark; } div { background-color: light-dark(white, black); }",
    );
    let styled = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Light));
    assert_eq!(styled.get(child).unwrap().get("color-scheme"), Some("dark"));
    assert_eq!(styled.get(child).unwrap().get("background-color"), Some("#000000ff"));
}

#[test]
fn light_dark_currentcolor_branch_uses_the_elements_own_color() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    let sheet = parse_stylesheet(
        "div { color: red; color-scheme: dark; border-top-color: light-dark(blue, currentcolor); }",
    );
    let styled = compute_with_environment(&dom, &[sheet], &env(ColorScheme::Light));
    assert_eq!(styled.get(element).unwrap().get("border-top-color"), Some("#ff0000ff"));
}

#[test]
fn invalid_light_dark_does_not_displace_the_cascade_winner() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for invalid in ["light-dark(red)", "light-dark(bogus, blue)", "light-dark(red, blue, green)"] {
        let sheet = parse_stylesheet(&format!(
            "div {{ background-color: red; background-color: {invalid}; }}"
        ));
        let styled = compute(&dom, &[sheet]);
        // An invalid `light-dark()` is dropped before the cascade, so the
        // earlier valid `red` declaration remains the winner (its literal
        // text: unlike the relative-color/light-dark seams, plain named
        // colors are not normalized to hex at computed-value time).
        assert_eq!(styled.get(element).unwrap().get("background-color"), Some("red"), "{invalid}");
    }
}

#[test]
fn invalid_color_scheme_does_not_displace_the_cascade_winner() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for invalid in ["light dark dark", "loud", "only only", ""] {
        let sheet = parse_stylesheet(&format!(
            "div {{ color-scheme: light; color-scheme: {invalid}; }}"
        ));
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(element).unwrap().get("color-scheme"), Some("light"), "{invalid:?}");
    }
}

#[test]
fn color_scheme_grammar_accepts_the_documented_forms() {
    let mut dom = Dom::new();
    let element = dom.create_element(NodeIdx(0), "div");
    for value in ["normal", "light", "dark", "light dark", "dark light",
        "only dark", "dark only", "light dark only"] {
        let sheet = parse_stylesheet(&format!("div {{ color-scheme: {value}; }}"));
        let styled = compute(&dom, &[sheet]);
        assert_eq!(styled.get(element).unwrap().get("color-scheme"), Some(value), "{value}");
    }
}
