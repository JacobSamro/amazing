//! Actual first-font metrics shared by every driver style boundary.
use amazing_css::{AnimatedValues, ComputedStyle, MediaEnvironment, StyledDom, Stylesheet};
use amazing_css::font_metric_context::FontMetricContext;
use amazing_css::{font_size_adjust::FontSizeAdjustMetric, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_text_provider::TextProvider;

pub(crate) struct FontMetrics<'a> {
    pub(crate) dom: &'a Dom,
    pub(crate) provider: &'a TextProvider,
}
impl FontMetricContext for FontMetrics<'_> {
    fn aspect_ratio(&self, node: NodeIdx, _pseudo: Option<PseudoElement>, style: &ComputedStyle,
        metric: FontSizeAdjustMetric) -> Option<f64> {
        // CSS has resolved computed lengths already; normalization never feeds
        // its adjusted used size back into the first-font selection query.
        let size = amazing_layout::values::resolve_used_font_size(style, 16.0);
        let text = amazing_layout::font_metric_text_style(self.dom, node, style, size);
        let metric = match metric {
            FontSizeAdjustMetric::ExHeight => amazing_text::FontSizeAdjustMetric::ExHeight,
            FontSizeAdjustMetric::CapHeight => amazing_text::FontSizeAdjustMetric::CapHeight,
            FontSizeAdjustMetric::ChWidth => amazing_text::FontSizeAdjustMetric::ChWidth,
            FontSizeAdjustMetric::IcWidth => amazing_text::FontSizeAdjustMetric::IcWidth,
            FontSizeAdjustMetric::IcHeight => amazing_text::FontSizeAdjustMetric::IcHeight,
        };
        self.provider.font_size_adjust_ratio(&text, metric).map(f64::from)
    }
    fn contextual_length_px(&self, node: NodeIdx, pseudo: Option<PseudoElement>, style: &ComputedStyle,
        quantity: f64, unit: &str) -> Option<f64> {
        let size = f64::from(amazing_layout::values::resolve_used_font_size(style, 16.0));
        let metric = match unit {
            "px" => return Some(quantity),
            "em" => return Some(quantity * size),
            "ex" => FontSizeAdjustMetric::ExHeight,
            "cap" => FontSizeAdjustMetric::CapHeight,
            "ch" | "ic" => {
                // ch/ic follow the inline axis; normalization axes are physical.
                if style.get("writing-mode").is_some_and(|mode| mode != "horizontal-tb") {
                    return None;
                }
                if unit == "ch" { FontSizeAdjustMetric::ChWidth } else { FontSizeAdjustMetric::IcWidth }
            }
            _ => return None,
        };
        self.aspect_ratio(node, pseudo, style, metric).map(|ratio| quantity * size * ratio)
    }

}
pub(crate) fn compute(dom: &Dom, sheets: &[Stylesheet], environment: &MediaEnvironment,
    animated: Option<&AnimatedValues>, provider: &TextProvider) -> StyledDom {
    amazing_css::compute_with_font_metrics(dom, sheets, environment, animated,
        &FontMetrics { dom, provider })
}

/// Font source URLs come from the same scoped sheets used by CSS selection.
pub(crate) fn load(dom: &Dom, page: &str) -> (Vec<Stylesheet>, crate::page::PageText) {
    let mut sources = Vec::new();
    let _ = crate::page::collect_author_css(dom, NodeIdx(0), page, &mut sources);
    let fonts = sources.iter().map(|(source, url, _)| (source.clone(), url.clone())).collect::<Vec<_>>();
    let text = crate::page::load_page_text_from_sheets(&fonts, page);
    (sources.into_iter().map(|(_, _, sheet)| sheet).collect(), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_css::font_size_adjust::{FontSizeAdjust, FontSizeAdjustAspect};
    fn provider() -> TextProvider {
        let mut provider = TextProvider::new();
        provider.register_web_font(amazing_text_provider::WebFont {
            family: "Registered".into(), weight: 400, stretch: 100,
            slant: amazing_text::FontSlant::Normal,
            format: amazing_text_provider::FontFormat::OpenType,
            bytes: include_bytes!("../tests/fixtures/font-metrics-space.ttf").to_vec(),
        }).unwrap();
        provider
    }
    fn ratio(style: &ComputedStyle) -> f64 {
        match FontSizeAdjust::parse(style.get("font-size-adjust").unwrap()).unwrap() {
            FontSizeAdjust::Value { aspect: FontSizeAdjustAspect::Number(ratio), .. } => ratio,
            other => panic!("unresolved metric: {other:?}"),
        }
    }
    #[test]
    fn registered_first_font_resolves_all_axes_before_principal_and_pseudo_inheritance() {
        let provider = provider();
        let mut dom = Dom::new();
        let parent = dom.create_element(NodeIdx(0), "div");
        let child = dom.create_element(parent, "span");
        for axis in ["ex-height", "cap-height", "ch-width", "ic-width", "ic-height"] {
            let sheets = [amazing_css::parse_stylesheet(&format!("div {{font-family:Registered;font-size-adjust:{axis} from-font}} span {{font-family:Missing}} div::before {{content:'A';font-family:Missing}} div::after {{content:'A';font-family:Registered;font-size-adjust:{axis} from-font}}"))];
            for writing in ["horizontal-tb", "vertical-rl"] {
                dom.set_attribute(parent, "style", &format!("writing-mode:{writing};font-size:24px")).unwrap();
                let styles = compute(&dom, &sheets, &MediaEnvironment::default(), None, &provider);
                let expected = ratio(styles.get(parent).unwrap());
                assert!(expected.is_finite() && expected >= 0.0);
                assert_eq!(ratio(styles.get(child).unwrap()), expected);
                assert_eq!(ratio(styles.get_pseudo(parent, PseudoElement::Before).unwrap()), expected);
                assert_eq!(ratio(styles.get_pseudo(parent, PseudoElement::After).unwrap()), expected);
            }
        }
        let styles = compute(&dom, &[amazing_css::parse_stylesheet("div{font-size-adjust:from-font}")], &MediaEnvironment::default(), None, &TextProvider::new());
        assert_eq!(styles.get(parent).unwrap().get("font-size-adjust"), Some("none"));
    }
    #[test]
    fn registered_font_metrics_agree_across_animated_incremental_and_full_oracle() {
        let provider = provider();
        let mut dom = Dom::new();
        let parent = dom.create_element(NodeIdx(0), "div");
        let child = dom.create_element(parent, "span");
        let sheets = [amazing_css::parse_stylesheet("div{font-family:Registered;font-size-adjust:cap-height from-font}.changed{font-size-adjust:ic-height from-font} span{font-family:Missing}")];
        let environment = MediaEnvironment { viewport_width_px:64.0,viewport_height_px:64.0,..Default::default() };
        let mut coordinator = crate::render_oracle::RenderCoordinator::default();
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let first = coordinator.candidate_with_font_metrics(&dom, &sheets, (64,64), journal, &provider);
        coordinator.commit(&dom, &sheets, (64,64), &first.styles);
        dom.set_attribute(parent, "class", "changed").unwrap();
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let incremental = coordinator.candidate_with_font_metrics(&dom, &sheets, (64,64), journal, &provider);
        assert!(!incremental.full_fallback);
        let full = compute(&dom, &sheets, &environment, None, &provider);
        assert_eq!(crate::render_oracle::first_style_mismatch(&dom, &incremental.styles, &full), None);
        assert_eq!(ratio(full.get(parent).unwrap()), ratio(full.get(child).unwrap()));
        let mut animated = AnimatedValues::default();
        assert!(animated.insert(parent, "font-size-adjust", "ch-width from-font"));
        let sampled = compute(&dom, &sheets, &environment, Some(&animated), &provider);
        assert_eq!(ratio(sampled.get(parent).unwrap()), ratio(sampled.get(child).unwrap()));
        let mut scene = crate::animation_scene::AnimationScene::new();
        scene.frame_with_font_metrics(&dom, &sheets, &environment, &provider, 0.0).unwrap();
    }
    #[test]
    fn page_loading_preserves_multiple_scoped_sources_at_the_same_document_url() {
        let directory = std::env::temp_dir().join(format!("amazing-font-driver-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let font = directory.join("face.ttf");
        std::fs::write(&font, include_bytes!("../tests/fixtures/font-metrics-space.ttf")).unwrap();
        let page = directory.join("page.html").to_string_lossy().into_owned();
        let dom = amazing_html::parse("<section><style>div{color:red}</style><style>@font-face{font-family:Loaded;src:url(face.ttf)}div{font-family:Loaded;font-size-adjust:cap-height from-font}</style><div>A</div></section>");
        let (sheets, text) = load(&dom, &page);
        assert_eq!(sheets.len(), 2);
        assert!(sheets.iter().all(|sheet| sheet.base_url.as_deref() == Some(page.as_str())));
        assert_eq!(sheets[0].scope_root, sheets[1].scope_root);
        assert!(text.provider.font_face_registrations().iter().any(|face| face.family.eq_ignore_ascii_case("Loaded") && face.key.is_some()), "{:?}", text.warnings);
        let styles = compute(&dom, &sheets, &MediaEnvironment::default(), None, &text.provider);
        let node = (0..dom.node_count()).map(|index| NodeIdx(index as u32)).find(|node| dom.name(*node) == Some("div")).unwrap();
        assert!(ratio(styles.get(node).unwrap()).is_finite());
        std::fs::remove_dir_all(directory).unwrap();
    }

}
