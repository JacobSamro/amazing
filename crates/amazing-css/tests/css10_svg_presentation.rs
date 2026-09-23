use amazing_css::{compute, compute_element_paint_style, parse_declarations, parse_stylesheet,
    values::{ComputedValue, SvgPaint}, ElementStyleSource as Source};
use amazing_dom::{Dom, NodeIdx};
const SVG: &str = "http://www.w3.org/2000/svg";

fn svg_tree() -> (Dom, NodeIdx, NodeIdx, NodeIdx) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let svg = dom.create_element_ns(html, Some(SVG), "svg").unwrap();
    let group = dom.create_element_ns(svg, Some(SVG), "g").unwrap();
    let rect = dom.create_element_ns(group, Some(SVG), "rect").unwrap();
    (dom, svg, group, rect)
}

#[test]
fn svg_fill_hint_beats_initial_but_author_and_inline_beat_hint() {
    let (mut dom, _, _, rect) = svg_tree();
    dom.set_attrs(rect, vec![("fill".into(), "red".into())]);
    let hints = compute(&dom, &[]);
    assert_eq!(hints.get(rect).unwrap().get("fill"), Some("red"));
    assert_eq!(hints.source(rect, "fill"), Some(Source::PresentationHint));
    assert_eq!(hints.get(rect).unwrap().typed("fill"), Some(&ComputedValue::SvgPaint(SvgPaint::Color([255, 0, 0, 255]))));
    assert_eq!(compute(&dom, &[parse_stylesheet("rect{fill:black}")]).source(rect, "fill"), Some(Source::Author));
    dom.set_attrs(rect, vec![("fill".into(), "red".into()), ("style".into(), "fill:blue".into())]);
    let inline = compute(&dom, &[parse_stylesheet("rect{fill:black}")]);
    assert_eq!(inline.get(rect).unwrap().get("fill"), Some("blue"));
    assert_eq!(inline.source(rect, "fill"), Some(Source::Inline));
}

#[test]
fn svg_hint_inherits_and_invalid_or_non_svg_attributes_do_not_cascade() {
    let (mut dom, _, group, rect) = svg_tree();
    dom.set_attrs(group, vec![("fill".into(), "green".into())]);
    let styled = compute(&dom, &[]);
    assert_eq!(styled.get(rect).unwrap().get("fill"), Some("green"));
    assert_eq!(styled.source(group, "fill"), Some(Source::PresentationHint));
    assert_eq!(styled.source(rect, "fill"), Some(Source::Inherited));
    dom.set_attrs(rect, vec![("fill".into(), "not-a-color".into()), ("stroke-width".into(), "-2".into())]);
    let invalid = compute(&dom, &[]);
    assert_eq!(invalid.get(rect).unwrap().get("fill"), Some("green"));
    assert_eq!(invalid.source(rect, "fill"), Some(Source::Inherited));
    assert_eq!(invalid.get(rect).unwrap().get("stroke-width"), Some("1px"));
    assert_eq!(invalid.source(rect, "stroke-width"), Some(Source::Inherited));
    let html = dom.create_element(NodeIdx(0), "div");
    dom.set_attrs(html, vec![("fill".into(), "red".into())]);
    let html_style = compute(&dom, &[]);
    assert_eq!(html_style.get(html).unwrap().get("fill"), Some("black"));
    assert_eq!(html_style.source(html, "fill"), Some(Source::Initial));
}

#[test]
fn svg_paint_geometry_and_transform_attribute_grammars_are_typed() {
    let (mut dom, _, _, rect) = svg_tree();
    dom.set_attrs(rect, vec![
        ("stroke".into(), "#008000".into()),
        ("stroke-width".into(), "5".into()),
        ("stroke-opacity".into(), ".5".into()),
        ("stroke-dasharray".into(), "5, 3".into()),
        ("stroke-linecap".into(), "round".into()),
        ("fill-rule".into(), "evenodd".into()),
        ("clip-path".into(), "url(#clip)".into()),
        ("transform".into(), "translate(10 20) rotate(45)".into()),
    ]);
    let styled = compute(&dom, &[]);
    let s = styled.get(rect).unwrap();
    assert_eq!(s.get("stroke-width"), Some("5px"));
    assert_eq!(s.get("stroke-dasharray"), Some("5px, 3px"));
    assert_eq!(s.get("transform"), Some("translate(10px, 20px) rotate(45deg)"));
    for property in ["stroke", "stroke-width", "stroke-opacity", "stroke-dasharray", "stroke-linecap", "fill-rule", "clip-path", "transform"] {
        assert_eq!(styled.source(rect, property), Some(Source::PresentationHint), "{property}");
        assert!(!matches!(s.typed(property), Some(ComputedValue::Unparsed) | None), "{property}");
    }
}

#[test]
fn document_and_standalone_paint_use_same_hint_winner_and_provenance() {
    let (mut dom, _, _, rect) = svg_tree();
    dom.set_attrs(rect, vec![("fill".into(), "red".into()), ("fill-opacity".into(), ".5".into())]);
    let sheet = parse_stylesheet("rect{fill:black}");
    let document = compute(&dom, &[sheet]);
    let standalone = compute_element_paint_style(None,
        &parse_declarations("fill:red;fill-opacity:.5"),
        &parse_declarations("fill:black"), &[]);
    assert_eq!(document.get(rect).unwrap().get("fill"), standalone.computed.get("fill"));
    assert_eq!(document.get(rect).unwrap().get("fill-opacity"), standalone.computed.get("fill-opacity"));
    assert_eq!(document.source(rect, "fill"), Some(Source::Author));
    assert_eq!(document.source(rect, "fill-opacity"), Some(Source::PresentationHint));
}

#[test]
fn svg_transform_attribute_uses_element_specific_name() {
    let (mut dom, svg, _, _) = svg_tree();
    let pattern = dom.create_element_ns(svg, Some(SVG), "pattern").unwrap();
    let gradient = dom.create_element_ns(svg, Some(SVG), "linearGradient").unwrap();
    dom.set_attrs(pattern, vec![("patternTransform".into(), "scale(2)".into()),
        ("transform".into(), "translate(9)".into())]);
    dom.set_attrs(gradient, vec![("gradientTransform".into(), "rotate(30)".into())]);
    let styled = compute(&dom, &[]);
    assert_eq!(styled.get(pattern).unwrap().get("transform"), Some("scale(2)"));
    assert_eq!(styled.get(gradient).unwrap().get("transform"), Some("rotate(30deg)"));
    assert_eq!(styled.source(pattern, "transform"), Some(Source::PresentationHint));
    assert_eq!(styled.source(gradient, "transform"), Some(Source::PresentationHint));
}
