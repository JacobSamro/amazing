//! Horizontal composition inside one vertical em, with font width variants.
use super::*;
use std::collections::BTreeMap;

fn signatures(glyphs: &[GlyphFrag]) -> BTreeMap<(usize, usize), Vec<(u64, u32, u32)>> {
    let mut out = BTreeMap::new();
    for glyph in glyphs {
        out.entry((glyph.cluster.start, glyph.cluster.end))
            .or_insert_with(Vec::new)
            .push((glyph.font.0, glyph.glyph.0, glyph.advance_x.to_bits()));
    }
    out
}

fn narrow_fullwidth(
    text: &str,
    mappings: &[crate::TextSourceMapping],
) -> Option<(String, Vec<crate::TextSourceMapping>)> {
    let mut out = String::with_capacity(text.len());
    let mut mapped = Vec::new();
    let mut first = 0;
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        let narrowed = match ch {
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(ch as u32 - 0xfee0).unwrap(),
            '\u{3000}' => ' ',
            '\u{ffe0}' => '¢',
            '\u{ffe1}' => '£',
            '\u{ffe2}' => '¬',
            '\u{ffe3}' => '¯',
            '\u{ffe4}' => '¦',
            '\u{ffe5}' => '¥',
            '\u{ffe6}' => '₩',
            _ => ch,
        };
        let output_start = out.len();
        out.push(narrowed);
        while first < mappings.len() && mappings[first].text_range.end <= start {
            first += 1;
        }
        for mapping in &mappings[first..] {
            if mapping.text_range.start >= end {
                break;
            }
            if mapped.len() >= 262_144 {
                return None;
            }
            let source_range = if mapping.text_range.len() == mapping.source_range.len() {
                mapping.source_range.start + start.max(mapping.text_range.start)
                    - mapping.text_range.start
                    ..mapping.source_range.start + end.min(mapping.text_range.end)
                        - mapping.text_range.start
            } else {
                mapping.source_range.clone()
            };
            mapped.push(crate::TextSourceMapping {
                text_range: output_start..out.len(),
                source: mapping.source,
                source_range,
            });
        }
    }
    Some((out, mapped))
}

pub(super) fn shape_combined_text(
    node: NodeIdx,
    text: &str,
    source_mappings: &[crate::TextSourceMapping],
    font_size: f32,
    style: &amazing_text::TextStyle,
    color: &str,
    text_system: &dyn TextSystem,
) -> Option<ShapedRuby> {
    let outer_writing_mode = style.writing_mode;
    let mut inner_style = style.clone();
    inner_style.writing_mode = amazing_text::WritingMode::HorizontalTb;
    inner_style.text_combine = amazing_text::TextCombine::None;
    let mut combined = crate::RubyText {
        node,
        raw_text: text.to_owned(),
        format: Default::default(),
        nested: Vec::new(),
        text: text.to_owned(),
        source_mappings: source_mappings.to_vec(),
        font_size,
        style: Box::new(inner_style),
        color: color.to_owned(),
    };
    let (mut glyphs, mut natural_width, metrics) = shape_ruby_text(&combined, 0.0, text_system)?;
    let mut clusters = glyphs
        .iter()
        .map(|g| (g.cluster.start, g.cluster.end))
        .collect::<Vec<_>>();
    clusters.sort_unstable();
    clusters.dedup();
    if clusters.len() > 1 {
        let (narrowed, mappings) = narrow_fullwidth(&combined.text, &combined.source_mappings)?;
        if narrowed != combined.text {
            combined.text = narrowed;
            combined.source_mappings = mappings;
            (glyphs, natural_width, _) = shape_ruby_text(&combined, 0.0, text_system)?;
        }
    }
    if natural_width > font_size {
        let normal = signatures(&glyphs);
        for tag in [*b"hwid", *b"twid", *b"qwid"] {
            let mut variant = combined.clone();
            variant
                .style
                .features
                .retain(|feature| !matches!(&feature.tag, b"hwid" | b"twid" | b"qwid"));
            variant
                .style
                .features
                .push(amazing_text::FeatureSetting { tag, value: 1 });
            if let Some((candidate, width, _)) = shape_ruby_text(&variant, 0.0, text_system) {
                let changed = signatures(&candidate);
                // Require a width variant for every shaped typographic unit;
                // partial font coverage must not distort only half a sequence.
                let complete = !normal.is_empty()
                    && normal.len() == changed.len()
                    && normal.iter().all(|(cluster, signature)| {
                        changed.get(cluster).is_some_and(|v| v != signature)
                    });
                if complete && width < natural_width {
                    glyphs = candidate;
                    natural_width = width;
                    if width <= font_size {
                        break;
                    }
                }
            }
        }
    }
    let scale = if natural_width > 0.0 {
        (font_size / natural_width).min(1.0)
    } else {
        1.0
    };
    let fitted_width = natural_width * scale;
    let baseline = (font_size - metrics.line_height()) * 0.5 + metrics.ascent;
    for glyph in &mut glyphs {
        glyph.x = -fitted_width * 0.5 + glyph.x * scale;
        glyph.y = baseline;
        glyph.writing_mode = outer_writing_mode;
        glyph.orientation = amazing_text::GlyphOrientation::Upright;
        glyph.scale_x = scale;
    }
    Some(ShapedRuby {
        paragraph_offset: 0,
        node,
        text: text.to_owned(),
        width: font_size,
        base_width: font_size,
        overhang_spaces: false,
        font_size,
        glyphs,
        line_over: 0.0,
        line_under: 0.0,
        break_after: true,
        space_before: 0.0,
        preserved_space_before: 0.0,
    })
}
