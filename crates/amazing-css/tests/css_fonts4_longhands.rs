use amazing_css::{compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};

fn styles(css: &str) -> (amazing_css::StyledDom, NodeIdx, NodeIdx) {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(root, "span");
    (compute(&dom, &[parse_stylesheet(css)]), root, child)
}

#[test]
fn fonts4_longhands_accept_and_inherit() {
    let declarations = [
        ("font-kerning", "normal"),
        ("font-variant-caps", "all-small-caps"),
        (
            "font-variant-numeric",
            "oldstyle-nums tabular-nums slashed-zero",
        ),
        ("font-variant-east-asian", "jis04 full-width ruby"),
        ("font-variant-alternates", "stylistic(alt) historical-forms"),
        ("font-variant-position", "super"),
        ("font-variant-emoji", "text"),
        ("font-synthesis-weight", "none"),
        ("font-synthesis-style", "none"),
        ("font-synthesis-small-caps", "none"),
        ("font-synthesis-position", "none"),
        ("font-optical-sizing", "none"),
        ("font-palette", "dark"),
        ("font-size-adjust", "0.5"),
        ("font-language-override", "\"SRB\""),
        ("font-variation-settings", "\"wght\" 625"),
    ];
    for (name, value) in declarations {
        let (computed, root, child) = styles(&format!("div {{ {name}: {value} }}"));
        assert_eq!(computed.get(root).unwrap().get(name), Some(value), "{name}");
        assert_eq!(
            computed.get(child).unwrap().get(name),
            Some(value),
            "{name} inheritance"
        );
        let (computed, root, _) = styles(&format!(
            "div {{ {name}: {value}; {name}: invalid-value extra }}"
        ));
        assert_eq!(
            computed.get(root).unwrap().get(name),
            Some(value),
            "{name} invalid fallback"
        );
        let (computed, root, _) = styles(&format!("div {{ {name}: invalid-value extra }} @supports ({name}: {value}) {{ div {{ {name}: {value} }} }}"));
        assert_eq!(
            computed.get(root).unwrap().get(name),
            Some(value),
            "{name} @supports"
        );
    }
}

#[test]
fn font_shorthand_resets_font_owned_longhands_only() {
    let (computed, _, child) = styles(
        "div { font-kerning:normal; font-variant-caps:all-small-caps; font-variant-numeric:tabular-nums; \
         font-optical-sizing:none; font-variation-settings:'wght' 625; font-palette:dark; \
         font-synthesis-weight:none } span { font: small-caps 16px serif }",
    );
    let style = computed.get(child).unwrap();
    for (name, expected) in [
        ("font-kerning", "auto"),
        ("font-variant-caps", "small-caps"),
        ("font-variant-numeric", "normal"),
        ("font-optical-sizing", "auto"),
        ("font-variation-settings", "normal"),
        ("font-palette", "dark"),
        ("font-synthesis-weight", "none"),
    ] {
        assert_eq!(style.get(name), Some(expected), "{name}");
    }
}

#[test]
fn synthesis_shorthand_expands_and_supports_checks_grammar() {
    let (computed, root, _) = styles("div { font-synthesis: weight position; }");
    let style = computed.get(root).unwrap();
    for (name, expected) in [
        ("font-synthesis-weight", "auto"),
        ("font-synthesis-style", "none"),
        ("font-synthesis-small-caps", "none"),
        ("font-synthesis-position", "auto"),
    ] {
        assert_eq!(style.get(name), Some(expected), "{name}");
    }
    let (computed, root, _) = styles(
        "div { font-kerning:normal } @supports (font-kerning: bogus) { div { font-kerning:none } }",
    );
    assert_eq!(
        computed.get(root).unwrap().get("font-kerning"),
        Some("normal")
    );
    let (computed, root, _) = styles("div { font-synthesis:none } @supports (font-synthesis: weight weight) { div { font-synthesis: style } }");
    assert_eq!(
        computed.get(root).unwrap().get("font-synthesis-weight"),
        Some("none")
    );
}
