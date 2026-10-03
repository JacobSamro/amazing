//! Computed pseudo styles independently of whether layout generates a box.
use super::*;
use crate::font_metric_context::FontMetricContext;

/// Compute a recognized pseudo in the selected native Document's arena.
///
/// Unlike `StyledDom::get_pseudo`, this includes hidden origins, absent generated
/// content and recognized pseudos without an applicable box. The driver owns
/// pseudo-string parsing, active-document checks and resolved used geometry.
/// Principal animations enter inheritance; pseudo animation samples require a
/// separate sampling interface and are not taken from the originating element.
/// Selection highlight inheritance/property applicability remains a separate
/// policy; this entry point reuses the engine's existing pseudo cascade.
pub fn compute_document_pseudo_with_font_metrics(
    dom: &Dom,
    document: NodeIdx,
    node: NodeIdx,
    pseudo: PseudoElement,
    author_sheets: &[Stylesheet],
    environment: &MediaEnvironment,
    animated: Option<&AnimatedValues>,
    context: &dyn FontMetricContext,
) -> Option<ComputedStyle> {
    if node.0 as usize >= dom.node_count() || dom.name(node).is_none() {
        return None;
    }
    let mut ancestor = dom.get(node).parent;
    while let Some(parent) = ancestor {
        if matches!(dom.get(parent).kind, amazing_dom::NodeKind::Document) {
            if parent != document { return None; }
            break;
        }
        ancestor = dom.get(parent).parent;
    }
    // A detached element without a Document is not a member of this query.
    ancestor?;
    let principal = compute_document_with_font_metrics(
        dom, document, author_sheets, environment, animated, context,
    );
    // Membership in this selected document is established by the subtree walk,
    // never by cloning an iframe/document into a new index-zero DOM.
    let origin = principal.get(node)?;
    let mut ua_sheets = vec![parse_stylesheet(UA_SHEET)];
    if pseudo == PseudoElement::Backdrop {
        // CSS Positioned Layout 4's recommended UA default. This does not
        // manufacture a top-layer box or modify the generated style map.
        ua_sheets.push(parse_stylesheet("::backdrop { position: fixed; inset: 0; }"));
    }
    let ua = compile_sheets(&ua_sheets, true, environment);
    let author = compile_sheets(author_sheets, false, environment);
    let body_link = hints::body_link_in(dom, document);
    let (mut result, _) = compute_one(
        dom, node, Some(origin), origin.root_font_size_px(), &ua, &author,
        body_link.as_deref(), Some(pseudo), None, environment,
    );
    crate::font_metric_context::resolve(&mut result, node, Some(pseudo), Some(context));
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_size_adjust::FontSizeAdjustMetric;

    struct Metrics;
    impl FontMetricContext for Metrics {
        fn aspect_ratio(&self, _: NodeIdx, pseudo: Option<PseudoElement>, _: &ComputedStyle,
            _: FontSizeAdjustMetric) -> Option<f64> {
            Some(if pseudo.is_some() { 0.7 } else { 0.5 })
        }
    }

    fn query(dom: &Dom, document: NodeIdx, node: NodeIdx, pseudo: PseudoElement,
        sheets: &[Stylesheet]) -> ComputedStyle {
        compute_document_pseudo_with_font_metrics(dom, document, node, pseudo,
            sheets, &MediaEnvironment::default(), None, &Metrics).unwrap()
    }

    #[test]
    fn nonexistent_and_hidden_pseudos_have_real_cascade_without_layout_boxes() {
        let mut dom = Dom::new();
        let html = dom.create_element(NodeIdx(0), "html");
        let body = dom.create_element(html, "body");
        let absent = dom.create_element(body, "div");
        let hidden = dom.create_element(body, "aside");
        let sheets = [parse_stylesheet("body {position:relative;color:red;width:100px;--tone:green} aside {display:none} aside::before, aside::after {content:'Foo';color:var(--tone)}")];
        let generated = compute_document_with_font_metrics(&dom, NodeIdx(0), &sheets,
            &MediaEnvironment::default(), None, &Metrics);
        for pseudo in [PseudoElement::Before, PseudoElement::After] {
            for node in [absent, hidden] { assert!(generated.get_pseudo(node, pseudo).is_none()); }
            let style = query(&dom, NodeIdx(0), absent, pseudo, &sheets);
            assert_eq!(style.get("position"), Some("static"));
            assert_eq!(style.get("width"), Some("auto"));
            assert_eq!(style.get("color"), Some("red"));
            let hidden_style = query(&dom, NodeIdx(0), hidden, pseudo, &sheets);
            assert_eq!(hidden_style.get("content"), Some("\"Foo\""));
            assert_eq!(hidden_style.get("color"), Some("green"));
        }
    }

    #[test]
    fn pseudo_display_context_and_recognized_non_generated_families() {
        let mut dom = Dom::new();
        let origin = dom.create_element(NodeIdx(0), "li");
        let sheets = [parse_stylesheet("li {display:flex;color:red} li::after {content:'foo';display:contents;position:absolute} li::file-selector-button,li::placeholder,li::backdrop,li::marker,li::first-line,li::first-letter,li::selection {color:green}")];
        assert_eq!(query(&dom, NodeIdx(0), origin, PseudoElement::Before, &sheets).get("display"), Some("block"));
        let after = query(&dom, NodeIdx(0), origin, PseudoElement::After, &sheets);
        assert_eq!(after.get("display"), Some("contents"));
        assert_eq!(after.get("position"), Some("absolute"));
        for pseudo in [PseudoElement::FileSelectorButton, PseudoElement::Placeholder,
            PseudoElement::Backdrop, PseudoElement::Marker, PseudoElement::FirstLine,
            PseudoElement::FirstLetter, PseudoElement::Selection] {
            assert_eq!(query(&dom, NodeIdx(0), origin, pseudo, &sheets).get("color"), Some("green"));
        }
        assert_eq!(query(&dom, NodeIdx(0), origin, PseudoElement::Backdrop, &sheets).get("position"), Some("fixed"));
    }

    #[test]
    fn native_document_membership_root_units_and_preinheritance_font_context() {
        let mut dom = Dom::new();
        let main = dom.create_element(NodeIdx(0), "html");
        let other = dom.append_child(NodeIdx(0), amazing_dom::NodeKind::Document);
        let root = dom.create_element(other, "html");
        let node = dom.create_element(root, "div");
        let sheets = [parse_stylesheet("html {font-size:30px;font-size-adjust:from-font} div::before {font-size:2rem;font-size-adjust:from-font} div::after {font-size-adjust:inherit}")];
        let before = query(&dom, other, node, PseudoElement::Before, &sheets);
        assert_eq!(before.get("font-size"), Some("60px"));
        assert_eq!(before.get("font-size-adjust"), Some("0.7"));
        assert_eq!(query(&dom, other, node, PseudoElement::After, &sheets).get("font-size-adjust"), Some("0.5"));
        for (document, target) in [(other, main), (NodeIdx(0), node), (main, node), (other, NodeIdx(u32::MAX)), (other, other)] {
            assert!(compute_document_pseudo_with_font_metrics(&dom, document, target,
                PseudoElement::Before, &sheets, &MediaEnvironment::default(), None, &Metrics).is_none());
        }
    }
}
