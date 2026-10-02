use amazing_css::font_metric_context::FontMetricContext;
use amazing_css::font_size_adjust::{FontSizeAdjust, FontSizeAdjustAspect, FontSizeAdjustMetric};
use amazing_css::selectors::PseudoElement;
use amazing_css::{
    ComputedStyle, MediaEnvironment, RenderMutation, StyleInvalidator, compute_with_font_metrics,
    parse_stylesheet, recompute_roots_with_font_metrics,
};
use amazing_dom::{Dom, NodeIdx};
use std::cell::RefCell;
#[derive(Default)]
struct Fonts {
    calls: RefCell<Vec<(NodeIdx, Option<PseudoElement>, FontSizeAdjustMetric)>>,
}
impl FontMetricContext for Fonts {
    fn aspect_ratio(
        &self,
        node: NodeIdx,
        pseudo: Option<PseudoElement>,
        style: &ComputedStyle,
        metric: FontSizeAdjustMetric,
    ) -> Option<f64> {
        self.calls.borrow_mut().push((node, pseudo, metric));
        match style.get("font-family")? {
            "Parent" => Some(0.6),
            "Child" => Some(1.2),
            _ => None,
        }
    }
    fn contextual_length_px(
        &self,
        _: NodeIdx,
        _: Option<PseudoElement>,
        style: &ComputedStyle,
        quantity: f64,
        unit: &str,
    ) -> Option<f64> {
        match unit {
            "px" => Some(quantity),
            "ex" => Some(
                quantity
                    * style
                        .get("font-size")?
                        .strip_suffix("px")?
                        .parse::<f64>()
                        .ok()?
                    * 0.75,
            ),
            "cqw" => Some(quantity * 0.1),
            _ => None,
        }
    }
}
#[test]
fn first_font_resolves_before_inheritance_and_explicit_reselection() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let inherited = dom.create_element(parent, "span");
    let explicit = dom.create_element(parent, "em");
    let fonts = Fonts::default();
    let sheets = [parse_stylesheet(
        "div {font-family:Parent;font-size-adjust:ic-height from-font} span, em {font-family:Child} em {font-size-adjust:ch-width from-font} div::before {content:'a';font-family:Child} div::after {content:'b';font-family:Child;font-size-adjust:cap-height from-font}",
    )];
    let styles =
        compute_with_font_metrics(&dom, &sheets, &MediaEnvironment::default(), None, &fonts);
    assert_eq!(
        styles.get(parent).unwrap().get("font-size-adjust"),
        Some("ic-height 0.6")
    );
    assert_eq!(
        styles.get(inherited).unwrap().get("font-size-adjust"),
        Some("ic-height 0.6")
    );
    assert_eq!(
        styles.get(explicit).unwrap().get("font-size-adjust"),
        Some("ch-width 1.2")
    );
    assert_eq!(
        styles
            .get_pseudo(parent, PseudoElement::Before)
            .unwrap()
            .get("font-size-adjust"),
        Some("ic-height 0.6")
    );
    assert_eq!(
        styles
            .get_pseudo(parent, PseudoElement::After)
            .unwrap()
            .get("font-size-adjust"),
        Some("cap-height 1.2")
    );
    assert!(
        !fonts
            .calls
            .borrow()
            .iter()
            .any(|(node, pseudo, _)| *node == inherited || *pseudo == Some(PseudoElement::Before))
    );
    assert!(
        matches!(styles.get(inherited).unwrap().typed("font-size-adjust"), Some(amazing_css::values::ComputedValue::FontSizeAdjust(FontSizeAdjust::Value {aspect:FontSizeAdjustAspect::Number(n),..})) if *n == 0.6)
    );
}
#[test]
fn animation_selection_and_full_rebuild_use_the_same_font_context() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    let sheets = [parse_stylesheet(
        "div {font-family:Parent;font-size-adjust:from-font}",
    )];
    let fonts = Fonts::default();
    let mut animated = amazing_css::AnimatedValues::default();
    assert!(animated.insert(parent, "font-family", "Child"));
    let styles = compute_with_font_metrics(
        &dom,
        &sheets,
        &MediaEnvironment::default(),
        Some(&animated),
        &fonts,
    );
    assert_eq!(
        styles.get(parent).unwrap().get("font-size-adjust"),
        Some("1.2")
    );
    assert_eq!(
        styles.get(child).unwrap().get("font-size-adjust"),
        Some("1.2")
    );
    let old = compute_with_font_metrics(&dom, &sheets, &MediaEnvironment::default(), None, &fonts);
    let invalidator = StyleInvalidator::new(&sheets, 1);
    let roots = invalidator.collect(&[], 2);
    let rebuilt = recompute_roots_with_font_metrics(&dom, &old, roots, &fonts);
    assert!(rebuilt.full_rebuild);
    assert_eq!(
        rebuilt.styles.get(child).unwrap().get("font-size-adjust"),
        Some("0.6")
    );
}

#[test]
fn actual_metric_math_and_missing_font_defaults_are_not_guessed() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    let fonts = Fonts::default();
    let styles = compute_with_font_metrics(
        &dom,
        &[parse_stylesheet(
            "div {font-family:Missing;font-size-adjust:from-font} span {font-size:20px;font-size-adjust:cap-height calc(1 + sign(1ex - 12px))}",
        )],
        &MediaEnvironment::default(),
        None,
        &fonts,
    );
    assert_eq!(
        styles.get(parent).unwrap().get("font-size-adjust"),
        Some("none")
    );
    assert_eq!(
        styles.get(child).unwrap().get("font-size-adjust"),
        Some("cap-height 2")
    );
    let legacy = amazing_css::compute(
        &dom,
        &[parse_stylesheet(
            "div {font-size:20px;font-size-adjust:calc(1 + sign(1ex - 12px))}",
        )],
    );
    assert!(
        legacy
            .get(parent)
            .unwrap()
            .get("font-size-adjust")
            .unwrap()
            .contains("ex")
    );
}
#[test]
fn incremental_font_selection_changes_resolve_affected_subtrees() {
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    let sheets = [parse_stylesheet(
        "div {font-family:Parent;font-size-adjust:from-font} span {font-family:Child}",
    )];
    let fonts = Fonts::default();
    let old = compute_with_font_metrics(&dom, &sheets, &MediaEnvironment::default(), None, &fonts);
    dom.set_attrs(parent, vec![("style".into(), "font-family:Child".into())]);
    let invalidator = StyleInvalidator::new(&sheets, 1);
    let roots = invalidator.collect(
        &[RenderMutation::attribute(
            parent,
            "style",
            None,
            Some("font-family:Child"),
        )],
        1,
    );
    let delta = recompute_roots_with_font_metrics(&dom, &old, roots, &fonts);
    assert_eq!(
        delta.styles.get(parent).unwrap().get("font-size-adjust"),
        Some("1.2")
    );
    assert_eq!(
        delta.styles.get(child).unwrap().get("font-size-adjust"),
        Some("1.2")
    );
    let full = compute_with_font_metrics(&dom, &sheets, &MediaEnvironment::default(), None, &fonts);
    assert_eq!(delta.styles.get(child), full.get(child));
}

#[test]
fn unavailable_math_context_does_not_switch_to_a_descendants_font_scope() {
    struct Deferred;
    impl FontMetricContext for Deferred {
        fn aspect_ratio(
            &self,
            _: NodeIdx,
            _: Option<PseudoElement>,
            _: &ComputedStyle,
            _: FontSizeAdjustMetric,
        ) -> Option<f64> {
            Some(0.75)
        }
        fn contextual_length_px(
            &self,
            _: NodeIdx,
            _: Option<PseudoElement>,
            style: &ComputedStyle,
            n: f64,
            unit: &str,
        ) -> Option<f64> {
            match unit {
                "px" => Some(n),
                "ex" if style.get("font-family") == Some("Child") => Some(n * 15.0),
                _ => None,
            }
        }
    }
    let mut dom = Dom::new();
    let parent = dom.create_element(NodeIdx(0), "div");
    let child = dom.create_element(parent, "span");
    let explicit = dom.create_element(parent, "em");
    let styles = compute_with_font_metrics(
        &dom,
        &[parse_stylesheet(
            "div {font-family:Parent;font-size-adjust:calc(1 + sign(1ex - 12px))} span,em {font-family:Child} span {font-size-adjust:inherit} em {font-size-adjust:calc(1 + sign(1ex - 12px))}",
        )],
        &MediaEnvironment::default(),
        None,
        &Deferred,
    );
    let source = styles.get(parent).unwrap().get("font-size-adjust").unwrap();
    assert!(source.contains("ex"));
    assert_eq!(
        styles.get(child).unwrap().get("font-size-adjust"),
        Some(source)
    );
    assert_eq!(
        styles.get(explicit).unwrap().get("font-size-adjust"),
        Some("2")
    );
}
