use amazing_css::{ComputedStyle, compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
fn style(css: &str) -> ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let parent = dom.create_element(html, "div");
    let child = dom.create_element(parent, "p");
    compute(&dom, &[parse_stylesheet(css)])
        .get(child)
        .unwrap()
        .clone()
}
fn border(style: &ComputedStyle, side: &str, expected: [&str; 3]) {
    for (part, expected) in ["width", "style", "color"].into_iter().zip(expected) {
        assert_eq!(
            style.get(&format!("border-{side}-{part}")),
            Some(expected),
            "{side}-{part}"
        );
    }
}

#[test]
fn each_side_accepts_any_component_order_without_affecting_other_sides() {
    for side in SIDES {
        for value in [
            "2px solid red",
            "solid red 2px",
            "red 2px solid",
            "red solid 2px",
            "solid 2px red",
            "2px red solid",
        ] {
            let s = style(&format!("p{{border-{side}:{value}}}"));
            border(&s, side, ["2px", "solid", "red"]);
            for other in SIDES.into_iter().filter(|other| *other != side) {
                border(&s, other, ["medium", "none", "currentcolor"]);
                assert!(!s.is_author_specified(&format!("border-{other}-style")));
            }
        }
    }
}

#[test]
fn partial_shorthands_reset_omitted_components() {
    for side in SIDES {
        for (value, expected) in [
            ("solid", ["medium", "solid", "currentcolor"]),
            ("red", ["medium", "none", "red"]),
            ("0", ["0", "none", "currentcolor"]),
            ("thin dashed", ["thin", "dashed", "currentcolor"]),
            ("none", ["medium", "none", "currentcolor"]),
        ] {
            let s = style(&format!(
                "p{{border-{side}:9px double blue; border-{side}:{value}}}"
            ));
            border(&s, side, expected);
            for part in ["width", "style", "color"] {
                assert!(s.is_author_specified(&format!("border-{side}-{part}")));
            }
        }
    }
}

#[test]
fn invalid_shorthands_are_rejected_atomically() {
    for side in SIDES {
        for value in [
            "",
            "solid solid",
            "1px 2px",
            "red blue",
            "-1px solid",
            "1% solid",
            "auto solid",
            "solid banana",
            "solid initial",
            "1px solid red blue",
            "1px solid rgb(1,2)",
            "1px solid garbage(1)",
            "1px solid red !oops",
        ] {
            let s = style(&format!(
                "p{{border-{side}:7px dotted blue; border-{side}:{value}}}"
            ));
            border(&s, side, ["7px", "dotted", "blue"]);
        }
    }
}

#[test]
fn color_functions_and_token_boundaries_survive_expansion() {
    for side in SIDES {
        let s = style(&format!("p{{border-{side}:2px solid rgb(1, 2, 3)}}"));
        assert_eq!(
            s.typed(&format!("border-{side}-color")),
            Some(&amazing_css::values::ComputedValue::Color(
                amazing_css::values::Color::Rgba([1, 2, 3, 255])
            ))
        );
        let s = style(&format!("p{{--n:2;border-{side}:var(--n)px solid red}}"));
        border(&s, side, ["medium", "none", "currentcolor"]);
    }
}

#[test]
fn variables_expand_after_cascade_without_reviving_invalid_fallbacks() {
    for side in SIDES {
        let s = style(&format!(
            "p{{--edge:3px dashed red;border-{side}:var(--edge);border-{side}-color:blue}}"
        ));
        border(&s, side, ["3px", "dashed", "blue"]);
        for value in [
            "var(--missing)",
            "var(--missing, solid solid)",
            "var(--missing, -1px red)",
        ] {
            let s = style(&format!(
                "p{{border-{side}:7px dotted blue;border-{side}:{value}}}"
            ));
            border(&s, side, ["medium", "none", "currentcolor"]);
            for part in ["width", "style", "color"] {
                assert!(s.is_author_specified(&format!("border-{side}-{part}")));
            }
        }
    }
}

#[test]
fn css_wide_keywords_apply_to_every_longhand_before_and_after_substitution() {
    for side in SIDES {
        for keyword in ["initial", "inherit", "unset", "revert", "revert-layer"] {
            for value in [keyword.to_owned(), format!("var(--missing, {keyword})")] {
                let s = style(&format!(
                    "div{{border-{side}:7px dashed blue}} p{{border-{side}:{value}}}"
                ));
                border(
                    &s,
                    side,
                    if keyword == "inherit" {
                        ["7px", "dashed", "blue"]
                    } else {
                        ["medium", "none", "currentcolor"]
                    },
                );
                for part in ["width", "style", "color"] {
                    assert_eq!(
                        s.is_author_specified(&format!("border-{side}-{part}")),
                        !matches!(keyword, "revert" | "revert-layer")
                    );
                }
            }
        }
    }
}

#[test]
fn side_shorthand_order_importance_and_layer_rollback_are_per_longhand() {
    for side in SIDES {
        let s = style(&format!(
            "p{{border-{side}:1px solid red!important; border-{side}-width:2px!important; border-{side}:3px dashed blue}}"
        ));
        border(&s, side, ["2px", "solid", "red"]);
        let s = style(&format!(
            "@layer a,b; @layer a{{p{{border-{side}:7px dotted blue}}}} @layer b{{p{{border-{side}:var(--missing, revert-layer)}}}}"
        ));
        border(&s, side, ["7px", "dotted", "blue"]);
        assert!(s.is_author_specified(&format!("border-{side}-style")));
    }
}
