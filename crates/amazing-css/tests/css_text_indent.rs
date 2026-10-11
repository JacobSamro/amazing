//! Whole text-indent producer, computed policy and authentic used bases.
use amazing_css::{compute, parse_stylesheet, TextIndent, used_text_indent};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    compute(&dom, &[parse_stylesheet(&format!("div{{{css}}}"))]).get(node).unwrap().clone()
}

#[test]
fn complete_grammar_preserves_signed_length_and_canonical_unique_flags() {
    for (css, expected) in [
        ("-10px", "-10px"), ("20%", "20%"),
        ("hanging 20% each-line", "20% hanging each-line"),
        ("each-line hanging calc(50% + 60px)", "calc(50% + 60px) hanging each-line"),
        ("HANGING 10px", "10px hanging"),
        ("10px/**/each-line", "10px each-line"),
    ] {
        assert_eq!(TextIndent::parse(css).unwrap().serialize(), expected, "{css}");
    }
    for css in ["", "hanging", "each-line", "hanging each-line", "1", "10px 20px", "10px hanging hanging", "each-line 20% each-line", "auto", "10px unknown", "calc(1)", "10deg", "calc(10px + )"] {
        assert!(TextIndent::parse(css).is_none(), "{css}");
    }
}

#[test]
fn comment_separated_flags_are_tokens_and_never_math_operators() {
    for (css, expected) in [
        ("hanging/**/10px", "10px hanging"),
        ("10px/**/hanging/**/each-line", "10px hanging each-line"),
        ("each-line/**/hanging/**/calc(25% + 10px)", "calc(25% + 10px) hanging each-line"),
        ("calc(25% + 10px)/**/each-line/**/hanging", "calc(25% + 10px) hanging each-line"),
        (r"10px/**/\68 anging", "10px hanging"),
    ] {
        assert_eq!(TextIndent::parse(css).unwrap().serialize(), expected, "{css}");
    }
    for css in ["10px/**/hanging/**/hanging", "calc(10px + hanging)",
        "calc(10px + each-line)", "[hanging]10px", "10px/**/20px"] {
        assert!(TextIndent::parse(css).is_none(), "{css}");
    }
}

#[test]
fn logical_line_eligibility_uses_first_line_and_real_forced_break_provenance() {
    for hanging in [false, true] {
        for each_line in [false, true] {
            let css = format!("10px{}{}", if hanging { " hanging" } else { "" }, if each_line { " each-line" } else { "" });
            let indent = TextIndent::parse(&css).unwrap();
            assert_eq!(indent.applies_to_line(true, false), !hanging);
            assert_eq!(indent.applies_to_line(false, false), hanging);
            assert_eq!(indent.applies_to_line(false, true), each_line != hanging);
        }
    }
}

#[test]
fn computed_math_uses_actual_element_font_and_keeps_negative_values() {
    for (font, expected) in [(20, "0px hanging"), (40, "-10px hanging"), (60, "-20px hanging")] {
        let current = style(&format!("font-size:{font}px;text-indent:calc(10px - .5em) hanging"));
        assert_eq!(current.get("text-indent"), Some(expected));
    }
    assert_eq!(style("font-size:20px;text-indent:2em each-line").get("text-indent"), Some("40px each-line"));
    assert_eq!(style("text-indent:calc(-10% - 20%) hanging").get("text-indent"), Some("-30% hanging"));
}

#[test]
fn percentages_use_authentic_content_inline_size_and_intrinsic_zero_basis() {
    let current = style("text-indent:calc(25% + 10px) hanging each-line");
    for (width, expected) in [(100.0, 35.0), (240.0, 70.0), (0.0, 10.0)] {
        assert_eq!(used_text_indent(&current, width, false), Some(expected));
        assert_eq!(used_text_indent(&current, width, true), Some(10.0));
    }
    assert_eq!(used_text_indent(&current, f64::NAN, false), None);
    assert_eq!(used_text_indent(&current, -10.0, false), None);
    assert_eq!(used_text_indent(&style("text-indent:-20%"), 200.0, false), Some(-40.0));
}

#[test]
fn inheritance_carries_parent_computed_font_values_and_all_flag_axes() {
    let mut dom = Dom::new();
    let root = dom.create_element(NodeIdx(0), "html");
    let parent = dom.create_element(root, "section");
    let child = dom.create_element(parent, "div");
    let styles = compute(&dom, &[parse_stylesheet("html{font-size:30px}section{font-size:20px;text-indent:calc(1rem + 1em) hanging each-line}div{font-size:10px}")]);
    assert_eq!(styles.get(parent).unwrap().get("text-indent"), Some("50px hanging each-line"));
    assert_eq!(styles.get(child).unwrap().get("text-indent"), Some("50px hanging each-line"));
}

#[test]
fn declarations_remain_atomic_through_variables_invalid_values_and_live_mutation() {
    assert_eq!(style("--i:2em hanging each-line;font-size:20px;text-indent:var(--i)").get("text-indent"), Some("40px hanging each-line"));
    assert_eq!(style("text-indent:10px hanging;text-indent:20px hanging hanging").get("text-indent"), Some("10px hanging"));
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    dom.set_attrs(node, vec![("style".into(), "font-size:20px;text-indent:2em each-line".into())]);
    let first = compute(&dom, &[]).get(node).unwrap().clone();
    dom.set_attrs(node, vec![("style".into(), "font-size:30px;text-indent:2em hanging".into())]);
    assert_eq!(compute(&dom, &[]).get(node).unwrap().get("text-indent"), Some("60px hanging"));
    assert_eq!(first.get("text-indent"), Some("40px each-line"));
}

#[test]
fn wide_fallback_layers_and_winning_longhand_preserve_whole_policy() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "section");
    let child = dom.create_element(parent, "div");
    for keyword in ["inherit", "var(--missing, inherit)"] {
        let css = format!("section{{font-size:20px;text-indent:2em hanging each-line}}div{{font-size:10px;text-indent:{keyword}}}");
        let styles = compute(&dom, &[parse_stylesheet(&css)]);
        assert_eq!(styles.get(child).unwrap().get("text-indent"), Some("40px hanging each-line"));
    }
    for keyword in ["initial", "unset"] {
        let current=style(&format!("text-indent:20px hanging;text-indent:{keyword}"));
        assert_eq!(current.get("text-indent"),Some("0px"));
    }
    let current=style("text-indent:20px hanging!important;text-indent:40px each-line");
    assert_eq!(current.get("text-indent"),Some("20px hanging"));
    let css="@layer a,b;@layer a{div{text-indent:20px hanging}}@layer b{div{text-indent:40px each-line;text-indent:revert-layer}}";
    assert_eq!(compute(&dom,&[parse_stylesheet(css)]).get(child).unwrap().get("text-indent"),Some("20px hanging"));
}

#[test]
fn typed_attributes_setters_and_clones_keep_actual_font_context() {
    let mut dom = Dom::new();
    let node = dom.create_element(NodeIdx(0), "div");
    dom.set_attrs(node,vec![("data-size".into(),"2".into())]);
    let css=parse_stylesheet("div{font-size:20px;text-indent:calc(attr(data-size type(<number>)) * 1em) hanging}");
    let mut current=compute(&dom,&[css]).get(node).unwrap().clone();
    assert_eq!(current.get("text-indent"),Some("40px hanging"));
    let original=current.clone();
    current.set("text-indent","-2em each-line");
    assert_eq!(current.get("text-indent"),Some("-40px each-line"));
    current.set("text-indent","hanging each-line");
    assert_eq!(current.get("text-indent"),Some("-40px each-line"));
    assert_eq!(original.get("text-indent"),Some("40px hanging"));
    assert!(matches!(current.typed("text-indent"),Some(amazing_css::values::ComputedValue::TextIndent(_))));
}
