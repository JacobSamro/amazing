use amazing_css::font_metric_context::FontMetricContext;
use amazing_css::font_size_adjust::FontSizeAdjustMetric;
use amazing_css::selectors::PseudoElement;
use amazing_css::{
    ComputedStyle, MediaEnvironment, compute_document_with_font_metrics, parse_stylesheet,
};
use amazing_dom::{Dom, NodeIdx};
struct Metrics;
impl FontMetricContext for Metrics {
    fn aspect_ratio(
        &self,
        _: NodeIdx,
        _: Option<PseudoElement>,
        _: &ComputedStyle,
        _: FontSizeAdjustMetric,
    ) -> Option<f64> {
        Some(0.5)
    }
}
#[test]
fn native_document_context_keeps_indices_sheets_hints_and_generated_state_scoped() {
    let mut dom = Dom::new();
    let main = dom.create_element(NodeIdx(0), "body");
    dom.set_attrs(main, vec![("link".into(), "red".into())]);
    let main_child = dom.create_element(main, "a");
    dom.set_attrs(main_child, vec![("href".into(), "/".into())]);
    let document = dom.create_document();
    let html = dom.create_element(document, "html");
    let body = dom.create_element(html, "body");
    dom.set_attrs(body, vec![("link".into(), "blue".into())]);
    let child = dom.create_element(body, "a");
    dom.set_attrs(child, vec![("href".into(), "/".into())]);
    let sheet = parse_stylesheet(
        "html{font-size:40px;font-size-adjust:from-font;counter-reset:item 6} a{counter-increment:item}a::before{content:counter(item)}",
    );
    let styles = compute_document_with_font_metrics(
        &dom,
        document,
        &[sheet],
        &MediaEnvironment::default(),
        None,
        &Metrics,
    );
    assert!(styles.get(main).is_none());
    assert!(styles.get(main_child).is_none());
    assert_eq!(
        styles.get(html).unwrap().get("font-size-adjust"),
        Some("0.5")
    );
    assert_eq!(styles.get(child).unwrap().get("font-size"), Some("40px"));
    assert_eq!(styles.get(child).unwrap().get("color"), Some("blue"));
    assert_eq!(
        styles
            .generated_content(child, PseudoElement::Before)
            .unwrap()
            .text(),
        "7"
    );
    assert!(
        compute_document_with_font_metrics(
            &dom,
            child,
            &[],
            &MediaEnvironment::default(),
            None,
            &Metrics
        )
        .get(child)
        .is_none()
    );
    assert!(
        compute_document_with_font_metrics(
            &dom,
            NodeIdx((dom.node_count() + 1) as u32),
            &[],
            &MediaEnvironment::default(),
            None,
            &Metrics
        )
        .get(child)
        .is_none()
    );
}
