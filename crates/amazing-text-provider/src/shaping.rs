//! Shaping glue: the low-level helpers `shape_runs` (in `lib.rs`) calls once
//! it has an itemized, single-font run to actually hand to HarfRust, plus
//! font metrics, glyph rasterization, and the small per-cluster utilities
//! (orientation, representative character, cluster-end mapping, run
//! reordering) they share with each other and with `shape_line`. Split out
//! of `lib.rs` (TEXT-27).

use super::font_selection::is_default_ignorable;
use super::{FontRecord, MAX_GLYPH_BYTES, MAX_GLYPH_DIMENSION, font_variant_synthesis, font_variations};
use amazing_text::{
    Direction, FallbackTextSystem, FontAttributes, FontMetrics, FontSynthesis, GlyphContent,
    GlyphFlags, GlyphId, GlyphImage, GlyphOrientation, GlyphRasterRequest, Paragraph, ShapedGlyph,
    ShapedRun, ShapingBoundary, TextError, TextOrientation, TextSpan, TextStyle, TextSystem,
    WritingMode, resolve_synthesis,
};
use harfrust::{
    BufferFlags, Feature, FontRef as HarfFontRef, ShapeOptions, ShaperData, ShaperInstance, Tag,
    UnicodeBuffer, Variation,
};
// Only for the `cmap()` accessor the small-caps synthesis fallback uses to
// map an originally-lowercase letter to its uppercase glyph; see
// `font_variant_synthesis`'s module comment for why this crate is pinned
// explicitly rather than left as a transitive-only dependency of harfrust.
use read_fonts::TableProvider;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use swash::scale::image::Content;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::{FontRef as SwashFontRef, zeno};
use unicode_vo::Orientation as VerticalOrientation;

pub(super) fn runs_in_visual_cluster_order(
    runs: &[ShapedRun],
    visual_clusters: &[std::ops::Range<usize>],
) -> Vec<ShapedRun> {
    let mut by_cluster: HashMap<usize, Vec<ShapedRun>> = HashMap::new();
    for run in runs {
        let mut glyphs_by_cluster: HashMap<usize, Vec<ShapedGlyph>> = HashMap::new();
        for glyph in &run.glyphs {
            glyphs_by_cluster
                .entry(glyph.cluster.start)
                .or_default()
                .push(glyph.clone());
        }
        for (cluster_start, glyphs) in glyphs_by_cluster {
            let text_range = glyphs
                .iter()
                .map(|glyph| glyph.cluster.clone())
                .reduce(|left, right| left.start.min(right.start)..left.end.max(right.end))
                .unwrap_or(cluster_start..cluster_start);
            let visual_run = clone_run_with_glyphs(run, text_range, glyphs);
            by_cluster
                .entry(cluster_start)
                .or_default()
                .push(visual_run);
        }
    }
    let mut visual_runs = Vec::new();
    for cluster in visual_clusters {
        if let Some(mut runs) = by_cluster.remove(&cluster.start) {
            visual_runs.append(&mut runs);
        }
    }
    visual_runs
}

fn clone_run_with_glyphs(
    run: &ShapedRun,
    text_range: std::ops::Range<usize>,
    glyphs: Vec<ShapedGlyph>,
) -> ShapedRun {
    ShapedRun {
        text_range,
        source: run.source,
        font: run.font,
        font_size: run.font_size,
        direction: run.direction,
        writing_mode: run.writing_mode,
        metrics: run.metrics,
        variations: run.variations.clone(),
        synthesis: run.synthesis,
        glyphs,
    }
}

pub(super) fn font_metrics(font: &FontRecord, size: f32) -> Result<FontMetrics, TextError> {
    if !size.is_finite() || size <= 0.0 || size > 4096.0 {
        return Err(TextError::InvalidStyle);
    }
    catch_unwind(AssertUnwindSafe(|| {
        let face = SwashFontRef::from_index(&font.sfnt, 0).ok_or(TextError::MalformedFont)?;
        let face_rule = font.author_face.as_ref();
        let adjusted_size = size * face_rule.map_or(1.0, |rule| rule.size_adjust);
        let metrics = face.metrics(&[]).scale(adjusted_size);
        let ascent = face_rule
            .and_then(|rule| rule.ascent_override)
            .map_or(metrics.ascent.max(0.0), |value| value * adjusted_size);
        let descent = face_rule
            .and_then(|rule| rule.descent_override)
            .map_or(metrics.descent.abs(), |value| value * adjusted_size);
        let leading = face_rule
            .and_then(|rule| rule.line_gap_override)
            .map_or(metrics.leading.max(0.0), |value| value * adjusted_size);
        Ok(FontMetrics {
            ascent,
            descent,
            leading,
            x_height: metrics.x_height.max(0.0),
            cap_height: metrics.cap_height.max(0.0),
            vertical_ascent: metrics.vertical_ascent.max(0.0),
            vertical_descent: metrics.vertical_descent.abs(),
            vertical_leading: metrics.vertical_leading.max(0.0),
            has_canonical_vertical_metrics: metrics.has_vertical_metrics,
        })
    }))
    .map_err(|_| TextError::MalformedFont)?
}

pub(super) fn fallback_grapheme_run(
    fallback: &FallbackTextSystem,
    text: &str,
    cluster: std::ops::Range<usize>,
    span: &TextSpan,
) -> Result<ShapedRun, TextError> {
    let first = text[cluster.clone()]
        .chars()
        .next()
        .ok_or(TextError::InvalidParagraph)?;
    let first = first.to_string();
    let local_spans = [TextSpan {
        range: 0..first.len(),
        source: span.source,
        style: span.style.clone(),
        boundary_before: span.boundary_before,
    }];
    let mut shaped = fallback.shape(Paragraph {
        text: &first,
        spans: &local_spans,
        sources: &[],
        bidi_scopes: &[],
        base_direction: span.style.direction,
    })?;
    let mut run = shaped.runs.pop().ok_or(TextError::InvalidParagraph)?;
    run.text_range = cluster.clone();
    for glyph in &mut run.glyphs {
        glyph.cluster = cluster.clone();
    }
    Ok(run)
}

pub(super) fn hard_context_range(
    spans: &[TextSpan],
    start: usize,
    end: usize,
    text_len: usize,
) -> std::ops::Range<usize> {
    let mut context_start = 0;
    for span in spans[..=start].iter().rev() {
        if span.boundary_before == ShapingBoundary::Hard {
            context_start = span.range.start;
            break;
        }
    }
    let mut context_end = text_len;
    for span in &spans[end..] {
        if span.boundary_before == ShapingBoundary::Hard {
            context_end = span.range.start;
            break;
        }
    }
    context_start..context_end
}

pub(super) fn shape_range(
    font: &FontRecord,
    text: &str,
    spans: &[TextSpan],
    target: std::ops::Range<usize>,
    context: std::ops::Range<usize>,
    direction: Direction,
    orientation: GlyphOrientation,
) -> Result<Vec<ShapedRun>, TextError> {
    let style = &spans[0].style;
    let metrics = font_metrics(font, style.size)?;
    catch_unwind(AssertUnwindSafe(|| {
        // `from_index(_, 0)` accepts both a lone sfnt and a collection's
        // first font (`FontFormat::Collection`, registered and validated as
        // the collection's font 0 in `validate_font_structure`).
        let face = HarfFontRef::from_index(&font.sfnt, 0).map_err(|_| TextError::MalformedFont)?;
        let shaper_data = ShaperData::new(&face);
        let resolved_variations = font_variations::for_face(font, style);
        let variations = resolved_variations
            .iter()
            .map(|variation| Variation {
                tag: Tag::new(&variation.tag),
                value: variation.value,
            })
            .collect::<Vec<_>>();
        let instance = ShaperInstance::from_variations(&face, &variations);
        let shaper = shaper_data.shaper(&face).instance(Some(&instance)).build();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&text[target.clone()]);
        if target.start > context.start {
            buffer.set_pre_context(&text[context.start..target.start]);
        }
        if target.end < context.end {
            buffer.set_post_context(&text[target.end..context.end]);
        }
        buffer.set_direction(
            if style.writing_mode.uses_vertical_metrics()
                && orientation == GlyphOrientation::Upright
            {
                harfrust::Direction::TopToBottom
            } else {
                match direction {
                    Direction::LeftToRight => harfrust::Direction::LeftToRight,
                    Direction::RightToLeft => harfrust::Direction::RightToLeft,
                }
            },
        );
        if let Some(language) = style
            .language
            .as_deref()
            .and_then(|value| value.parse().ok())
        {
            buffer.set_language(language);
        }
        buffer.guess_segment_properties();
        let mut flags = BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        if target.start == context.start {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if target.end == context.end {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        let adjusted_size = style.size
            * font
                .author_face
                .as_ref()
                .map_or(1.0, |face| face.size_adjust);
        let scale = (adjusted_size * 64.0).round().clamp(1.0, i32::MAX as f32) as i32;
        let mut features = Vec::with_capacity(style.features.len() + 2);
        if style.writing_mode.uses_vertical_metrics() {
            let upright = orientation == GlyphOrientation::Upright;
            features.push(Feature::new(Tag::new(b"vert"), u32::from(upright), ..));
            // CSS selects mixed glyph orientation through UAX50 rather than
            // delegating it to the font's vrt2 feature.
            features.push(Feature::new(Tag::new(b"vrt2"), 0, ..));
        }
        // CSS Fonts 4 §5.2.1: `kern` is the horizontal-axis feature and `vkrn`
        // the vertical-axis feature. HarfBuzz applies an explicitly requested
        // tag in either buffer direction, so a `font-kerning:normal` face that
        // enables both would kern on the wrong axis. Keep only the tag that
        // matches the shaping direction (verified with the pinned
        // FontWithFancyFeatures fixture: explicit `vkrn` changes horizontal
        // shaping and explicit `kern` changes vertical shaping).
        let vertical_buffer = style.writing_mode.uses_vertical_metrics()
            && orientation == GlyphOrientation::Upright;
        let kerning_tag_matches = |tag: &[u8; 4]| {
            if tag == b"vkrn" {
                vertical_buffer
            } else if tag == b"kern" {
                !vertical_buffer
            } else {
                true
            }
        };
        if let Some(face) = font.author_face.as_ref() {
            features.extend(
                face.features
                    .iter()
                    .filter(|feature| kerning_tag_matches(&feature.tag))
                    .map(|feature| Feature::new(Tag::new(&feature.tag), feature.value, ..)),
            );
        }
        features.extend(
            style
                .features
                .iter()
                .filter(|feature| kerning_tag_matches(&feature.tag))
                .map(|feature| Feature::new(Tag::new(&feature.tag), feature.value, ..)),
        );
        // CSS Fonts 4 section 6 synthesis fallback: only probe/parse GSUB and
        // cmap once per shaping call (not once per glyph) for whichever of
        // smcp/c2sc/sups/subs are actually requested and allowed.
        let caps_position_support = font_variant_synthesis::FaceFeatureSupport::probe(&face);
        let needs_smcp = style.synthesis.small_caps
            && font_variant_synthesis::feature_on(&features, *b"smcp")
            && !caps_position_support.smcp;
        let needs_c2sc = style.synthesis.small_caps
            && font_variant_synthesis::feature_on(&features, *b"c2sc")
            && !caps_position_support.c2sc;
        let needs_sups = style.synthesis.position
            && font_variant_synthesis::feature_on(&features, *b"sups")
            && !caps_position_support.sups;
        let needs_subs = style.synthesis.position
            && font_variant_synthesis::feature_on(&features, *b"subs")
            && !caps_position_support.subs;
        let smcp_cmap = needs_smcp.then(|| face.cmap().ok()).flatten();
        let lookup_uppercase = |character: char| -> Option<GlyphId> {
            smcp_cmap
                .as_ref()
                .and_then(|cmap| cmap.map_codepoint(character))
                .map(|glyph| GlyphId(glyph.to_u32()))
        };
        let base_synthesis = resolve_synthesis(
            FontAttributes {
                weight: style.weight,
                stretch: style.stretch,
                slant: style.slant,
            },
            font.attributes_for(style),
            style.synthesis,
        );
        let output = shaper.shape(
            buffer,
            ShapeOptions::new().scale(Some(scale)).features(&features),
        );
        let cluster_ends = cluster_end_map(output.glyph_infos(), target.clone());
        let mut runs: Vec<ShapedRun> = Vec::new();
        for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
            let cluster_start = target.start + info.cluster as usize;
            let cluster_end = cluster_ends
                .iter()
                .find_map(|(start, end)| (*start == cluster_start).then_some(*end))
                .unwrap_or(target.end);
            let source_span = spans
                .iter()
                .find(|span| span.range.contains(&cluster_start))
                .unwrap_or(&spans[0]);
            let plan = font_variant_synthesis::for_cluster(
                needs_smcp,
                needs_c2sc,
                needs_sups,
                needs_subs,
                &lookup_uppercase,
                representative_char(text, cluster_start..cluster_end),
            );
            let glyph_synthesis = FontSynthesis {
                small_caps: plan.small_caps,
                position: plan.position,
                ..base_synthesis
            };
            let glyph = ShapedGlyph {
                id: plan.glyph_id.unwrap_or(GlyphId(info.glyph_id)),
                cluster: cluster_start..cluster_end,
                advance_x: position.x_advance as f32 / 64.0,
                advance_y: position.y_advance as f32 / 64.0,
                offset_x: position.x_offset as f32 / 64.0,
                offset_y: position.y_offset as f32 / 64.0 + plan.offset_em * adjusted_size,
                orientation,
                flags: GlyphFlags {
                    unsafe_to_break: info.unsafe_to_break(),
                    unsafe_to_concat: info.unsafe_to_concat(),
                },
            };
            if let Some(run) = runs.last_mut()
                && run.source == source_span.source
                && run.synthesis == glyph_synthesis
            {
                run.glyphs.push(glyph);
                run.text_range.start = run.text_range.start.min(cluster_start);
                run.text_range.end = run.text_range.end.max(cluster_end);
                continue;
            }
            runs.push(ShapedRun {
                text_range: cluster_start..cluster_end,
                source: source_span.source,
                font: font.key,
                font_size: adjusted_size,
                direction,
                writing_mode: style.writing_mode,
                metrics,
                variations: resolved_variations.clone(),
                synthesis: glyph_synthesis,
                glyphs: vec![glyph],
            });
        }
        Ok(runs)
    }))
    .map_err(|_| TextError::MalformedFont)?
}

pub(super) fn cluster_orientation(
    style: &TextStyle,
    text: &str,
    cluster: std::ops::Range<usize>,
) -> GlyphOrientation {
    match style.writing_mode {
        WritingMode::HorizontalTb => GlyphOrientation::Upright,
        WritingMode::SidewaysRl => GlyphOrientation::SidewaysRight,
        WritingMode::SidewaysLr => GlyphOrientation::SidewaysLeft,
        WritingMode::VerticalRl | WritingMode::VerticalLr => match style.text_orientation {
            TextOrientation::Upright => GlyphOrientation::Upright,
            TextOrientation::Sideways => GlyphOrientation::SidewaysRight,
            TextOrientation::Mixed => {
                match unicode_vo::char_orientation(representative_char(text, cluster)) {
                    VerticalOrientation::Upright
                    | VerticalOrientation::TransformedOrUpright
                    | VerticalOrientation::TransformedOrRotated => GlyphOrientation::Upright,
                    VerticalOrientation::Rotated => GlyphOrientation::SidewaysRight,
                }
            }
        },
    }
}

/// The first non-default-ignorable character of `cluster`, or its first
/// character, or U+FFFD if the range is somehow empty. Shared by vertical
/// orientation (per-cluster UAX50 lookup) and small-caps/position synthesis
/// (per-cluster case check), both of which only need one representative
/// character per shaped cluster.
fn representative_char(text: &str, cluster: std::ops::Range<usize>) -> char {
    let unit = &text[cluster];
    unit.chars()
        .find(|character| !is_default_ignorable(*character))
        .or_else(|| unit.chars().next())
        .unwrap_or('\u{fffd}')
}

fn cluster_end_map(
    infos: &[harfrust::GlyphInfo],
    target: std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    let mut starts = infos
        .iter()
        .map(|info| target.start + info.cluster as usize)
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| (*start, starts.get(index + 1).copied().unwrap_or(target.end)))
        .collect()
}

pub(super) fn rasterize_font_glyph(
    font: &FontRecord,
    request: &GlyphRasterRequest<'_>,
) -> Result<Option<GlyphImage>, TextError> {
    let key = request.key;
    let glyph_id = u16::try_from(key.glyph.0).map_err(|_| TextError::MalformedFont)?;
    catch_unwind(AssertUnwindSafe(|| {
        let face = SwashFontRef::from_index(&font.sfnt, 0).ok_or(TextError::MalformedFont)?;
        let mut context = ScaleContext::new();
        let variation_tags = request
            .variations
            .iter()
            .map(|variation| {
                (
                    String::from_utf8_lossy(&variation.tag).into_owned(),
                    variation.value,
                )
            })
            .collect::<Vec<_>>();
        let mut scaler = context
            .builder(face)
            .size(key.size())
            .hint(true)
            .variations(
                variation_tags
                    .iter()
                    .map(|(tag, value)| (tag.as_str(), *value)),
            )
            .build();
        let mut render = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ]);
        render.format(zeno::Format::Alpha).offset(zeno::Vector::new(
            key.subpixel_x_64 as f32 / 64.0,
            key.subpixel_y_64 as f32 / 64.0,
        ));
        if request.synthesis.weight {
            render.embolden(key.size() / 32.0);
        }
        // Style (oblique skew), small-caps, and position synthesis compose:
        // e.g. an italic face with a synthesized small-caps letter needs
        // both the skew and the scale. Starting from identity and folding in
        // only the active transforms keeps the style-only case byte-for-byte
        // identical to the single `skew` transform used before this compose
        // path existed.
        let mut transform = zeno::Transform::IDENTITY;
        let mut needs_transform = false;
        if request.synthesis.style {
            transform = transform.then(&zeno::Transform::skew(
                zeno::Angle::from_degrees(-14.0),
                zeno::Angle::ZERO,
            ));
            needs_transform = true;
        }
        if request.synthesis.small_caps {
            transform =
                transform.then_scale(font_variant_synthesis::SMALL_CAPS_SCALE, font_variant_synthesis::SMALL_CAPS_SCALE);
            needs_transform = true;
        }
        if request.synthesis.position {
            transform =
                transform.then_scale(font_variant_synthesis::POSITION_SCALE, font_variant_synthesis::POSITION_SCALE);
            needs_transform = true;
        }
        if needs_transform {
            render.transform(Some(transform));
        }
        let image = render.render(&mut scaler, glyph_id);
        let Some(image) = image else {
            return Ok(None);
        };
        if image.placement.width > MAX_GLYPH_DIMENSION
            || image.placement.height > MAX_GLYPH_DIMENSION
            || image.data.len() > MAX_GLYPH_BYTES
        {
            return Err(TextError::ResourceLimit);
        }
        let (content, data) = match image.content {
            Content::Mask => (GlyphContent::Mask, image.data),
            Content::Color => (GlyphContent::Color, image.data),
            Content::SubpixelMask => {
                let data = image
                    .data
                    .chunks_exact(3)
                    .map(|channels| channels[0].max(channels[1]).max(channels[2]))
                    .collect();
                (GlyphContent::Mask, data)
            }
        };
        let channels = if content == GlyphContent::Color { 4 } else { 1 };
        let expected = image
            .placement
            .width
            .checked_mul(image.placement.height)
            .and_then(|pixels| pixels.checked_mul(channels))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(TextError::ResourceLimit)?;
        if data.len() != expected {
            return Err(TextError::MalformedFont);
        }
        Ok(Some(GlyphImage {
            content,
            data,
            width: image.placement.width,
            height: image.placement.height,
            bearing_x: image.placement.left,
            bearing_y: image.placement.top,
        }))
    }))
    .map_err(|_| TextError::MalformedFont)?
}
