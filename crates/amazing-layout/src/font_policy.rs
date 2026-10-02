//! Used font normalization and synthetic-face policy at text consumers.
use amazing_css::{ComputedStyle, font_size_adjust::{FontSizeAdjust, FontSizeAdjustAspect, FontSizeAdjustMetric as CssMetric}};
use amazing_text::{FontSizeAdjustMetric, FontSynthesis, TextStyle, TextSystem};

/// CSS has already computed explicit numeric expressions. Actual-font and
/// contextual math values must be resolved before inheritance by the CSS/font
/// computation seam; do not invent their ratios independently per descendant.
pub(crate) fn size_adjust_request(style: &ComputedStyle) -> Option<(FontSizeAdjustMetric, f32)> {
    let value = match style.typed("font-size-adjust") {
        Some(amazing_css::values::ComputedValue::FontSizeAdjust(value)) => value.clone(),
        _ => FontSizeAdjust::parse(style.get("font-size-adjust")?)?,
    };
    let FontSizeAdjust::Value { metric, aspect: FontSizeAdjustAspect::Number(aspect) } = value else {
        return None;
    };
    let aspect = aspect as f32;
    if !aspect.is_finite() || aspect < 0.0 { return None; }
    let metric = match metric {
        CssMetric::ExHeight => FontSizeAdjustMetric::ExHeight,
        CssMetric::CapHeight => FontSizeAdjustMetric::CapHeight,
        CssMetric::ChWidth => FontSizeAdjustMetric::ChWidth,
        CssMetric::IcWidth => FontSizeAdjustMetric::IcWidth,
        CssMetric::IcHeight => FontSizeAdjustMetric::IcHeight,
    };
    Some((metric, aspect))
}

/// Oblique-only permits synthetic oblique requests, while an italic request
/// still selects a real italic face but cannot substitute synthetic oblique.
pub(crate) fn synthesis(style: &ComputedStyle) -> FontSynthesis {
    let enabled = |property| style.get(property).unwrap_or("auto") != "none";
    FontSynthesis {
        weight: enabled("font-synthesis-weight"),
        style: enabled("font-synthesis-style")
            && !(style.get("font-synthesis-style") == Some("oblique-only")
                && style.get("font-style") == Some("italic")),
        small_caps: enabled("font-synthesis-small-caps"),
        position: enabled("font-synthesis-position"),
    }
}

/// Preserve computed CSS font size and spacing; only glyph/metric selection
/// receives the provider's normalized used size. The same seam serves ordinary,
/// combined, ruby base/annotation and generated text.
pub(crate) fn adjusted_style(
    style: &TextStyle,
    request: Option<(FontSizeAdjustMetric, f32)>,
    system: &dyn TextSystem,
) -> Option<TextStyle> {
    let (metric, aspect) = request?;
    let size = system.adjusted_font_size(style, metric, aspect);
    if !size.is_finite() || size < 0.0 { return None; }
    let mut adjusted = style.clone();
    adjusted.size = size;
    Some(adjusted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Flow, LayoutBox, NoReplacedElements};
    use amazing_text::{FallbackTextSystem, FontMetrics, GlyphImage, GlyphRasterRequest,
        LineRequest, Paragraph, ShapedLine, ShapedParagraph, TextError};
    use std::cell::RefCell;

    /// Fixed font metric table makes each accidentally selected axis produce a
    /// distinct glyph size. The fallback shaper then measures actual spans.
    struct MetricFont {
        fallback: FallbackTextSystem,
        requests: RefCell<Vec<(FontSizeAdjustMetric, f32)>>,
        shaped: RefCell<Vec<TextStyle>>,
    }
    impl MetricFont {
        fn new() -> Self {
            Self { fallback: FallbackTextSystem::new(), requests: RefCell::new(Vec::new()),
                shaped: RefCell::new(Vec::new()) }
        }
    }
    impl TextSystem for MetricFont {
        fn generation(&self) -> u64 { self.fallback.generation() }
        fn metrics(&self, style: &TextStyle) -> Result<FontMetrics, TextError> {
            self.fallback.metrics(style)
        }
        fn adjusted_font_size(&self, style: &TextStyle, metric: FontSizeAdjustMetric, aspect: f32) -> f32 {
            self.requests.borrow_mut().push((metric, aspect));
            let ratio = match metric {
                FontSizeAdjustMetric::ExHeight => 0.5,
                FontSizeAdjustMetric::CapHeight => 0.8,
                FontSizeAdjustMetric::ChWidth => 0.6,
                FontSizeAdjustMetric::IcWidth => 1.0,
                FontSizeAdjustMetric::IcHeight => 1.2,
            };
            style.size * aspect / ratio
        }
        fn shape(&self, paragraph: Paragraph<'_>) -> Result<ShapedParagraph, TextError> {
            self.shaped.borrow_mut().extend(paragraph.spans.iter().map(|span| span.style.clone()));
            self.fallback.shape(paragraph)
        }
        fn shape_line(&self, request: LineRequest<'_>) -> Result<ShapedLine, TextError> {
            self.shaped.borrow_mut().extend(request.paragraph.spans.iter().map(|span| span.style.clone()));
            self.fallback.shape_line(request)
        }
        fn rasterize(&self, request: GlyphRasterRequest<'_>) -> Result<Option<GlyphImage>, TextError> {
            self.fallback.rasterize(request)
        }
    }

    fn layout(html: &str, css: &str, font: &MetricFont) -> LayoutBox {
        let dom = amazing_html::parse(&format!("<body>{html}"));
        crate::layout_dom_document_with_text_and_replaced_in_viewport(
            &dom, &format!("body{{margin:0;font:20px/20px serif}}{css}"),
            1000.0, Some(1000.0), font, &NoReplacedElements,
        )
    }
    fn glyph_sizes(box_: &LayoutBox) -> Vec<f32> {
        let mut sizes: Vec<_> = box_.lines.iter().flat_map(|line| &line.glyphs).map(|glyph| glyph.font_size).collect();
        if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
            for child in children { sizes.extend(glyph_sizes(child)); }
        }
        sizes
    }
    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn every_explicit_metric_normalizes_shaped_glyphs_without_changing_css_size() {
        for (metric, ratio) in [("ex-height", 0.5), ("cap-height", 0.8),
            ("ch-width", 0.6), ("ic-width", 1.0), ("ic-height", 1.2)] {
            for aspect in ["2", "calc(1 + 1)", "var(--aspect)"] {
                let font = MetricFont::new();
                let css = format!("p{{--aspect:2;font-size-adjust:{metric} {aspect};letter-spacing:1px}}");
                let root = layout("<p>A</p>", &css, &font);
                let Flow::Blocks(children) = &root.flow else { panic!("blocks") };
                close(children[0].font_size, 20.0);
                let sizes = glyph_sizes(&root);
                assert!(!sizes.is_empty(), "{metric};{aspect}");
                for size in sizes { close(size, 40.0 / ratio); }
                assert!(font.requests.borrow().iter().all(|(_, aspect)| *aspect == 2.0));
            }
        }
    }

    #[test]
    fn inherited_explicit_metric_and_generated_ruby_annotations_use_their_own_css_sizes() {
        let font = MetricFont::new();
        let root = layout("<p><ruby><rb>A</rb><rt>B</rt></ruby></p>",
            "p{font-size-adjust:cap-height 2}rt{font-size:10px;font-size-adjust:ic-height 1.2}ruby::before{content:'C';font-size:30px;font-size-adjust:ch-width .6}", &font);
        let sizes = glyph_sizes(&root);
        assert!(sizes.iter().any(|size| (*size - 50.0).abs() < 0.01), "base: {sizes:?}");
        assert!(sizes.iter().any(|size| (*size - 10.0).abs() < 0.01), "annotation: {sizes:?}");
        assert!(sizes.iter().any(|size| (*size - 30.0).abs() < 0.01), "generated: {sizes:?}");
        let requests = font.requests.borrow();
        assert!(requests.iter().any(|(metric, _)| *metric == FontSizeAdjustMetric::CapHeight));
        assert!(requests.iter().any(|(metric, _)| *metric == FontSizeAdjustMetric::IcHeight));
        assert!(requests.iter().any(|(metric, _)| *metric == FontSizeAdjustMetric::ChWidth));
    }

    #[test]
    fn generated_and_vertical_combined_text_receive_explicit_normalization() {
        for css in ["p::before{content:'A';font-size-adjust:cap-height 2}",
            "p{writing-mode:vertical-rl;text-combine-upright:all;font-size-adjust:cap-height 2}"] {
            let font = MetricFont::new();
            let _root = layout("<p>12</p>", css, &font);
            assert!(font.shaped.borrow().iter().any(|style| (style.size - 50.0).abs() < 0.01), "{css}");
            assert!(font.requests.borrow().iter().any(|(metric, aspect)|
                *metric == FontSizeAdjustMetric::CapHeight && *aspect == 2.0));
        }
    }

    #[test]
    fn synthesis_style_policies_preserve_requested_real_face_and_independent_axes() {
        for slant in ["normal", "italic", "oblique", "oblique 12deg"] {
            for policy in ["auto", "none", "oblique-only"] {
                let font = MetricFont::new();
                let css = format!("p{{font-style:{slant};font-synthesis-style:{policy};font-synthesis-weight:none;font-synthesis-small-caps:none;font-synthesis-position:none}}");
                let _root = layout("<p>A<ruby>B<rt>C</rt></ruby></p>", &css, &font);
                let expected = policy != "none" && !(policy == "oblique-only" && slant == "italic");
                let shaped = font.shaped.borrow();
                assert!(!shaped.is_empty(), "{slant};{policy}");
                for style in shaped.iter() {
                    assert_eq!(style.synthesis.style, expected, "{slant};{policy}");
                    assert!(!style.synthesis.weight && !style.synthesis.small_caps && !style.synthesis.position);
                    if slant == "italic" { assert_eq!(style.slant, amazing_text::FontSlant::Italic); }
                    if slant.starts_with("oblique") { assert_eq!(style.slant, amazing_text::FontSlant::Oblique); }
                }
            }
        }
    }
}
