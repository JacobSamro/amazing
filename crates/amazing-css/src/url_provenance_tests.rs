//! Winning declaration URL contexts; CSS Values 4 URL resolution after var().
use super::*;

fn sheet(css: &str, base: &str) -> Stylesheet {
    let mut sheet = parse_stylesheet(css);
    sheet.base_url = Some(base.into());
    sheet
}

#[test]
fn background_url_collision_uses_winner_after_variables_and_rollback() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let first = dom.create_element(html, "article");
    let second = dom.create_element(html, "section");
    let reverted = dom.create_element(html, "aside");
    let sheets = [
        sheet("html{--image:url(shared.png)} article,section{background-image:var(--image)} @layer base,reset; @layer base{aside{background-image:var(--image)}}", "https://a.test/theme/a.css"),
        sheet("section{background-image:var(--image)} @layer reset{aside{background-image:revert-layer}}", "https://b.test/theme/b.css"),
    ];
    let result = compute(&dom, &sheets);
    for (node, base) in [(first, "https://a.test/theme/a.css"), (second, "https://b.test/theme/b.css"), (reverted, "https://a.test/theme/a.css")] {
        let style = result.get(node).unwrap();
        assert!(style.get("background-image").unwrap().contains("shared.png"));
        assert_eq!(style.url_base("background-image"), Some(base));
        assert_eq!(style.clone().url_base("background-image"), Some(base));
    }
}

#[test]
fn background_inheritance_shorthand_resets_and_pseudo_keep_their_actual_source() {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let parent = dom.create_element(html, "section");
    let nodes: Vec<_> = ["inherit", "shorthand", "variable", "initial", "unset", "reset", "ordinary"]
        .iter().map(|class| {
            let node = dom.create_element(parent, "i");
            dom.set_attribute(node, "class", class);
            node
        }).collect();
    let sheets = [
        sheet("section{background-image:url(shared.png)} section::before{content:'x';background-image:url(shared.png)}", "https://a.test/theme/a.css"),
        sheet(".inherit{background-image:inherit}.shorthand{background:inherit}.variable{background-image:var(--absent,inherit)}.initial{background-image:initial}.unset{background-image:unset}.reset{background:none}section::after{content:'y';background-image:url(shared.png)}", "https://b.test/theme/b.css"),
    ];
    let result = compute(&dom, &sheets);
    for node in &nodes[..3] {
        assert_eq!(result.get(*node).unwrap().url_base("background-image"), Some("https://a.test/theme/a.css"));
    }
    for node in &nodes[3..] {
        let style = result.get(*node).unwrap();
        assert_eq!(style.get("background-image"), Some("none"));
        // Reset declarations may retain an unused source; no inherited source
        // survives on an undeclared or CSS-wide initial/unset image longhand.
        if *node != nodes[5] { assert_eq!(style.url_base("background-image"), None); }
    }
    for (pseudo, base) in [(PseudoElement::Before, "https://a.test/theme/a.css"), (PseudoElement::After, "https://b.test/theme/b.css")] {
        assert_eq!(result.get_pseudo(parent, pseudo).unwrap().url_base("background-image"), Some(base));
    }
}

struct NoMetrics;
impl crate::font_metric_context::FontMetricContext for NoMetrics {
    fn aspect_ratio(&self, _: NodeIdx, _: Option<PseudoElement>, _: &ComputedStyle,
        _: crate::font_size_adjust::FontSizeAdjustMetric) -> Option<f64> { None }
}

#[test]
fn scoped_principal_and_computed_pseudo_use_shadow_sheet_url_without_document_leakage() {
    use amazing_dom::shadow::{ShadowMode, ShadowOptions, SlotAssignment};
    for mode in [ShadowMode::Open, ShadowMode::Closed] {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let host = dom.create_element(html, "section");
        let light = dom.create_element(html, "i");
        let root = dom.attach_shadow(host, ShadowOptions {mode, slot_assignment:SlotAssignment::Named,
            delegates_focus:false, clonable:false, serializable:false}).unwrap();
        let inner = dom.create_element(root, "i");
        let document_sheets = [sheet("i,i::before{background-image:url(shared.png)}", "https://a.test/a.css")];
        let shadows = [(root, vec![sheet("i,i::before{background-image:url(shared.png)}", "https://b.test/b.css")])];
        let environment = MediaEnvironment::default();
        let result = compute_document_with_shadow_font_metrics(&dom, NodeIdx(0), &document_sheets, &shadows, &environment, None, &NoMetrics);
        for (node, base) in [(light, "https://a.test/a.css"), (inner, "https://b.test/b.css")] {
            assert_eq!(result.get(node).unwrap().url_base("background-image"), Some(base));
            let pseudo = compute_document_pseudo_with_shadow_font_metrics(&dom, NodeIdx(0), node,
                PseudoElement::Before, &document_sheets, &shadows, &environment, None, &NoMetrics).unwrap();
            assert_eq!(pseudo.url_base("background-image"), Some(base));
        }
    }
}
