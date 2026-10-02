//! CSS Values 4 font-relative measures from the first available font.

use super::{FontRecord, cluster_orientation, font_covers, font_metrics, shape_range};
use amazing_text::{FontRelativeMetrics, GlyphOrientation, ShapingBoundary, TextSpan, TextStyle};

pub(super) fn for_font(font: &FontRecord, style: &TextStyle) -> FontRelativeMetrics {
    let mut result = FontRelativeMetrics::fallback(style);
    if let Ok(metrics) = font_metrics(font, style.size) {
        result = result.with_font_metrics(metrics);
    }
    if let Some(advance) = glyph_advance(font, style, '0') {
        result.ch_advance = advance;
    }
    if let Some(advance) = glyph_advance(font, style, '水') {
        result.ic_advance = advance;
    }
    result
}

/// CSS Fonts normalization uses physical glyph advances, not `ch`/`ic`
/// inline-axis units. Font lookup has already selected the first actual font.
pub(super) fn aspect_ratio(font: &FontRecord, style: &TextStyle, metric: amazing_text::FontSizeAdjustMetric) -> Option<f32> {
    use amazing_text::{Direction, FontSizeAdjustMetric, TextOrientation, WritingMode};
    if !style.size.is_finite() || style.size < 0.0 { return None; }
    let mut physical = style.clone();
    // A zero CSS size still has a font-design aspect ratio; probe at one px.
    if physical.size == 0.0 { physical.size = 1.0; }
    let size = physical.size;
    physical.direction = Direction::LeftToRight;
    physical.writing_mode = WritingMode::HorizontalTb;
    physical.text_orientation = TextOrientation::Upright;
    let px = match metric {
        FontSizeAdjustMetric::ExHeight => for_font(font, &physical).x_height,
        FontSizeAdjustMetric::CapHeight => for_font(font, &physical).cap_height,
        FontSizeAdjustMetric::ChWidth => glyph_advance(font, &physical, '0').unwrap_or(size * 0.5),
        FontSizeAdjustMetric::IcWidth => glyph_advance(font, &physical, '水').unwrap_or(size),
        FontSizeAdjustMetric::IcHeight => {
            physical.writing_mode = WritingMode::VerticalRl;
            glyph_advance(font, &physical, '水').unwrap_or(size)
        }
    };
    let ratio = px / size;
    (ratio.is_finite() && ratio >= 0.0).then_some(ratio)
}

fn glyph_advance(font: &FontRecord, style: &TextStyle, character: char) -> Option<f32> {
    let text = character.to_string();
    if !font_covers(font, &text) {
        return None;
    }
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 0,
        style: style.clone(),
        boundary_before: ShapingBoundary::Continue,
    }];
    let orientation = cluster_orientation(style, &text, 0..text.len());
    let runs = shape_range(
        font,
        &text,
        &spans,
        0..text.len(),
        0..text.len(),
        style.direction,
        orientation,
    )
    .ok()?;
    let vertical_advance =
        style.writing_mode.uses_vertical_metrics() && orientation == GlyphOrientation::Upright;
    let advance = runs
        .iter()
        .flat_map(|run| &run.glyphs)
        .map(|glyph| {
            if vertical_advance {
                glyph.advance_y
            } else {
                glyph.advance_x
            }
        })
        .sum::<f32>()
        .abs();
    (advance.is_finite() && advance > 0.0).then_some(advance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::synthetic_font_with_metrics;
    use crate::{FontFormat, FontSlant, TextProvider, WebFont};
    use amazing_text::{FontSizeAdjustMetric, TextSystem};

    #[test]
    fn zero_advance_uses_selected_font_glyph() {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Metric Zero".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font_with_metrics('0', None, 800),
            })
            .unwrap();
        let style = TextStyle {
            size: 16.0,
            ..TextStyle::default()
        };
        assert!((glyph_advance(&provider.fonts[0], &style, '0').unwrap() - 8.0).abs() < 0.02);
        assert!(glyph_advance(&provider.fonts[0], &style, '水').is_none());
    }

    fn adjust_provider() -> TextProvider {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Adjust".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font_with_metrics(' ', None, 800),
            })
            .unwrap();
        provider
    }

    fn adjust_style() -> TextStyle {
        TextStyle {
            families: vec!["Adjust".into()],
            size: 16.0,
            ..TextStyle::default()
        }
    }

    #[test]
    fn normalization_metrics_are_physical_in_every_paragraph_mode() {
        use amazing_text::{TextOrientation, WritingMode};
        let mut provider = TextProvider::new();
        provider.register_web_font(WebFont { family: "Physical".into(), weight: 400, stretch: 100, slant: FontSlant::Normal, format: FontFormat::OpenType, bytes: crate::tests::synthetic_font_with_case_pair(' ', '水') }).unwrap();
        let mut style = TextStyle { families: vec!["Physical".into()], size: 20.0, ..TextStyle::default() };
        let width = provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcWidth).unwrap();
        let height = provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcHeight).unwrap();
        assert!((width - 0.5).abs() < 0.01, "{width}");
        assert!((height - 1.0).abs() < 0.01, "{height}");
        for mode in [WritingMode::HorizontalTb, WritingMode::VerticalRl, WritingMode::VerticalLr, WritingMode::SidewaysRl, WritingMode::SidewaysLr] {
            for orientation in [TextOrientation::Mixed, TextOrientation::Upright, TextOrientation::Sideways] {
                style.writing_mode = mode; style.text_orientation = orientation;
                assert_eq!(provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcWidth), Some(width));
                assert_eq!(provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcHeight), Some(height));
                assert!((provider.adjusted_font_size(&style, FontSizeAdjustMetric::IcWidth, width) - 20.0).abs() < 0.05);
                assert!((provider.adjusted_font_size(&style, FontSizeAdjustMetric::IcHeight, 0.6) - 12.0).abs() < 0.05);
            }
        }
        let mut zero = TextProvider::new();
        zero.register_web_font(WebFont { family: "Physical".into(), weight: 400, stretch: 100, slant: FontSlant::Normal, format: FontFormat::OpenType, bytes: crate::tests::synthetic_font_with_case_pair(' ', '0') }).unwrap();
        assert!((zero.font_size_adjust_ratio(&style, FontSizeAdjustMetric::ChWidth).unwrap() - 0.5).abs() < 0.01);
        assert_eq!(TextProvider::new().font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcHeight), None);
        style.size = 0.0;
        assert_eq!(provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcWidth), Some(width));
        assert_eq!(provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcHeight), Some(height));
        style.size = f32::NAN;
        assert_eq!(provider.font_size_adjust_ratio(&style, FontSizeAdjustMetric::IcHeight), None);
    }

    #[test]
    fn font_size_adjust_scales_the_used_size_by_the_metric_ratio() {
        let provider = adjust_provider();
        let style = adjust_style();
        let metrics = provider.font_relative_metrics(&style);
        let ex_ratio = metrics.x_height / style.size;
        let cap_ratio = metrics.cap_height / style.size;
        assert!(ex_ratio > 0.0 && cap_ratio > ex_ratio);

        let unchanged = provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, ex_ratio);
        assert!((unchanged - style.size).abs() < 0.01, "{unchanged}");
        let doubled = provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, ex_ratio * 2.0);
        assert!((doubled - style.size * 2.0).abs() < 0.05, "{doubled}");

        // The metric argument selects the basis: cap-height differs from ex-height.
        let cross = provider.adjusted_font_size(&style, FontSizeAdjustMetric::CapHeight, cap_ratio);
        assert!((cross - style.size).abs() < 0.01, "{cross}");
        let ex_with_cap = provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, cap_ratio);
        assert!((ex_with_cap - style.size * cap_ratio / ex_ratio).abs() < 0.05, "{ex_with_cap}");
        assert!(ex_with_cap > style.size);
    }

    #[test]
    fn font_size_adjust_zero_negative_and_missing_fonts_are_bounded() {
        let provider = adjust_provider();
        let style = adjust_style();
        assert_eq!(provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, 0.0), 0.0);
        assert_eq!(provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, -1.0), style.size);
        assert_eq!(
            provider.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, f32::NAN),
            style.size
        );
        let empty = TextProvider::new();
        assert_eq!(
            empty.adjusted_font_size(&style, FontSizeAdjustMetric::ExHeight, 1.0),
            style.size
        );
    }
}
