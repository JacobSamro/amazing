use amazing_css::{
    compute, parse_stylesheet,
    values::{self, ComputedValue, WhiteSpace},
};
use amazing_dom::{Dom, NodeIdx};
fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let div = dom.create_element(body, "div");
    compute(&dom, &[parse_stylesheet(&format!("div{{{css}}}"))])
        .get(div)
        .unwrap()
        .clone()
}
#[test]
fn whitespace_and_wrap_shorthands_share_only_mode() {
    for (value, collapse, mode) in [
        ("normal", "collapse", "wrap"),
        ("nowrap", "collapse", "nowrap"),
        ("pre", "preserve", "nowrap"),
        ("pre-wrap", "preserve", "wrap"),
        ("pre-line", "preserve-breaks", "wrap"),
        ("break-spaces", "break-spaces", "wrap"),
    ] {
        let s = style(&format!("white-space:{value}"));
        assert_eq!(s.get("white-space"), Some(value));
        assert_eq!(s.get("white-space-collapse"), Some(collapse));
        assert_eq!(s.get("text-wrap-mode"), Some(mode));
        assert_eq!(s.get("white-space-trim"), Some("none"));
    }
    let s = style("white-space:pre;text-wrap:wrap balance");
    assert_eq!(s.get("white-space"), Some("pre-wrap"));
    assert_eq!(s.get("text-wrap"), Some("balance"));
    assert_eq!(
        s.typed("white-space"),
        Some(&ComputedValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    let mut s = style("text-wrap:nowrap pretty;white-space:normal");
    assert_eq!(s.get("text-wrap"), Some("pretty"));
    s.set("text-wrap-mode", "nowrap");
    assert_eq!(s.get("white-space"), Some("nowrap"));
    assert_eq!(s.get("text-wrap"), Some("nowrap pretty"));
    for css in [
        "white-space:preserve nowrap discard-after discard-before",
        "white-space:discard-before nowrap discard-after preserve",
    ] {
        let s = style(css);
        assert_eq!(s.get("white-space-collapse"), Some("preserve"));
        assert_eq!(
            s.get("white-space-trim"),
            Some("discard-before discard-after")
        );
    }
}
#[test]
fn keyword_flags_are_exclusive_canonical_and_invalid_atomically() {
    for (property, valid, expected) in [
        (
            "text-transform",
            "full-size-kana uppercase full-width",
            "uppercase full-width full-size-kana",
        ),
        (
            "text-autospace",
            "replace punctuation ideograph-numeric ideograph-alpha",
            "ideograph-alpha ideograph-numeric punctuation replace",
        ),
        (
            "hanging-punctuation",
            "last first allow-end",
            "first allow-end last",
        ),
        (
            "white-space-trim",
            "discard-inner discard-before",
            "discard-before discard-inner",
        ),
        (
            "word-space-transform",
            "auto-phrase ideographic-space",
            "ideographic-space auto-phrase",
        ),
        (
            "text-justify",
            "no-compress inter-word",
            "inter-word no-compress",
        ),
    ] {
        assert!(values::parses(property, valid));
        assert_eq!(
            style(&format!("{property}:{valid}")).get(property),
            Some(expected)
        );
        assert!(!values::parses(property, &format!("{valid} {valid}")));
    }
    for property in [
        "white-space",
        "white-space-collapse",
        "text-wrap-mode",
        "text-wrap-style",
        "text-group-align",
        "text-autospace",
        "text-spacing",
    ] {
        for wide in ["initial", "inherit", "unset", "revert", "revert-layer"] {
            assert!(values::parses(property, wide), "{property}: {wide}");
        }
    }
    for css in [
        "text-spacing:normal trim-start",
        "text-spacing:trim-start normal",
        "text-spacing:normal normal",
    ] {
        let value = style(css);
        assert_eq!(value.get("text-autospace"), Some("normal"));
    }
    for (property, bad) in [
        ("text-transform", "uppercase lowercase"),
        ("text-transform", "none full-width"),
        ("text-transform", "math-auto uppercase"),
        ("text-autospace", "insert replace"),
        ("text-autospace", "no-autospace ideograph-alpha"),
        ("white-space", "pre nowrap"),
        ("white-space", "wrap nowrap"),
        ("text-wrap", "balance pretty"),
        ("text-wrap-mode", "wrap balance"),
        ("text-spacing", "normal auto"),
        ("hanging-punctuation", "force-end allow-end"),
        ("word-space-transform", "auto-phrase"),
        ("word-break", "normal keep-all"),
        ("line-break", "none"),
        ("hyphens", "auto none"),
    ] {
        assert!(!values::parses(property, bad), "{property}:{bad}");
    }
    assert_eq!(
        style("white-space:pre;white-space:pre nowrap").get("white-space"),
        Some("pre")
    );
    assert_eq!(
        style("word-wrap:anywhere").get("overflow-wrap"),
        Some("anywhere")
    );
    assert_eq!(
        style("overflow-wrap:break-word").get("word-wrap"),
        Some("break-word")
    );
}
#[test]
fn variables_and_inheritance_reconstruct_winning_axes() {
    let s = style("--w:preserve nowrap;--t:balance wrap;white-space:var(--w);text-wrap:var(--t)");
    assert_eq!(s.get("white-space"), Some("pre-wrap"));
    assert_eq!(s.get("text-wrap-style"), Some("balance"));
    let s = style("white-space:pre;--bad:wrap nowrap;white-space:var(--bad)");
    assert_eq!(s.get("white-space"), Some("normal"));
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let div = dom.create_element(html, "div");
    let span = dom.create_element(div, "span");
    let styles = compute(
        &dom,
        &[parse_stylesheet(
            "div{white-space:preserve nowrap discard-before;text-wrap-style:balance;text-transform:full-width uppercase}span{white-space-collapse:preserve;text-wrap-mode:wrap;--t:inherit;text-wrap-style:var(--t)}",
        )],
    );
    let s = styles.get(span).unwrap();
    assert_eq!(s.get("white-space-trim"), Some("none"));
    assert_eq!(s.get("white-space"), Some("pre-wrap"));
    assert_eq!(
        s.typed("white-space"),
        Some(&ComputedValue::WhiteSpace(WhiteSpace::PreWrap))
    );
    assert_eq!(s.get("text-wrap"), Some("balance"));
    assert_eq!(s.get("text-transform"), Some("uppercase full-width"));
}
#[test]
fn spacing_summary_tracks_each_longhand_and_defaults() {
    let mut s = style("text-spacing:trim-start ideograph-alpha;text-autospace:no-autospace");
    assert_eq!(s.get("text-spacing"), Some("trim-start no-autospace"));
    s.set("text-spacing-trim", "space-all");
    assert_eq!(s.get("text-spacing"), Some("none"));
    assert_eq!(
        style("text-spacing:auto").get("text-autospace"),
        Some("auto")
    );
    assert_eq!(
        style("text-spacing:none").get("text-spacing-trim"),
        Some("space-all")
    );
    assert_eq!(
        style("text-spacing:trim-all;--s:ideograph-numeric replace;text-spacing:var(--s)")
            .get("text-spacing-trim"),
        Some("normal")
    );
}
