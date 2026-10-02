use amazing_css::{blockified_display, compute, parse_stylesheet, values};
use amazing_dom::{Dom, NodeIdx};
fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let node = dom.create_element(body, "div");
    compute(&dom, &[parse_stylesheet(&format!("div{{{css}}}"))])
        .get(node)
        .unwrap()
        .clone()
}
#[test]
fn unordered_outer_inner_and_list_item_grammar() {
    for outer in ["block", "inline", "run-in"] {
        for inner in [
            "flow",
            "flow-root",
            "table",
            "flex",
            "grid",
            "ruby",
            "grid-lanes",
        ] {
            let a = format!("{outer} {inner}");
            let b = format!("{inner} {outer}");
            assert!(values::parses("display", &a));
            assert!(values::parses("display", &b));
            assert_eq!(
                style(&format!("display:{a}")).get("display"),
                style(&format!("display:{b}")).get("display")
            );
        }
        for inner in ["flow", "flow-root"] {
            let words = [outer, inner, "list-item"];
            let expected = style(&format!("display:{}", words.join(" ")))
                .get("display")
                .unwrap()
                .to_owned();
            for [a, b, c] in [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ] {
                let display = format!("{} {} {}", words[a], words[b], words[c]);
                assert!(values::parses("display", &display));
                assert_eq!(
                    style(&format!("display:{display}")).get("display"),
                    Some(expected.as_str())
                );
            }
        }
    }
    for value in [
        "inline inline",
        "grid flex",
        "list-item list-item",
        "block none",
        "contents inline",
        "inline-block flow",
        "flow grid",
        "inline table list-item",
        "ruby list-item",
        "block flow list-item extra",
    ] {
        assert!(!values::parses("display", value), "{value}");
    }
}
#[test]
fn default_axes_and_var_context_keep_typed_display() {
    for (specified, expected) in [
        ("flow", "block"),
        ("flow list-item", "list-item"),
        ("ruby", "ruby"),
        ("list-item flow-root", "flow-root list-item"),
        ("INLINE-FLEX", "inline-flex"),
        ("CONTENTS", "contents"),
        ("table-row", "table-row"),
    ] {
        let value = style(&format!("--d:{specified};display:var(--d)"));
        assert_eq!(value.get("display"), Some(expected));
        assert!(matches!(
            value.typed("display"),
            Some(values::ComputedValue::Display(_))
        ));
    }
    for display in [
        "run-in",
        "run-in flex",
        "run-in grid",
        "run-in table",
        "run-in ruby",
        "inline list-item",
        "inline flow-root list-item",
    ] {
        let value = style(&format!(
            "--d:{display};display:var(--d);position:absolute;float:left"
        ));
        assert_eq!(
            value.get("display"),
            Some(blockified_display(display).as_str())
        );
        assert_eq!(value.display_before_blockification(), Some(display));
        assert_eq!(value.get("float"), Some("none"));
    }
    assert_eq!(
        style("display:flex;--d:block none;display:var(--d)").get("display"),
        Some("inline")
    );
}

#[test]
fn list_item_flag_generates_markers_for_compound_displays() {
    use amazing_css::selectors::PseudoElement;
    for display in [
        "list-item",
        "inline list-item",
        "flow-root list-item",
        "inline flow-root list-item",
        "run-in list-item",
    ] {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let node = dom.create_element(body, "div");
        let styles = compute(
            &dom,
            &[parse_stylesheet(&format!(
                "div{{display:{display};list-style-type:disc}}div::marker{{content:'x'}}"
            ))],
        );
        assert!(
            styles.get_pseudo(node, PseudoElement::Marker).is_some(),
            "{display}"
        );
    }
}
