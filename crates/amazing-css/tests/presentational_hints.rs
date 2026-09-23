use amazing_css::{ComputedStyle, compute, parse_stylesheet};
use amazing_dom::{Dom, NodeIdx};
fn style(tag: &str, attrs: &[(&str, &str)], css: &str) -> ComputedStyle {
    let mut dom = Dom::new();
    let el = dom.create_element(NodeIdx(0), tag);
    dom.set_attrs(
        el,
        attrs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    );
    compute(&dom, &[parse_stylesheet(css)])
        .get(el)
        .unwrap()
        .clone()
}
#[test]
fn replaced_dimensions_are_hints_not_intrinsic_or_author_values() {
    for tag in ["img", "embed", "iframe", "object", "video", "input"] {
        let s = style(
            tag,
            &[
                ("width", " 120.5px junk"),
                ("height", "25%"),
                ("type", "IMAGE"),
            ],
            "",
        );
        assert_eq!(s.get("width"), Some("120.5px"), "{tag}");
        assert_eq!(s.get("height"), Some("25%"), "{tag}");
        assert!(!s.is_author_specified("width"));
    }
    for tag in ["div", "audio", "canvas", "span", "input"] {
        assert_eq!(
            style(tag, &[("width", "120")], "").get("width"),
            Some("auto"),
            "{tag}"
        );
    }
}
#[test]
fn every_author_layer_beats_hints_and_rollback_obeys_origin() {
    for css in [
        "*{width:40px}",
        "@layer first{*{width:40px}}",
        "*{width:40px!important}",
    ] {
        let s = style("img", &[("width", "120")], css);
        assert_eq!(s.get("width"), Some("40px"));
        assert!(s.is_author_specified("width"));
    }
    for keyword in ["revert", "var(--absent,revert)"] {
        let s = style(
            "img",
            &[("width", "120")],
            &format!("img{{width:{keyword}}}"),
        );
        assert_eq!(s.get("width"), Some("auto"));
        assert!(!s.is_author_specified("width"));
    }
    for keyword in ["revert-layer", "var(--absent,revert-layer)"] {
        let s = style(
            "img",
            &[("width", "120")],
            &format!("@layer a{{img{{width:{keyword}}}}}"),
        );
        assert_eq!(s.get("width"), Some("120px"));
        assert!(!s.is_author_specified("width"));
    }
}
#[test]
fn dimensions_use_html_microsyntax_and_element_zero_rules() {
    for (raw, expected) in [
        ("+20", "auto"),
        ("-1", "auto"),
        (".5", "auto"),
        ("0", "0px"),
        ("20 %", "20px"),
        ("1e2", "1px"),
        ("20.%", "20%"),
    ] {
        assert_eq!(
            style("img", &[("width", raw)], "").get("width"),
            Some(expected),
            "{raw}"
        );
    }
    assert_eq!(
        style("td", &[("width", "0")], "").get("width"),
        Some("auto")
    );
    assert_eq!(
        style("table", &[("width", "0"), ("height", "0")], "").get("height"),
        Some("0px")
    );
}
#[test]
fn aspect_ratio_does_not_turn_canvas_attributes_into_css_dimensions() {
    let s = style("canvas", &[("width", "300"), ("height", "150")], "");
    assert_eq!(s.get("aspect-ratio"), Some("auto 300 / 150"));
    assert_eq!(s.get("width"), Some("auto"));
    assert!(!s.is_author_specified("aspect-ratio"));
}
#[test]
fn legacy_colors_are_not_css_color_parsing() {
    for (raw, expected) in [
        ("chucknorris", "#c00000"),
        ("#abc", "#aabbcc"),
        (" ", "#000000"),
        ("ReD", "red"),
        ("papayawhip", "papayawhip"),
    ] {
        let s = style("body", &[("bgcolor", raw)], "");
        assert_eq!(s.get("background-color"), Some(expected), "{raw}");
        assert!(!s.is_author_specified("background-color"));
    }
    for raw in ["", "transparent"] {
        assert_eq!(
            style("body", &[("bgcolor", raw)], "").get("background-color"),
            Some("transparent")
        );
    }
    let s = style(
        "body",
        &[("bgcolor", "red")],
        "@layer first{*{background:transparent}}",
    );
    assert_eq!(s.get("background-color"), Some("transparent"));
    assert!(s.is_author_specified("background-color"));
}
#[test]
fn font_list_alignment_and_whitespace_mappings() {
    for (tag, attrs, property, expected) in [
        ("font", vec![("size", "+1")], "font-size", "18px"),
        ("li", vec![("type", "A")], "list-style-type", "upper-alpha"),
        ("ul", vec![("type", "CIRCLE")], "list-style-type", "circle"),
        (
            "li",
            vec![("value", " -3suffix")],
            "counter-set",
            "list-item -3",
        ),
        (
            "ol",
            vec![("start", "4"), ("reversed", "")],
            "counter-reset",
            "reversed(list-item) 5",
        ),
        ("div", vec![("align", "MiDdLe")], "text-align", "center"),
        ("pre", vec![("wrap", "")], "white-space", "pre-wrap"),
        ("textarea", vec![("wrap", "OFF")], "white-space", "pre"),
        ("br", vec![("clear", "ALL")], "clear", "both"),
        (
            "body",
            vec![("marginheight", "bad"), ("topmargin", "17")],
            "margin-top",
            "8px",
        ),
    ] {
        let s = style(tag, &attrs, "");
        assert_eq!(s.get(property), Some(expected), "{tag}/{property}");
        assert!(!s.is_author_specified(property));
    }
}
#[test]
fn table_hints_target_cells_and_stop_at_nested_tables() {
    let mut dom = Dom::new();
    let table = dom.create_element(NodeIdx(0), "table");
    dom.set_attrs(
        table,
        vec![
            ("cellpadding".into(), "7".into()),
            ("rules".into(), "cols".into()),
        ],
    );
    let row = dom.create_element(table, "tr");
    let cell = dom.create_element(row, "td");
    let nested = dom.create_element(cell, "table");
    let nested_row = dom.create_element(nested, "tr");
    let nested_cell = dom.create_element(nested_row, "td");
    let styles = compute(
        &dom,
        &[parse_stylesheet("@layer first { td {padding-left:3px} }")],
    );
    let s = styles.get(cell).unwrap();
    assert_eq!(s.get("padding-top"), Some("7px"));
    assert_eq!(s.get("padding-left"), Some("3px"));
    assert!(!s.is_author_specified("padding-top"));
    assert!(s.is_author_specified("padding-left"));
    assert_eq!(s.get("border-left-style"), Some("solid"));
    assert_eq!(s.get("border-top-style"), Some("none"));
    assert_ne!(
        styles.get(nested_cell).unwrap().get("padding-top"),
        Some("7px")
    );
    assert_ne!(
        styles.get(nested_cell).unwrap().get("border-left-style"),
        Some("solid")
    );
}
#[test]
fn hints_do_not_apply_to_foreign_namespaces() {
    use amazing_dom::{Namespace, NodeKind};
    let mut dom = Dom::new();
    let el = dom.append_child(
        NodeIdx(0),
        NodeKind::Element {
            name: "img".into(),
            namespace: Namespace::Svg,
            attrs: vec![("width".into(), "27".into())],
        },
    );
    let styles = compute(&dom, &[]);
    assert_eq!(styles.get(el).unwrap().get("width"), Some("auto"));
}
#[test]
fn reset_keywords_and_inheritance_keep_provenance_honest() {
    for keyword in ["initial", "unset"] {
        let s = style(
            "img",
            &[("width", "12"), ("height", "6")],
            &format!("img{{all:{keyword}}}"),
        );
        assert_eq!(s.get("width"), Some("auto"));
        assert_eq!(s.get("aspect-ratio"), Some("auto"));
        assert!(s.is_author_specified("width"));
        assert!(s.is_author_specified("aspect-ratio"));
    }
    let mut dom = Dom::new();
    let body = dom.create_element(NodeIdx(0), "body");
    dom.set_attrs(
        body,
        vec![
            ("text".into(), "red".into()),
            ("link".into(), "green".into()),
        ],
    );
    let link = dom.create_element(body, "a");
    dom.set_attrs(link, vec![("href".into(), "x".into())]);
    let child = dom.create_element(body, "p");
    let s = compute(&dom, &[]);
    assert_eq!(s.get(child).unwrap().get("color"), Some("red"));
    assert!(!s.get(child).unwrap().is_author_specified("color"));
    assert_eq!(s.get(link).unwrap().get("color"), Some("green"));
    assert!(!s.get(link).unwrap().is_author_specified("color"));
    let s = compute(&dom, &[parse_stylesheet("@layer low {a{color:inherit}}")]);
    assert_eq!(s.get(link).unwrap().get("color"), Some("red"));
    assert!(s.get(link).unwrap().is_author_specified("color"));
}
#[test]
fn image_spacing_borders_and_document_background_are_low_priority() {
    let s = style(
        "img",
        &[("hspace", "3%"), ("vspace", "5"), ("border", "+2junk")],
        "@layer low{img{border-left-width:7px}}",
    );
    assert_eq!(s.get("margin-left"), Some("3%"));
    assert_eq!(s.get("margin-top"), Some("5px"));
    assert_eq!(s.get("border-top-style"), Some("solid"));
    assert_eq!(s.get("border-top-width"), Some("2px"));
    assert_eq!(s.get("border-left-width"), Some("7px"));
    assert!(!s.is_author_specified("border-top-style"));
    assert!(s.is_author_specified("border-left-width"));
    let s = style("body", &[("background", "images/a.png")], "");
    assert_eq!(s.get("background-image"), Some("url(\"images/a.png\")"));
    assert!(!s.is_author_specified("background-image"));
    let s = style(
        "body",
        &[("background", "images/a.png")],
        "@layer low{*{background:none}}",
    );
    assert_eq!(s.get("background-image"), Some("none"));
    assert!(s.is_author_specified("background-image"));
}

#[test]
fn simple_background_shorthand_resets_hints_including_after_variables() {
    for value in [
        "none",
        "red",
        "initial",
        "var(--absent,none)",
        "var(--absent,red)",
    ] {
        let s = style(
            "body",
            &[("background", "a.png")],
            &format!("@layer low{{*{{background:{value}}}}}"),
        );
        assert_eq!(s.get("background-image"), Some("none"), "{value}");
        assert!(s.is_author_specified("background-image"));
    }
    for value in ["revert-layer", "var(--absent,revert-layer)"] {
        let s = style(
            "body",
            &[("background", "a.png")],
            &format!("@layer low{{*{{background:{value}}}}}"),
        );
        assert_eq!(s.get("background-image"), Some("url(\"a.png\")"));
        assert!(!s.is_author_specified("background-image"));
    }
    for value in ["revert", "var(--absent,revert)"] {
        let s = style(
            "body",
            &[("background", "a.png")],
            &format!("*{{background:{value}}}"),
        );
        assert_eq!(s.get("background-image"), Some("none"));
        assert!(!s.is_author_specified("background-image"));
    }
}

#[test]
fn table_cellspacing_zero_is_a_non_author_hint_below_every_author_layer() {
    let attrs = [("cellspacing", "0")];
    let hint = style("table", &attrs, "");
    assert_eq!(hint.get("border-spacing"), Some("0px"));
    assert!(!hint.is_author_specified("border-spacing"));
    let author = style("table", &attrs, "@layer first{*{border-spacing:7px 3px}}");
    assert_eq!(author.get("border-spacing"), Some("7px 3px"));
    assert!(author.is_author_specified("border-spacing"));
    let hint = style(
        "table",
        &[("cellspacing", " +6junk")],
        "@layer first{*{border-spacing:revert-layer}}",
    );
    assert_eq!(hint.get("border-spacing"), Some("6px"));
    assert!(!hint.is_author_specified("border-spacing"));
    let reverted = style("table", &[("cellspacing", "6")], "*{border-spacing:revert}");
    assert_eq!(reverted.get("border-spacing"), Some("2px"));
    assert!(!reverted.is_author_specified("border-spacing"));
}
