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
}
