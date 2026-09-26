//! COMP-15: filter/backdrop-filter, mix-blend-mode, isolation, clip-path, and
//! mask longhand parsing, computed values, and the CSS-13 compositor delta path.

use amazing_css::effects::{BasicShape, BlendMode, ClipPath, FilterFunction, FilterList, MaskMode};
use amazing_css::{compute, parse_stylesheet, values::ComputedValue, AttributeStyleUpdate, StyleChangeKind, StyleInvalidator};
use amazing_dom::{Dom, NodeIdx};

fn style(css: &str) -> amazing_css::ComputedStyle {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let target = dom.create_element(body, "div");
    compute(&dom, &[parse_stylesheet(css)]).get(target).unwrap().clone()
}

#[test]
fn filter_and_backdrop_filter_compute_to_typed_function_lists() {
    let s = style("div{filter: blur(4px) brightness(2) hue-rotate(90deg) drop-shadow(1px 2px #f00)}");
    assert!(s.get("filter").is_some_and(|value| value.contains("blur(4px)")), "{:?}", s.get("filter"));
    let Some(ComputedValue::FilterList(FilterList::Functions(functions))) = s.typed("filter") else {
        panic!("filter not typed: {:?}", s.typed("filter"));
    };
    assert!(matches!(functions[0], FilterFunction::Blur(_)));
    assert_eq!(functions[1], FilterFunction::Brightness(2.0));
    assert!(matches!(functions[2], FilterFunction::HueRotate(_)));
    assert!(matches!(functions[3], FilterFunction::DropShadow(_)));

    let s = style("div{backdrop-filter:none}");
    assert!(matches!(s.typed("backdrop-filter"), Some(ComputedValue::FilterList(FilterList::None))));
    assert_eq!(s.get("backdrop-filter"), Some("none"));
}

#[test]
fn blend_isolation_and_mask_longhands_compute() {
    let s = style("div{mix-blend-mode:multiply;isolation:isolate;mask-mode:alpha;mask-type:luminance;mask-composite:exclude;mask-clip:content-box;mask-origin:padding-box;mask-repeat:no-repeat;mask-position:10px 20px;mask-size:cover;mask-image:url(img.png)}");
    assert_eq!(s.typed("mix-blend-mode"), Some(&ComputedValue::BlendMode(BlendMode::Multiply)));
    assert_eq!(s.typed("isolation"), Some(&ComputedValue::Isolation(true)));
    assert_eq!(s.typed("mask-mode"), Some(&ComputedValue::MaskMode(MaskMode::Alpha)));
    assert_eq!(s.get("mask-type"), Some("luminance"));
    assert_eq!(s.get("mask-composite"), Some("exclude"));
    assert_eq!(s.get("mask-clip"), Some("content-box"));
    assert_eq!(s.get("mask-origin"), Some("padding-box"));
    assert_eq!(s.get("mask-repeat"), Some("no-repeat"));
    assert_eq!(s.get("mask-position"), Some("10px 20px"));
    assert_eq!(s.get("mask-size"), Some("cover"));
    assert!(matches!(
        s.typed("mask-image"),
        Some(ComputedValue::MaskImage(images)) if images.len() == 1
    ));
}

#[test]
fn clip_path_basic_shapes_and_geometry_boxes_compute() {
    let circle = style("div{clip-path:circle(7% at 8% 9%)}");
    let Some(ComputedValue::ClipPath(ClipPath::Shape(BasicShape::Circle { radius: Some(_), position: Some(_) }))) =
        circle.typed("clip-path")
    else {
        panic!("circle not typed: {:?}", circle.typed("clip-path"));
    };
    assert_eq!(style("div{clip-path:none}").typed("clip-path"), Some(&ComputedValue::ClipPath(ClipPath::None)));
    assert!(matches!(
        style("div{clip-path:border-box}").typed("clip-path"),
        Some(ComputedValue::ClipPath(ClipPath::Geometry(_)))
    ));
    assert!(matches!(
        style("div{clip-path:url(#c)}").typed("clip-path"),
        Some(ComputedValue::ClipPath(ClipPath::Url(_)))
    ));
    assert!(style("div{clip-path:polygon(1px 2px, 3em 4em, 5pt 6%)}")
        .get("clip-path")
        .is_some_and(|value| value.starts_with("polygon(")));
    assert!(matches!(
        style("div{clip-path:path(\"m 20 0 h -100\")}").typed("clip-path"),
        Some(ComputedValue::ClipPath(ClipPath::Shape(BasicShape::Path { .. })))
    ));
}

#[test]
fn invalid_effect_declarations_do_not_displace_valid_winners() {
    for declaration in [
        "blur(10)",
        "brightness(30px)",
        "hue-rotate(90)",
        "drop-shadow(10% 20%)",
    ] {
        assert_eq!(
            style(&format!("div{{filter:blur(4px);filter:{declaration}}}")).get("filter"),
            Some("blur(4px)"),
            "{declaration}"
        );
    }
    for declaration in ["inset(123)", "circle(at)", "polygon(1%)", "unknown-box"] {
        assert_eq!(
            style(&format!("div{{clip-path:circle(50%);clip-path:{declaration}}}")).get("clip-path"),
            Some("circle(50%)"),
            "{declaration}"
        );
    }
    assert_eq!(style("div{mix-blend-mode:multiply-again}").get("mix-blend-mode"), Some("normal"));
    assert_eq!(style("div{isolation:isolated}").get("isolation"), Some("auto"));
}

#[test]
fn effect_only_changes_classify_as_compositor_only() {
    for property in [
        "filter",
        "backdrop-filter",
        "mix-blend-mode",
        "isolation",
        "clip-path",
        "mask-image",
        "mask-mode",
        "mask-repeat",
        "mask-position",
        "mask-clip",
        "mask-origin",
        "mask-size",
        "mask-composite",
        "mask-type",
    ] {
        assert_eq!(
            amazing_css::classify_style_property(property),
            Some(StyleChangeKind::CompositorOnly),
            "{property}"
        );
    }
}

fn fixture(css: &str, class: &str) -> (Dom, amazing_css::parser::Stylesheet, NodeIdx) {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let target = dom.create_element(html, "div");
    dom.set_attribute(target, "class", class).unwrap();
    (dom, parse_stylesheet(css), target)
}

#[test]
fn filter_class_edit_is_a_value_only_compositor_delta() {
    let (mut dom, sheet, node) = fixture(".a{filter:blur(2px)} .b{filter:blur(4px)}", "a");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 7);
    let mut styles = compute(&dom, &[sheet]);
    dom.set_attribute(node, "class", "b").unwrap();
    let update = invalidator.update_attribute(&dom, &styles, node, "class", Some("a"), Some("b"), 7);
    assert_eq!(update.change_kind(node), StyleChangeKind::CompositorOnly);
    let AttributeStyleUpdate::CompositorOnly(delta) = update else {
        panic!("filter class edit recomputed instead of using the delta path");
    };
    assert_eq!(delta.effects, vec![("filter".to_string(), "blur(4px)".to_string())]);
    assert!(delta.apply_to_styles(&mut styles));
    assert_eq!(styles.get(node).unwrap().get("filter"), Some("blur(4px)"));
    invalidator.compare_to_full(&dom, &styles).unwrap();
}

#[test]
fn blend_mode_class_edit_is_a_value_only_compositor_delta() {
    let (mut dom, sheet, node) = fixture(".a{mix-blend-mode:normal} .b{mix-blend-mode:screen}", "a");
    let invalidator = StyleInvalidator::new(&[sheet.clone()], 3);
    let mut styles = compute(&dom, &[sheet]);
    dom.set_attribute(node, "class", "b").unwrap();
    let update = invalidator.update_attribute(&dom, &styles, node, "class", Some("a"), Some("b"), 3);
    let AttributeStyleUpdate::CompositorOnly(delta) = update else {
        panic!("blend-mode class edit recomputed");
    };
    assert_eq!(delta.effects, vec![("mix-blend-mode".to_string(), "screen".to_string())]);
    assert!(delta.apply_to_styles(&mut styles));
    assert_eq!(
        styles.get(node).unwrap().typed("mix-blend-mode"),
        Some(&ComputedValue::BlendMode(BlendMode::Screen))
    );
}
